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
//! ## Refusal granularity: the doors differ, and #370's first fix broke a written rule
//!
//! #370 asked whether a malformed *rendition reference* should sink the whole clinical event.
//! It refused at both doors. That contradicted [ADR-0063], written eight days earlier, which
//! decides the same shape for the §5.9 `safety` field in a table — *malformed field: local
//! door REFUSE, remote door ADMIT* — and states the rule generally: **an envelope-level field
//! is constrained where it is MINTED and read permissively where it ARRIVES.**
//!
//! Its rejected-alternatives section rejects apply-door refusal in terms that never mention
//! `safety`: *"the safety signal is a field on a clinical event, so refusing it at apply drops
//! the medication assertion — an advisory field cancelling clinical content, which ADR-0060
//! forbids in as many words. It also forks the event set between honest peers running
//! different versions (the #342 trap, hit four times in this project already)."* #370 made it
//! five.
//!
//! An attachment rendition reference is the same category. A **sensitivity assertion** IS an
//! event, so refusing a malformed one drops one assertion. `safety`, `clock_grade` and a
//! rendition reference are **fields on** a clinical event — refusing one at apply drops the
//! note, the medication assertion, the clinical act it rode on. ADR-0063 names the deciding
//! argument: **blast radius, not category.**
//!
//! So `submit_event` refuses (the field is being minted, the author is present, and this node
//! is the only one that can stop a permanently-defective event entering an append-only
//! replicating record) and `apply_remote_event` admits-and-flags (the event is already a fact;
//! refusing forks the event set, and the pen never releases because the malformed field sits
//! inside a signature the author cannot re-issue). Issue #461 proposes giving that rule its own
//! ADR, since it is currently findable only under another field's title — which is exactly how
//! #370 missed it.
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

/// A rendition with a chosen digest, media type and role — for events that carry more than one.
fn rendition(role: &str, digest_hex: &str) -> Rendition {
    Rendition {
        role: role.into(),
        alg: "blake3".into(),
        digest_hex: digest_hex.into(),
        media_type: "image/jpeg".into(),
        byte_len: 1024,
        inline: None,
        seal: None,
    }
}

/// The flag rows this node recorded for one event, as `(attachment_index, rendition_index, reason)`.
async fn flags_for(c: &Client, event_id: &str) -> Vec<(i32, i32, String)> {
    c.query(
        "SELECT attachment_index, rendition_index, reason FROM attachment_reference_flag \
         WHERE event_id = $1::text::uuid ORDER BY attachment_index, rendition_index",
        &[&event_id],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}

async fn is_in_event_log(c: &Client, event_id: &str) -> bool {
    c.query_one(
        "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
        &[&event_id],
    )
    .await
    .unwrap()
    .get::<_, i64>(0)
        == 1
}

/// **The #460 regression test.** A validly-signed peer event with a malformed `digest_hex` is
/// ADMITTED, and the unlearnable reference is recorded.
///
/// The event is already a fact of the world by the time it reaches this door. Refusing does not
/// un-mint it — it forks the event set, hiding clinical content this node's peers can read, and
/// the pen never releases because the malformed field sits inside a signature the author cannot
/// re-issue. So: withhold the REFERENCE, never the EVENT (Slice 66's rule, one level down).
///
/// The assertion is deliberately in three parts — admitted, no reference learned, and the fact
/// RECORDED — because any two of them without the third is a defect. Admitted-and-silent is the
/// "record looks complete" untruth #370's first fix was right to fear; admitted-and-learned would
/// put a garbage address in `blob_store`.
#[tokio::test]
async fn the_apply_door_admits_a_malformed_digest_and_flags_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = enrolled_signer(&c).await;

    let body = note_with_digest(&kid, Uuid::now_v7(), "0xABC");
    let signed = sign(&body, &sk).unwrap().signed_bytes;

    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect("a malformed rendition reference must not sink the clinical event");

    assert!(
        is_in_event_log(&c, &body.event_id).await,
        "the event must be in the record: refusing it forks the event set (issue #460)"
    );

    let flags = flags_for(&c, &body.event_id).await;
    assert_eq!(
        flags.len(),
        1,
        "exactly one unlearnable rendition must be recorded, got {flags:?}"
    );
    let (att, ren, reason) = &flags[0];
    assert_eq!((*att, *ren), (0, 0), "the flag must NAME which rendition");
    assert!(
        reason.contains("digest_hex"),
        "the recorded reason must be the accessor's own refusal text: {reason}"
    );
}

/// The same body is still REFUSED at the local door — the other half of the asymmetry.
///
/// At `submit_event` the event is not yet a fact of the world and this node is the only one that
/// can stop it: admitting mints a permanently-defective event into an append-only, replicating
/// record, correctable only by overlay, with the broken original resident for the life of the
/// record. This is the same strict/lenient split the floor already uses for #345's registration
/// precedence and for the shred target-existence requirement.
///
/// If this ever goes green by the submit door admitting, the asymmetry has collapsed into "admit
/// everywhere" and the local guard is gone.
#[tokio::test]
async fn the_submit_door_still_refuses_the_same_body() {
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
        .expect_err("a malformed digest_hex must be refused at the SUBMIT door");
    let db_err = e.as_db_error().expect("a database error");
    assert_eq!(db_err.code(), &SqlState::RAISE_EXCEPTION);
    assert!(
        db_err.message().contains("digest_hex"),
        "the refusal must name the field: {}",
        db_err.message()
    );
    assert!(
        !is_in_event_log(&c, &body.event_id).await,
        "a refused submit must write nothing"
    );
}

/// **A defect on one rendition never invalidates another** — ADR-0060, applied where it does fit.
///
/// Three renditions, the middle one malformed. The two good references are learned and the bad one
/// is flagged. A fix that abandoned the whole attachment at the first fault would pass every other
/// test in this file while quietly losing a preview the node could have fetched.
#[tokio::test]
async fn a_defect_on_one_rendition_never_invalidates_its_siblings() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = enrolled_signer(&c).await;

    let good_a = "1e20d0d0d0d0";
    let good_b = "1e20d1d1d1d1";
    for hex in [good_a, good_b] {
        c.execute(
            "DELETE FROM blob_store WHERE blob_address = decode($1, 'hex')",
            &[&hex],
        )
        .await
        .unwrap();
    }

    let mut body = note_with_digest(&kid, Uuid::now_v7(), good_a);
    body.attachments[0].renditions = vec![
        rendition("original", good_a),
        rendition("preview", "0xNOPE"),
        rendition("extracted-text", good_b),
    ];
    let signed = sign(&body, &sk).unwrap().signed_bytes;

    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect("the event is admitted");

    for hex in [good_a, good_b] {
        let n: i64 = c
            .query_one(
                "SELECT count(*) FROM blob_store WHERE blob_address = decode($1, 'hex')",
                &[&hex],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            n, 1,
            "the well-formed rendition {hex} must still be learned — a defect on one line never \
             invalidates another (ADR-0060)"
        );
    }

    let flags = flags_for(&c, &body.event_id).await;
    assert_eq!(
        flags.len(),
        1,
        "only the malformed rendition is flagged, got {flags:?}"
    );
    assert_eq!(
        (flags[0].0, flags[0].1),
        (0, 1),
        "the flag must name rendition index 1, the middle one"
    );
}

/// Re-applying the same event records no second flag — sync is set-union.
///
/// `cairn-sync` re-offers bytes freely (a full sweep, a re-pull from zero, a peer serving the same
/// event twice). Without a dedup key the ledger would grow one row per delivery and an operator
/// reading it would see one defect as many.
#[tokio::test]
async fn re_applying_the_same_event_adds_no_second_flag() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = enrolled_signer(&c).await;

    let body = note_with_digest(&kid, Uuid::now_v7(), "0xABC");
    let signed = sign(&body, &sk).unwrap().signed_bytes;

    for pass in 1..=2 {
        c.execute("SELECT apply_remote_event($1)", &[&signed])
            .await
            .unwrap_or_else(|e| panic!("apply pass {pass} must succeed: {e}"));
    }

    assert_eq!(
        flags_for(&c, &body.event_id).await.len(),
        1,
        "a re-offered event must dedupe onto its existing flag row"
    );
}

/// A flagged event is NOT deferred: it projects, and confers what it normally confers.
///
/// `event_deferred` (ADR-0056) means "admitted uninterpreted — projects nothing, confers nothing",
/// and it is the tempting place to put this flag because a row already exists for "something about
/// this event is not fully handled". Reusing it would suppress the clinical content, which is
/// nearly as harmful as the refusal being removed. The whole point is that only the blob reference
/// is unlearnable; the event itself is ordinary.
#[tokio::test]
async fn a_flagged_event_is_not_treated_as_deferred() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = enrolled_signer(&c).await;

    let body = note_with_digest(&kid, Uuid::now_v7(), "0xABC");
    let signed = sign(&body, &sk).unwrap().signed_bytes;
    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .unwrap();

    let deferred: i64 = c
        .query_one(
            "SELECT count(*) FROM event_deferred WHERE event_id = $1::text::uuid",
            &[&body.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        deferred, 0,
        "an unlearnable attachment reference must not defer the event: a deferred event projects \
         nothing and confers nothing, which would suppress the clinical content this fix exists \
         to preserve (issue #460)"
    );
}

/// **The safety property that makes the lenient path acceptable at all: a REAL fault still
/// propagates.**
///
/// The lenient learner records a refusal instead of raising it, which is only sound while it can
/// tell *our* refusal from someone else's failure. It catches `raise_exception` (P0001) — the code
/// our own accessors raise — and nothing else. `WHEN OTHERS` would be the disaster: a disk error, a
/// serialization failure or a broken constraint would be silently written down as "the peer sent
/// garbage" and the event admitted as if nothing had gone wrong. (`OTHERS` also does not catch a
/// statement timeout — 57014 is one of the two codes it excludes — the Slice 68 lesson.)
///
/// Driven by a temporary trigger that raises a 22-class error on one specific address, so the fault
/// is real, deterministic, and provably not ours.
#[tokio::test]
async fn the_lenient_learner_does_not_swallow_a_real_fault() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = enrolled_signer(&c).await;

    // A fault that is emphatically NOT one of our accessors' P0001 refusals.
    c.batch_execute(
        "CREATE OR REPLACE FUNCTION cairn_test_blob_fault() RETURNS trigger \
         LANGUAGE plpgsql AS $f$ BEGIN \
             IF NEW.blob_address = decode('1e20facade01', 'hex') THEN \
                 RAISE EXCEPTION 'injected infrastructure fault' USING ERRCODE = '22023'; \
             END IF; RETURN NEW; END $f$;
         DROP TRIGGER IF EXISTS cairn_test_blob_fault_trg ON blob_store;
         CREATE TRIGGER cairn_test_blob_fault_trg BEFORE INSERT ON blob_store \
             FOR EACH ROW EXECUTE FUNCTION cairn_test_blob_fault();",
    )
    .await
    .unwrap();

    let body = note_with_digest(&kid, Uuid::now_v7(), "1e20facade01");
    let signed = sign(&body, &sk).unwrap().signed_bytes;
    let result = c.execute("SELECT apply_remote_event($1)", &[&signed]).await;

    // Tear the trigger down BEFORE asserting, so a failure cannot poison every later test.
    c.batch_execute(
        "DROP TRIGGER IF EXISTS cairn_test_blob_fault_trg ON blob_store; \
         DROP FUNCTION IF EXISTS cairn_test_blob_fault();",
    )
    .await
    .unwrap();

    let e = result.expect_err(
        "a genuine infrastructure fault must NOT be swallowed and recorded as a malformed \
         reference — that is the silent-failure species this ledger must never become",
    );
    let code = e.as_db_error().expect("a database error").code().clone();
    assert_eq!(
        code,
        SqlState::from_code("22023"),
        "the real fault must reach the caller unchanged, so cairn-sync can treat it as transient \
         and RETRY; got {code:?}"
    );
    assert_eq!(
        flags_for(&c, &body.event_id).await.len(),
        0,
        "a real fault must leave no flag row claiming the peer sent a malformed reference"
    );
}

// Shared scaffolding, for `submit_registration`: since #345 the first event on a chart must
// be its registration, so every suite that mints a patient arranges one (#120/#327 — one copy).
mod common;
