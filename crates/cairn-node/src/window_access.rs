//! Provisioning the reference window's own database login (#898/#899).
//!
//! The window must never connect as the owner or as the node's runtime role: it gets its own
//! LOGIN role that is only a member of `cairn_window` (db/062). This module is the owner-run half:
//! a pure verdict on the server's `pg_hba.conf` rules (a cluster that lets anyone in with no
//! password makes the window's password pointless, so we refuse to provision on one), plus the two
//! DB functions the `provision-window-role` verb orchestrates. `main.rs` only sequences them.

use crate::conn_posture::{over_powered, read_posture_of};
use crate::db::is_safe_role_ident;
use crate::db_diagnosis::legible_db_error;
use tokio_postgres::Client;

/// One row of `pg_hba_file_rules`, reduced to what the verdict reads.
#[derive(Debug, Clone)]
pub struct HbaRule {
    pub line_number: i32,
    /// `local`, `host`, `hostssl`, ...
    pub kind: String,
    pub auth_method: String,
    pub options: Option<Vec<String>>,
    /// Set when the server could not parse this line.
    pub error: Option<String>,
}

/// Judge the rules. `Err` names every line to fix (refuse BEFORE mutating anything); `Ok` carries
/// warnings to print. `trust` lets anyone in with no credential, so it is refused; `peer` without
/// a `map=` is only a warning (it trusts the OS account, which is legitimate but worth saying).
pub fn hba_verdict(rules: &[HbaRule]) -> Result<Vec<String>, String> {
    let mut refusals = Vec::new();
    let mut warnings = Vec::new();
    for r in rules {
        if let Some(e) = &r.error {
            refusals.push(format!(
                "line {}: the server could not read this access rule ({e}); fix pg_hba.conf, reload the server, and run this again.",
                r.line_number
            ));
        } else if r.auth_method == "trust" {
            refusals.push(format!(
                "line {}: '{}' connections are let in with no password (trust). Change it to scram-sha-256 in pg_hba.conf, reload the server, and run this again.",
                r.line_number, r.kind
            ));
        } else if r.auth_method == "peer"
            && !r.options.iter().flatten().any(|o| o.starts_with("map="))
        {
            warnings.push(format!(
                "line {}: '{}' uses peer authentication with no map; any process running as the matching operating-system account is let in.",
                r.line_number, r.kind
            ));
        }
    }
    if refusals.is_empty() {
        Ok(warnings)
    } else {
        Err(refusals.join("\n"))
    }
}

/// Pure: the on-disk `pg_hba.conf` is newer than what the server loaded, so the rules we read
/// are not the rules in force (an operator who removed `trust` but did not reload would pass).
pub fn hba_freshness(file_newer_than_load: bool) -> Result<(), String> {
    if file_newer_than_load {
        Err("pg_hba.conf changed since the server last read it. Reload the server, and run this again.".into())
    } else {
        Ok(())
    }
}

/// Has `pg_hba.conf` been modified since the server last (re)loaded its configuration?
pub async fn hba_file_is_stale(db: &Client) -> anyhow::Result<bool> {
    let row = db
        .query_one(
            "SELECT (pg_stat_file(current_setting('hba_file'))).modification > pg_conf_load_time()",
            &[],
        )
        .await
        .map_err(|e| anyhow::anyhow!("checking pg_hba.conf freshness: {}", legible_db_error(&e)))?;
    Ok(row.get::<_, Option<bool>>(0).unwrap_or(true))
}

/// Read the server's parsed `pg_hba.conf`.
///
/// `pg_hba_file_rules` shows the FILE as it is now, not the rules in force: pair this with
/// [`hba_file_is_stale`]. Lines pulled in by an `include` directive are not judged here.
/// Only a superuser may; a 42501 becomes an instruction.
pub async fn read_hba_rules(db: &Client) -> anyhow::Result<Vec<HbaRule>> {
    let rows = db
        .query(
            "SELECT line_number, type, auth_method, options, error FROM pg_hba_file_rules ORDER BY rule_number",
            &[],
        )
        .await
        .map_err(|e| {
            if e.code().map(|c| c.code()) == Some("42501") {
                anyhow::anyhow!(
                    "could not read this server's access rules; run this step as a database superuser"
                )
            } else {
                anyhow::anyhow!("reading the server's access rules: {}", legible_db_error(&e))
            }
        })?;
    Ok(rows
        .iter()
        .map(|r| HbaRule {
            line_number: r.get::<_, Option<i32>>(0).unwrap_or(0),
            kind: r.get::<_, Option<String>>(1).unwrap_or_default(),
            auth_method: r.get::<_, Option<String>>(2).unwrap_or_default(),
            options: r.get(3),
            error: r.get(4),
        })
        .collect())
}

/// Does a role of this name already exist? (The verb asks before prompting for a password.)
pub async fn role_exists(db: &Client, role: &str) -> anyhow::Result<bool> {
    Ok(db
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)",
            &[&role],
        )
        .await
        .map_err(|e| anyhow::anyhow!("probing for role {role}: {}", legible_db_error(&e)))?
        .get(0))
}

/// The catalog facts that say whether an existing role is exactly a window login.
#[derive(Debug, Clone, Copy)]
pub struct RoleFacts {
    pub can_login: bool,
    /// superuser, createrole, createdb, replication or bypassrls.
    pub has_attribute_power: bool,
    /// a direct membership in any role other than `cairn_window`.
    pub other_membership: bool,
}

/// Pure: why an existing role may not be taken over as the window login, or `None` if it may.
pub fn existing_role_refusal(f: &RoleFacts, posture_why: Option<&str>) -> Option<String> {
    let why = if !f.can_login {
        "it cannot log in"
    } else if f.has_attribute_power {
        "it has superuser, createrole, createdb, replication or bypassrls"
    } else if f.other_membership {
        "it is a member of roles other than cairn_window"
    } else if let Some(w) = posture_why {
        return Some(refusal_text(w.trim_end_matches('.')));
    } else {
        return None;
    };
    Some(refusal_text(why))
}

fn refusal_text(why: &str) -> String {
    format!(
        "that role already exists and is not a window login ({why}); nothing was changed. \
         Choose a new name, or the existing window login."
    )
}

async fn read_role_facts(db: &Client, role: &str) -> anyhow::Result<RoleFacts> {
    let r = db
        .query_one(
            "SELECT r.rolcanlogin, \
                    (r.rolsuper OR r.rolcreaterole OR r.rolcreatedb OR r.rolreplication OR r.rolbypassrls), \
                    EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid \
                             WHERE m.member = r.oid AND g.rolname <> 'cairn_window') \
               FROM pg_roles r WHERE r.rolname = $1",
            &[&role],
        )
        .await
        .map_err(|e| anyhow::anyhow!("reading role {role}: {}", legible_db_error(&e)))?;
    Ok(RoleFacts {
        can_login: r.get(0),
        has_attribute_power: r.get(1),
        other_membership: r.get(2),
    })
}

/// Create (or refresh) the window's LOGIN role, member of `cairn_window`.
///
/// A new role REQUIRES a password. With a password the role gets a SCRAM-SHA-256 verifier, so the
/// cleartext never reaches the server (or its logs). With none on an existing role the stored
/// verifier is left alone. Precondition: the schema is loaded (db/062 creates `cairn_window`).
/// The password is never logged and never put in an error.
pub async fn provision_window_login(
    db: &Client,
    role: &str,
    password: Option<&str>,
) -> anyhow::Result<()> {
    // Injection floor: the role name is interpolated into DDL (identifiers take no bind parameter).
    if !is_safe_role_ident(role) {
        anyhow::bail!(
            "invalid window role name {role:?}: use lowercase letters, digits, and underscores \
             (starting with a letter or underscore), max 63 chars"
        );
    }
    let exists = role_exists(db, role).await?;
    if exists {
        // Refuse BEFORE any DDL: re-passwording or re-granting an arbitrary existing login would
        // lock out whoever uses it, or hand cairn_window to a whole group.
        let facts = read_role_facts(db, role).await?;
        let posture = read_posture_of(db, role).await?;
        if let Some(why) = existing_role_refusal(&facts, over_powered(&posture)) {
            anyhow::bail!(why);
        }
    }
    if !exists && password.is_none() {
        anyhow::bail!("role {role} does not exist yet, so it needs a password; none was given");
    }
    let ddl = |sql: String| async move {
        db.batch_execute(&sql).await.map_err(|e| {
            anyhow::anyhow!("provisioning window role {role}: {}", legible_db_error(&e))
        })
    };
    if !exists {
        ddl(format!("CREATE ROLE {role} LOGIN")).await?;
    }
    ddl(format!("GRANT cairn_window TO {role}")).await?;
    if let Some(pw) = password {
        let verifier = postgres_protocol::password::scram_sha_256(pw.as_bytes());
        // ALTER ROLE ... PASSWORD takes no bind parameter, so the verifier is interpolated. It is
        // not the password: it is base64 plus `$ : =`, which we assert (no quote can occur), so
        // no escaping is needed.
        if !verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "$:=+/-".contains(c))
        {
            anyhow::bail!(
                "internal error: unexpected characters in the generated password verifier"
            );
        }
        ddl(format!("ALTER ROLE {role} PASSWORD '{verifier}'")).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "window_access_tests.rs"]
mod tests;
