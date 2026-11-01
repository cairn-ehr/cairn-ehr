//! Provisioning the reference window's own database login (#898/#899).
//!
//! The window must never connect as the owner or as the node's runtime role: it gets its own
//! LOGIN role that is only a member of `cairn_window` (db/062). This module is the owner-run half:
//! pure verdicts on the server's `pg_hba.conf` rules (a cluster that lets anyone in with no
//! password makes the window's password pointless, so we refuse to provision on one), on their
//! freshness, and on whether an existing role may be taken over; plus the DB reads and the one
//! write the `provision-window-role` verb orchestrates. `main.rs` only sequences them.

use crate::conn_posture::{over_powered, read_posture_of};
use crate::db::is_safe_role_ident;
use crate::db_diagnosis::legible_db_error;
use tokio_postgres::Client;

/// One row of `pg_hba_file_rules`, reduced to what the verdict reads.
#[derive(Debug, Clone)]
pub struct HbaRule {
    /// The file the line is in: `pg_hba.conf` itself, or a file it `include`s (PG16+ lists those rules too).
    pub file_name: String,
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
                "{}, line {}: the server could not read this access rule ({e}); fix it, reload the server, and run this again.",
                r.file_name, r.line_number
            ));
        } else if r.auth_method == "trust" {
            refusals.push(format!(
                "{}, line {}: '{}' connections are let in with no password (trust). First give every login that connects through this line a password (the schema owner, and the node daemon's login), then change it to scram-sha-256, reload the server, and run this again.",
                r.file_name, r.line_number, r.kind
            ));
        } else if r.auth_method == "peer"
            && !r.options.iter().flatten().any(|o| o.starts_with("map="))
        {
            warnings.push(format!(
                "{}, line {}: '{}' uses peer authentication with no map; any process running as the matching operating-system account is let in.",
                r.file_name, r.line_number, r.kind
            ));
        }
    }
    if refusals.is_empty() {
        Ok(warnings)
    } else {
        Err(refusals.join("\n"))
    }
}

/// Pure: an on-disk access-rules file (`pg_hba.conf` or one it includes) is newer than what the
/// server loaded, so the rules we read are not the rules in force (an operator who removed `trust`
/// but did not reload would pass).
pub fn hba_freshness(file_newer_than_load: bool) -> Result<(), String> {
    if file_newer_than_load {
        Err("The access rules (pg_hba.conf or a file it includes) changed since the server last read them. Reload the server, and run this again.".into())
    } else {
        Ok(())
    }
}

/// Has `pg_hba.conf`, or any file it includes, been modified since the server last (re)loaded its
/// configuration? One query: the main file plus every distinct `pg_hba_file_rules.file_name`.
pub async fn hba_file_is_stale(db: &Client) -> anyhow::Result<bool> {
    // shortcut: file mtime has 1 s resolution vs the load time's microseconds, so an edit within the
    // same second as a reload reads fresh; upgrade by comparing the file's content hash instead.
    let row = db
        .query_one(
            "SELECT coalesce(bool_or((pg_stat_file(f)).modification > pg_conf_load_time()), true) \
               FROM (SELECT current_setting('hba_file') AS f \
                     UNION SELECT file_name FROM pg_hba_file_rules WHERE file_name IS NOT NULL) t",
            &[],
        )
        .await
        .map_err(|e| anyhow::anyhow!("checking pg_hba.conf freshness: {}", legible_db_error(&e)))?;
    Ok(row.get::<_, Option<bool>>(0).unwrap_or(true))
}

/// Read the server's parsed `pg_hba.conf`.
///
/// `pg_hba_file_rules` shows the FILE as it is now, not the rules in force: pair this with
/// [`hba_file_is_stale`]. On PG16+ lines pulled in by an `include` directive appear here too (with
/// their own `file_name`), so they are judged like any other.
/// Only a superuser may; a 42501 becomes an instruction to run as the schema OWNER (who must be a
/// superuser for this step), never "as a superuser", which could steer a non-superuser owner into
/// running the schema replay as some other role that would then own it.
pub async fn read_hba_rules(db: &Client) -> anyhow::Result<Vec<HbaRule>> {
    let rows = db
        .query(
            "SELECT file_name, line_number, type, auth_method, options, error FROM pg_hba_file_rules ORDER BY rule_number",
            &[],
        )
        .await
        .map_err(|e| {
            if e.code().map(|c| c.code()) == Some("42501") {
                anyhow::anyhow!(
                    "could not read this server's access rules; run this step as the schema owner, who must be a database superuser for it"
                )
            } else {
                anyhow::anyhow!("reading the server's access rules: {}", legible_db_error(&e))
            }
        })?;
    Ok(rows
        .iter()
        .map(|r| HbaRule {
            file_name: r.get::<_, Option<String>>(0).unwrap_or_default(),
            line_number: r.get::<_, Option<i32>>(1).unwrap_or(0),
            kind: r.get::<_, Option<String>>(2).unwrap_or_default(),
            auth_method: r.get::<_, Option<String>>(3).unwrap_or_default(),
            options: r.get(4),
            error: r.get(5),
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
    /// other roles are members of this one, so a grant to it would reach them too.
    pub has_members: bool,
}

/// Pure: why an existing role may not be taken over as the window login, or `None` if it may.
pub fn existing_role_refusal(f: &RoleFacts, posture_why: Option<&str>) -> Option<String> {
    let why = if !f.can_login {
        "it cannot log in"
    } else if f.has_attribute_power {
        "it has superuser, createrole, createdb, replication or bypassrls"
    } else if f.other_membership {
        "it is a member of roles other than cairn_window"
    } else if f.has_members {
        "other roles are members of it"
    } else if let Some(w) = posture_why {
        return Some(refusal_text(w.trim_end_matches('.')));
    } else {
        return None;
    };
    Some(refusal_text(why))
}

fn refusal_text(why: &str) -> String {
    format!(
        "that role already exists and is not a window login ({why}); the window login was not created or changed. \
         Choose a new name, or the existing window login."
    )
}

async fn read_role_facts(db: &Client, role: &str) -> anyhow::Result<RoleFacts> {
    let r = db
        .query_one(
            "SELECT r.rolcanlogin, \
                    (r.rolsuper OR r.rolcreaterole OR r.rolcreatedb OR r.rolreplication OR r.rolbypassrls), \
                    EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid \
                             WHERE m.member = r.oid AND g.rolname <> 'cairn_window'), \
                    EXISTS (SELECT 1 FROM pg_auth_members m WHERE m.roleid = r.oid) \
               FROM pg_roles r WHERE r.rolname = $1",
            &[&role],
        )
        .await
        .map_err(|e| anyhow::anyhow!("reading role {role}: {}", legible_db_error(&e)))?;
    Ok(RoleFacts {
        can_login: r.get(0),
        has_attribute_power: r.get(1),
        other_membership: r.get(2),
        has_members: r.get(3),
    })
}

/// Create (or refresh) the window's LOGIN role, member of `cairn_window`.
///
/// A new role REQUIRES a password. With a password the role gets a SCRAM-SHA-256 verifier, so the
/// cleartext never reaches the server (or its logs). With none on an existing role the stored
/// verifier is left alone. Precondition: the schema is loaded (db/062 creates `cairn_window`).
/// The password is never logged and never put in an error.
///
/// All of the DDL is ONE multi-statement message, which the server runs as one transaction: a
/// step that fails leaves nothing behind. (Sent one statement at a time, a failed GRANT would
/// leave a LOGIN with no password, and a re-run, which asks for none on an existing role, would
/// then report it provisioned.)
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
    let mut ddl = Vec::new();
    let password_clause = match password {
        Some(pw) => {
            let verifier = postgres_protocol::password::scram_sha_256(pw.as_bytes());
            // CREATE/ALTER ROLE ... PASSWORD takes no bind parameter, so the verifier is
            // interpolated. It is not the password: it is base64 plus `$ : =`, which we assert (no
            // quote can occur), so no escaping is needed.
            if !verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "$:=+/-".contains(c))
            {
                anyhow::bail!(
                    "internal error: unexpected characters in the generated password verifier"
                );
            }
            format!(" PASSWORD '{verifier}'")
        }
        None => String::new(),
    };
    if exists {
        if !password_clause.is_empty() {
            ddl.push(format!("ALTER ROLE {role}{password_clause}"));
        }
    } else {
        ddl.push(format!("CREATE ROLE {role} LOGIN{password_clause}"));
    }
    ddl.push(format!("GRANT cairn_window TO {role}"));
    // A window login holding a transaction open holds hlc_state's tick lock and stalls every
    // writer (db/062). The window cuts each command off after 15 s (cairn-gui-live's
    // `NodeConnection::ANSWER_WITHIN`), so a live transaction never sits idle for 60 s between
    // statements; only an abandoned one is ended.
    ddl.push(format!(
        "ALTER ROLE {role} SET idle_in_transaction_session_timeout = '60s'"
    ));
    db.batch_execute(&ddl.join("; ")).await.map_err(|e| {
        anyhow::anyhow!("provisioning window role {role}: {}", legible_db_error(&e))
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "window_access_tests.rs"]
mod tests;
