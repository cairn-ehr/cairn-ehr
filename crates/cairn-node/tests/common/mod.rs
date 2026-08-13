//! Shared scaffolding for cairn-node's integration tests generally. It started as
//! scaffolding for the identity integration tests (#120) and has since grown helpers for
//! other clusters too (`medication_setup`, #288) — the #120 guard below
//! (`identity_scaffolding_shared.rs`) still binds only the identity-cluster helpers, not
//! everything this file publishes.
//!
//! Every identity suite needs the same four things before it can assert anything: a
//! connection string from the environment, a clean database with one enrolled signer, a
//! way to sign-and-submit an event through the real `submit_event` door, and a way to read
//! the resulting projection rows back. Those helpers used to be copy-pasted into each
//! suite — `identity_identify.rs` was the third copy — and the copies had already drifted.
//! They live here once instead.
//!
//! **How Cargo sees this file.** Cargo compiles every top-level `tests/*.rs` as its own
//! test binary, but treats a SUBdirectory like `tests/common/` as an ordinary module
//! directory. So this is not a test binary; each suite pulls it in with `mod common;`.
//! That is the standard Rust idiom for shared integration-test code.
//!
//! **What belongs here.** Only helpers that are generic across identity suites. A suite's
//! own event builders (`submit_link`, `open_dispute`, `mark_pending`, …) stay in the suite,
//! where their canned clinical strings are readable next to the assertions that depend on
//! them. The line is: if two suites would write it identically, it goes here.
//!
//! `tests/identity_scaffolding_shared.rs` enforces adoption — but only of the helpers that
//! are specific to this cluster: `submit_signed`, `submit_registration`, `trust_of`,
//! `person_chart_trust`. It deliberately does NOT bind `cs` / `db_msg` / `setup`, which are
//! project-wide test idioms declared in dozens of this directory's files; see that file's
//! `REPO_WIDE` const for why, and #327 for unifying them. So a suite re-declaring its own
//! `setup` is caught by review, not by the guard.

// Each test binary uses only the subset of helpers it needs, so from the perspective of
// any single binary the rest are dead. Without this, every suite would warn about the
// helpers the OTHER suites use. Scoped to this module, so it never masks dead code in a
// test suite itself.
#![allow(dead_code)]

use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, ClockGrade, EventBody, Hlc, SigningKey,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// The test Postgres connection string, or `None` when the suite should self-skip.
///
/// Every DB-gated test opens with `let Some(base) = cs() else { return };` — absent
/// `$CAIRN_TEST_PG` the suite quietly passes, so a plain `cargo test` on a machine with no
/// database still works. Two things set it so the tests actually run: CI declares it as a
/// workflow-step `env:` entry (`.github/workflows/rust.yml`), and locally
/// `scripts/run-db-gated-tests.sh` bakes it in.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement.
///
/// `tokio_postgres::Error`'s `Display` renders only the literal "db error"; the actual
/// `RAISE EXCEPTION` message from the in-DB floor lives in the `DbError` payload. Every
/// floor-rejection assertion must therefore go through here, or it asserts against a
/// constant string and passes for the wrong reason.
pub fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical tables, truncate the caller's identity-overlay tables, and enroll
/// one agent signer. Returns `(signing key, key id)`.
///
/// `extra_tables` names the projection/overlay tables this suite writes to — e.g.
/// `&["patient_link", "person_member"]` for linkage, `&["chart_dispute"]` for disputes.
/// They are truncated behind a `to_regclass` guard because each is created by a LATER
/// migration than the core clinical tables: the guard keeps one shared `setup()` correct
/// on a database migrated only partway, instead of erroring on a table that does not exist
/// yet. Hand-maintaining that `DO $$` block per copy is exactly the drift #120 is about,
/// so it is generated from the list here.
pub async fn setup(c: &Client, extra_tables: &[&str]) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic CASCADE",
    )
    .await
    .unwrap();

    if !extra_tables.is_empty() {
        // Table names are interpolated (an identifier cannot be a bind parameter), so they
        // are asserted to be bare lower-case identifiers first. These are compile-time
        // literals from test code rather than input, but the check costs nothing and keeps
        // the generated SQL obviously safe to a reader.
        let mut sql = String::from("DO $$ BEGIN ");
        for t in extra_tables {
            assert!(
                !t.is_empty()
                    && t.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "extra table name must be a bare lower-case identifier, got {t:?}"
            );
            sql.push_str(&format!(
                "IF to_regclass('public.{t}') IS NOT NULL THEN TRUNCATE {t}; END IF; "
            ));
        }
        sql.push_str("END $$;");
        c.batch_execute(&sql).await.unwrap();
    }

    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// The parts of an event that actually vary between identity test cases.
///
/// A named-field struct rather than a long positional argument list: at a call site
/// `event_type: "identity.dispute.asserted"` reads as itself, where the eighth positional
/// `&str` would not. Everything an identity test never varies — the UUIDv7 event id, the
/// single-contributor set, the absent `t_effective`, the empty attachments — is filled in
/// by [`submit_signed`] rather than repeated per call.
pub struct EventSpec<'a> {
    /// The chart this event is "about". Identity assertions use their subject's UUID.
    pub patient: Uuid,
    pub event_type: &'a str,
    pub schema_version: &'a str,
    /// The event body content, which becomes the DB `body`.
    pub payload: serde_json::Value,
    /// The §3.13 legibility twin. `Some` for types with a renderer (identity assertions,
    /// where the in-DB floor requires it); `None` for types where `submit_event` still
    /// derives the honest-degrade skeleton itself (db/015).
    pub plaintext_twin: Option<String>,
    /// HLC wall clock — higher is newer. How a test orders overlays deterministically
    /// without sleeping on a real clock.
    pub wall: i64,
}

/// Sign `spec` and submit it through the real `submit_event` door.
///
/// Returns the raw submit result — NOT unwrapped — because about a third of these tests
/// assert a *rejection* and match on [`db_msg`] of the error. A helper that unwrapped here
/// would make the floor's rejections untestable.
pub async fn submit_signed(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    spec: EventSpec<'_>,
) -> Result<u64, tokio_postgres::Error> {
    submit_signed_with_id(c, sk, kid, Uuid::now_v7(), spec).await
}

/// As [`submit_signed`], but with the event id CHOSEN by the caller.
///
/// Only needed when a test must name the event afterwards — a recall query over the authoring
/// actor's epoch, say, which selects every event that actor wrote and therefore has to be
/// compared against ids the test knows. Everything else should call [`submit_signed`].
pub async fn submit_signed_with_id(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    event_id: Uuid,
    spec: EventSpec<'_>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: spec.patient.to_string(),
        event_type: spec.event_type.into(),
        schema_version: spec.schema_version.into(),
        hlc: Hlc {
            wall: spec.wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: spec.payload,
        attachments: vec![],
        plaintext_twin: spec.plaintext_twin,
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
}

/// Enroll a SECOND signer as a HUMAN actor, distinct from the agent key `setup` already
/// enrolls. Lifted from `identity_repudiate.rs` (review round 2, #344 N3) so any suite that
/// needs to author a genuine *suppressing*-mode event — one whose db/005 attestation gate
/// always demands a responsibility-bearing human, §5.7 "Human" — can reach for it instead of
/// re-copying the enrollment. Returns `(human sk, human kid)`.
pub async fn enroll_human(c: &Client) -> (SigningKey, String) {
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_h, kid_h)
}

/// Sign `body` with `sk` and submit it WITH a human attestation token from `sk_h`/`kid_h` —
/// the 3-argument `submit_event` shape a *suppressing*-mode event (e.g.
/// `identity.repudiate.asserted`) needs to pass db/005's attestation gate. Also lifted from
/// `identity_repudiate.rs`; kept separate from [`submit_signed`] rather than folded into it
/// because most suites never need an attestation token and [`EventSpec`] has no field for
/// one — a caller that DOES need this builds its own `EventBody` (the suppressing event
/// types' payload shapes are one-off enough that a shared `EventSpec` would not pull its
/// weight for them the way it does for the additive types `submit_signed` serves).
pub async fn submit_attested(
    c: &Client,
    sk: &SigningKey,
    body: EventBody,
    sk_h: &SigningKey,
    kid_h: &str,
) -> Result<u64, tokio_postgres::Error> {
    let signed = sign(&body, sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, kid_h, "attested", sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &vk_h],
    )
    .await
}

/// Submit the §5.3 registration act that brings a chart into being.
///
/// **Call this before any other event for a fresh `patient_id`.** Since #345 the in-DB floor
/// requires the FIRST event carrying a `patient_id` to be a registration (`submit_event`,
/// db/005 step 8b), so this is not a convenience — it is the arrangement step every chart
/// needs, and a suite that skips it gets a legible refusal naming the rule. It also
/// materialises the `patient_chart` row that chart-shaped reads (`person_chart` and the trust
/// reads composed on it, the candidate list's last-activity) join against, which is the job the
/// retired `submit_registration` used to do here.
///
/// Class `standard` with a search that found nothing (`displayed: []`) is the honest fixture:
/// it is the NORMAL case for a genuinely new patient, and it exercises the fuller floor path —
/// the non-standard classes skip db/045's search rules (2d–2g) entirely. The query names the
/// patient's own UUID as its single token so the attestation is well-formed without inventing a
/// name the suite would then have to keep consistent with its own assertions.
///
/// `wall` orders the registration against the suite's own events: pass something BELOW them. A
/// registration is the chart's birth act, and `patient_registration_current` picks the EARLIEST
/// by HLC — a fixture registering above its own events would assert a birth that came after the
/// life. Unwrapped, because this is always setup for the real assertion, never the thing under
/// test.
/// Returns the registration event's own `event_id`, because a chart's birth act is a real event
/// on the log: a recall over the authoring actor's epoch selects it like any other, and a suite
/// asserting an exact recall set has to be able to name it (`recall_epoch.rs`).
pub async fn submit_registration(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    wall: i64,
) -> Uuid {
    let tokens = [p.to_string()];
    let a = RegistrationAssertion {
        class: RegistrationClass::Standard,
        basis: None,
        search: Some(SearchAttestationInput {
            terms: SearchTerms {
                name_tokens: &tokens,
                birth_date: None,
                identifiers: &[],
            },
            displayed: &[],
            incomplete: false,
        }),
    };
    let event_id = Uuid::now_v7();
    submit_signed_with_id(
        c,
        sk,
        kid,
        event_id,
        EventSpec {
            patient: p,
            event_type: REGISTRATION_EVENT_TYPE,
            schema_version: REGISTRATION_SCHEMA_VERSION,
            payload: registration_assertion_body(&a),
            plaintext_twin: Some(render_registration_twin(&a)),
            wall,
        },
    )
    .await
    .expect("registration accepted");
    event_id
}

/// Register BOTH charts of a candidate/proposed pair (#345).
///
/// The matching suites (`apply_proposal.rs`, `auto_apply.rs`) seed a `match_proposal` row
/// directly, which is a projection seed rather than an event — so neither chart exists yet, and
/// since the precedence rule landed a link may only be authored between charts that DO. Both
/// wrote this pair of [`submit_registration`] calls identically, which is exactly the drift
/// #120/#327 exist to stop, so it lives here once.
///
/// The registrations are authored by the caller's SEEDER key, never the matcher's: registration
/// is a registrar act, and the matcher never registers anyone — it only ever proposes links
/// between charts other actors created. Wall 1 keeps the birth act below every event these
/// suites author (see [`submit_registration`] on why the wall must be low).
pub async fn register_pair(c: &Client, sk: &SigningKey, kid: &str, low: Uuid, high: Uuid) {
    submit_registration(c, sk, kid, low, 1).await;
    submit_registration(c, sk, kid, high, 1).await;
}

/// The effective trust state `chart_trust` reports for a subject, or `None` (== confirmed).
///
/// `chart_trust` is the authoritative pre-sync safety signal: it answers for a chart
/// whether or not that chart has replicated anywhere.
pub async fn trust_of(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT trust_state FROM chart_trust WHERE patient_id = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// `person_chart_trust.trust_state` for a subject's chart row, or `None`.
///
/// The unified read composed on top of the `person_chart` linkage view — so unlike
/// [`trust_of`] it surfaces a subject only once that subject has a chart. Tests assert
/// against both to pin the difference.
pub async fn person_chart_trust(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT trust_state FROM person_chart_trust WHERE patient_id = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// Truncate the event log, the custody plane and every medication projection, then enroll
/// one DEVICE actor (mints medication threads) and one HUMAN actor (signs and attests).
/// Returns `(device_sk, device_kid, human_sk, human_kid)`.
///
/// Each medication table is truncated behind a `to_regclass` guard because it is created
/// by a later migration than the core clinical tables: the guard keeps one shared helper
/// correct on a database migrated only partway, instead of erroring on a table that does
/// not exist yet. Same discipline as `setup` above.
///
/// NOT INTERCHANGEABLE with `medication_attestation.rs`'s own `setup`, which looks nearly
/// identical but deliberately OMITS `medication_attestation` from its truncation list —
/// that suite scopes its counts by patient and relies on attestation rows surviving across
/// tests. Repointing it here would quietly change its semantics. Issue #340 tracks
/// consolidating the three medication truncation lists properly (this one,
/// `medication_attestation.rs`'s, and `medication_coding.rs`'s narrower one with its
/// registry sweep); do not "tidy" them into one without reading it first.
pub async fn medication_setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
           IF to_regclass('public.medication_dose_event') IS NOT NULL THEN TRUNCATE medication_dose_event; END IF; \
           IF to_regclass('public.medication_dose_correction') IS NOT NULL THEN TRUNCATE medication_dose_correction; END IF; \
           IF to_regclass('public.medication_reconciliation') IS NOT NULL THEN TRUNCATE medication_reconciliation; END IF; \
           IF to_regclass('public.medication_group_member') IS NOT NULL THEN TRUNCATE medication_group_member; END IF; \
           IF to_regclass('public.medication_projection_flag') IS NOT NULL THEN TRUNCATE medication_projection_flag; END IF; \
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
           IF to_regclass('public.medication_attestation') IS NOT NULL THEN TRUNCATE medication_attestation; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_d, kid_d) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid_d],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    // ADR-0052: register THIS node's unwrap key (derived from the device key) so the strict
    // door can wrap every sealed event's DEK into custody — attestation events are
    // clinical.* and born-sealed too. A node has exactly ONE unwrap key regardless of who
    // signs individual events; deriving it from the human key would collide on the
    // node_unwrap_key singleton.
    let secret = cairn_event::seal::derive_unwrap_secret(&sk_d.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret).as_slice()],
    )
    .await
    .unwrap();
    (sk_d, kid_d, sk_h, kid_h)
}

/// How many attestation rows a medication thread carries.
///
/// Shared by the two #288 medication suites (`medication_read.rs`, `medication_signoff.rs`),
/// which both need to assert "this thread was / was not vouched" — the line this module's
/// header draws ("if two suites would write it identically, it goes here").
///
/// UUID BINDING: `cairn-node` does not enable tokio-postgres's `with-uuid-1` feature (see
/// `medication/read.rs`'s "UUID BINDING" module comment), so `thread` is bound as text and
/// cast in SQL rather than passed as a `Uuid` parameter directly.
pub async fn attestation_count(c: &Client, thread: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
        &[&thread.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}
