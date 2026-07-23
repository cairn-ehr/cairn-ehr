//! ADR-0051 contributor-role vocabulary + responsibility wire-shape floor tests
//! (issues #203 / #96, review finding C2).
//!
//! The contract under test, door by door:
//!
//!   * `submit_event` (the AUTHORING door) fails closed on any contributor role
//!     outside the ratified 12-member vocabulary — a node only authors roles it can
//!     stand behind. It also enforces the responsibility wire shape: an object
//!     `{held_by, on_behalf_of?}` (never the retired flat string), `held_by` naming
//!     the entry's own actor (the issue-#195 binding chain), `on_behalf_of` refused
//!     until a proxy-grant ADR defines its verification, and responsibility only on
//!     a responsibility-BEARING role (a signed "I bear responsibility as a
//!     non-bearing contributor" is incoherent and never lawful).
//!
//!   * `apply_remote_event` (the SYNC door) must NOT reject on role membership —
//!     set-union losslessness (#96): a 2031 vocabulary member arriving here must be
//!     admitted, classified by its mandatory partition prefix (`bearing:x` /
//!     `contrib:x`). The apply door refuses only the never-lawful shapes no
//!     conformant door of ANY schema version could mint.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via db::test_serial_guard (project
//! convention). Key material is minted at runtime via `generate_key` (house rule 6).
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

const SUBMIT1: &str = "SELECT submit_event($1)";
const SUBMIT3: &str = "SELECT submit_event($1,$2,$3)";
const APPLY1: &str = "SELECT apply_remote_event($1)";
const APPLY3: &str = "SELECT apply_remote_event($1,$2,$3)";

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical tables and enroll one agent signer + one human attester
/// (the same minimal cast as tests/attestation.rs).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"role-floor-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// A minimal signed-able note.added whose contributor set is the variable under test.
fn note_with(contributors: serde_json::Value, signer_kid: &str, patient: Uuid) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "role-test".into(),
        },
        t_effective: None,
        signer_key_id: signer_kid.into(),
        contributors,
        payload: serde_json::json!({"text": "role-vocabulary floor probe"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

/// Sign `body` with `sk`, submit through the 1-arg door, expect a refusal whose
/// message contains `needle` — the legible-rejection discipline (Spike 0002).
async fn expect_submit_refusal(c: &Client, body: &EventBody, sk: &SigningKey, needle: &str) {
    let signed = sign(body, sk).unwrap();
    let err = c
        .execute(SUBMIT1, &[&signed.signed_bytes])
        .await
        .expect_err("the submit door must refuse this contributor set");
    let m = db_msg(&err);
    assert!(
        m.contains(needle),
        "refusal must be legible (want {needle:?}): {m}"
    );
}

// ---------------------------------------------------------------------------
// The vocabulary table itself (the floor-queryable ratified set).
// ---------------------------------------------------------------------------

/// Drift guard: the SQL vocabulary and the cairn-event Rust mirror must agree —
/// 12 members, 6 bearing + 6 contributory, `recorded` ratified as contributory
/// (ADR-0051 ratifies what every orchestrator already mints).
#[tokio::test]
async fn vocabulary_table_matches_rust_mirror() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // COLLATE "C" pins the sort to byte order, matching Rust's Vec::sort below —
    // the ADR-0045/#69 discipline: a cluster's linguistic collation may e.g. ignore
    // the hyphen in `co-signed`, making this guard collation-dependent otherwise.
    let rows = c
        .query(
            "SELECT role, bears FROM contributor_role ORDER BY role COLLATE \"C\"",
            &[],
        )
        .await
        .unwrap();
    let sql: Vec<(String, bool)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();

    let mut rust: Vec<(String, bool)> = cairn_event::contributor::ROLE_VOCABULARY
        .iter()
        .map(|(r, b)| (r.to_string(), *b))
        .collect();
    rust.sort();

    assert_eq!(
        sql, rust,
        "SQL contributor_role and Rust ROLE_VOCABULARY drifted"
    );
    assert_eq!(sql.len(), 12, "ADR-0051 ratifies exactly 12 members");
    assert_eq!(sql.iter().filter(|(_, b)| *b).count(), 6, "6 bearing");
    assert!(
        sql.iter().any(|(r, b)| r == "recorded" && !*b),
        "`recorded` is ratified as contributory"
    );
}

// ---------------------------------------------------------------------------
// Submit door: fail closed on anything outside the ratified vocabulary.
// ---------------------------------------------------------------------------

/// `reviewed` is ADR-0028's deliberately-REJECTED candidate — the sharpest probe
/// that "closed" is now floor, not convention (#203: no validator checked this).
#[tokio::test]
async fn submit_refuses_unknown_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _, _) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_a, "role": "reviewed"}]),
        &kid_a,
        Uuid::now_v7(),
    );
    expect_submit_refusal(&c, &b, &sk_a, "vocabulary").await;
}

/// A partition-prefixed FUTURE member is likewise refused at the authoring door:
/// the prefix convention exists for the sync plane; a node never authors a member
/// its own vocabulary version has not ratified.
#[tokio::test]
async fn submit_refuses_prefixed_future_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _, _) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_a, "role": "bearing:delegated"}]),
        &kid_a,
        Uuid::now_v7(),
    );
    expect_submit_refusal(&c, &b, &sk_a, "vocabulary").await;
}

/// A contributor entry without actor_id/role is illegible authorship.
#[tokio::test]
async fn submit_refuses_missing_role_key() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _, _) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_a}]),
        &kid_a,
        Uuid::now_v7(),
    );
    expect_submit_refusal(&c, &b, &sk_a, "actor_id/role").await;
}

/// An empty contributor set never states who authored the record.
#[tokio::test]
async fn submit_refuses_empty_contributor_set() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _, _) = setup(&c).await;
    let b = note_with(serde_json::json!([]), &kid_a, Uuid::now_v7());
    expect_submit_refusal(&c, &b, &sk_a, "non-empty").await;
}

// ---------------------------------------------------------------------------
// Submit door: the responsibility wire shape (spec §3.9 {held_by, on_behalf_of?}).
// ---------------------------------------------------------------------------

/// The pre-ADR-0051 flat string (`"responsibility": "attested"`) is retired —
/// the proxy case is inexpressible in it (#203), so the shape is an object now.
#[tokio::test]
async fn submit_refuses_flat_string_responsibility() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "attested", "responsibility": "attested"}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk])
        .await
        .expect_err("flat-string responsibility must be refused");
    let m = db_msg(&err);
    assert!(m.contains("held_by"), "legible shape refusal: {m}");
}

/// The new happy path: object responsibility, held_by = the entry's own actor =
/// the verified attester. This is the shape every orchestrator mints from now on.
#[tokio::test]
async fn submit_accepts_object_responsibility_self_attested() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "attested",
                            "responsibility": {"held_by": kid_h}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let r = c
        .execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk])
        .await;
    assert!(
        r.is_ok(),
        "the ratified object shape must be accepted: {r:?}"
    );
}

/// held_by naming anyone but the entry's own actor re-opens the #195 hole
/// (an unverified responsibility claim about a person who never touched the event).
#[tokio::test]
async fn submit_refuses_held_by_naming_other_actor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, kid_a, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "attested",
                            "responsibility": {"held_by": kid_a}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk])
        .await
        .expect_err("held_by must name the entry's own actor");
    let m = db_msg(&err);
    assert!(m.contains("held_by"), "legible binding refusal: {m}");
}

/// on_behalf_of (the proxy case) is wire-expressible from day one but REFUSED at
/// the authoring door until a proxy-grant ADR defines how the principal's consent
/// is verified — otherwise the record accumulates unverifiable principal claims.
#[tokio::test]
async fn submit_refuses_on_behalf_of_until_proxy_grant_adr() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "attested",
                            "responsibility": {"held_by": kid_h, "on_behalf_of": "practice-entity"}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk])
        .await
        .expect_err("on_behalf_of is not yet admissible at the authoring door");
    let m = db_msg(&err);
    assert!(m.contains("on_behalf_of"), "legible proxy refusal: {m}");
}

/// Responsibility on a CONTRIBUTORY role is incoherent authorship — a signed body
/// reading "bears responsibility" in one field and "non-bearing" in the other is
/// exactly the self-contradiction the partition encoding exists to prevent (#96).
#[tokio::test]
async fn submit_refuses_responsibility_on_contributory_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "triaged",
                            "responsibility": {"held_by": kid_h}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk])
        .await
        .expect_err("responsibility on a contributory role must be refused");
    let m = db_msg(&err);
    assert!(m.contains("bearing"), "legible coherence refusal: {m}");
}

// ---------------------------------------------------------------------------
// Apply door: set-union losslessness (#96) — membership never rejects here.
// ---------------------------------------------------------------------------

/// THE #96 scenario: a future vocabulary member (mandatorily partition-prefixed on
/// the wire) arrives by sync. The apply door must ADMIT it — a role this node
/// cannot name must never exclude a signed clinical event from the set-union.
#[tokio::test]
async fn apply_admits_unknown_prefixed_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _, _) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_a, "role": "contrib:annotated"}]),
        &kid_a,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_a).unwrap();
    let r = c.execute(APPLY1, &[&signed.signed_bytes]).await;
    assert!(
        r.is_ok(),
        "the sync door must admit unknown vocabulary members: {r:?}"
    );
}

/// Even a wholly-unknown UNPREFIXED role (a non-conformant peer's invention) is
/// admitted when it claims nothing — it degrades to the vouching-unknown reading,
/// it never excludes content. Refusal is reserved for never-lawful claims.
#[tokio::test]
async fn apply_admits_unknown_bare_role_without_responsibility() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _, _) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_a, "role": "curated"}]),
        &kid_a,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_a).unwrap();
    let r = c.execute(APPLY1, &[&signed.signed_bytes]).await;
    assert!(
        r.is_ok(),
        "an unknown role claiming nothing must not exclude content: {r:?}"
    );
}

/// A responsibility claim on a `bearing:`-prefixed future member is future-lawful
/// and fully verifiable (token + held_by binding are role-agnostic) — admitted.
#[tokio::test]
async fn apply_admits_responsibility_on_bearing_prefixed_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "bearing:delegated",
                            "responsibility": {"held_by": kid_h}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let r = c
        .execute(APPLY3, &[&signed.signed_bytes, &token, &vk])
        .await;
    assert!(
        r.is_ok(),
        "a verifiable future bearing member must be admitted: {r:?}"
    );
}

/// on_behalf_of at the APPLY door is admitted as a signed, display-gated claim:
/// spec §3.9 promises the proxy transition "with no schema migration", so refusing
/// it here would wedge every future proxy event out of the set-union (the #201
/// lesson). Asymmetric with submit by design.
#[tokio::test]
async fn apply_admits_on_behalf_of_as_unverified_claim() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "attested",
                            "responsibility": {"held_by": kid_h, "on_behalf_of": "remote-entity"}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let r = c
        .execute(APPLY3, &[&signed.signed_bytes, &token, &vk])
        .await;
    assert!(
        r.is_ok(),
        "future-lawful proxy claims must not wedge sync: {r:?}"
    );
}

/// A contributor entry with no actor_id is illegible authorship — never-lawful,
/// refused even at the lenient door. This deliberately pins the wedge for
/// pre-ADR-0051 event logs: old cairn-sync binaries minted exactly this shape
/// (`{role:"author"}`, no actor_id), so any dev rig still holding such events must
/// be wiped, not synced through (the ADR retires the shape pre-production).
#[tokio::test]
async fn apply_refuses_contributor_missing_actor_id() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _, _) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"role": "recorded"}]),
        &kid_a,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_a).unwrap();
    let err = c
        .execute(APPLY1, &[&signed.signed_bytes])
        .await
        .expect_err("a contributor without actor_id is refused even at the lenient door");
    let m = db_msg(&err);
    assert!(m.contains("actor_id/role"), "legible refusal: {m}");
}

/// The retired flat-string responsibility shape is never-lawful at the apply door
/// too — same class as the missing-actor_id shape: pre-ADR-0051 logs holding it
/// (medication attestations, link proposals) wedge here by design; wipe, don't sync.
#[tokio::test]
async fn apply_refuses_flat_string_responsibility() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "attested", "responsibility": "attested"}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(APPLY3, &[&signed.signed_bytes, &token, &vk])
        .await
        .expect_err("the retired flat-string shape is refused at the apply door too");
    let m = db_msg(&err);
    assert!(m.contains("held_by"), "legible shape refusal: {m}");
}

/// Responsibility on an unknown UNPREFIXED role refuses at the lenient door: every
/// post-ADR-0051 conformant door prefixes future members, so an unprefixed unknown
/// carrying a responsibility claim is unmintable by any conformant door — unlike the
/// bare unknown role above, which claims nothing and is admitted.
#[tokio::test]
async fn apply_refuses_responsibility_on_unknown_bare_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "curated",
                            "responsibility": {"held_by": kid_h}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(APPLY3, &[&signed.signed_bytes, &token, &vk])
        .await
        .expect_err("responsibility on an unprefixed unknown role is never-lawful");
    let m = db_msg(&err);
    assert!(m.contains("bearing"), "legible coherence refusal: {m}");
}

/// Responsibility on a known-CONTRIBUTORY role is never-lawful under any schema
/// version (partitions are additive-only, they never flip) — refused even at the
/// lenient door, same class as an invalid attestation token.
#[tokio::test]
async fn apply_refuses_responsibility_on_contributory_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, sk_h, kid_h) = setup(&c).await;
    let b = note_with(
        serde_json::json!([{"actor_id": kid_h, "role": "triaged",
                            "responsibility": {"held_by": kid_h}}]),
        &kid_h,
        Uuid::now_v7(),
    );
    let signed = sign(&b, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(APPLY3, &[&signed.signed_bytes, &token, &vk])
        .await
        .expect_err("never-lawful incoherence is refused at both doors");
    let m = db_msg(&err);
    assert!(m.contains("bearing"), "legible coherence refusal: {m}");
}
