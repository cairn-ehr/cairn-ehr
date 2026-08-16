//! #426 — a pinned `search_path` must place `pg_temp` LAST, or it pins nothing.
//!
//! # The mechanism, in one paragraph
//!
//! `SET search_path = public` reads like a lockdown and is not one. PostgreSQL searches the
//! session's **temporary** schema *first* for relation names whenever the path does not name
//! `pg_temp` explicitly — the setting only controls where `pg_temp` sits when you DO name it.
//! Any role holding `TEMPORARY` on the database (PUBLIC's default; nothing in `db/*.sql`
//! revokes it) can therefore `CREATE TEMP TABLE event_log (…)` and have every unqualified
//! `event_log` inside a definer body resolve to their own decoy. PostgreSQL's own *"Writing
//! SECURITY DEFINER Functions Safely"* calls this out and prescribes writing `pg_temp` last.
//!
//! # Why this file exists rather than another assertion in `safety_read_grants.rs`
//!
//! That file pins the property for the two functions where it was first found (`db/049`'s
//! safety readers, 2026-08-16). The defect was never specific to them: it was the house
//! spelling of the clause, repeated at ~25 sites, including the two **owner-rights write
//! doors** — `submit_event` and `apply_remote_event`. A per-function assertion cannot catch
//! the twenty-sixth site, so the guard here is written over the CATALOGUE: whatever a future
//! migration adds, it is covered the moment it loads.
//!
//! The two behavioural tests are not redundant with the catalogue ones. A catalogue assertion
//! proves the clause is spelled correctly; it does not prove that spelling it correctly is
//! what keeps a hostile caller from diverting a clinical write. That claim is worth
//! demonstrating once, against the real door, with the real attack.
//!
//! Every test self-skips without `$CAIRN_TEST_PG` (see `common::cs`) — a skipped run prints
//! `ok` and proves nothing. CI sets it; `scripts/run-db-gated-tests.sh` bakes it in locally.
mod common;
use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{sign, EventBody, SigningKey};
use common::{body_from_spec, cs, setup, submit_signed_with_id, EventSpec};
use tokio_postgres::Client;
use uuid::Uuid;

/// Functions this repo defines, with the `search_path` each one pins, as
/// `(signature, setting)`.
///
/// Extension-owned functions are excluded (`pg_depend … deptype = 'e'`): `cairn_pgx` and
/// `pgcrypto` install into `public` too, and this repo's invariant is not theirs to satisfy.
/// `prokind` is left unfiltered on purpose — a future PROCEDURE with a pinned path is
/// shadowable in exactly the same way and should be caught here, not exempted.
const PINNED_PATHS: &str = "
    SELECT p.oid::regprocedure::text, cfg
      FROM pg_proc p
      JOIN pg_namespace n ON n.oid = p.pronamespace
      CROSS JOIN LATERAL unnest(p.proconfig) AS cfg
     WHERE n.nspname = 'public'
       AND cfg LIKE 'search_path=%'
       AND NOT EXISTS (SELECT 1 FROM pg_depend d
                        WHERE d.objid = p.oid AND d.deptype = 'e')
     ORDER BY 1";

/// Every `SECURITY DEFINER` function this repo defines, and whether it pins a path at all.
const DEFINERS: &str = "
    SELECT p.oid::regprocedure::text,
           coalesce((SELECT true FROM unnest(p.proconfig) c WHERE c LIKE 'search_path=%'), false)
      FROM pg_proc p
      JOIN pg_namespace n ON n.oid = p.pronamespace
     WHERE n.nspname = 'public'
       AND p.prosecdef
       AND NOT EXISTS (SELECT 1 FROM pg_depend d
                        WHERE d.objid = p.oid AND d.deptype = 'e')
     ORDER BY 1";

/// The trailing element of a `search_path=…` proconfig entry, lower-cased and trimmed.
///
/// Kept as a pure function of the setting string so the rule under test — "`pg_temp` is the
/// LAST element" — is stated once and reads the same way at both call sites. Postgres
/// stores the setting verbatim as written in the DDL, so `public, pg_temp` and
/// `public,pg_temp` are both possible spellings of the same safe path; only the ORDER is
/// load-bearing, never the whitespace.
fn last_element(setting: &str) -> String {
    setting
        .trim_start_matches("search_path=")
        .rsplit(',')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// The repo-wide rule: a pinned path that omits `pg_temp` leaves the temp schema FIRST, and a
/// pinned path that names it anywhere but last puts it AHEAD of `public` — both are the same
/// hole. Only "last" is safe.
///
/// Asserted over the whole catalogue rather than a hand-kept list of function names: a list
/// is a thing to forget to update, and forgetting here is silent.
#[tokio::test]
async fn every_pinned_search_path_places_pg_temp_last() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rows = c.query(PINNED_PATHS, &[]).await.unwrap();

    // A guard against the guard: if the query stops matching (a schema rename, a migration
    // that never loaded), "no offenders" would otherwise be indistinguishable from "clean".
    assert!(
        rows.len() > 20,
        "expected the schema's ~25 pinned functions, found {} — has the migration set \
         failed to load, or the catalogue query gone stale?",
        rows.len()
    );

    let offenders: Vec<String> = rows
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
        .filter(|(_, cfg)| last_element(cfg) != "pg_temp")
        .map(|(sig, cfg)| format!("  {sig} → {cfg}"))
        .collect();

    assert!(
        offenders.is_empty(),
        "these functions pin a search_path that does not end in pg_temp, so any caller \
         holding TEMPORARY can shadow an unqualified relation inside them (#426):\n{}\n\
         Fix: append `, pg_temp` LAST to the SET clause in db/*.sql.",
        offenders.join("\n")
    );
}

/// The paired half: a definer with NO pinned path is strictly worse than one pinning the old
/// spelling — it resolves every name in the CALLER's path, temp schema and all.
///
/// This holds today (every `SECURITY DEFINER` in `db/*.sql` pins one), so it is a ratchet,
/// not a fix: it stops the next definer from shipping without the clause. Deliberately says
/// nothing about invoker-rights functions — whether `cairn_sensitivity_standing` should pin
/// one is #420's open question, and it carries a measured inlining cost this test must not
/// pre-empt.
#[tokio::test]
async fn every_security_definer_pins_a_search_path() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rows = c.query(DEFINERS, &[]).await.unwrap();
    assert!(
        rows.len() > 10,
        "expected the schema's ~15 SECURITY DEFINER functions, found {} — stale catalogue \
         query?",
        rows.len()
    );

    let unpinned: Vec<String> = rows
        .iter()
        .filter(|r| !r.get::<_, bool>(1))
        .map(|r| format!("  {}", r.get::<_, String>(0)))
        .collect();

    assert!(
        unpinned.is_empty(),
        "these SECURITY DEFINER functions pin no search_path at all, so every unqualified \
         name in them resolves under the CALLER's path with the definer's privilege:\n{}",
        unpinned.join("\n")
    );
}

/// The 21 columns both write doors name in their `INSERT INTO event_log (…)`, as a decoy any
/// caller may create — no privilege on the real table is needed to build it, which is the
/// whole point.
///
/// `event_id` carries the PRIMARY KEY because both doors insert `ON CONFLICT (event_id) DO
/// NOTHING`; without a matching unique index the diverted insert would fail loudly (42P10)
/// and the test would "pass" on an error rather than on the fix.
const DECOY: &str = "CREATE TEMP TABLE event_log (
        event_id uuid PRIMARY KEY, patient_id uuid, event_type text, schema_version text,
        hlc_wall bigint, hlc_counter integer, node_origin text, t_effective timestamptz,
        signed_bytes bytea, content_address bytea, body jsonb, contributors jsonb,
        signer_key_id text, plaintext_twin text, attachments jsonb, attestation bytea,
        attester_key bytea, actor_id bytea, sealed boolean, clock_grade text, safety jsonb)";

/// A registration [`EventSpec`] for `p` — the chart's birth act, and the one event type that
/// is always admissible on a fresh chart (the §5.3 precedence rule, #345).
///
/// Built here rather than through `common::submit_registration` because these two tests need
/// the door's RAW result: under the pre-fix schema the diverted write may either succeed
/// (into the decoy) or fail, and a helper that `.expect()`s success cannot tell those apart
/// in its failure message.
fn registration_spec(p: Uuid, tokens: &[String]) -> EventSpec<'_> {
    let a = RegistrationAssertion {
        class: RegistrationClass::Standard,
        basis: None,
        search: Some(SearchAttestationInput {
            terms: SearchTerms {
                name_tokens: tokens,
                birth_date: None,
                identifiers: &[],
            },
            displayed: &[],
            incomplete: false,
        }),
    };
    EventSpec {
        patient: p,
        event_type: REGISTRATION_EVENT_TYPE,
        schema_version: REGISTRATION_SCHEMA_VERSION,
        payload: registration_assertion_body(&a),
        plaintext_twin: Some(render_registration_twin(&a)),
        wall: 1,
    }
}

/// Read back what landed where, then clear the decoy: `(rows in public.event_log for this
/// event, rows in the decoy)`.
///
/// Schema-qualified on both sides deliberately — an unqualified count from the test session
/// would itself be answered by the decoy while it exists, which is the failure mode under
/// test and would make the assertion agree with the bug.
async fn landed(c: &Client, event_id: Uuid) -> (i64, i64) {
    let real: i64 = c
        .query_one(
            "SELECT count(*) FROM public.event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    let decoy: i64 = c
        .query_one("SELECT count(*) FROM pg_temp.event_log", &[])
        .await
        .unwrap()
        .get(0);
    c.batch_execute("DROP TABLE pg_temp.event_log")
        .await
        .unwrap();
    (real, decoy)
}

/// The LOCAL write door, under a caller-created decoy.
///
/// This is the serious direction of #426 and the reason it is not merely hygiene:
/// `submit_event` is `SECURITY DEFINER` owned by the migration role, so the diverted
/// `INSERT` runs with the OWNER's privilege against an attacker-controlled table. The
/// clinician's client sees the door return an event id — the write reported success — while
/// the record went to a table that vanishes when the session ends, the append-only log never
/// saw it, and no projection trigger fired. Silent clinical data loss, from a role holding
/// nothing but `EXECUTE` on the door and the `TEMPORARY` every role has by default.
#[tokio::test]
async fn a_shadowed_event_log_cannot_divert_the_local_write_door() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &[]).await;

    let p = Uuid::now_v7();
    let event_id = Uuid::now_v7();
    let tokens = [p.to_string()];

    // As the runtime role, exactly as a hostile client would arrive: no privilege beyond
    // EXECUTE on the door, and the TEMPORARY that PUBLIC holds on every database.
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    c.batch_execute(DECOY)
        .await
        .expect("any role may create temp tables — that is what makes this reachable");

    let submitted =
        submit_signed_with_id(&c, &sk, &kid, event_id, registration_spec(p, &tokens)).await;

    c.batch_execute("RESET ROLE").await.unwrap();
    let (real, decoy) = landed(&c, event_id).await;

    let refusal = submitted
        .as_ref()
        .err()
        .map(common::db_msg)
        .unwrap_or_default();
    assert!(
        submitted.is_ok(),
        "the door must behave identically with a decoy present; it errored: {refusal}"
    );
    assert_eq!(
        real, 1,
        "submit_event returned success but the event is NOT in public.event_log — the \
         owner-rights INSERT was diverted into the caller's temp schema (#426). This is \
         silent clinical data loss: the client was told the write succeeded."
    );
    assert_eq!(decoy, 0, "nothing may reach the decoy at all");
}

/// The REMOTE apply door, same decoy.
///
/// It needs its own test rather than trust in the local one: it is a separately-declared
/// definer in a different migration (`db/020`) with its own `SET` clause, and it is the door
/// a *peer's* events arrive through — a diversion here loses replicated records with no
/// local actor to notice, and leaves the node's sync watermark claiming events it does not
/// hold.
///
/// Reached as `cairn_node`, because `EXECUTE` on this door is granted to that role alone
/// (`db/020`), not to `cairn_agent`. Be precise about what that means for the threat model:
/// unlike the local door, this one is not reachable by any enrolled writer, so the shadow
/// needs a compromised — or merely buggy — sync daemon session rather than a hostile client.
/// That makes it defence in depth, and it is still worth having: `cairn_node` is a role the
/// product hands to a long-lived process, and the failure it prevents (replicated records
/// silently discarded while the door reports success) is unrecoverable and unalarmed.
#[tokio::test]
async fn a_shadowed_event_log_cannot_divert_the_remote_apply_door() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &[]).await;

    let p = Uuid::now_v7();
    let event_id = Uuid::now_v7();
    let tokens = [p.to_string()];
    let body: EventBody = body_from_spec(event_id, &kid, registration_spec(p, &tokens));
    let signed = sign(&body, &sk as &SigningKey).unwrap();

    c.batch_execute("SET ROLE cairn_node").await.unwrap();
    c.batch_execute(DECOY).await.unwrap();
    let applied = c
        .execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await;
    c.batch_execute("RESET ROLE").await.unwrap();
    let (real, decoy) = landed(&c, event_id).await;

    let refusal = applied
        .as_ref()
        .err()
        .map(common::db_msg)
        .unwrap_or_default();
    assert!(
        applied.is_ok(),
        "the remote door must behave identically with a decoy present; it errored: {refusal}"
    );
    assert_eq!(
        real, 1,
        "apply_remote_event returned success but the event is NOT in public.event_log — a \
         replicated record was diverted into a temp table and lost (#426)"
    );
    assert_eq!(decoy, 0, "nothing may reach the decoy at all");
}
