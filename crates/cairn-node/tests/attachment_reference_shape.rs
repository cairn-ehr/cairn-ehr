//! Issue #370 — a malformed attachment reference is refused LEGIBLY and SKIPPABLY,
//! instead of freezing the clinical pull from the peer that sent it.
//!
//! ## The defect
//!
//! `cairn_learn_attachment_refs` (db/027) is called by **both** clinical doors —
//! `submit_event` (db/005, local authoring) and `apply_remote_event` (db/020, peer
//! admission). It read three fields straight out of a signed body and handed them to
//! `blob_note_reference` with no shape check at all:
//!
//! ```sql
//! PERFORM blob_note_reference(
//!     decode(r ->> 'digest_hex', 'hex'),
//!     r ->> 'media_type',
//!     (r ->> 'byte_len')::bigint);
//! ```
//!
//! A signature proves the bytes are what the author signed — **not** that the payload is
//! well formed. A buggy peer signs its own garbage perfectly, and
//! `Rendition::digest_hex` is a plain `String` with no floor anywhere in `db/*.sql`.
//!
//! ## Why an illegible error is an availability defect
//!
//! `cairn-sync`'s pull loop reads the SQLSTATE, not the message. `refusal_is_deliberate`
//! (crates/cairn-sync/src/main.rs) treats **P0001** — the code a bare `RAISE EXCEPTION`
//! carries — as "the floor decided against these bytes": pen them verbatim, advance the
//! cursor, keep the link alive. **Any other code** means "something broke, the same bytes
//! may apply next cycle": freeze the cursor and retry. So one malformed string from a
//! trusted peer froze that peer's *clinical* pull permanently — re-fetched and re-frozen
//! every cycle, reported to the operator as "transient?", waiting for something that could
//! never clear. Availability over consistency is a governing invariant; this broke it.
//!
//! ## The family is nine, not one
//!
//! #370 names `digest_hex`. Measured against PostgreSQL 18.1 before the fix, the same
//! function had **nine** distinct freeze paths across four SQLSTATE classes —
//!
//! | malformed input | SQLSTATE before |
//! |---|---|
//! | `attachments` not an array (incl. JSON `null`) | 22023 |
//! | `renditions` not an array (incl. JSON `null`) | 22023 |
//! | `digest_hex` not hex / odd length | 22023 |
//! | `digest_hex` absent (NULL address) | 23502 |
//! | a rendition that is a scalar | 23502 |
//! | `media_type` absent | 23502 |
//! | `byte_len` fractional | 22P02 |
//! | `byte_len` beyond bigint | 22003 |
//!
//! — plus **four silent** paths that raised nothing and wrote something wrong:
//! an EMPTY `digest_hex` (every such rendition from every peer collides into one
//! `blob_store` row, because the address is the primary key), a NEGATIVE `byte_len`, a
//! BLANK `media_type`, and an attachment that is a scalar (learns nothing, says nothing).
//! Finding one and fixing one would have left the freeze in place for the other eight.
//!
//! ## Refusal granularity — the question #370 left open, and how it is answered
//!
//! The issue asks whether a malformed *rendition reference* should sink the whole clinical
//! event, or be quarantined while the event is admitted. This fix refuses the **event**,
//! for three reasons:
//!
//! 1. A P0001 refusal is not a loss. ADR-0056 decision 5's residual refusal contract pens
//!    the bytes verbatim, re-offers them, and auto-releases when the refusal stops
//!    applying — and a malformed digest is deterministic, which is exactly the pen's case.
//! 2. Admitting the event while dropping the reference would make the record *look*
//!    complete while an attachment it names is unrecorded and unfetchable — a precise
//!    untruth in the reassuring direction (principle 4), and ADR-0060 decision 2 requires
//!    partial completion to be REPORTED, never implied.
//! 3. ADR-0060's "a defect on one line never invalidates another" governs independent
//!    lines of a composite clinical object. A rendition reference is not another line; it
//!    is part of this event's own body.
//!
//! ## What this suite pins
//!
//! 1. **Source-level: `decode(… 'hex')` is gone from db/027 and the #228 helper is
//!    called.** A later "simplification" back to a bare `decode` restores the freeze with
//!    every behaviour test still passing only if that test does not exist — so it does.
//! 2. **Behaviour: every malformed shape raises P0001**, the skip-and-advance code. This
//!    is the contract with the pull loop and is invisible to any message-only assertion.
//! 3. **Behaviour: each refusal names the field and the reason**, so an operator reading a
//!    pen entry knows what the peer sent that was wrong.
//! 4. **Behaviour: the four silent paths are refused too** — a wrong row written quietly is
//!    the failure mode this floor exists to prevent.
//! 5. **Behaviour: everything that worked still works.** Every refusal added here is a new
//!    way for a peer's clinical event to be penned, so the happy paths are pinned in the
//!    same suite: absent attachments, inline renditions, uppercase hex, an absent/null
//!    `byte_len`, and a digit-string `byte_len` (which the old cast accepted).
//! 6. **End-to-end: the refusal reaches the pull loop through the real apply door**, which
//!    is the only one of the two whose SQLSTATE a *program* consumes.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, Attachment, EventBody, Hlc, Rendition, SigningKey};
use cairn_node::db;
use std::fs;
use std::path::PathBuf;
use tokio_postgres::error::SqlState;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21): safely in the past, so the apply
/// door's drift ceiling admits it.
const WALL_2026: i64 = 1_782_000_000_000;

// ---------------------------------------------------------------------------
// 1. Source-level: the bare decode is gone, the #228 helper is wired in.
// ---------------------------------------------------------------------------

/// db/027, with whole-line `--` comments stripped.
///
/// Stripping is load-bearing rather than tidy: the file *discusses* `decode` and the helper
/// by name in its header, so a prose mention would satisfy either check below while the real
/// call site had changed — a guard passing on documentation instead of code, which is the
/// species PR #448 existed to remove. Trailing comments beside code are left alone; a line is
/// dropped only if it *starts* with `--`, so a real call can never be stripped with it.
fn db027_code() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db/027_attachment_rendition_references.sql")
        .canonicalize()
        .expect("db/027 exists");
    fs::read_to_string(path)
        .expect("read db/027")
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The bare `decode(… 'hex')` must not come back.
///
/// This is the one-line edit that reinstates the whole defect, and it looks like a
/// simplification: `cairn_decode_hex_or_raise` and `decode` have the same shape and the same
/// happy-path result. The difference is only visible in the SQLSTATE of the failure, which no
/// happy-path test observes.
#[test]
fn db027_decodes_hex_through_the_raising_helper_not_bare_decode() {
    let code = db027_code();
    assert!(
        !code.contains("decode(r ->> 'digest_hex'"),
        "db/027 must not decode digest_hex with a bare `decode`: it raises in the 22 class, \
         which cairn-sync reads as transient and freezes the pull cursor on (issue #370)"
    );
    assert!(
        code.contains("cairn_decode_hex_or_raise("),
        "db/027 must route digest_hex through cairn_decode_hex_or_raise (db/001, issue #228), \
         which raises P0001 — the code the pull loop skips past"
    );
}

// ---------------------------------------------------------------------------
// 2-5. Behaviour, driven directly against the function both doors call.
// ---------------------------------------------------------------------------

/// Call `cairn_learn_attachment_refs` with a raw body and report what happened.
///
/// Raw jsonb rather than a built `EventBody`, deliberately: several of the nine malformed
/// shapes (`byte_len` fractional, a rendition that is a scalar, `attachments` that is not an
/// array) are **not expressible** through the typed builders, and those are exactly the ones a
/// foreign encoder produces. Testing only what our own types can emit would leave the floor
/// unexercised against the only writers it is there to catch.
async fn learn(c: &Client, body_json: &str) -> Result<(), tokio_postgres::Error> {
    c.execute(
        "SELECT cairn_learn_attachment_refs($1::text::jsonb)",
        &[&body_json],
    )
    .await
    .map(|_| ())
}

/// The SQLSTATE and message of a refusal, or a panic if the call unexpectedly succeeded.
async fn refusal(c: &Client, body_json: &str, what: &str) -> (SqlState, String) {
    let e = learn(c, body_json)
        .await
        .expect_err(&format!("{what} must be refused, not accepted"));
    let db = e
        .as_db_error()
        .unwrap_or_else(|| panic!("{what}: expected a database error, got {e}"));
    (db.code().clone(), db.message().to_string())
}

/// Every malformed shape carries **P0001** — the skip-and-advance code.
///
/// The headline assertion of #370. Before the fix these produced 22023, 22P02, 22003 and
/// 23502, and `cairn-sync` freezes its cursor on every one of them. The table is the measured
/// family (see the module header), so a fix that repaired `digest_hex` alone fails here on the
/// other eight rather than looking complete.
#[tokio::test]
async fn every_malformed_reference_is_refused_with_the_skip_and_advance_code() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let cases: [(&str, &str); 12] = [
        ("attachments is a scalar", r#"{"attachments":"hello"}"#),
        (
            "renditions is a scalar",
            r#"{"attachments":[{"renditions":"hello"}]}"#,
        ),
        (
            "a rendition is a scalar",
            r#"{"attachments":[{"renditions":[42]}]}"#,
        ),
        (
            "digest_hex is not hex",
            r#"{"attachments":[{"renditions":[{"digest_hex":"0xABC","media_type":"image/png","byte_len":3}]}]}"#,
        ),
        (
            "digest_hex has an odd length",
            r#"{"attachments":[{"renditions":[{"digest_hex":"abc","media_type":"image/png","byte_len":3}]}]}"#,
        ),
        (
            "digest_hex is absent",
            r#"{"attachments":[{"renditions":[{"media_type":"image/png","byte_len":3}]}]}"#,
        ),
        (
            "digest_hex is empty",
            r#"{"attachments":[{"renditions":[{"digest_hex":"","media_type":"image/png","byte_len":3}]}]}"#,
        ),
        (
            "media_type is absent",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20aa","byte_len":3}]}]}"#,
        ),
        (
            "media_type is blank",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20ab","media_type":"   ","byte_len":3}]}]}"#,
        ),
        (
            "byte_len is fractional",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20ac","media_type":"image/png","byte_len":3.5}]}]}"#,
        ),
        (
            "byte_len is negative",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20ad","media_type":"image/png","byte_len":-5}]}]}"#,
        ),
        (
            "byte_len is beyond bigint",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20ae","media_type":"image/png","byte_len":999999999999999999999}]}]}"#,
        ),
    ];

    for (what, body) in cases {
        let (code, message) = refusal(&c, body, what).await;
        assert_eq!(
            code,
            SqlState::RAISE_EXCEPTION,
            "{what}: refused with {code:?} instead of P0001 — cairn-sync reads anything but \
             P0001 as a transient fault and FREEZES the pull cursor (issue #370). Message: {message}"
        );
    }
}

/// A refusal names the field that was wrong.
///
/// Without this, all twelve refusals above could share one message — "malformed attachment
/// reference" — and satisfy every SQLSTATE assertion while telling an operator reading the
/// quarantine pen nothing about what the peer actually sent.
#[tokio::test]
async fn a_refusal_names_the_field_that_was_malformed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let cases: [(&str, &str); 4] = [
        (
            "digest_hex",
            r#"{"attachments":[{"renditions":[{"digest_hex":"0xABC","media_type":"image/png","byte_len":3}]}]}"#,
        ),
        (
            "media_type",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20aa","byte_len":3}]}]}"#,
        ),
        (
            "byte_len",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20ab","media_type":"image/png","byte_len":3.5}]}]}"#,
        ),
        ("attachments", r#"{"attachments":"hello"}"#),
    ];

    for (field, body) in cases {
        let (_, message) = refusal(&c, body, field).await;
        assert!(
            message.contains(field),
            "the refusal must name the malformed field `{field}`, got: {message}"
        );
    }
}

/// Every shape that worked before the fix still works.
///
/// Each refusal added above is a new way for a peer's clinical event to be penned, so the
/// happy paths need pinning in the same suite as the refusals — otherwise a later tightening
/// of the floor turns working replication into a pen full of valid events, and nothing here
/// would notice. `byte_len` as a digit STRING is on the list because the old
/// `(… ->> 'byte_len')::bigint` accepted it: refusing it would be a silent behaviour change
/// dressed as a bug fix.
#[tokio::test]
async fn well_formed_and_previously_accepted_shapes_still_pass() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let cases: [(&str, &str); 8] = [
        ("no attachments key at all", r#"{"payload":{}}"#),
        ("an empty attachment list", r#"{"attachments":[]}"#),
        (
            "attachments as JSON null (means none)",
            r#"{"attachments":null}"#,
        ),
        (
            "renditions as JSON null (means none)",
            r#"{"attachments":[{"renditions":null}]}"#,
        ),
        (
            "an inline rendition, which has no lazy blob to learn",
            r#"{"attachments":[{"renditions":[{"inline":"AAEC","media_type":"image/png"}]}]}"#,
        ),
        (
            "uppercase hex",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1E20B1B2","media_type":"image/png","byte_len":3}]}]}"#,
        ),
        (
            "byte_len absent (length unknown until the bytes arrive)",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20b3b4","media_type":"image/png"}]}]}"#,
        ),
        (
            "byte_len as a digit string, which the old cast accepted",
            r#"{"attachments":[{"renditions":[{"digest_hex":"1e20b5b6","media_type":"image/png","byte_len":"7"}]}]}"#,
        ),
    ];

    for (what, body) in cases {
        learn(&c, body)
            .await
            .unwrap_or_else(|e| panic!("{what} must still be accepted, but was refused: {e}"));
    }
}

/// A well-formed reference still lands in `blob_store` — the function's actual job.
///
/// Pinned separately because every assertion above is about *refusing*, and a validator that
/// refused everything would pass all of them. The reference-eager half of ADR-0013 is the
/// behaviour the door exists for; a guard that never checks it can be satisfied by a floor
/// that has quietly stopped learning anything.
#[tokio::test]
async fn a_valid_reference_is_still_learned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Distinctive address so the assertion cannot be satisfied by a row some other test left.
    let addr_hex = "1e20c0ffee11";
    c.execute(
        "DELETE FROM blob_store WHERE blob_address = decode($1, 'hex')",
        &[&addr_hex],
    )
    .await
    .unwrap();

    let body = format!(
        r#"{{"attachments":[{{"renditions":[{{"digest_hex":"{addr_hex}","media_type":"image/png","byte_len":3}}]}}]}}"#
    );
    learn(&c, &body)
        .await
        .expect("a valid reference is learned");

    let row = c
        .query_one(
            "SELECT media_type, byte_len, present FROM blob_store \
             WHERE blob_address = decode($1, 'hex')",
            &[&addr_hex],
        )
        .await
        .expect("the reference must be in blob_store");
    assert_eq!(row.get::<_, String>(0), "image/png");
    assert_eq!(row.get::<_, i64>(1), 3);
    assert!(
        !row.get::<_, bool>(2),
        "reference-eager, byte-lazy: the row is learned with present = FALSE"
    );
}

/// `cairn_json_list_or_raise` is TOTAL: it always returns a jsonb array, never SQL NULL.
///
/// This exists because the first version of the comment on that function claimed dropping its
/// `COALESCE` would make the guard fail OPEN, and mutation testing showed it would not — every
/// test still passed with the COALESCE removed, because `jsonb_array_elements(NULL)` yields
/// zero rows instead of raising. The claim was wrong in its mechanism and unpinned in its
/// substance, which is the worse half: a safety argument nothing checks disarms the guard it
/// describes (the #385 lesson).
///
/// What is actually true is the contract asserted here — the function never hands its caller a
/// NULL — and it is worth keeping because `jsonb_typeof(NULL)` is NULL, so any future
/// `jsonb_typeof(x) <> \'array\'` check written against a possibly-NULL value silently does
/// nothing (issue #346's fail-open pattern). Totality is what makes such a check safe to add.
#[tokio::test]
async fn the_list_coercion_is_total() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // SQL NULL (an absent key), JSON null, and a real list all come back as jsonb arrays.
    for (what, arg) in [
        ("SQL NULL (the key was absent)", "NULL::jsonb"),
        ("JSON null", "'null'::jsonb"),
        ("an empty list", "'[]'::jsonb"),
    ] {
        let out: Option<String> = c
            .query_one(
                &format!("SELECT cairn_json_list_or_raise({arg}, 'attachments', 'test')::text"),
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            out.as_deref(),
            Some("[]"),
            "{what} must coerce to an empty jsonb ARRAY, not to NULL: a NULL return makes the \
             function non-total, and a later jsonb_typeof check against it fails open (#346)"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. End-to-end: the refusal reaches the pull loop through the real apply door.
// ---------------------------------------------------------------------------

/// A signed `note.added` whose single attachment rendition carries `digest_hex`.
///
/// `Rendition::digest_hex` is a plain `String`, so the malformed value goes through the typed
/// builder and is signed exactly like a well-formed one — which is the point: the signature is
/// valid, and the payload is still garbage.
fn note_with_digest(kid: &str, patient: Uuid, digest_hex: &str) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc {
            wall: WALL_2026,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "arrived by sync"}),
        attachments: vec![Attachment {
            descriptor: "a photograph of the wound".into(),
            renditions: vec![Rendition {
                role: "original".into(),
                alg: "blake3".into(),
                digest_hex: digest_hex.into(),
                media_type: "image/jpeg".into(),
                byte_len: 1024,
                inline: None,
                seal: None,
            }],
        }],
        plaintext_twin: Some("Progress note: arrived by sync".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

async fn enrolled_signer(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event CASCADE")
        .await
        .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"sync-peer-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// **The #370 regression test.** A validly-signed peer event with a malformed `digest_hex`
/// is refused by `apply_remote_event` with P0001, so `cairn-sync` pens it and advances.
///
/// This is the door whose SQLSTATE a *program* reads, so it is the one that turns the defect
/// from a legibility complaint into a permanent clinical-sync outage. Asserting the code and
/// not only the message is the whole point: a fix that produced a beautiful message under
/// SQLSTATE 22023 would leave the freeze exactly where it was.
#[tokio::test]
async fn the_apply_door_refuses_a_malformed_digest_skippably() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = enrolled_signer(&c).await;

    let body = note_with_digest(&kid, Uuid::now_v7(), "0xABC");
    let signed = sign(&body, &sk).unwrap().signed_bytes;

    let e = c
        .execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect_err("a malformed digest_hex must be refused at the apply door");
    let db_err = e.as_db_error().expect("a database error");

    assert_eq!(
        db_err.code(),
        &SqlState::RAISE_EXCEPTION,
        "the apply door must refuse with P0001 so cairn-sync pens and ADVANCES; \
         {:?} freezes the clinical pull from this peer permanently (issue #370). Message: {}",
        db_err.code(),
        db_err.message()
    );
    assert!(
        db_err.message().contains("digest_hex"),
        "the refusal must name the field: {}",
        db_err.message()
    );
}

/// The same body refused at the LOCAL door too — one floor, two doors.
///
/// `submit_event` is where a local author would sign this, and the two doors share
/// `cairn_learn_attachment_refs` precisely so they cannot drift. A fix wired into only the
/// remote door would leave a local client able to write a reference the sync door refuses,
/// which is the divergence db/027 was extracted to prevent.
#[tokio::test]
async fn the_submit_door_refuses_the_same_body() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = enrolled_signer(&c).await;

    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient, WALL_2026).await;

    let body = note_with_digest(&kid, patient, "0xABC");
    let signed = sign(&body, &sk).unwrap().signed_bytes;

    let e = c
        .execute("SELECT submit_event($1)", &[&signed])
        .await
        .expect_err("a malformed digest_hex must be refused at the submit door");
    let db_err = e.as_db_error().expect("a database error");
    assert_eq!(db_err.code(), &SqlState::RAISE_EXCEPTION);
    assert!(
        db_err.message().contains("digest_hex"),
        "the refusal must name the field: {}",
        db_err.message()
    );
}

// Shared scaffolding, for `submit_registration`: since #345 the first event on a chart must
// be its registration, so every suite that mints a patient arranges one (#120/#327 — one copy).
mod common;
