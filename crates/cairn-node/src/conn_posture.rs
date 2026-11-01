//! Is this database connection safe to carry passphrases and signing keys?
//!
//! The vault's console verbs and the reference window both send secrets (a passphrase, a released
//! signing seed) over the node's database connection. Over a Unix socket or loopback that never
//! leaves the machine; over a wire it would travel in clear text. This module answers the question
//! from the connection string alone, before anything connects.
//!
//! It also answers a second question, once connected: does this login have MORE power than a
//! window may have (#898)? The vault's protections (counted guesses, purpose checks) hold only
//! against a connection that cannot read the sealed keys or write around the floor itself, so a
//! release over such a connection is refused before any passphrase is sent ([`over_powered`]).
use anyhow::{anyhow, bail, Context};
use tokio_postgres::config::Host;
use tokio_postgres::{Client, Config};

/// True when `host` names this machine: `localhost` (any case) or a loopback IP literal.
fn is_local_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Refuse a connection string that reaches a database on another machine.
///
/// A `hostaddr` is what libpq actually dials, so when present it wins over `host`. A Unix-socket
/// path, or no host at all (the local default), is local. The error names the offending host but
/// never the connection string, which can carry a password.
pub fn require_local(conn: &str) -> anyhow::Result<()> {
    // Parse errors can quote the input; say only that it failed.
    let cfg: Config = conn
        .parse()
        .map_err(|_| anyhow!("the connection string could not be parsed"))?;
    let addrs: Vec<String> = cfg.get_hostaddrs().iter().map(|a| a.to_string()).collect();
    let hosts: Vec<String> = if addrs.is_empty() {
        cfg.get_hosts()
            .iter()
            .filter_map(|h| match h {
                Host::Tcp(name) => Some(name.clone()),
                Host::Unix(_) => None,
            })
            .collect()
    } else {
        addrs
    };
    if let Some(remote) = hosts.iter().find(|h| !is_local_host(h)) {
        bail!(
            "the node's database is on another machine ({remote}), and this connection would carry \
             passphrases and signing keys in clear text. Run this on the node's own machine (a Unix \
             socket or localhost)."
        );
    }
    Ok(())
}

/// What a connection's login can do that a window must not. Read by [`read_posture`], judged by
/// [`over_powered`]; split so the judgement is a pure function tested without a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostureFacts {
    /// The login is a database superuser: every check below is moot for it.
    pub superuser: bool,
    /// The login owns, or is a member of the role that owns, a table in `public`. An owner can
    /// read the vault and disable its triggers, whatever was granted.
    pub owns_cairn_table: bool,
    /// The login can `SELECT` from `vault_member` (any column): the sealed keys themselves.
    pub reads_vault: bool,
    /// The login can `INSERT` into `event_log` (any column): writes around the validated doors.
    pub inserts_event_log: bool,
    /// The login can run a peer's admission door (`apply_remote_event`, `submit_node_event`) or
    /// `cairn_vault_unseal` directly, skipping the vault's count.
    pub runs_peer_door: bool,
}

/// Pure: the first reason this connection has more power than a window may have, in plain words,
/// or `None` when it has none of them. The order is the order of [`PostureFacts`]' fields.
pub fn over_powered(f: &PostureFacts) -> Option<&'static str> {
    [
        (f.superuser, "the login is a database superuser."),
        (
            f.owns_cairn_table,
            "the login owns the node's tables, or is a member of the role that does.",
        ),
        (f.reads_vault, "the login can read the vault's sealed keys."),
        (
            f.inserts_event_log,
            "the login can write the event log directly, around the floor.",
        ),
        (
            f.runs_peer_door,
            "the login can run a peer's admission door or unseal a key directly.",
        ),
    ]
    .into_iter()
    .find_map(|(has, why)| has.then_some(why))
}

/// Read this connection's [`PostureFacts`] in ONE query, as its own login (`current_user`).
///
/// Ownership is `pg_has_role(…, 'USAGE')`, not `relowner = current_user`: a login that merely
/// inherits the owner role through membership has the owner's power too. The object names are
/// given as `public.`-qualified text, so a missing one is an error (no key released), never a
/// silent "no", and no `search_path` can point a check at a decoy.
pub async fn read_posture(db: &Client) -> anyhow::Result<PostureFacts> {
    let row = db
        .query_one(
            "SELECT r.rolsuper, \
                    EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                             WHERE n.nspname = 'public' \
                               AND pg_has_role(current_user, c.relowner, 'USAGE')), \
                    has_any_column_privilege('public.vault_member', 'SELECT'), \
                    has_any_column_privilege('public.event_log', 'INSERT'), \
                    has_function_privilege('public.apply_remote_event(bytea, bytea, bytea, bytea)', 'EXECUTE') \
                    OR has_function_privilege('public.submit_node_event(bytea)', 'EXECUTE') \
                    OR has_function_privilege('public.cairn_vault_unseal(bytea, text)', 'EXECUTE') \
               FROM pg_roles r WHERE r.rolname = current_user",
            &[],
        )
        .await
        .context("reading what this connection's login may do")?;
    Ok(PostureFacts {
        superuser: row.get(0),
        owns_cairn_table: row.get(1),
        reads_vault: row.get(2),
        inserts_event_log: row.get(3),
        runs_peer_door: row.get(4),
    })
}

#[cfg(test)]
#[path = "conn_posture_tests.rs"]
mod tests;
