use tokio_postgres::{Client, NoTls};

const SCHEMA: [(&str, &str); 13] = [
    ("001_envelope",      include_str!("../../../db/001_envelope.sql")),
    ("002_projection",    include_str!("../../../db/002_projection.sql")),
    ("003_blobs",         include_str!("../../../db/003_blobs.sql")),
    ("004_actors",        include_str!("../../../db/004_actors.sql")),
    ("005_submit",        include_str!("../../../db/005_submit.sql")),
    ("006_recall",        include_str!("../../../db/006_recall.sql")),
    ("007_node_federation", include_str!("../../../db/007_node_federation.sql")),
    // NOTE: db/008_surrogate_projection.sql is INTENTIONALLY not loaded here. It is a
    // spike artefact (the ADR-0031 dense-bigint surrogate-key measurement, exercised on
    // Bet B), not part of the node's runtime schema — hence the 007 -> 009 jump. Leave
    // the gap; do not "fix" it by inserting 008. (Confirmed spike-only; see issue #67.)
    ("009_node_supersede_and_restore", include_str!("../../../db/009_node_supersede_and_restore.sql")),
    ("010_demographics",  include_str!("../../../db/010_demographics.sql")),
    ("011_demographics_fields", include_str!("../../../db/011_demographics_fields.sql")),
    ("012_demographics_names",  include_str!("../../../db/012_demographics_names.sql")),
    ("013_demographics_sex_gender", include_str!("../../../db/013_demographics_sex_gender.sql")),
    ("014_demographics_address", include_str!("../../../db/014_demographics_address.sql")),
];

pub async fn connect(conn: &str) -> anyhow::Result<Client> {
    let (client, connection) = tokio_postgres::connect(conn, NoTls).await?;
    tokio::spawn(async move { let _ = connection.await; });
    Ok(client)
}

/// Is `role` a conservative, safe-to-interpolate PostgreSQL identifier?
///
/// Identifiers cannot be bind parameters, so a runtime role name is interpolated
/// directly into DDL — this is the SQL-injection floor for [`provision_runtime_role`].
/// We accept only lowercase ASCII letters, digits, and underscores, starting with a
/// letter or underscore, length 1..=63 (PostgreSQL identifiers are <= 63 bytes).
/// Lowercase-only keeps the charset tight and matches Postgres' unquoted-identifier
/// folding, so there is never a quoting ambiguity. Pure (no DB) so it is unit-testable.
pub fn is_safe_role_ident(role: &str) -> bool {
    !role.is_empty()
        && role.len() <= 63
        && role.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && role.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
}

/// Provision the unprivileged runtime login role and grant it `cairn_node`.
///
/// The in-DB submit/admission floor (`db/007`) only *binds* a connection that is
/// neither superuser nor table owner — a superuser raw-INSERTs around the gate. So
/// the "enforced in Postgres" guarantee holds iff the daemon connects as a login
/// role that merely *inherits* `cairn_node` (which is NOLOGIN). This is the one DDL
/// step that creates that role; run it once, with owner privileges, then point the
/// runtime `--conn`/`CAIRN_CONN` at `user=<role>`. `status` then reports
/// `db_floor ENFORCED`.
///
/// Idempotent: re-running is a no-op (the role is created only if absent, and the
/// GRANT is harmless to repeat). The role is created with LOGIN and NO password —
/// fine for a local-socket/trust deployment; a networked deployment should `ALTER
/// ROLE … PASSWORD` afterwards (we never embed a secret here).
///
/// Precondition: the schema must already be loaded (the `cairn_node` group role is
/// created by `db/007`). Run this *after* `init` / `connect_and_load_schema`; on a
/// fresh database it fails with a legible "load the schema first" error rather than a
/// raw catalog error from the GRANT.
pub async fn provision_runtime_role(client: &Client, role: &str) -> anyhow::Result<()> {
    // Identifiers cannot be passed as bind parameters, so this name is interpolated
    // into DDL. Reject anything but a conservative identifier charset to close the
    // SQL-injection door rather than trusting the caller (defence in depth — the
    // CLI also constrains it). PostgreSQL identifiers are <= 63 bytes.
    if !is_safe_role_ident(role) {
        anyhow::bail!(
            "invalid runtime role name {role:?}: use lowercase letters, digits, and underscores \
             (starting with a letter or underscore), max 63 chars"
        );
    }
    // Precondition: the `cairn_node` group role must exist (created by the schema
    // load). Without it the GRANT below fails with an opaque catalog error; check
    // first so the operator gets an actionable message ("load the schema / run init").
    let cairn_node_exists: bool = client
        .query_one("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node')", &[])
        .await?
        .get(0);
    if !cairn_node_exists {
        anyhow::bail!(
            "the `cairn_node` group role does not exist: load the schema first \
             (run `cairn-node init`, or connect_and_load_schema) before provisioning a runtime role"
        );
    }
    // CREATE ROLE has no IF NOT EXISTS, so guard with a catalog check; the name is
    // safe to interpolate after the charset gate above.
    let ddl = format!(
        "DO $$ BEGIN \
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{role}') THEN \
             CREATE ROLE {role} LOGIN; \
           END IF; \
         END $$; \
         GRANT cairn_node TO {role};"
    );
    client
        .batch_execute(&ddl)
        .await
        .map_err(|e| anyhow::anyhow!("provisioning runtime role {role}: {e}"))?;
    Ok(())
}

pub async fn connect_and_load_schema(conn: &str) -> anyhow::Result<Client> {
    let client = connect(conn).await?;
    for (name, sql) in SCHEMA.iter() {
        client.batch_execute(sql).await.map_err(|e| anyhow::anyhow!("loading {name}: {e}"))?;
    }
    Ok(client)
}

/// Test-support: reset the node-federation tables to a clean slate between tests.
///
/// `TRUNCATE hlc_state` drops the singleton row the HLC door (`node_hlc_tick`) reads,
/// so every reset MUST re-seat it — otherwise the next authored event silently mints a
/// `0/0` HLC again (the very placeholder issue #38 removed, and `node_hlc_tick`'s
/// `UPDATE ... WHERE id` would no-op against the missing row). Folding the
/// truncate+reseed into one helper removes the copy-paste foot-gun where a test
/// truncates `hlc_state` but forgets the re-insert. Idempotent and safe to call after
/// `connect_and_load_schema`.
pub async fn reset_node_federation_tables(client: &Client) -> anyhow::Result<()> {
    client
        .batch_execute(
            "TRUNCATE node_event, local_node, sync_cursor, hlc_state;
             INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING;",
        )
        .await
        .map_err(|e| anyhow::anyhow!("resetting node-federation tables: {e}"))?;
    Ok(())
}

/// Test-support: a serialization guard for the DB-gated integration tests. They
/// share Postgres databases and each `TRUNCATE`s its tables on entry, so running
/// them concurrently — across test binaries OR within one binary — races. This
/// acquires a SESSION-level advisory lock on a fixed key; the returned `Client`
/// holds the lock until it is dropped at the end of the test (a panic still drops
/// it, releasing the lock). Every caller must lock against the SAME database
/// (`CAIRN_TEST_PG`) so the guard serializes regardless of whether the server
/// scopes advisory locks per-cluster or per-database. (PR #28 review follow-up.)
pub async fn test_serial_guard(conn: &str) -> anyhow::Result<Client> {
    let client = connect(conn).await?;
    // 0x4341524E = "CARN": a fixed project-specific key shared by every guard.
    client
        .execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64])
        .await
        .map_err(|e| anyhow::anyhow!("acquiring test serialization lock: {e}"))?;
    Ok(client)
}
