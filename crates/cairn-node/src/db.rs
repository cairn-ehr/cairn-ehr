use tokio_postgres::{Client, NoTls};

// A slice (not a fixed-size array) so appending a migration is a one-line change
// — the hand-counted length annotation bought nothing and taxed every migration.
const SCHEMA: &[(&str, &str)] = &[
    ("001_envelope", include_str!("../../../db/001_envelope.sql")),
    (
        "002_projection",
        include_str!("../../../db/002_projection.sql"),
    ),
    ("003_blobs", include_str!("../../../db/003_blobs.sql")),
    ("004_actors", include_str!("../../../db/004_actors.sql")),
    ("005_submit", include_str!("../../../db/005_submit.sql")),
    ("006_recall", include_str!("../../../db/006_recall.sql")),
    (
        "007_node_federation",
        include_str!("../../../db/007_node_federation.sql"),
    ),
    // NOTE: db/008_surrogate_projection.sql is INTENTIONALLY not loaded here. It is a
    // spike artefact (the ADR-0031 dense-bigint surrogate-key measurement, exercised on
    // Bet B), not part of the node's runtime schema — hence the 007 -> 009 jump. Leave
    // the gap; do not "fix" it by inserting 008. (Confirmed spike-only; see issue #67.)
    (
        "009_node_supersede_and_restore",
        include_str!("../../../db/009_node_supersede_and_restore.sql"),
    ),
    (
        "010_demographics",
        include_str!("../../../db/010_demographics.sql"),
    ),
    (
        "011_demographics_fields",
        include_str!("../../../db/011_demographics_fields.sql"),
    ),
    (
        "012_demographics_names",
        include_str!("../../../db/012_demographics_names.sql"),
    ),
    (
        "013_demographics_sex_gender",
        include_str!("../../../db/013_demographics_sex_gender.sql"),
    ),
    (
        "014_demographics_address",
        include_str!("../../../db/014_demographics_address.sql"),
    ),
    (
        "015_globalise_twin",
        include_str!("../../../db/015_globalise_twin.sql"),
    ),
    (
        "016_match_veto",
        include_str!("../../../db/016_match_veto.sql"),
    ),
    (
        "017_match_proposal",
        include_str!("../../../db/017_match_proposal.sql"),
    ),
    (
        "018_identity_linkage",
        include_str!("../../../db/018_identity_linkage.sql"),
    ),
    (
        "019_apply_proposal",
        include_str!("../../../db/019_apply_proposal.sql"),
    ),
    (
        "020_apply_remote_event",
        include_str!("../../../db/020_apply_remote_event.sql"),
    ),
    // Durable quarantine + re-offer floor for unverifiable pulled CLINICAL
    // events (issue #108): node-local operational state beside sync_state,
    // granted to cairn_node so the cairn-sync runtime can quarantine/requeue
    // without owner privileges.
    (
        "021_sync_quarantine",
        include_str!("../../../db/021_sync_quarantine.sql"),
    ),
    // The node-plane sibling (issue #111): the same durable-trace + re-offer
    // floor for a node_event the pull loop (sync.rs) refuses as UNVERIFIABLE.
    // Keyed off the seq-ordered node plane (derived floor = MIN(refused_seq)),
    // and a separate table so a node-plane requeue is unambiguously routed
    // through apply_remote_node_event, never the clinical door.
    (
        "022_node_event_quarantine",
        include_str!("../../../db/022_node_event_quarantine.sql"),
    ),
    // §5.7 identity `dispute` + the chart trust-state projection (C3): two additive
    // dispute event types through the reused submit_event door, a chart_dispute standing
    // overlay, and the chart_trust (confirmed / under-review) projection surfaced on
    // person_chart — the projection-side contract the rest of the §5.7 algebra composes into.
    (
        "023_identity_dispute",
        include_str!("../../../db/023_identity_dispute.sql"),
    ),
    // §5.4/§5.7 identity-pending + `identify` + the *unconfirmed* trust state (C4): two
    // additive event types through the reused submit_event door, a chart_identity_state
    // standing overlay keyed by subject, and the reworked chart_trust projection that
    // composes under-review (dispute) ⊔ unconfirmed (pending) by highest severity —
    // completing the §5.7 confirmed/unconfirmed/under-review contract C3 opened. Leaves
    // db/023 untouched (CREATE-OR-REPLACEs the shared twin hook + chart_trust view).
    (
        "024_identity_identify",
        include_str!("../../../db/024_identity_identify.sql"),
    ),
    // §5.5(a)/§5.7 `repudiate` + the known-alias pool (C5): the FIRST *suppressing* identity
    // event. A fabricated-persona name marked known-false is struck from the display winner
    // (patient_name_current anti-joins a new name_repudiation overlay) and surfaced to the
    // matcher as a reusable alias (patient_alias_pool) — a digital strike-through that leaves
    // the assertion event and db/012's retained set untouched. suppressing-mode forces the
    // db/005 human-attestation gate (§5.7 "Human"). Leaves db/010–024 untouched
    // (CREATE-OR-REPLACEs the shared twin hook + patient_name_current, same column contract).
    (
        "025_identity_repudiate",
        include_str!("../../../db/025_identity_repudiate.sql"),
    ),
    // The blob self-verification floor (ADR-0013 point 11): bytes that do not
    // BLAKE3-hash to the blob_address naming them can never sit present = TRUE —
    // in-DB via cairn_blob_verify (cairn_pgx >= 0.3.0), closing the honest gap
    // db/003 recorded (the check was previously an L2 promise in cairn-sync).
    (
        "026_blob_verify_floor",
        include_str!("../../../db/026_blob_verify_floor.sql"),
    ),
    // ADR-0042: the attachment reference nests under a rendition set; both submit (db/005)
    // and remote-apply (db/020) doors now learn a blob reference per rendition through this
    // one shared helper (PL/pgSQL is late-bound, so the doors above may reference it before
    // this migration defines it — all migrations load before any submit).
    (
        "027_attachment_rendition_references",
        include_str!("../../../db/027_attachment_rendition_references.sql"),
    ),
    (
        "028_identity_evidence",
        include_str!("../../../db/028_identity_evidence.sql"),
    ),
    // #157: the Byzantine HLC-triple collision advisory signal. Defines the shared
    // cairn_hlc_triple_collision predicate + the convergent hlc_collision_log + the never-gating
    // recorder; the five overlay triggers (db/002/018/023/024/025) call the recorder. PL/pgSQL is
    // late-bound, so those triggers may reference this file's functions before it loads — all
    // migrations load before any event is applied.
    (
        "029_hlc_collision_log",
        include_str!("../../../db/029_hlc_collision_log.sql"),
    ),
    // §5.4 node-local friendly John-Doe ordinal (display aid): a read-only VIEW ranking
    // each node's own callsign registrations, surfaced as "this node's John Doe #N" at
    // registration. The callsign identity string is untouched (partition-safety unchanged);
    // pure read-side, no floor/wire/event change.
    (
        "030_john_doe_local_ordinal",
        include_str!("../../../db/030_john_doe_local_ordinal.sql"),
    ),
    // §3.3 the first clinical-content surface: medication assert + cessation
    // floor, the medication_statement projection, and the assert-only current view.
    (
        "031_medication",
        include_str!("../../../db/031_medication.sql"),
    ),
    // §3.3 slice 2: medication dose change/correction floor + dose-timeline projection.
    (
        "032_medication_dose",
        include_str!("../../../db/032_medication_dose.sql"),
    ),
    // §3.3 slice 3 part 1: medication reconciliation/separation floor + twin
    // dispatch. Parts 2/3 (projection + view reworks) append to the SAME db/033 file
    // as separate BEGIN;/COMMIT; blocks — this include_str! auto-picks them up too.
    (
        "033_medication_reconciliation",
        include_str!("../../../db/033_medication_reconciliation.sql"),
    ),
    // §3.3 slice 4 part 1: medication attestation floor + set-commitment fn.
    // Part 2 (overlay/projection) is a follow-on task; this include_str! auto-picks
    // it up too once appended to the same db/034 file.
    (
        "034_medication_attestation",
        include_str!("../../../db/034_medication_attestation.sql"),
    ),
    // §3.3 slice 5: dose-correction per-field patch — effective/reason columns,
    // strike-aware floor + trigger, corrected-effective winner selection (ADR-0050).
    (
        "035_medication_dose_effective_correction",
        include_str!("../../../db/035_medication_dose_effective_correction.sql"),
    ),
    // The clinical-plane seq cursor (issue #196): event_log.seq +
    // sync_state.last_seq + sync_quarantine.refused_seq. Loaded here too because a
    // real node holds event_log; without it the clinical column is missing on a node.
    (
        "036_clinical_sync_seq",
        include_str!("../../../db/036_clinical_sync_seq.sql"),
    ),
    // ADR-0052 born-sealed custody plane: node_unwrap_key / event_dek / event_clear /
    // erasure_shred_log + erasure.shred.asserted registration (issues #189/#92).
    (
        "037_born_sealed",
        include_str!("../../../db/037_born_sealed.sql"),
    ),
    // The node's recorded schema generation (issue #188): the table behind the
    // downgrade-refusal guard in connect_and_load_schema below — the first brick of
    // the ADR-0012 code plane. The generation itself is the repo-wide
    // cairn_event::schema_generation::SCHEMA_GENERATION, not a property of this
    // list's tail; the unit test below pins that this FULL list really does carry
    // the repo's newest migration.
    (
        "038_node_schema",
        include_str!("../../../db/038_node_schema.sql"),
    ),
    // #208/ADR-0057: cairn_reproject + reproject_log + the event_type index. In
    // BOTH lists: each loader's gated heal step (generation change) calls it.
    (
        "039_projection_registry",
        include_str!("../../../db/039_projection_registry.sql"),
    ),
    // db/040 (issue #216): the grade-gated ceiling helpers + t_effective_ceiling_flag +
    // born clock-confidence grade. Must land in BOTH lists (this one and cairn-sync's) —
    // this is the full list the SCHEMA_GENERATION guard test below pins against, so a
    // node started fresh at generation 40 without this entry never loads the ceiling
    // helpers Tasks 4/5 depend on.
    (
        "040_clock_confidence_grade",
        include_str!("../../../db/040_clock_confidence_grade.sql"),
    ),
];

pub async fn connect(conn: &str) -> anyhow::Result<Client> {
    let (client, connection) = tokio_postgres::connect(conn, NoTls).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
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
        && role
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node')",
            &[],
        )
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

/// The schema generation this binary carries — the repo-wide constant, NOT a value
/// derived from this loader's own list. Both write doors (this full replay and
/// cairn-sync's deliberate SUBSET replay) must report the SAME generation for one
/// git build, and the subset may legitimately lag behind db/'s newest file — so a
/// per-list derivation would make the two doors disagree the moment a node-only
/// migration lands, and the #188 guard would start refusing healthy databases (see
/// `cairn_event::schema_generation` module docs for the full argument). Two guards
/// keep the constant honest: cairn-event's fs-derived test (constant == newest
/// db/*.sql) and this crate's unit test (this FULL list embeds that newest file).
pub fn embedded_schema_version() -> i32 {
    cairn_event::schema_generation::SCHEMA_GENERATION
}

/// Connect and replay every embedded migration — guarded against DOWNGRADE (#188).
///
/// The replay is idempotent for a database at or below this binary's generation. But
/// `CREATE OR REPLACE` cuts both ways: replayed by an OLDER binary against a NEWER
/// database it silently rewrites newer function bodies — including the in-DB
/// safety-floor checks — back to their older versions. So before replaying, read the
/// generation the last successful loader recorded (db/038 `node_schema`) and refuse
/// when it exceeds ours; after a successful replay, stamp our own. An absent table or
/// row means "generation unknown" (a pre-#188 database, or a rig loaded by hand via
/// psql) and the replay proceeds — the guard stops known downgrades, it does not lock
/// out hand-built rigs. First brick of the ADR-0012 code plane.
pub async fn connect_and_load_schema(conn: &str) -> anyhow::Result<Client> {
    let client = connect(conn).await?;
    let embedded = embedded_schema_version();
    // Serialize the whole check→replay→stamp against every OTHER loader on this
    // database (2026-07-19 review of PR #251, finding 1). Without it the guard is
    // check-then-act: an old and a new binary connecting together can interleave so
    // the old one reads a stale generation, passes, and still replays over the
    // schema the new one just loaded. BLOCKING and session-level: a concurrent
    // loader waits its turn, then reads the now-current record. Released explicitly
    // after the stamp — this client lives on as the daemon's connection, and a lock
    // held for its lifetime would park every later loader forever. Error paths drop
    // the client, and the session close releases the lock.
    client
        .execute(
            "SELECT pg_advisory_lock($1)",
            &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
        )
        .await
        .map_err(|e| anyhow::anyhow!("acquiring schema load-lock: {e}"))?;
    // Two round-trips, not one CASE: SQL references to node_schema are checked at
    // plan time, so a single statement naming the table errors on a database that
    // does not have it yet (fresh, or pre-#188) — exactly the databases that must
    // pass the guard.
    let table_exists: bool = client
        .query_one("SELECT to_regclass('public.node_schema') IS NOT NULL", &[])
        .await?
        .get(0);
    // query_opt: an absent ROW (never stamped — hand-loaded rig) is a legitimate
    // "generation unknown", but a real query error must still fail loudly.
    let recorded: Option<i32> = if table_exists {
        client
            .query_opt("SELECT version FROM node_schema", &[])
            .await?
            .map(|row| row.get(0))
    } else {
        None
    };
    if let Some(recorded) = recorded {
        if recorded > embedded {
            anyhow::bail!(
                "refusing to load schema: this database was last loaded at schema \
                 generation {recorded}, but this binary embeds only generation \
                 {embedded}. Replaying an older schema would silently downgrade the \
                 in-DB safety floor (issue #188 / ADR-0012). Upgrade this binary, or \
                 point it at a database of its own generation."
            );
        }
    }
    for (name, sql) in SCHEMA.iter() {
        client
            .batch_execute(sql)
            .await
            .map_err(|e| anyhow::anyhow!("loading {name}: {e}"))?;
    }
    // #208/ADR-0057: heal replay on generation CHANGE only, and BEFORE the stamp
    // below. New projection capability (and any projection-logic fix) arrives only
    // via a code-plane update — i.e. a generation change — so an unchanged
    // generation means there is nothing to heal and the connect path does zero
    // reprojection work (the old db/013 every-connect backfill is retired by this
    // branch's demographics conversion). An UNKNOWN recorded generation (fresh DB:
    // free no-op; hand-built rig: converges once) errs toward healing. Runs inside
    // SCHEMA_LOAD_LOCK: concurrent loaders serialize, and the second sees the
    // stamped generation.
    //
    // Ordered BEFORE the stamp deliberately: if the heal query below errors, the
    // stamp never runs, so the recorded generation stays at its OLD (pre-upgrade)
    // value and the `?` propagates the failure up to the caller. The NEXT connect
    // attempt then sees the same stale `recorded`, so it retries the FULL
    // replay-then-heal — exactly the loud, self-retrying failure mode a broken
    // migration file already has in this loader (a bad `db/*.sql` blocks connect
    // until fixed; it never silently half-applies). Stamp-then-heal would invert
    // this: a heal failure AFTER the stamp leaves the generation already advanced,
    // so the next connect reads `recorded == embedded`, skips the heal entirely,
    // and the projections stay SILENTLY stale — the worst failure mode, and the
    // reason this order is load-bearing, not cosmetic.
    if recorded != Some(embedded) {
        client
            .execute(
                "SELECT count(*) FROM cairn_reproject('', false, 'loader')",
                &[],
            )
            .await
            .map_err(|e| anyhow::anyhow!("post-upgrade heal replay: {e}"))?;
    }
    // Stamp only AFTER the full replay (and any heal above) succeeded: a
    // half-applied load must not claim the new generation. loaded_at defaults to
    // now() on insert; refresh it on conflict so the record always says who
    // touched the schema last, and when.
    client
        .execute(
            "INSERT INTO node_schema (version, loader_build) VALUES ($1, $2)
             ON CONFLICT (id) DO UPDATE
               SET version = EXCLUDED.version,
                   loaded_at = now(),
                   loader_build = EXCLUDED.loader_build",
            &[
                &embedded,
                &concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")),
            ],
        )
        .await
        .map_err(|e| anyhow::anyhow!("recording schema generation: {e}"))?;
    client
        .execute(
            "SELECT pg_advisory_unlock($1)",
            &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
        )
        .await
        .map_err(|e| anyhow::anyhow!("releasing schema load-lock: {e}"))?;
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
            "TRUNCATE node_event, local_node, sync_cursor, hlc_state, node_event_quarantine;
             INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING;",
        )
        .await
        .map_err(|e| anyhow::anyhow!("resetting node-federation tables: {e}"))?;
    Ok(())
}

/// Tick the node HLC once (`node_hlc_tick`, the same door node authoring uses) and stamp
/// `node_origin`. Authoring is single-threaded on a node, so a tick->sign->submit per event
/// is safe. The single home for every in-node authoring path — auto_apply's C2b link and
/// john_doe's §5.4 registration both call this, rather than each re-writing the round-trip.
pub async fn next_hlc(client: &Client, node_origin: &str) -> anyhow::Result<cairn_event::Hlc> {
    let row = client
        .query_one("SELECT wall, counter FROM node_hlc_tick()", &[])
        .await?;
    Ok(cairn_event::Hlc {
        wall: row.get(0),
        counter: row.get(1),
        node_origin: node_origin.into(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The FULL loader list must actually carry the repo's newest migration. The
    /// generation this binary stamps is the shared SCHEMA_GENERATION constant, so if
    /// db/039_*.sql lands (cairn-event's fs guard forces the constant bump) but is
    /// never appended to THIS list, the node would stamp generation 39 while
    /// replaying only generation-38 bodies — a silent gap the stamp then papers
    /// over. Completeness, not position: the max prefix anywhere in the list, so a
    /// mis-ordered append still counts.
    #[test]
    fn full_schema_list_carries_the_repo_generation() {
        let newest = cairn_event::schema_generation::newest_migration_prefix(
            SCHEMA.iter().map(|(name, _)| *name),
        )
        .expect("SCHEMA is never empty and every entry has a numeric prefix");
        assert_eq!(
            newest,
            embedded_schema_version(),
            "cairn-node's FULL migration list ends at {newest} but the repo's schema \
             generation is {}: append the new db/*.sql to SCHEMA in the same commit \
             that bumps SCHEMA_GENERATION",
            embedded_schema_version()
        );
    }
}
