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
//!
//! And a third, for the window's launch: was this database prepared for exactly this build
//! ([`window_schema_verdict`])? The window's login cannot load the schema itself.
use crate::db::{
    embedded_schema_hash, embedded_schema_version, replay_can_be_skipped, SchemaStamp, LOADER_BUILD,
};
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
/// A `hostaddr` is what is actually dialled, so when present it wins over `host`. A Unix-socket
/// path is local. No host at all names nothing to dial (tokio-postgres then refuses to connect),
/// so it is not refused here. The error names the offending host but
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
                // `Host::Unix` exists only on unix targets; any other variant is not a wire.
                #[allow(unreachable_patterns)]
                _ => None,
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
    /// The login is, or is a member of, a database superuser: every check below is moot for it.
    /// Powers that reach the sealed keys outside every grant count here too: `REPLICATION` (a
    /// base backup or the WAL carries `vault_member`'s rows) and membership of
    /// `pg_read_server_files`, `pg_write_server_files` or `pg_execute_server_program` (read the
    /// data files, or get a superuser to run code that does).
    pub superuser: bool,
    /// The login owns, or is a member of the role that owns, a relation in `public` (any one,
    /// not only the node's: the owner of the node's tables is the usual case). That owner can
    /// read the vault and disable its triggers, whatever was granted.
    pub owns_cairn_table: bool,
    /// The login can shape the schema the vault's doors resolve names in: it owns `public`
    /// (in PG15+ the database owner does, through `pg_database_owner`), may `CREATE` in it, or
    /// owns a function in it. Every vault door is `SECURITY DEFINER` with `search_path = public,
    /// pg_temp` and calls helpers by bare name, so a planted or replaced function runs with the
    /// owner's power over `vault_member` (db/061).
    pub shapes_schema: bool,
    /// The login can `SELECT` from `vault_member` (any column): the sealed keys themselves.
    pub reads_vault: bool,
    /// The login can `INSERT` into `event_log` (any column): writes around the validated doors.
    pub inserts_event_log: bool,
    /// The login can run a peer's admission door (`apply_remote_event`, `submit_node_event`:
    /// writes the floor admits from a peer, not from a window), or `cairn_vault_unseal` directly
    /// (an unseal that skips the vault's count).
    pub runs_peer_door: bool,
}

/// Pure: the first reason this connection has more power than a window may have, in plain words,
/// or `None` when it has none of them. The order is the order of [`PostureFacts`]' fields.
pub fn over_powered(f: &PostureFacts) -> Option<&'static str> {
    [
        (
            f.superuser,
            "the login is a database superuser, or can read the server's files or WAL.",
        ),
        (
            f.owns_cairn_table,
            "the login owns tables in the node's schema, or is a member of the role that does.",
        ),
        (
            f.shapes_schema,
            "the login can create or replace objects in the node's schema.",
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

/// Read this connection's [`PostureFacts`] as its own login (`session_user`): the login's name,
/// then every fact in one query ([`read_posture_of`]).
///
/// `session_user`, not `current_user`: the current role is whatever the session last `SET ROLE`
/// to, and a connection string can start it there (`options='-c role=cairn_window'`, or an
/// `ALTER ROLE … SET role`), so the owner's login could present the window's empty powers and
/// `SET ROLE NONE` back after the key is released. `session_user` is the login that authenticated;
/// a startup `-c session_authorization=` is ignored by the server. Every role the login could
/// `SET ROLE` into is already in its MEMBER set below, and a superuser is caught by `rolsuper`.
///
/// Every fact is judged over EVERY role the login is a member of, itself included
/// (`pg_has_role(…, 'MEMBER')`), not over the login's own privileges alone. A member can take a
/// role's whole power with `SET ROLE` even when the grant passes nothing on (`WITH INHERIT
/// FALSE`; PG16+ keeps the SET option by default), so a member of the owner role, of a superuser
/// or of `cairn_node` is as over-powered as that role. The object names are given as
/// `public.`-qualified text, so a missing one is an error (no key released), never a silent "no",
/// and no `search_path` can point a check at a decoy. (The login is always a member of itself, so
/// the set is never empty; were it empty, `coalesce(…, true)` would refuse rather than pass.)
pub async fn read_posture(db: &Client) -> anyhow::Result<PostureFacts> {
    let me: String = db
        .query_one("SELECT session_user::text", &[])
        .await
        .context("reading this connection's login")?
        .get(0);
    read_posture_of(db, &me).await
}

/// The same [`PostureFacts`] for a NAMED role, read from any connection that can see the catalogs.
/// `provision-window-role` uses it to refuse reusing an existing role that is more than a window
/// login. [`read_posture`] calls this, so the two share one query and cannot drift.
pub async fn read_posture_of(db: &Client, role: &str) -> anyhow::Result<PostureFacts> {
    let row = db
        .query_one(
            "SELECT coalesce(bool_or(r.rolsuper OR r.rolreplication OR r.rolname IN ( \
                        'pg_read_server_files', 'pg_write_server_files', \
                        'pg_execute_server_program')), true), \
                    coalesce(bool_or(EXISTS ( \
                        SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = 'public' AND c.relowner = r.oid)), true), \
                    coalesce(bool_or(has_schema_privilege(r.oid, 'public', 'CREATE') \
                        OR EXISTS (SELECT 1 FROM pg_namespace n \
                                    WHERE n.nspname = 'public' AND n.nspowner = r.oid) \
                        OR EXISTS (SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                                    WHERE n.nspname = 'public' AND p.proowner = r.oid)), true), \
                    coalesce(bool_or(has_any_column_privilege(r.oid, 'public.vault_member', 'SELECT')), true), \
                    coalesce(bool_or(has_any_column_privilege(r.oid, 'public.event_log', 'INSERT')), true), \
                    coalesce(bool_or( \
                        has_function_privilege(r.oid, 'public.apply_remote_event(bytea, bytea, bytea, bytea)', 'EXECUTE') \
                        OR has_function_privilege(r.oid, 'public.submit_node_event(bytea)', 'EXECUTE') \
                        OR has_function_privilege(r.oid, 'public.cairn_vault_unseal(bytea, text)', 'EXECUTE')), true) \
               FROM pg_roles r WHERE pg_has_role($1::text::name, r.oid, 'MEMBER')",
            &[&role],
        )
        .await
        .context("reading what this role may do")?;
    Ok(PostureFacts {
        superuser: row.get(0),
        owns_cairn_table: row.get(1),
        shapes_schema: row.get(2),
        reads_vault: row.get(3),
        inserts_event_log: row.get(4),
        runs_peer_door: row.get(5),
    })
}

/// May the reference window start against this database's schema stamp? **Pure.**
///
/// The window's own login cannot replay DDL (db/062), so it never loads the schema; it only
/// checks that the owner's install/upgrade step left a stamp for exactly this build. `Ok` iff the
/// loader's own skip test would pass. A database newer than this window is told apart from one
/// not yet prepared, because the remedies differ (install a newer window vs. run the owner step).
pub fn window_schema_verdict(stamp: Option<&SchemaStamp>) -> Result<(), String> {
    let ours = embedded_schema_version();
    if replay_can_be_skipped(stamp, ours, LOADER_BUILD, &embedded_schema_hash()) {
        return Ok(());
    }
    match stamp {
        Some(s) if s.version > ours => Err(format!(
            "This window is older than the node's database (generation {} against {ours}). \
             Install the window that matches the node.",
            s.version
        )),
        // Same generation, different build (hash or loader): any other cairn-node would stamp
        // its own build again, so only the one built with this window can satisfy it.
        Some(s) if s.version == ours => Err(format!(
            "The node's database was prepared by a different build (generation {ours}, but not \
             this window's build). At the node, run provision-window-role with the cairn-node \
             built with this window and the owner's connection, then start this window again."
        )),
        _ => {
            let db = stamp.map_or("none".to_string(), |s| s.version.to_string());
            Err(format!(
                "The node's database has not been prepared for this window (generation {db} \
                 against {ours}). At the node, run cairn-node provision-window-role with the \
                 owner's connection, then start this window again."
            ))
        }
    }
}

#[cfg(test)]
#[path = "conn_posture_tests.rs"]
mod tests;
