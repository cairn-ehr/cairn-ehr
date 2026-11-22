//! The gateway's route to Postgres, and the owner verb that checks and records it (#909 slice B).
//!
//! WHY IT EXISTS. The gateway forwards an admitted window's bytes to Postgres over a loopback
//! route: TCP on 127.0.0.1 / ::1, or a Unix socket. SCRAM must still run end to end, so the
//! cluster's `pg_hba.conf` must not hand the gateway's own OS user a login by `peer`/`ident`
//! (the gateway would then be a way in with no password). `cairn-node gateway-setup` CHECKS the
//! rules once, records the route in `terminal_gateway` (db/084) and prints the node's fingerprint
//! for the terminal ceremony. It never edits, reloads or restarts Postgres: a refusal names the
//! offending `file:line` and the fix is the site's own, followed by its own reload.

use crate::identity::load_local;
use crate::window_access::{hba_file_is_stale, hba_verdict, read_hba_rules, HbaRule};
use anyhow::Context;
use std::net::SocketAddr;
use std::path::{Component, PathBuf};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_postgres::Client;

/// A byte stream to Postgres, whichever route it came by (what the gateway pipes into).
pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

/// Where the gateway forwards to. Loopback only: the LAN-facing door is the gateway itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upstream {
    Tcp(SocketAddr),
    Unix(PathBuf),
}

impl std::fmt::Display for Upstream {
    /// The same text [`parse`] accepts, so the recorded value reads back.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Upstream::Tcp(a) => write!(f, "{a}"),
            Upstream::Unix(p) => write!(f, "{}", p.display()),
        }
    }
}

/// Parse `127.0.0.1:5432`, `[::1]:5432` or `/dir/.s.PGSQL.5432`. A remote address, a relative path
/// or a file that is not a Postgres socket name is refused, so a typo cannot aim the gateway
/// at the LAN or at some other socket.
pub fn parse(s: &str) -> Result<Upstream, String> {
    if s.starts_with('/') {
        let p = PathBuf::from(s);
        // `..` could point the route anywhere; a trailing `/` makes the socket a directory.
        if s.ends_with('/') || p.components().any(|c| c == Component::ParentDir) {
            return Err(format!(
                "'{s}' must be a plain absolute path to the socket file (no '..', no trailing '/')"
            ));
        }
        let port = p
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix(".s.PGSQL."));
        return match port {
            Some(d) if !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()) => {
                Ok(Upstream::Unix(p))
            }
            _ => Err(format!(
                "'{s}' is not a Postgres socket: the file must be named .s.PGSQL.<port>"
            )),
        };
    }
    match s.parse::<SocketAddr>() {
        Ok(a) if a.ip().is_loopback() => Ok(Upstream::Tcp(a)),
        Ok(_) => Err(format!(
            "'{s}' is not a loopback address (127.0.0.1 or ::1)"
        )),
        Err(_) => Err(format!(
            "'{s}' is neither host:port on loopback (127.0.0.1:5432, [::1]:5432) nor an absolute \
             path to .s.PGSQL.<port>"
        )),
    }
}

impl Upstream {
    /// Open the route. (Covered end to end by the gateway's own tests.)
    pub async fn connect(&self) -> std::io::Result<Box<dyn AsyncReadWrite>> {
        match self {
            Upstream::Tcp(a) => Ok(Box::new(tokio::net::TcpStream::connect(a).await?)),
            #[cfg(unix)]
            Upstream::Unix(p) => Ok(Box::new(tokio::net::UnixStream::connect(p).await?)),
            #[cfg(not(unix))]
            Upstream::Unix(_) => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Unix sockets are not available on this platform",
            )),
        }
    }
}

/// Pure: may the gateway reach Postgres by `up` under these rules? `Err` names every line to fix;
/// `Ok` carries warnings. Deliberately conservative: any line of the route's connection type counts,
/// whatever its database, user or address, because the gateway cannot know which line a given
/// login will hit.
pub fn route_verdict(rules: &[HbaRule], up: &Upstream) -> Result<Vec<String>, String> {
    let mut warnings = hba_verdict(rules)?;
    let mut refusals = Vec::new();
    for r in rules {
        let at = format!("{}:{}", r.file_name, r.line_number);
        let m = r.auth_method.as_str();
        match up {
            // peer/ident/cert and the rest would let the gateway's OS user in with no password.
            Upstream::Unix(_) if r.kind == "local" => {
                if matches!(m, "md5" | "password") {
                    warnings.push(weak_hash(&at, &r.kind, m));
                } else if !matches!(m, "scram-sha-256" | "reject") {
                    refusals.push(format!(
                        "{at}: a 'local' line uses '{m}', which lets the gateway's own \
                         operating-system user in without the window's password. Change it to \
                         scram-sha-256 (or use a loopback TCP route), reload the server \
                         yourself, and run this again; this command never changes or reloads \
                         Postgres."
                    ));
                }
            }
            Upstream::Tcp(_)
                if matches!(
                    r.kind.as_str(),
                    "host" | "hostssl" | "hostnossl" | "hostgssenc" | "hostnogssenc"
                ) =>
            {
                if matches!(m, "ident" | "gss" | "sspi") {
                    refusals.push(format!(
                        "{at}: a '{}' line uses '{m}', which does not make the window's \
                         password the credential. Change it to scram-sha-256, reload the server \
                         yourself, and run this again; this command never changes or reloads \
                         Postgres.",
                        r.kind
                    ));
                } else if m == "md5" || m == "password" {
                    warnings.push(weak_hash(&at, &r.kind, m));
                }
            }
            _ => {}
        }
    }
    if refusals.is_empty() {
        Ok(warnings)
    } else {
        Err(refusals.join("\n"))
    }
}

fn weak_hash(at: &str, kind: &str, method: &str) -> String {
    format!("{at}: '{kind}' uses {method}; scram-sha-256 is stronger.")
}

/// The write and the report, run once every check has passed: upsert the one `terminal_gateway`
/// row and return the warnings followed by the node's fingerprint line.
pub async fn record_gateway(
    db: &Client,
    up: &Upstream,
    warnings: &[String],
) -> anyhow::Result<String> {
    // On a database the schema was never loaded into, the read fails with only
    // `relation "local_node" does not exist`: name the step (final review C1).
    let node = load_local(db)
        .await
        .context("reading this node's identity (a database not yet set up: run cairn-node init)")?;
    db.execute(
        "INSERT INTO terminal_gateway (upstream, checked_at, checked_by) \
         VALUES ($1, now(), session_user::text) \
         ON CONFLICT (only_row) DO UPDATE SET upstream = EXCLUDED.upstream, \
             checked_at = EXCLUDED.checked_at, checked_by = EXCLUDED.checked_by",
        &[&up.to_string()],
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "recording the gateway route: {}",
            crate::db_diagnosis::legible_db_error(&e)
        )
    })?;
    let mut out: Vec<String> = warnings.to_vec();
    out.push(format!("node fingerprint: {}", node.fingerprint));
    Ok(out.join("\n"))
}

/// `cairn-node gateway-setup`: every refusal (not a superuser, stale rules, an unsafe route)
/// comes BEFORE the one write.
pub async fn gateway_setup(db: &Client, up: &Upstream) -> anyhow::Result<String> {
    let rules = read_hba_rules(db).await?;
    if hba_file_is_stale(db).await? {
        anyhow::bail!(
            "the access rules (pg_hba.conf or a file it includes) changed since the server last \
             read them; reload the server yourself, and run this again"
        );
    }
    let warnings = route_verdict(&rules, up).map_err(|e| anyhow::anyhow!(e))?;
    record_gateway(db, up, &warnings).await
}

#[cfg(test)]
#[path = "upstream_tests.rs"]
mod tests;
