use tokio_postgres::{Client, NoTls};

const SCHEMA: [(&str, &str); 7] = [
    ("001_envelope",      include_str!("../../../db/001_envelope.sql")),
    ("002_projection",    include_str!("../../../db/002_projection.sql")),
    ("003_blobs",         include_str!("../../../db/003_blobs.sql")),
    ("004_actors",        include_str!("../../../db/004_actors.sql")),
    ("005_submit",        include_str!("../../../db/005_submit.sql")),
    ("006_recall",        include_str!("../../../db/006_recall.sql")),
    ("007_node_federation", include_str!("../../../db/007_node_federation.sql")),
];

pub async fn connect(conn: &str) -> anyhow::Result<Client> {
    let (client, connection) = tokio_postgres::connect(conn, NoTls).await?;
    tokio::spawn(async move { let _ = connection.await; });
    Ok(client)
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
pub async fn provision_runtime_role(client: &Client, role: &str) -> anyhow::Result<()> {
    // Identifiers cannot be passed as bind parameters, so this name is interpolated
    // into DDL. Reject anything but a conservative identifier charset to close the
    // SQL-injection door rather than trusting the caller (defence in depth — the
    // CLI also constrains it). PostgreSQL identifiers are <= 63 bytes.
    if role.is_empty()
        || role.len() > 63
        || !role.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        || !role.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
    {
        anyhow::bail!(
            "invalid runtime role name {role:?}: use lowercase letters, digits, and underscores \
             (starting with a letter or underscore), max 63 chars"
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
