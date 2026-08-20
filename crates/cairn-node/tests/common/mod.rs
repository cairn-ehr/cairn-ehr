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

/// Build the `EventBody` an [`EventSpec`] describes, with a caller-chosen event id and the
/// single non-bearing "recorded" contributor every generic identity/medication fixture
/// uses. Factored out of [`submit_signed_with_id`] so a caller that needs the SAME body
/// WITHOUT immediately submitting it through `submit_event` — a withdrawal destined for
/// the remote door, e.g. (`claim_authority.rs`) — can build one identically rather than
/// hand-assembling the fields (contributors, `t_effective`, clock grade) that never vary
/// across these fixtures.
///
/// PUBLIC since the #380 arrival-order tests (`claim_authority.rs`): a withdrawal must be
/// able to name its target's content address BEFORE that target has been submitted at
/// all (set-union sync has no ordering), which means signing the target body once to
/// learn its address and only submitting it later. This is the same "build now, submit
/// later" need [`withdrawal_body_with_id`] already has, just for the assertion side.
pub fn body_from_spec(event_id: Uuid, kid: &str, spec: EventSpec<'_>) -> EventBody {
    EventBody {
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
    }
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
    let body = body_from_spec(event_id, kid, spec);
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
}

/// The content address of an already-submitted event. Sensitivity withdrawals name their
/// target by content address, not by event id, so tests need the mapping.
pub async fn content_address_of(c: &Client, event_id: Uuid) -> Vec<u8> {
    c.query_one(
        "SELECT content_address FROM event_log WHERE event_id = $1::text::uuid",
        &[&event_id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// A standing `sensitivity.grade.asserted` event, chart-wide (`SubjectKind::Patient`),
/// submitted by `sk`/`kid` at HLC wall `wall`, naming `grade`. Returns the assertion's
/// own event id.
///
/// PROMOTED from `claim_authority.rs`'s file-local `assert_grade` (#380 Task 4):
/// `claim_authority_worklist.rs` needs to mint a chart-wide grade in the identical
/// shape — the module header's own rule, "if two suites would write it identically, it
/// goes here". `grade` is now a parameter rather than a hardcoded `"sequestered"`
/// literal, since a shared helper should not bake in one caller's specific choice.
pub async fn assert_chart_grade(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    grade: &str,
) -> Uuid {
    let a = cairn_event::sensitivity::SensitivityAssertion {
        subject_kind: cairn_event::sensitivity::SubjectKind::Patient,
        subject_id: patient,
        grade,
        source: cairn_event::sensitivity::Provenance::Human,
        rationale: Some("protected witness"),
    };
    let id = Uuid::now_v7();
    submit_signed_with_id(
        c,
        sk,
        kid,
        id,
        EventSpec {
            patient,
            event_type: cairn_event::sensitivity::SENSITIVITY_EVENT_TYPE,
            schema_version: cairn_event::sensitivity::SENSITIVITY_SCHEMA_VERSION,
            payload: cairn_event::sensitivity::sensitivity_assertion_body(&a),
            plaintext_twin: Some(cairn_event::sensitivity::render_sensitivity_twin(&a)),
            wall,
        },
    )
    .await
    .unwrap();
    id
}

/// A withdrawal `EventBody` with a caller-chosen event id and a plain non-bearing
/// "recorded" contributor — what a genuinely UN-ATTESTED withdrawal looks like on the
/// wire (the same shape `sensitivity_ceremony.rs`'s `peer_withdrawal` already uses), so the
/// test can ask the predicate about it afterwards. Because this contributor claims no
/// responsibility, neither door's attestation gate ever engages for it — an attestation
/// token offered alongside it would be silently discarded, never stored — so this shape can
/// only ever grade 'self' or 'unverified', never 'attested'. A caller that needs an
/// attested withdrawal to land needs the OTHER contributor shape (see
/// [`bearing_withdrawal_body`], below).
pub fn withdrawal_body_with_id(
    patient: Uuid,
    event_id: Uuid,
    kid: &str,
    w: &cairn_event::sensitivity::SensitivityWithdrawal,
    wall: i64,
) -> EventBody {
    body_from_spec(
        event_id,
        kid,
        EventSpec {
            patient,
            event_type: cairn_event::sensitivity::WITHDRAWAL_EVENT_TYPE,
            schema_version: cairn_event::sensitivity::WITHDRAWAL_SCHEMA_VERSION,
            payload: cairn_event::sensitivity::sensitivity_withdrawal_body(w),
            plaintext_twin: Some(cairn_event::sensitivity::render_withdrawal_twin(w)),
            wall,
        },
    )
}

/// A withdrawal `EventBody` whose contributor claims RESPONSIBILITY for `attester_kid`
/// rather than for the signer — the ONLY shape either door's attestation gate will
/// validate and STORE. `cairn_responsibility_bound` (db/005, mirrored at db/020) requires
/// the bearing contributor's `actor_id` (and `cairn_check_contributors`'s
/// `responsibility.held_by`) to equal the verified attester's own key, so a device may
/// sign while a human attests, and the token still lands on `event_log.attester_key`.
/// Mirrors production's `sensitivity::withdraw_sensitivity` (`crates/cairn-node/src/
/// sensitivity.rs`), generalised to a different-signer/different-attester pair — passing
/// the SAME `kid` for both arguments reproduces production's own self-signed,
/// self-attested local shape exactly.
///
/// PROMOTED from `claim_authority.rs`'s file-local copy (#380 Task 4):
/// `claim_authority_worklist.rs` needs the identical bearing shape to build an ATTESTED
/// withdrawal (`withdrawal_body_with_id` above can never grade 'attested' — its token is
/// silently discarded by both doors) — two suites writing it identically is exactly the
/// module header's promotion rule.
pub fn bearing_withdrawal_body(
    kid: &str,
    attester_kid: &str,
    patient: Uuid,
    event_id: Uuid,
    w: &cairn_event::sensitivity::SensitivityWithdrawal,
    wall: i64,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: cairn_event::sensitivity::WITHDRAWAL_EVENT_TYPE.into(),
        schema_version: cairn_event::sensitivity::WITHDRAWAL_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        // "attested" + a responsibility marker naming the ATTESTER (not the signer): the
        // ADR-0051 wire shape both doors' attestation gate demands before it will verify
        // and STORE the token as this event's `attester_key`.
        contributors: serde_json::json!([{"actor_id": attester_kid, "role": "attested",
                                          "responsibility": {"held_by": attester_kid}}]),
        payload: cairn_event::sensitivity::sensitivity_withdrawal_body(w),
        attachments: vec![],
        plaintext_twin: Some(cairn_event::sensitivity::render_withdrawal_twin(w)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Sign and apply a pre-built body with NO attestation token, through the REMOTE door
/// (`apply_remote_event`) — the shape a cross-node write actually arrives in when its
/// author attested to nobody. This is the ONLY way to land a genuinely un-attested
/// `sensitivity.grade-withdrawal.asserted` event at all: the local door's ceremony (db/048)
/// refuses every un-attested withdrawal unconditionally, but `apply_remote_event` never
/// calls that ceremony (ADR-0062 decision 7 — a door check at apply would fork the event set).
pub async fn apply_remote_raw(
    c: &Client,
    sk: &SigningKey,
    body: EventBody,
) -> Result<u64, tokio_postgres::Error> {
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
}

/// As [`apply_remote_raw`], but WITH a human attestation token — the 3-argument
/// `apply_remote_event` shape, mirroring [`submit_attested`] but through the remote door.
/// The token is validated and STORED only when the body's own contributors claim
/// responsibility for `kid_h` (`cairn_responsibility_bound`'s check, mirrored at both
/// doors) — a body built by [`withdrawal_body_with_id`] never does, so a caller reaching
/// for this needs the bearing contributor shape instead.
pub async fn apply_remote_attested(
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
        "SELECT apply_remote_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &vk_h],
    )
    .await
}

/// Enroll a SECOND signer as a HUMAN actor, distinct from the agent key `setup` already
/// enrolls. Lifted from `identity_repudiate.rs` (review round 2, #344 N3) so any suite that
/// needs to author a genuine *suppressing*-mode event — one whose db/005 attestation gate
/// always demands a responsibility-bearing human, §5.7 "Human" — can reach for it instead of
/// re-copying the enrollment. Returns `(human sk, human kid)`.
pub async fn enroll_human(c: &Client) -> (SigningKey, String) {
    enroll_human_with_role(c, "records-officer").await
}

/// Enroll a human actor whose PINNED SET carries `role` — the way a suite gets TWO (or more)
/// genuinely DISTINCT human actors.
///
/// Why a caller cannot simply call [`enroll_human`] twice: an actor's `actor_id` is
/// content-addressed from its pinned set (db/004), so two different signing keys pinned to
/// the SAME set resolve to the SAME actor, and `enroll_actor` refuses the second with
/// `cairn_key_actor_id_conflict` — the silent-identity-merge guard from principle 2
/// ("never merge — always link"), pinned by `actor_enroll_collision.rs`. Varying the role
/// varies the pinned set, so each call yields a separate human with its own `actor_id`.
///
/// That distinctness is the whole point for `cairn_claim_authority`'s R2 branch, which asks
/// whether the withdrawer's actor IS the actor that made the claim being withdrawn: testing
/// "a DIFFERENT human may not self-withdraw" is impossible without two of them.
pub async fn enroll_human_with_role(c: &Client, role: &str) -> (SigningKey, String) {
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', jsonb_build_object('role', $2::text), $1)",
        &[&kid_h, &role],
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

/// Build, seal, sign and submit a `clinical.medication.asserted` event whose CLEAR
/// `safety` field is set to `safety` VERBATIM — bypassing `apply_safety_rung`'s
/// grade-driven coarsening entirely. That bypass IS the scenario: a hostile client with
/// direct DB access (or an ordinary older client that never ran the daemon's coarsening
/// step) can sign and submit any shape it likes; #405 part 2 is the door-side record of
/// that fact, never a refusal (ADR-0060, ADR-0064).
///
/// Modelled on `crates/cairn-node/src/medication/sealed_submit.rs`'s `seal_sign_submit`
/// path, minus the ONE call (`apply_safety_rung`) that chooses the rung from the chart's
/// standing grade — everything else (seal, register the unwrap key, sign, submit through
/// the strict door with the DEK as the 4th argument) is the same pipeline production uses.
///
/// Signs with the HUMAN key and takes ADR-0053 authorship (`with_human_author`) — the
/// shape every real medication assert in this slice carries — while `sk`/`kid` (the
/// device/node key) re-registers the node's unwrap key, exactly as `ensure_unwrap_key`
/// does on every real submit. Re-registering is a no-op when `medication_setup` already
/// registered the same key (idempotent — see `cairn_register_unwrap_key`'s own doc), so
/// this helper does not depend on being called only after that fixture.
///
/// Returns the submitted event's content address (`event_log.content_address`), or the
/// door's rejection — NOT unwrapped, so a caller asserting ADMISSION can still say why a
/// rejection is a test failure, and a caller proving the shape floor still refuses
/// something malformed can match on it.
#[allow(clippy::too_many_arguments)] // one parameter per wire value, mirroring assert_medication's own allow
pub async fn submit_medication_with_raw_safety(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    sk_h: &SigningKey,
    kid_h: &str,
    patient: Uuid,
    wall: i64,
    safety: serde_json::Value,
) -> Result<Vec<u8>, tokio_postgres::Error> {
    // Re-register the node's unwrap key from the DEVICE key, exactly as
    // `ensure_unwrap_key(client, node_sk)` does in the real pipeline — custody is always
    // the NODE's regardless of who signs (born-sealed erasability, ADR-0052). Idempotent:
    // a second registration of the same key is a no-op.
    //
    // `.expect()`, deliberately NOT `?` (2026-08-15 review, Important #3): this function's
    // `Result` is the caller's proxy for "did the DOOR admit or refuse this write" —
    // `safety_overclaim.rs` reads an `Err` here as ADR-0060/ADR-0063 evidence about
    // `submit_event`'s own behaviour. A `?` on this SETUP statement would let an
    // unrelated infrastructure failure (this call has never been observed to fail; it
    // exists only for parity with the real pipeline) surface as an indistinguishable
    // `Err`, misreporting an environment problem as a clinical-write cancellation in the
    // one suite whose entire purpose is attributing a failure to the right cause.
    let secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret).as_slice()],
    )
    .await
    .expect("submit_medication_with_raw_safety: registering the node's unwrap key failed — an environment/setup problem, not the door behaviour this helper exists to exercise");

    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let hlc = Hlc {
        wall,
        counter: 0,
        node_origin: "hostile-probe".into(),
    };
    let input = cairn_node::medication::AssertMedicationInput {
        term: "raw-safety-probe",
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "clinician-observed",
        started: None,
        started_precision: None,
    };
    // `safety: None` — no PRECISE claim under the seal. Irrelevant to this scenario:
    // `cairn_check_safety_signal` and the new overclaim check both read only the CLEAR
    // top-level `safety` field this helper sets directly, below.
    let body = cairn_node::medication::build_assert_body(
        event_id,
        medication_id,
        patient,
        &input,
        kid,
        hlc,
        None,
    );
    // ADR-0053: the human takes authorship and becomes the signer.
    let mut body = cairn_event::contributor::with_human_author(body, kid_h);

    // THE BYPASS. Production's `seal_sign_submit` would call `apply_safety_rung` here,
    // which looks up the chart's standing grade and coarsens `payload.safety` (absent
    // above) down to a licensed rung. This helper skips that call entirely and writes the
    // caller's value straight onto the envelope — exactly what a peer signing raw bytes,
    // honest or hostile, would produce.
    body.safety = Some(safety);

    let clear_twin = body
        .plaintext_twin
        .take()
        .expect("build_assert_body always sets a plaintext twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &clear_twin, &body.event_id)
            .expect("seal a well-formed medication payload");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));

    let signed = sign(&body, sk_h).expect("sign the sealed medication body");
    let ca = signed.content_address.clone();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await?;
    Ok(ca)
}

/// The same overclaiming, raw-safety medication event as
/// [`submit_medication_with_raw_safety`] — landed through the REMOTE door
/// (`apply_remote_event`) instead of the local one (`submit_event`).
///
/// Exists to pin ADR-0064 decision 7's LOCAL-DOOR-ONLY asymmetry: the safety-overclaim
/// ledger is written at the local door and NOWHERE ELSE, deliberately. db/049 and the ADR
/// both warn in capitals that the asymmetry reads as an oversight and that a reviewer WILL
/// tidy it into symmetry — and nothing anywhere paired the ledger with the remote door, so
/// that tidy-up would have kept every test green (#410 review finding I3).
///
/// The two helpers are deliberately near-identical in construction and differ ONLY in the
/// door they call, because that is precisely the variable under test: same bytes, same
/// chart, same overclaim, different door, different outcome.
#[allow(clippy::too_many_arguments)] // mirrors its local-door twin
pub async fn apply_remote_medication_with_raw_safety(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    sk_h: &SigningKey,
    kid_h: &str,
    patient: Uuid,
    wall: i64,
    safety: serde_json::Value,
) -> Result<Vec<u8>, tokio_postgres::Error> {
    let (signed_bytes, ca, dek) =
        build_raw_safety_medication(c, sk, kid, sk_h, kid_h, patient, wall, safety).await;
    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&signed_bytes, &dek.as_slice()],
    )
    .await?;
    Ok(ca)
}

/// Build (and seal, and sign) the raw-safety medication event both door helpers submit.
///
/// Private on purpose: it returns unsubmitted wire bytes, which is only ever useful to the
/// two helpers above. Factored out so the two doors provably receive the SAME event — if
/// this were copy-pasted, a drift between the copies would silently turn the local-vs-remote
/// comparison into a comparison of two different events.
#[allow(clippy::too_many_arguments)]
async fn build_raw_safety_medication(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    sk_h: &SigningKey,
    kid_h: &str,
    patient: Uuid,
    wall: i64,
    safety: serde_json::Value,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    // Same unwrap-key parity as the local-door helper, and `.expect` for the same reason:
    // a setup failure must never be mistaken for door behaviour.
    let secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret).as_slice()],
    )
    .await
    .expect("build_raw_safety_medication: registering the node's unwrap key failed — an environment/setup problem, not door behaviour");

    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let hlc = Hlc {
        wall,
        counter: 0,
        node_origin: "hostile-probe".into(),
    };
    let input = cairn_node::medication::AssertMedicationInput {
        term: "raw-safety-probe",
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "clinician-observed",
        started: None,
        started_precision: None,
    };
    let body = cairn_node::medication::build_assert_body(
        event_id,
        medication_id,
        patient,
        &input,
        kid,
        hlc,
        None,
    );
    let mut body = cairn_event::contributor::with_human_author(body, kid_h);
    // THE BYPASS — see the local-door helper's own note.
    body.safety = Some(safety);

    let clear_twin = body
        .plaintext_twin
        .take()
        .expect("build_assert_body always sets a plaintext twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &clear_twin, &body.event_id)
            .expect("seal a well-formed medication payload");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));

    let signed = sign(&body, sk_h).expect("sign the sealed medication body");
    // `dek` is a `Zeroizing<[u8; 32]>`; copied out here because the caller binds it as a
    // query parameter, which outlives the guard. Test-only — production never widens a DEK's
    // lifetime this way.
    (signed.signed_bytes, signed.content_address, dek.to_vec())
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

// ---------------------------------------------------------------------------------
// Catalogue predicates: "which functions is this repo answerable for?"
//
// Every repo-wide guard written over `pg_proc` needs the same two filters, and they are
// security-relevant: getting either wrong makes the guard pass by seeing less. They live
// here once, because a filter copied into two suites is the hand-maintained mirror pair
// this project keeps being bitten by (#404) — and here the divergence would be SILENT,
// since a narrowed filter reports "no offenders", not an error.
//
// They are SQL fragments rather than a whole query on purpose: each guard selects
// different columns and applies its own extra conditions. `p` is the `pg_proc` alias and
// `n` the `pg_namespace` alias the caller must use.
// ---------------------------------------------------------------------------------

/// Schemas the repo is answerable for: everything a migration could create a function in.
///
/// NOT pinned to `public`, deliberately. A migration that introduces its own schema would be
/// invisible to a `nspname = 'public'` filter AND would not lower any guard's row count, so
/// neither the floor guards nor the offender lists would notice — silent under-coverage of
/// exactly the kind these guards exist to prevent. System schemas are Postgres's own.
pub const REPO_SCHEMAS: &str = "n.nspname NOT IN ('pg_catalog','information_schema','pg_toast')
       AND n.nspname NOT LIKE 'pg_temp%' AND n.nspname NOT LIKE 'pg_toast_temp%'";

/// Extension-owned objects: `cairn_pgx` and `pgcrypto` install into `public` too, and this
/// repo's invariants are not theirs to satisfy.
///
/// `classid` is constrained because `pg_depend.objid` is unique only WITHIN a catalog — an
/// unrelated object carrying an extension dependency whose OID happened to equal a function's
/// would otherwise drop that function silently out of every guard.
pub const NOT_EXTENSION_OWNED: &str = "NOT EXISTS (SELECT 1 FROM pg_depend d
                        WHERE d.objid = p.oid AND d.classid = 'pg_proc'::regclass
                          AND d.deptype = 'e')";
