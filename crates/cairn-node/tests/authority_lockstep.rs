//! ADR-0064: cairn_claim_authority (db/005) asks an overlapping question, at a different
//! read, from classify_authorship_confidence (Rust, crates/cairn-event/src/contributor.rs).
//! The two functions read DISJOINT inputs — this Rust function reads `contributors` /
//! `signer_key_id`, `cairn_claim_authority`'s R1 branch reads only `attester_key` /
//! `cairn_attestation_vouched` / `actor_current` and never looks at `contributors` at all —
//! so "mirror" overstates it. They are not a full mapping either: R2 ('self') has no Rust
//! counterpart, `Device` has no SQL counterpart, and R1 additionally demands the attester
//! resolve to EXACTLY ONE `kind = 'human'` actor, a check this Rust function does not
//! perform. Two door-admissible shapes are ALREADY KNOWN to diverge — a key mapped to more
//! than one actor can grade `Attested` here and `'unverified'` in SQL; a suppressing-mode
//! event whose only contributor is `recorded` (no `responsibility` object) can grade
//! `Device` here and `'attested'` in SQL — filed as an issue for #245's display half to
//! resolve, not something this test papers over.
//!
//! What this test pins is narrower: on the THREE SHAPES below — a single vouched human
//! attester, a claimed-but-unattested human author, a device-only contributor set (none of
//! them a dual-mapped key or a no-responsibility suppressing event) — the two verdicts
//! agree: `Attested` <-> `'attested'`, `Unverified`/`Device` <-> `'unverified'`. That is the
//! obligation a future display grade and today's enforcement grade owe each other on THESE
//! shapes; it is not a claim that every event this repo can admit agrees.
//!
//! THREE FIXTURES, ONE PER RUST VARIANT, each DOOR-ADMITTED (`apply_remote_event` or
//! `submit_event`, via the SAME `tests/common/mod.rs` helpers the other DB-gated suites use)
//! and then READ BACK from `event_log` — the same two columns `cairn_claim_authority`'s R1
//! branch reads — so the comparison is over what Postgres actually stored, not the in-memory
//! struct the test happened to build. (Fixture 2 overrides `contributors` with a literal
//! before signing, but that literal is READ BACK post-admission before either side sees it —
//! door-admitted-and-read-back is the property that matters, not "never a literal".)
//!
//! WHY AGREEMENT IS OBSERVED, NOT STRUCTURAL: nothing here computes one side FROM the
//! other — both verdicts are computed INDEPENDENTLY and then checked to land where the
//! mapping predicts. The bridge used to be worse than that: `verified_attester` was handed
//! over from the test's own `kid_h`, so the value whose provenance IS the invariant was
//! supplied by assertion (#412). It now comes from `event_log.attester_key` — the same
//! proof-carrying column R1 reads, written only after the door verified the token — so both
//! sides start from a fact Postgres established, and a `VerifiedKid` is the only shape the
//! Rust grader will accept for it. Two regressions this
//! test WOULD catch: Rust's `authenticated` check matching the SIGNER instead of the
//! verified ATTESTER (fixture 1 would then read `Unverified` while SQL still reads
//! `'attested'`), or the bearing/contributory partition misclassifying `"recorded"` as
//! bearing (fixture 3 would then read `Attested` while SQL stays `'unverified'`, since the
//! signer and the lone contributor are the same device key). What it would NOT catch: any of
//! R1's own internal conjuncts (`attester_key IS NOT NULL`, `cairn_attestation_vouched`,
//! `kind = 'human'`, the single-actor count) — no fixture below isolates them (fixture 1 is
//! a single vouched human; fixtures 2 and 3 carry no `attester_key` at all), so a regression
//! in any of those conjuncts would not move this test. That coverage belongs to, and already
//! exists in, `claim_authority.rs`.
mod common;

use cairn_event::contributor::{classify_authorship_confidence, AuthorshipConfidence, VerifiedKid};
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
struct Stored {
    contributors: serde_json::Value,
    /// `event_log.signer_key_id` — proof-carrying: db/005 step 1 runs `cairn_verify`, and
    /// that refuses bytes whose body claims a signer other than the key the signature used.
    signer: String,
    /// `event_log.attester_key`, hex — proof-carrying for the same kind of reason: the door
    /// verifies the attestation token before it stores this. NULL when the event carried no
    /// attestation, which fixtures 2 and 3 assert rather than assume.
    attester: Option<String>,
}

async fn stored_claim(c: &Client, event: Uuid) -> Stored {
    let row = c
        .query_one(
            "SELECT contributors::text, signer_key_id, encode(attester_key, 'hex') \
               FROM event_log WHERE event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .unwrap();
    // jsonb has no direct tokio-postgres FromSql impl in this workspace's setup (no
    // postgres-types "with-serde_json-1" feature enabled); cast to text and parse, the
    // same idiom `medication_authorship.rs` already uses for this exact column.
    let contributors: serde_json::Value = serde_json::from_str(&row.get::<_, String>(0)).unwrap();
    Stored {
        contributors,
        signer: row.get(1),
        attester: row.get(2),
    }
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
    // resolves to AT LEAST ONE `kind = 'human'` actor (`IF NOT EXISTS (... AND kind =
    // 'human')`) before it ever stores `attester_key` — so by the time admission succeeds,
    // "kid_h is a verified human attester of this event" is a fact the door itself already
    // established, not something this test re-derives.
    //
    // NOT "exactly one" — this comment used to say that, and it was wrong (#410 review
    // finding A1). "Exactly one" is `cairn_claim_authority`'s R1 alone
    // (`count(*) = 1 AND bool_and(a.kind = 'human')`, db/005), which db/005's own header
    // calls out as deliberately STRICTER than db/020's sibling: the door admits a key
    // mapped to BOTH a human and an agent, R1 does not. That gap is the first divergence
    // axis this file's header names as NOT covered by any fixture here (#408), so a comment
    // asserting the door already closed it contradicted both the header and the PR's own
    // central finding. The fixture is unaffected — `enroll_human` enrols exactly one actor
    // — but a reader trusting the old wording would conclude #408's first case is
    // impossible.
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

    let stored = stored_claim(&c, attested_id).await;
    // The attester is read from `event_log.attester_key`, NOT handed over from `kid_h`
    // (#412). The door stores that column only after verifying the token, so reading it is
    // the same proof `cairn_claim_authority`'s R1 relies on — which is what lets both sides
    // start from the same verified fact instead of from a value this test asserts is true.
    let attester = stored
        .attester
        .as_deref()
        .expect("the door must have stored the verified attester for a vouched withdrawal");
    assert_eq!(
        attester, kid_h,
        "the stored attester must be the human who signed the token"
    );
    let rust_verdict = classify_authorship_confidence(
        &stored.contributors,
        VerifiedKid::from_event_log_column(&stored.signer),
        Some(VerifiedKid::from_event_log_column(attester)),
    );
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
    // A claimed human author with NO attestation offered at all: the device signs, an
    // "authored" entry names a human, and nothing on the wire ties that claim to a verified
    // key. The contributor entry carries no "responsibility"
    // object, so db/020's attestation gate (`v_bears`, keyed on `e ? 'responsibility'`)
    // never engages — the event is ADMITTED with `attester_key` left NULL, which is the
    // point: apply grades, it never refuses (ADR-0064).
    //
    // DELIBERATELY *NOT* `with_human_author`'s output, though this comment used to claim it
    // was (#410 review finding A2). `with_human_author` also sets
    // `body.signer_key_id = human_kid`, making the HUMAN the signer — and on that shape
    // `classify_authorship_confidence` returns `Attested` (the bearing actor IS the signer),
    // the exact OPPOSITE of the `Unverified` this fixture exists to pin. Keeping the DEVICE
    // as signer is what makes the claim unverifiable, so the deviation is load-bearing. The
    // narrower statement below — that the contributor ARRAY mirrors `with_human_author`'s
    // ordering — is accurate and is all that was ever meant.
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

    let stored = stored_claim(&c, unverified_id).await;
    // No token ever travelled with this event, so no caller could honestly have verified
    // kid_h as its attester — the honest input here is None, not a re-derivation of R1.
    // Asserted from the STORED column rather than assumed, so a door that started writing
    // an attester here would fail this fixture instead of quietly changing what it pins.
    assert_eq!(
        stored.attester, None,
        "an unattested claim must land with attester_key NULL"
    );
    let rust_verdict = classify_authorship_confidence(
        &stored.contributors,
        VerifiedKid::from_event_log_column(&stored.signer),
        None,
    );
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
    let stored = stored_claim(&c, device_id).await;
    assert_eq!(
        stored.attester, None,
        "a device-only assertion carries no attestation"
    );
    let rust_verdict = classify_authorship_confidence(
        &stored.contributors,
        VerifiedKid::from_event_log_column(&stored.signer),
        None,
    );
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
