//! ADR-0064: cairn_claim_authority (db/005) is the SQL side of the same question
//! classify_authorship_confidence (crates/cairn-event/src/contributor.rs) answers in Rust.
//! They are not identical — R2 ('self') has no Rust counterpart and `Device` has no SQL
//! one — but where they overlap they must agree, or a display grade and an enforcement
//! grade would disagree about the very same event.
//!
//! THREE FIXTURES, ONE PER RUST VARIANT, each landed through the real door
//! (`apply_remote_event` or `submit_event`) via the SAME `tests/common/mod.rs` helpers the
//! other DB-gated suites use — never a hand-written JSON literal, which could let the Rust
//! side pass on a contributor shape the door never actually produces. After each fixture
//! lands, both `contributors` and `signer_key_id` are read back FROM `event_log` — the exact
//! columns `cairn_claim_authority` itself reads — so the two sides are compared against the
//! literal bytes Postgres stored, not the in-memory struct the test happened to build.
//!
//! WHY AGREEMENT IS OBSERVED, NOT STRUCTURAL: `classify_authorship_confidence` is a pure
//! three-line equality check over a JSON array; `cairn_claim_authority` is a `SECURITY
//! DEFINER` query joining `event_log`, `actor_current` and the attestation-vouch machinery.
//! Nothing here computes one side FROM the other — each fixture is independently graded by
//! both, and the only thing tying them together is the mapping this file asserts. A bug in
//! either implementation (e.g. SQL forgetting `attester_key IS NOT NULL`, or Rust matching
//! the signer instead of the attester) would desynchronise the pair on at least one of the
//! three fixtures below.
mod common;

use cairn_event::contributor::{classify_authorship_confidence, AuthorshipConfidence};
use cairn_event::sensitivity::{
    render_sensitivity_twin, sensitivity_assertion_body, SensitivityAssertion, SubjectKind,
    SENSITIVITY_EVENT_TYPE, SENSITIVITY_SCHEMA_VERSION,
};
use common::{
    apply_remote_attested, apply_remote_raw, assert_chart_grade, bearing_withdrawal_body,
    body_from_spec, content_address_of, cs, enroll_human, setup, submit_registration, EventSpec,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// Ask the predicate directly with an explicit NULL target — the same query
/// `claim_authority.rs`'s file-local `authority` helper runs, duplicated here rather than
/// shared because it is a single query and the two test binaries are independent crates'
/// worth of `mod common;` away from each other. NULL keeps R2 out of play (it has no Rust
/// counterpart), so every comparison below is over the R1/'unverified' overlap only.
async fn authority_null_target(c: &Client, event: Uuid) -> String {
    c.query_one(
        "SELECT cairn_claim_authority($1::text::uuid, NULL::uuid)",
        &[&event.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Read back the `contributors` / `signer_key_id` this event ACTUALLY landed with, straight
/// from `event_log` — the same two columns `cairn_claim_authority`'s R1 branch reads. Round-
/// tripping through the real door and back (rather than reusing the `EventBody` the test
/// built in memory) is what makes "byte-identical contributor set" a checked fact about this
/// run instead of an assumption about what the door does with what it was handed.
async fn stored_contributors_and_signer(c: &Client, event: Uuid) -> (serde_json::Value, String) {
    let row = c
        .query_one(
            "SELECT contributors::text, signer_key_id FROM event_log \
             WHERE event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .unwrap();
    // jsonb has no direct tokio-postgres FromSql impl in this workspace's setup (no
    // postgres-types "with-serde_json-1" feature enabled); cast to text and parse, the
    // same idiom `medication_authorship.rs` already uses for this exact column.
    let contributors: serde_json::Value = serde_json::from_str(&row.get::<_, String>(0)).unwrap();
    (contributors, row.get(1))
}

#[tokio::test]
async fn attested_in_rust_is_attested_in_sql_and_unverified_is_unverified() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // ------------------------------------------------------------------------------------
    // Fixture 1 — Attested / 'attested'.
    //
    // A device-signed withdrawal vouched by a real human attestation — the same shape
    // `claim_authority.rs`'s `a_vouched_human_attestation_is_attested` pins. `apply_remote_
    // attested`'s door gate (db/020 step 4) verifies the token AND that the attester
    // resolves to exactly one HUMAN actor before it ever stores `attester_key` — so by the
    // time admission succeeds, "kid_h is a verified human attester of this event" is a fact
    // the door itself already established, not something this test re-derives.
    // ------------------------------------------------------------------------------------
    let target = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;
    let withdraws_hex = hex::encode(content_address_of(&c, target).await);
    let w = cairn_event::sensitivity::SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "lockstep fixture: vouched",
    };
    let attested_id = Uuid::now_v7();
    let body = bearing_withdrawal_body(&kid, &kid_h, p, attested_id, &w, 20);
    apply_remote_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("a properly attested cross-node withdrawal must land");

    let (contributors, signer) = stored_contributors_and_signer(&c, attested_id).await;
    let rust_verdict = classify_authorship_confidence(&contributors, &signer, Some(kid_h.as_str()));
    let sql_verdict = authority_null_target(&c, attested_id).await;
    assert_eq!(
        rust_verdict,
        AuthorshipConfidence::Attested,
        "the stored contributor set must classify Attested: kid_h is both the bearing \
         actor and the verified attester"
    );
    assert_eq!(
        sql_verdict, "attested",
        "cairn_claim_authority's R1 must agree: a vouched human attestation is authoritative"
    );

    // ------------------------------------------------------------------------------------
    // Fixture 2 — Unverified / 'unverified'.
    //
    // A claimed human author with NO attestation offered at all — exactly the shape
    // `with_human_author` (contributor.rs) produces before anyone vouches for it: the
    // device signs, a bearing "authored" entry names a human, and nothing on the wire ties
    // that claim to a verified key. The contributor entry carries no "responsibility"
    // object, so db/020's attestation gate (`v_bears`, keyed on `e ? 'responsibility'`)
    // never engages — the event is ADMITTED with `attester_key` left NULL, which is the
    // point: apply grades, it never refuses (ADR-0064).
    // ------------------------------------------------------------------------------------
    let unverified_id = Uuid::now_v7();
    let assertion = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: p,
        grade: "sequestered",
        source: "human",
        rationale: Some("lockstep fixture: claimed, never attested"),
    };
    let mut unverified_body = body_from_spec(
        unverified_id,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&assertion),
            plaintext_twin: Some(render_sensitivity_twin(&assertion)),
            wall: 30,
        },
    );
    // Override body_from_spec's default single "recorded" contributor: a bearing claim for
    // the human PLUS the device's own contributory "recorded" entry, mirroring
    // `with_human_author`'s real output shape (human author listed first, device preserved
    // after) — but with no attestation ever offered for the human's claim.
    unverified_body.contributors = serde_json::json!([
        {"actor_id": kid_h, "role": "authored"},
        {"actor_id": kid,   "role": "recorded"},
    ]);
    apply_remote_raw(&c, &sk, unverified_body)
        .await
        .expect("an unattested bearing claim must still be ADMITTED, never refused, at apply");

    let (contributors, signer) = stored_contributors_and_signer(&c, unverified_id).await;
    // No token ever travelled with this event, so no caller could honestly have verified
    // kid_h as its attester — the honest input here is None, not a re-derivation of R1.
    let rust_verdict = classify_authorship_confidence(&contributors, &signer, None);
    let sql_verdict = authority_null_target(&c, unverified_id).await;
    assert_eq!(
        rust_verdict,
        AuthorshipConfidence::Unverified,
        "a bearing claim for an actor who is neither the signer nor a verified attester \
         must classify Unverified, not Device — dropping it would be the exact collapse \
         AuthorshipConfidence's doc comment forbids"
    );
    assert_eq!(
        sql_verdict, "unverified",
        "cairn_claim_authority must agree: attester_key is NULL, so R1 cannot fire, and \
         the NULL target keeps R2 out of play"
    );

    // ------------------------------------------------------------------------------------
    // Fixture 3 — Device / 'unverified'.
    //
    // A plain device-only "recorded" assertion, the shape `claim_authority.rs`'s
    // `an_unattested_claim_is_unverified` already pins on the SQL side alone. No bearing
    // contributor exists at all, which is exactly where Rust and SQL part ways: Rust has a
    // dedicated `Device` variant for "no responsibility-bearing contributor", while SQL has
    // no third state — a device-only event is simply 'unverified' there too. This fixture
    // pins that deliberate asymmetry, not an accidental one.
    // ------------------------------------------------------------------------------------
    let device_id = assert_chart_grade(&c, &sk, &kid, p, 40, "sequestered").await;
    let (contributors, signer) = stored_contributors_and_signer(&c, device_id).await;
    let rust_verdict = classify_authorship_confidence(&contributors, &signer, None);
    let sql_verdict = authority_null_target(&c, device_id).await;
    assert_eq!(
        rust_verdict,
        AuthorshipConfidence::Device,
        "a contributor set with no bearing entry must classify Device"
    );
    assert_eq!(
        sql_verdict, "unverified",
        "cairn_claim_authority has no Device state — a device-only event grades \
         'unverified' there, the deliberate half of the mapping that is NOT 1:1"
    );
}
