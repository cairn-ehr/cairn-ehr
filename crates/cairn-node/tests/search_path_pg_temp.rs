//! #426 — a pinned `search_path` must place `pg_temp` LAST, or it pins nothing.
//!
//! # The mechanism, in one paragraph
//!
//! `SET search_path = public` reads like a lockdown and is not one. PostgreSQL searches the
//! session's **temporary** schema *first* for relation and data-type names whenever the path
//! does not name `pg_temp` explicitly — the setting only controls where `pg_temp` sits when you
//! DO name it. Any role holding `TEMPORARY` on the database (PUBLIC's default; nothing in
//! `db/*.sql` revokes it) can therefore `CREATE TEMP TABLE event_log (…)` and have every
//! unqualified `event_log` inside a definer body resolve to their own decoy. PostgreSQL's own
//! *"Writing SECURITY DEFINER Functions Safely"* calls this out and prescribes `pg_temp` last.
//!
//! Function and operator names are NOT part of this: Postgres never searches the temp schema
//! for them, named or not. Do not re-derive a hazard there — `db/001`'s house-rule note carries
//! the verification, and the residual that IS real (a name `public` does not have).
//!
//! # Why this file exists rather than another assertion in `safety_read_grants.rs`
//!
//! That file pins the *behaviour* for the two functions where it was first found (`db/049`'s
//! safety readers, 2026-08-16); `db/tests/049` pins their exact `proconfig`. The defect was
//! never specific to them: it was the house spelling of the clause, carried by 21 of the 25
//! pinned sites in `db/*.sql` — including the two **owner-rights write doors**, `submit_event`
//! and `apply_remote_event`. A per-function assertion cannot catch the twenty-sixth site, so
//! the guard here is written over the CATALOGUE: whatever a future migration adds, it is
//! covered the moment it loads.
//!
//! The catalogue is the *database*, not `db/*.sql`, so it covers only what the loader replays
//! (`cairn_node::db::SCHEMA`). DDL that lives in test fixtures is outside it — `db/tests/048`
//! and `claim_authority.rs` each plant a definer and had to be updated by hand in #426. Both
//! restore the real definition afterwards, so neither can persist a bad spelling.
//!
//! The three behavioural tests are not redundant with the catalogue ones. A catalogue assertion
//! proves the clause is spelled correctly; it does not prove that spelling it correctly is
//! what keeps a hostile caller from diverting a clinical write. That claim is worth
//! demonstrating against the real doors, with the real attack — once per door, plus once for
//! the seeded variant that reaches past the `#345` precedence check to an ordinary clinical
//! write (see `a_seeded_decoy_cannot_divert_a_later_clinical_write` for why that matters).
//!
//! Every DB-backed test here self-skips without `$CAIRN_TEST_PG` (see `common::cs`) — a skipped
//! run prints `ok` and proves nothing. CI sets it; `scripts/run-db-gated-tests.sh` bakes it in
//! locally. The one exception is deliberate: [`the_rule_accepts_only_paths_that_deny_pg_temp_the_first_look`]
//! is a plain `#[test]` over strings, so the rule itself is exercised even on a machine with no
//! PostgreSQL — otherwise the whole file could go green having executed nothing.
//!
//! # Why there is no `db/tests/` SQL mirror
//!
//! A pure-SQL `DO $$ … ASSERT` twin would run this in the throwaway-DB pass too, which is real
//! coverage. It is deliberately NOT written: the floors below (`PINNED_TODAY`, `DEFINERS_TODAY`)
//! are counts, and this repo has already been bitten repeatedly by counts pinned in two places
//! that drift apart. One home for the rule, exercised in the gate that runs the whole workspace,
//! beats two homes that disagree.
mod common;
use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{sign, EventBody, SigningKey};
use common::{
    body_from_spec, cs, setup, submit_registration, submit_signed_with_id, EventSpec,
    NOT_EXTENSION_OWNED, REPO_SCHEMAS,
};
use tokio_postgres::Client;
use uuid::Uuid;

// `REPO_SCHEMAS` / `NOT_EXTENSION_OWNED` — "which functions is this repo answerable for?" —
// moved to `common/mod.rs` when `floor_execute_grants.rs` (#382) became the second guard
// needing them. A catalogue filter copied into two suites diverges silently: the narrowed
// copy reports "no offenders" rather than failing. See that module for the full reasoning.

/// Functions this repo defines, with the `search_path` each one pins, as
/// `(signature, setting)`.
///
/// `prokind` is left unfiltered on purpose — a future PROCEDURE with a pinned path is
/// shadowable in exactly the same way and should be caught here, not exempted.
fn pinned_paths() -> String {
    format!(
        "SELECT p.oid::regprocedure::text, cfg
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
           CROSS JOIN LATERAL unnest(p.proconfig) AS cfg
          WHERE {REPO_SCHEMAS}
            AND cfg LIKE 'search_path=%'
            AND {NOT_EXTENSION_OWNED}
          ORDER BY 1"
    )
}

/// Every `SECURITY DEFINER` function this repo defines, and whether it pins a path at all.
fn definers() -> String {
    format!(
        "SELECT p.oid::regprocedure::text,
                coalesce((SELECT true FROM unnest(p.proconfig) c
                           WHERE c LIKE 'search_path=%'), false)
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
          WHERE {REPO_SCHEMAS}
            AND p.prosecdef
            AND {NOT_EXTENSION_OWNED}
          ORDER BY 1"
    )
}

/// The elements of a `search_path=…` proconfig entry, lower-cased and trimmed.
///
/// Postgres CANONICALISES this setting rather than storing the DDL spelling: `= public,pg_temp`,
/// `TO public, pg_temp` and `= "public", pg_temp` all store `search_path=public, pg_temp`.
/// (That is also why `db/tests/049`'s exact-string `proconfig` assertion is sound, not fragile
/// — do not weaken it on the theory that a no-space edit could slip past.) Parsing here is
/// therefore whitespace-tolerant only as belt and braces; the ORDER is what is load-bearing.
fn path_elements(setting: &str) -> Vec<String> {
    let body = setting.trim_start_matches("search_path=");
    // `SET search_path = ''` canonicalises to the two-character token `""` — one empty path,
    // not one element named `""`.
    if body.trim() == "\"\"" {
        return Vec::new();
    }
    body.split(',')
        .map(|e| e.trim().to_ascii_lowercase())
        .collect()
}

/// Does this pinned path actually deny a caller's temp schema the first look at a relation?
///
/// Three ways to spell it wrong, all of which the bare-`last element is pg_temp` test would
/// have waved through or wrongly condemned:
///
/// * `public` — omits `pg_temp`, so Postgres searches the temp schema FIRST. The #426 defect.
/// * `pg_temp, public` — names it, but AHEAD of `public`. Same hole, stated explicitly.
/// * `pg_temp` alone — the worst spelling there is (every unqualified relation resolves in the
///   caller's temp schema and nowhere else), yet its LAST element is `pg_temp`. Requiring a
///   second element is what refuses it.
///
/// And one way to spell it *better* than the house rule, which must not be reported as a
/// violation: `SET search_path = ''` searches nothing implicitly, so there is no temp schema
/// to lose a race in. `db/001`'s house-rule note names this as the airtight end-state; a guard
/// that forbade the very hardening it recommends would be a trap for whoever attempts it.
fn pins_pg_temp_safely(setting: &str) -> bool {
    let elements = path_elements(setting);
    match elements.len() {
        0 => true,  // `SET search_path = ''` — nothing implicit, nothing to shadow.
        1 => false, // `public` alone, or `pg_temp` alone.
        n => elements[n - 1] == "pg_temp" && !elements[..n - 1].contains(&"pg_temp".to_string()),
    }
}

/// The predicate's spec, as executable examples.
///
/// These are deliberately NOT `#[tokio::test]` and touch no database: every other test in this
/// file self-skips without `$CAIRN_TEST_PG`, which would leave the rule itself unexercised on
/// a developer machine that has no PG. This is the one part that always runs.
#[test]
fn the_rule_accepts_only_paths_that_deny_pg_temp_the_first_look() {
    // The house spelling, and the whitespace variants Postgres would have canonicalised away.
    assert!(pins_pg_temp_safely("search_path=public, pg_temp"));
    assert!(pins_pg_temp_safely("search_path=public,pg_temp"));
    assert!(pins_pg_temp_safely(
        "search_path=\"$user\", public, pg_temp"
    ));
    // The airtight form is better than the house rule, not a violation of it.
    assert!(pins_pg_temp_safely("search_path=\"\""));

    // The #426 defect itself.
    assert!(!pins_pg_temp_safely("search_path=public"));
    // Named, but ahead of public — the subtle ordering mutation HANDOVER claims is caught.
    assert!(!pins_pg_temp_safely("search_path=pg_temp, public"));
    // Last element IS pg_temp, and it is still the worst possible path.
    assert!(!pins_pg_temp_safely("search_path=pg_temp"));
    // Named twice: the earlier mention is what wins, so the trailing one is decoration.
    assert!(!pins_pg_temp_safely("search_path=pg_temp, public, pg_temp"));
}

/// What the loaded schema holds today (2026-08-16), as floors rather than equalities — see
/// the `>=` reasoning at each use.
const PINNED_TODAY: usize = 25;
const DEFINERS_TODAY: usize = 15;

/// The repo-wide rule: a pinned path that omits `pg_temp` leaves the temp schema FIRST, and a
/// pinned path that names it anywhere but last puts it AHEAD of `public` — both are the same
/// hole. See [`pins_pg_temp_safely`] for the full set of spellings and why each is judged.
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

    let rows = c.query(&pinned_paths(), &[]).await.unwrap();

    // A guard against the guard: if the query stops matching (a schema rename, a migration
    // that never loaded), "no offenders" would otherwise be indistinguishable from "clean".
    // `>=` against today's exact count, not `>` against a round number: the count only ever
    // RISES as migrations add functions, so this never needs revising upward — but it now
    // trips the moment coverage shrinks.
    assert!(
        rows.len() >= PINNED_TODAY,
        "expected at least the schema's {PINNED_TODAY} pinned functions, found {} — has the \
         migration set failed to load, or the catalogue query gone stale?",
        rows.len()
    );

    let offenders: Vec<String> = rows
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
        .filter(|(_, cfg)| !pins_pg_temp_safely(cfg))
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
/// not a fix: it stops the next definer from shipping without the clause.
///
/// Deliberately says nothing about INVOKER-rights functions, of which ~100 pin no path at all.
/// That is not an oversight and not a small residual — it is #420's open question, and it is
/// a different trade: an invoker function runs with the CALLER's own privilege, so shadowing
/// it grants no privilege the caller lacked, and adding the clause blocks SQL inlining on hot
/// read paths. The sharp edge in that class is a function reached only from inside a pinned
/// definer (`cairn_patient_has_events`, the #345 precedence check), which is safe purely by
/// INHERITING its caller's path — an invariant nothing here guards. See #430.
#[tokio::test]
async fn every_security_definer_pins_a_search_path() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rows = c.query(&definers(), &[]).await.unwrap();
    assert!(
        rows.len() >= DEFINERS_TODAY,
        "expected at least the schema's {DEFINERS_TODAY} SECURITY DEFINER functions, found \
         {} — stale catalogue query?",
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
///
/// This list is hand-kept and must stay a superset of what the doors insert, so it is PINNED
/// against the catalogue by [`the_decoy_still_mirrors_every_column_the_doors_insert`] — see
/// there for why drift here is silent and dangerous rather than merely untidy.
const DECOY: &str = "CREATE TEMP TABLE event_log (
        event_id uuid PRIMARY KEY, patient_id uuid, event_type text, schema_version text,
        hlc_wall bigint, hlc_counter integer, node_origin text, t_effective timestamptz,
        signed_bytes bytea, content_address bytea, body jsonb, contributors jsonb,
        signer_key_id text, plaintext_twin text, attachments jsonb, attestation bytea,
        attester_key bytea, actor_id bytea, sealed boolean, clock_grade text, safety jsonb)";

/// `event_log` columns the write doors deliberately do NOT name, each for a stated reason.
///
/// Every other column of `event_log` must appear in [`DECOY`]. Keeping the exemptions explicit
/// is what lets the guard below be an exact equality instead of a "contains at least" — which
/// would not have caught the drift it exists to catch.
const NOT_INSERTED_BY_THE_DOORS: [&str; 3] = [
    "seq",         // GENERATED ALWAYS AS IDENTITY — the doors cannot name it.
    "recorded_at", // DEFAULT now() — the objective §3.6 ceiling, never client-supplied.
    "dek_wrapped", // Written by the born-sealed path (db/037), not by the INSERT itself.
];

/// The decoy is a hand-copied mirror of the doors' column list, and a stale mirror fails
/// SILENTLY in the direction that matters.
///
/// Drop a column the doors name and the post-fix tests still pass green (`real=1, decoy=0`) —
/// because a correctly-pinned door never writes to the decoy at all, so nothing exercises it.
/// The loss only shows up if the fix is ever regressed, at which point the diverted INSERT
/// fails with `42703 undefined column` and the failure message blames the *door* rather than
/// naming the diversion. The likely reading then is "the decoy is stale, delete it", which
/// discards the regression test at the exact moment it was right.
///
/// So pin it: when a migration adds a column to `event_log`, this fails and forces the choice
/// — add it to `DECOY`, or justify it in [`NOT_INSERTED_BY_THE_DOORS`]. Same discipline as
/// `safety_read_grants.rs`'s `GRANTED_COLUMNS`, for the same reason.
///
/// Be exact about what this does and does not pin, rather than let the name imply more. It
/// pins `DECOY` against **`event_log`'s columns**, minus an explicit exemption list — not
/// against the doors' `INSERT` lists, which no catalogue exposes. So it catches the drift that
/// actually happens (a migration adds a column and the doors start writing it) and does NOT
/// catch a door dropping a column it already names. That residual is small and one-directional:
/// a dropped column leaves `DECOY` a superset, which still captures a diverted write.
#[tokio::test]
async fn the_decoy_still_mirrors_every_column_the_doors_insert() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let mut expected: Vec<String> = c
        .query(
            "SELECT attname::text FROM pg_attribute
              WHERE attrelid = 'public.event_log'::regclass
                AND attnum > 0 AND NOT attisdropped
                AND attname <> ALL($1)",
            &[&NOT_INSERTED_BY_THE_DOORS.map(String::from).to_vec()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();
    expected.sort();

    // Parse DECOY's declared column names back out — the first word of each comma-separated
    // declaration inside the parentheses.
    let open = DECOY.find('(').expect("DECOY declares columns");
    let mut declared: Vec<String> = DECOY[open + 1..DECOY.rfind(')').unwrap()]
        .split(',')
        .filter_map(|d| d.split_whitespace().next())
        .map(str::to_string)
        .collect();
    declared.sort();

    assert_eq!(
        declared, expected,
        "the DECOY no longer mirrors the columns the write doors INSERT into event_log. \
         Add the new column to DECOY, or — if the doors genuinely do not name it — add it to \
         NOT_INSERTED_BY_THE_DOORS with the reason. Leaving it stale silently turns the \
         behavioural tests in this file into no-ops (#426)."
    );
}

/// A registration [`EventSpec`] for `p` — the chart's birth act, and the one event type that
/// is always admissible on a fresh chart (the §5.3 precedence rule, #345).
///
/// Built here rather than through `common::submit_registration` because these two tests need
/// the door's RAW result: a helper that `.expect()`s success reports "the door refused" for
/// both a refusal and a diversion, which are the two outcomes these tests must distinguish.
///
/// Registration is also the type that makes the LOCAL door's diversion silent with an empty
/// decoy, and that is worth naming rather than leaving as luck. `submit_event` step 8b calls
/// `cairn_patient_has_events` — itself an unqualified `event_log` read — but short-circuits
/// for registration (a chart's first event cannot require the chart to exist). For any OTHER
/// type on a registered chart, the empty decoy blinds 8b and the door RAISES "no chart exists"
/// instead: loud, not silent. Silent diversion of an arbitrary type needs a decoy SEEDED with
/// a row bearing the patient_id — see `a_seeded_decoy_cannot_divert_a_later_clinical_write`.
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
/// saw it, and no projection trigger fired. Silent clinical data loss, from a role holding **no
/// write privilege on `event_log` whatsoever** (`db/005` REVOKEs INSERT/UPDATE/DELETE from
/// `cairn_agent`) — only `EXECUTE` on the door and the `TEMPORARY` every role has by default.
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

/// The same diversion against an ORDINARY clinical write, not a chart's birth act.
///
/// Why this is not redundant with the test above. The local door's step 8b (`#345` precedence)
/// calls `cairn_patient_has_events`, which reads `event_log` unqualified — so an EMPTY decoy
/// blinds it and any non-registration type is refused LOUDLY ("no chart exists for patient …")
/// rather than diverted. That refusal is a real limit on the empty-decoy attack, and without
/// this test the file would be quietly demonstrating "registration can be silently lost" while
/// the surrounding prose claims "clinical writes can be silently lost".
///
/// One extra move by the attacker removes the limit: SEED the decoy with a row bearing the
/// patient_id. 8b then answers TRUE from the attacker's own table, and every subsequent type
/// diverts as silently as registration did. That is the attack the "silent clinical data loss"
/// claim in `db/001`'s house-rule note actually rests on, so it is demonstrated here rather
/// than asserted there.
#[tokio::test]
async fn a_seeded_decoy_cannot_divert_a_later_clinical_write() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &[]).await;

    // Register the chart honestly first — no decoy in play, so this is a real event_log row.
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let event_id = Uuid::now_v7();
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    c.batch_execute(DECOY).await.unwrap();
    // The seed: one row carrying this patient_id, which is all step 8b's EXISTS needs.
    c.execute(
        "INSERT INTO pg_temp.event_log (event_id, patient_id) VALUES ($1::text::uuid, $2::text::uuid)",
        &[&Uuid::now_v7().to_string(), &p.to_string()],
    )
    .await
    .expect("the attacker may write freely to a table they created");

    let submitted = submit_signed_with_id(
        &c,
        &sk,
        &kid,
        event_id,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note/1",
            payload: serde_json::json!({ "text": "seeded-decoy probe" }),
            plaintext_twin: None,
            wall: 2,
        },
    )
    .await;

    c.batch_execute("RESET ROLE").await.unwrap();
    let (real, decoy) = landed(&c, event_id).await;

    let refusal = submitted
        .as_ref()
        .err()
        .map(common::db_msg)
        .unwrap_or_default();
    assert!(
        submitted.is_ok(),
        "the door must behave identically with a seeded decoy present; it errored: {refusal}"
    );
    assert_eq!(
        real, 1,
        "submit_event returned success but the clinical note is NOT in public.event_log — a \
         seeded decoy satisfied the #345 precedence check and then swallowed the write (#426)"
    );
    assert_eq!(
        decoy, 1,
        "only the attacker's own seed row may remain in the decoy — a second row means the \
         clinical write landed there"
    );
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
    c.batch_execute(DECOY)
        .await
        .expect("cairn_node likewise holds the default TEMPORARY");
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
