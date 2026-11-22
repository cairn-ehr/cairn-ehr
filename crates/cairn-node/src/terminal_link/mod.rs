//! The terminal's side of the pinned link (#909 slice B, design D4/D5).
//!
//! A Cairn window on ANOTHER machine than the node's database never talks to Postgres in clear
//! text. It reaches the node's terminal gateway instead: TCP, Postgres's `SSLRequest`, then
//! mutual TLS 1.3 in which the terminal presents its own Ed25519 key and accepts only the node's
//! pinned key; Postgres's startup and SCRAM then run inside that session, end to end. A window on
//! the node's own machine keeps connecting directly, exactly as before.
//!
//! The ceremony, from the terminal's side:
//! 1. The node's operator runs `gateway-setup` at the node and reads out the node's fingerprint.
//! 2. The terminal's first launch is given `--terminal-dir <dir>` and `--node-fingerprint <fp>`.
//!    It makes its own key if it has none ([`files`]), dials the gateway and pins the node's full
//!    key once the node has proved it holds the key with that fingerprint ([`enrol`]). The
//!    gateway records the attempt; the launch then stops with the not-admitted refusal, which
//!    names the command to run at the node with this terminal's fingerprint.
//! 3. The operator runs `admit-terminal <fp>` at the node. Every later launch needs only
//!    `--terminal-dir`, and connects through [`Link::Pinned`] ([`connector`]).
//!
//! [`prepare`] is the one entry point: from the connection string and the two flags it returns
//! the [`Link`] to connect through, or the refusal that says what is missing.

pub mod connector;
pub mod enrol;
pub mod files;

pub use connector::{Link, PinnedTls};

use std::path::Path;

use anyhow::{anyhow, bail};
use tokio_postgres::config::Host;
use tokio_postgres::Config;

use crate::conn_posture::require_local;
use files::TerminalFiles;

/// The link to open `conn` through, given the window's `--terminal-dir` and `--node-fingerprint`.
///
/// * A local database: [`Link::Local`]. Either flag is refused there as meaningless, so a
///   mistyped host is not silently read as "local, ignore the flags".
/// * A remote one: [`Link::Pinned`], from the pin in `terminal_dir`, made on first launch from
///   the typed fingerprint. Refused with no `terminal_dir` (as before this slice, plus the flags
///   to give), with no pin and no fingerprint, with a pin the typed fingerprint contradicts, and
///   with more than one host or a `hostaddr` (the address dialled must be the one checked).
///
/// No refusal quotes the connection string: it can carry a password.
pub async fn prepare(
    conn: &str,
    terminal_dir: Option<&Path>,
    typed_fingerprint: Option<&str>,
) -> anyhow::Result<Link> {
    let cfg: Config = conn
        .parse()
        .map_err(|_| anyhow!("the connection string could not be parsed"))?;
    let remote = match require_local(conn) {
        Ok(()) => {
            if terminal_dir.is_some() || typed_fingerprint.is_some() {
                bail!(
                    "--terminal-dir and --node-fingerprint are for a node on another machine; \
                     this connection string reaches a database on this machine, so leave them out."
                );
            }
            return Ok(Link::Local);
        }
        Err(remote) => remote,
    };
    if !cfg.get_hostaddrs().is_empty() {
        bail!(
            "the connection string names a hostaddr; a terminal reaches the node by host= alone, \
             so the address it dials is the one it names."
        );
    }
    if cfg.get_hosts().len() > 1 {
        bail!("the connection string names more than one host; a terminal links to one node.");
    }
    let Some(dir) = terminal_dir else {
        bail!(
            "{remote} To reach it from this machine through the node's gateway, start with \
             --terminal-dir <dir> and, the first time, --node-fingerprint <the node's fingerprint>."
        );
    };
    pinned(&cfg, dir, typed_fingerprint).await
}

/// The remote half of [`prepare`]: read or make the pin, then build the connector.
async fn pinned(cfg: &Config, dir: &Path, typed: Option<&str>) -> anyhow::Result<Link> {
    let files = TerminalFiles::at(dir);
    let pin = match (files.read_pin()?, typed) {
        (Some(pin), None) => pin,
        (Some(pin), Some(typed)) => {
            let agrees = enrol::normalise_fingerprint(typed)
                .is_some_and(|want| enrol::has_fingerprint(&pin, &want));
            if !agrees {
                bail!(
                    "this terminal already pinned a node whose fingerprint is not {typed}, and a \
                     pin is never replaced by a flag. Ask the node's operator: if the node's key \
                     really changed, delete node.pin in the terminal directory and start again \
                     with the new fingerprint."
                );
            }
            pin
        }
        (None, None) => bail!(
            "this terminal has not pinned the node yet: start it once with --node-fingerprint \
             <fingerprint>, as the node's operator reads it out from gateway-setup."
        ),
        (None, Some(typed)) => {
            let key = files.load_or_create_key()?;
            let pin = enrol::pin_from_fingerprint(&gateway_addr(cfg)?, typed, &key).await?;
            files.write_pin(&pin)?;
            pin
        }
    };
    let key = files.load_or_create_key()?;
    Ok(Link::Pinned(PinnedTls::new(&key, &pin)?))
}

/// `host:port` of the one TCP host in `cfg` (port 5432 unless named), for the first-launch dial.
/// An IPv6 literal is bracketed so the port stays separable.
fn gateway_addr(cfg: &Config) -> anyhow::Result<String> {
    let host = match cfg.get_hosts().first() {
        Some(Host::Tcp(name)) => name.clone(),
        _ => bail!("the connection string names no network host for the node's gateway"),
    };
    let port = cfg.get_ports().first().copied().unwrap_or(5432);
    Ok(if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    })
}

#[cfg(test)]
#[path = "prepare_tests.rs"]
mod tests;
