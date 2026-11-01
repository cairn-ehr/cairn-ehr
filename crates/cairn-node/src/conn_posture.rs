//! Is this database connection safe to carry passphrases and signing keys?
//!
//! The vault's console verbs and the reference window both send secrets (a passphrase, a released
//! signing seed) over the node's database connection. Over a Unix socket or loopback that never
//! leaves the machine; over a wire it would travel in clear text. This module answers the question
//! from the connection string alone, before anything connects.
use anyhow::{anyhow, bail};
use tokio_postgres::config::Host;
use tokio_postgres::Config;

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

#[cfg(test)]
#[path = "conn_posture_tests.rs"]
mod tests;
