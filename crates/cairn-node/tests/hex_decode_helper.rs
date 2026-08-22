//! Issue #228 — a malformed hex payload field is refused LEGIBLY, naming its door.
//!
//! ## The defect
//!
//! Three node-plane doors read a node-id out of an event payload as a hex string and
//! `decode(…, 'hex')` it: `submit_node_event` and `apply_remote_node_event` (db/007) and
//! `restore_node_event` (db/009). All of them guarded the **NULL** case — db/007's four
//! guards by field name ("missing peer_node_id_hex in payload", door named), db/009's with
//! a generic "missing subject node id" — and then passed the **non-NULL, malformed** case
//! straight into `decode`, which raises PostgreSQL's own
//! `invalid hexadecimal digit: "x"` (or the odd-length variant) with **no door name, no
//! field name, no author**.
//!
//! A trusted-but-buggy peer that ships `"peer_node_id_hex": "0xABC"` therefore produced an
//! error indistinguishable from any other hex failure anywhere in the session — against the
//! house rule the db/007 header states: *every rejection is legible*.
//!
//! ## It was also a sync stall (which #228 did not know)
//!
//! Nothing unsafe is ever STORED — the doors fail closed either way — but the refusal's
//! SQLSTATE is read by a program, not only by a human. `sync.rs`'s pull loop treats a bare
//! `RAISE EXCEPTION` (P0001) as a deliberate, self-healing deny-all and skips past it,
//! and treats **any other code** as a possible transient DB fault, freezing the cursor
//! below that seq rather than risk losing a valid event (the #111 review's A1). A bare
//! `decode` raises in PostgreSQL's 22 class, and the signature check upstream never looks
//! at the payload — so one malformed field from a trusted peer froze node-plane pull from
//! that peer *permanently*, re-fetched and re-frozen every cycle, reported as "transient?".
//!
//! So the fix is an availability fix as well as a legibility one, and P0001 is now a
//! contract between the helper and the pull loop rather than an accident of how the raise
//! is written. `refuses_malformed_hex_with_the_skip_and_advance_code` is what keeps a
//! well-meaning later `USING ERRCODE = SQLSTATE` from silently reinstating the freeze.
//!
//! ## The fix
//!
//! One helper, `cairn_decode_hex_or_raise(field, value, door)` in **db/001**, at six call
//! sites. Modelled on the issue-#227 extraction (`cairn_node_hlc_merge`), including its
//! placement rule — see `the_helper_is_declared_in_db001_so_every_subset_can_reach_it`.
//!
//! ## What this suite pins
//!
//! 1. **Source-level: the helper is declared in db/001 and nowhere else.** Placement is
//!    load-bearing, not cosmetic (the #198 late-binding trap).
//! 2. **Source-level: every door still CALLS it.** The mirror-image failure the #227
//!    review caught — a guard against re-growing a copy says nothing about a call site
//!    silently vanishing, and a vanished call restores exactly the illegible error #228
//!    was filed about, with the whole tree green.
//! 3. **Behaviour: a malformed value names door, field and reason** — and is not
//!    PostgreSQL's bare hex error. Its DETAIL says WHICH hex fault, because truncation and
//!    wrong-encoding want opposite responses from whoever reads the log.
//! 4. **Behaviour: the refusal never echoes the whole value, AT ANY LENGTH.** Node-ids are
//!    not secret, but a general-purpose hex decoder outlives that assumption and door
//!    errors land in logs. The lengths tested straddle the cap on purpose — see
//!    `the_refusal_characterises_the_value_instead_of_echoing_it` for the version of this
//!    that a review had to catch.
//! 5. **Behaviour: NULL fails closed, valid hex is unchanged.** The helper must be a
//!    drop-in for `decode(…, 'hex')` on the happy path or the doors' semantics moved.
//! 6. **Behaviour: the refusal carries P0001**, the skip-and-advance code — the contract
//!    with the pull loop described above, invisible to every message-only assertion.
//! 7. **End-to-end: the message actually reaches a caller** through all three doors, and
//!    through BOTH payload fields (`peer_node_id_hex` and `superseded_node_id_hex`) on the
//!    peer path — proof the helper is wired in, not just declared.
//!    `apply_remote_node_event` matters most of the three: it is the only one on the peer
//!    path, so it is the one whose refusal a *program* consumes.
//!
//! DB-backed cases use real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide
//! via `db::test_serial_guard` (shared-DB pattern).
use cairn_event::{
    event_address, short_fingerprint, sign, EventBody, Hlc, PairingBundle, SigningKey,
};
use cairn_node::{db, identity, keystore};
use std::fs;
use std::path::PathBuf;
use tokio_postgres::error::SqlState;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Repo-root `db/` directory. `CARGO_MANIFEST_DIR` is `crates/cairn-node`; `db/` is two
/// levels up.
fn db_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db")
        .canonicalize()
        .expect("db/ dir")
}

/// Every `*.sql` migration directly under `db/`, as (file name, CODE). `read_dir` is not
/// recursive, so the SQL mirrors under `db/tests/` are correctly not included — they
/// mention the helper too and would skew every count below.
///
/// Whole-line `--` comments are stripped, and that is load-bearing rather than tidy: the
/// counting guard below would otherwise be the one way these source-level checks can fail
/// OPEN. These migrations discuss the helper by name in prose — db/007 twice, db/009 once,
/// db/001 throughout — so a future comment that quotes an example call would hold the count at its
/// expected value while a real call site was deleted, and the guard would pass. Stripping
/// comments makes prose unable to substitute for code. Trailing comments after code are
/// left alone: a line is only dropped if it *starts* with `--`, so a real call can never
/// be stripped along with a comment beside it.
fn migrations() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        let code: String = sql
            .lines()
            .filter(|l| !l.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        out.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            code,
        ));
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// 1. Source-level: one declaration, in db/001.
// ---------------------------------------------------------------------------

/// The helper must be declared exactly once, and in `db/001` — NOT beside the doors that
/// use it in db/007.
///
/// This is the same non-obvious placement rule `cairn_node_hlc_merge` carries, and it is
/// worth its own guard for the same reason: cairn-sync loads a SUBSET of the migrations
/// that includes db/001 but not db/007 or db/009, and PL/pgSQL resolves a function call at
/// first EXECUTION rather than at definition. A helper declared in db/007 and called from
/// any future clinical-plane door would let cairn-sync's schema load cleanly and then fail
/// on its first admitted event — a first-write outage, the late-binding trap issue #198
/// was filed for. Today's six call sites are all node-plane; the next one need not be, and
/// a decode-hex helper is exactly the sort of thing a later door reaches for.
#[test]
fn the_helper_is_declared_in_db001_so_every_subset_can_reach_it() {
    let needle = "CREATE OR REPLACE FUNCTION cairn_decode_hex_or_raise(";
    let declaring: Vec<String> = migrations()
        .into_iter()
        .filter(|(_, sql)| sql.contains(needle))
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        declaring,
        vec!["001_envelope.sql".to_string()],
        "cairn_decode_hex_or_raise must be declared ONLY in db/001 — a subset load that \
         omits the declaring migration turns PL/pgSQL late binding into a first-write \
         outage (#198). Found in: {declaring:?}"
    );
}

/// Every hex-decoding door must still CALL the helper.
///
/// The guard above forbids the declaration from moving or being duplicated. It says
/// nothing about the opposite failure: a call site reverting to a bare `decode(…, 'hex')`.
/// That reversion is invisible in review (one expression, unchanged shape) and restores
/// precisely the illegible refusal #228 was filed about — with no test anywhere in the
/// tree going red, because a malformed value is still *refused*, just not legibly. The
/// end-to-end cases below cover two of the six sites; this count covers all six.
///
/// The needle matches a CALL and not the declaration or the REVOKE, because every call
/// passes its field name as a literal first argument (`cairn_decode_hex_or_raise('peer_…`)
/// while the declaration takes named parameters and the REVOKE takes bare types. It counts
/// over `migrations()`, which strips comment lines — see there for why that is the
/// difference between this guard failing closed and failing open. Sharing the
/// whitespace-sensitivity limitation of the #173/#227 guards it is modelled on: a call
/// written with a different quoting or spacing would not be counted, which fails CLOSED
/// (the test goes red and a human looks) — the safe direction for a needle to be wrong.
#[test]
fn every_hex_door_still_calls_the_helper() {
    let needle = "cairn_decode_hex_or_raise('";
    // (migration, how many of its arms decode a hex node-id from a payload)
    let want: Vec<(String, usize)> = [
        // submit_node_event: supersede + peer/revoke;
        // apply_remote_node_event: supersede + peer/revoke
        ("007_node_federation.sql", 4),
        // restore_node_event: both branches of the v_subject CASE
        ("009_node_supersede_and_restore.sql", 2),
        // cairn_rendition_address: the content address of a by-reference attachment
        // rendition (issue #370). The FIRST clinical-plane call site, and it matters for the
        // same reason db/048's apply-side one does: this accessor is reached from BOTH
        // clinical doors (db/005 through the strict learner, db/020 through db/050's lenient
        // one since #460 — they differ only in what they do with the refusal, not in which
        // accessors they call), so a bare decode() here raised in the 22 class and froze the
        // CLINICAL pull from the peer that sent the malformed digest — strictly worse than
        // the node plane, which stalls peering metadata rather than the record.
        ("027_attachment_rendition_references.sql", 1),
        // §5.9 sensitivity (ADR-0062): the withdrawal names the assertion it withdraws by
        // hex content_address, and BOTH the structural floor and the projection apply fn
        // decode it — cairn_check_sensitivity_withdrawal, then sensitivity_withdrawal_apply.
        // The apply-side call is the one that matters most: it runs on the REMOTE door, so a
        // bare decode() there would raise in the 22 class and freeze that peer's pull cursor
        // rather than skipping past a malformed event.
        ("048_sensitivity_stream.sql", 2),
    ]
    .iter()
    .map(|(f, n)| (f.to_string(), *n))
    .collect();

    let got: Vec<(String, usize)> = migrations()
        .into_iter()
        .map(|(name, sql)| (name, sql.matches(needle).count()))
        .filter(|(_, calls)| *calls > 0)
        .collect();
    assert_eq!(
        got, want,
        "every door that decodes a hex payload field must route through \
         cairn_decode_hex_or_raise — a bare decode() there is the illegible refusal of \
         issue #228"
    );
}

// ---------------------------------------------------------------------------
// 2. Behaviour of the helper itself.
// ---------------------------------------------------------------------------

/// The three parts of a refusal this suite asserts on, each for a different audience.
struct Refusal {
    /// What a human reads in a log: door, field, reason, value characterisation.
    message: String,
    /// Which of the two hex faults it was — the part that tells a truncated value apart
    /// from a wrongly-encoded one.
    detail: String,
    /// What `sync.rs`'s pull loop branches on. Must be P0001.
    code: String,
}

/// Ask the helper to decode `value` and return the refusal it raised.
///
/// Passing the value as a bound parameter rather than interpolating it keeps the odd
/// shapes below (a `0x` prefix, stray quotes) from having to be SQL-escaped, and makes the
/// test exercise the same path a door does — a runtime TEXT value, not a literal the
/// planner could fold.
async fn decode_error(
    c: &tokio_postgres::Client,
    field: &str,
    value: Option<&str>,
    door: &str,
) -> Refusal {
    let err = c
        .query_one(
            "SELECT cairn_decode_hex_or_raise($1, $2, $3)",
            &[&field, &value, &door],
        )
        .await
        .expect_err("a malformed or missing hex value must be refused");
    let db = err
        .as_db_error()
        .expect("the refusal must be a database error, not a transport failure");
    Refusal {
        message: db.message().to_string(),
        detail: db.detail().unwrap_or_default().to_string(),
        code: db.code().code().to_string(),
    }
}

/// The refusal must carry P0001 — the code the pull loop reads as "deliberate, skip past
/// it" — and not PostgreSQL's own 22-class hex error.
///
/// This is the assertion that makes the whole change an availability fix rather than a
/// cosmetic one, and it is invisible to every message-based check in this file.
///
/// `crates/cairn-node/src/sync.rs` classifies a door refusal on a VERIFIED event by
/// SQLSTATE: P0001 (a bare `RAISE EXCEPTION`) is a deliberate deny-all → count it,
/// skip-and-advance, re-offer on a later full sweep. Anything else is assumed to be a
/// transient DB fault — serialization failure, deadlock, timeout — and the loop `break`s,
/// freezing the cursor below that seq so a valid event cannot be silently lost (#111 A1).
///
/// Before this change the bare `decode` raised 22P02/22000, which took the freeze arm: one
/// malformed hex field from a trusted-but-buggy peer stalled node-plane pull from that
/// peer forever, because the same event was re-offered and re-frozen on every cycle. It
/// even logged as "transient/unexpected … (not skipped past)", so the operator was told to
/// wait for something that would never clear.
///
/// The regression this guards is small and plausible: adding `USING ERRCODE = SQLSTATE` to
/// the helper's raise, to "preserve Postgres's own code" now that DETAIL carries the
/// reason. That reinstates the permanent freeze while every message assertion in this file
/// and in the SQL mirror stays green.
#[tokio::test]
async fn refuses_malformed_hex_with_the_skip_and_advance_code() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    for (value, case) in [(Some("0xABC"), "a malformed value"), (None, "a NULL value")] {
        let r = decode_error(&c, "peer_node_id_hex", value, "apply_remote_node_event").await;
        assert_eq!(
            r.code,
            SqlState::RAISE_EXCEPTION.code(),
            "{case} must be refused with P0001 (deliberate → skip-and-advance); a 22-class \
             code puts sync.rs's pull loop on its FREEZE arm and stalls the peer forever"
        );
    }
}

/// DETAIL names WHICH hex fault it was, so the message's characterisation is actionable.
///
/// The helper checks the value's shape itself rather than catching what `decode` raises
/// (db/034's idiom — see the helper's header for why a catch-all `WHEN others` was the
/// wrong tool: it relabels an unrelated internal fault as bad caller input). Checking
/// first means the helper owns the reason text, so the reason has to be asserted here or
/// it can rot into something useless without anything going red.
///
/// The two faults want opposite responses from whoever reads the log: an odd digit count
/// says the value was TRUNCATED in transit or assembly, a bad character says it was
/// ENCODED wrongly (a `0x` prefix, a UUID's dashes, base64). Collapsing them to one
/// message would put the operator back to guessing.
#[tokio::test]
async fn the_detail_says_which_of_the_two_hex_faults_it_was() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // "abcde" is all hex digits but an odd count → truncation. "zzzz" is an even count of
    // non-hex characters → wrong encoding. One case per arm, chosen so neither can be
    // explained by the other.
    let truncated = decode_error(&c, "peer_node_id_hex", Some("abcde"), "submit_node_event").await;
    assert!(
        truncated.detail.contains("odd number"),
        "an odd digit count must be reported as truncation; got detail: {}",
        truncated.detail
    );
    let misencoded = decode_error(&c, "peer_node_id_hex", Some("zzzz"), "submit_node_event").await;
    assert!(
        misencoded.detail.contains("not a hex digit"),
        "a bad character must be reported as a wrong encoding; got detail: {}",
        misencoded.detail
    );
}

/// A malformed value is refused with a message that names the DOOR, the FIELD and the
/// reason — the three things PostgreSQL's own `invalid hexadecimal digit: "x"` omits.
///
/// The three shapes are the three ways real payloads go wrong: a language that writes hex
/// with a `0x` prefix, a value truncated to an odd number of nibbles, and a value that is
/// not hex at all (a UUID with dashes, a base64 blob, a display name).
#[tokio::test]
async fn a_malformed_value_names_the_door_the_field_and_the_reason() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    for (value, why) in [
        ("0xABC", "a 0x-prefixed value"),
        ("abc", "an odd number of nibbles"),
        ("zzzz", "a non-hex digit"),
    ] {
        let r = decode_error(&c, "peer_node_id_hex", Some(value), "submit_node_event").await;
        let msg = &r.message;
        assert!(
            msg.contains("submit_node_event"),
            "{why} must name the door it was refused at; got: {msg}"
        );
        assert!(
            msg.contains("peer_node_id_hex"),
            "{why} must name the payload field; got: {msg}"
        );
        assert!(
            msg.contains("not valid hex"),
            "{why} must state the reason in the message, not only in DETAIL; got: {msg}"
        );
        // SHORT values must be truncated too — see the non-echo test below for why this
        // assertion lives here as well. These three fixtures are all under 8 characters,
        // and a cap of "at most 8" silently shows them whole.
        assert!(
            !msg.contains(value),
            "{why} must not be echoed in full even though it is short; got: {msg}"
        );
    }
}

/// The refusal characterises the value (length + a strict prefix) instead of echoing it —
/// at EVERY length, which is the part the first version of this suite got wrong.
///
/// Node-ids are content addresses and carry nothing secret, so this is not a leak fix
/// today — it is the habit that keeps it from becoming one. A general-purpose hex decoder
/// is exactly the helper a later door reaches for when reading a key, a token or a
/// wrapped DEK out of a payload, and a door error is written to logs that outlive the
/// session. The length and prefix are what a human debugging a buggy peer actually needs.
///
/// The lengths below are deliberately spread across the interesting boundary. A cap of
/// "at most 8 characters" reads as safe and is not: for anything 8 characters or shorter
/// it degrades to the whole value, and 8 hex characters is a 4-byte secret. This suite
/// originally asserted non-echo only against a 42-char fixture and so proved nothing about
/// the case that actually leaks (PR #371 review). The helper now shows at most HALF the
/// value, capped at 8, and always marks the elision — so something is hidden at every
/// length, and the `...` never lies.
#[tokio::test]
async fn the_refusal_characterises_the_value_instead_of_echoing_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Straddling the 8-char cap: well under it, exactly on it, and well over.
    for (value, why) in [
        ("zzzz".to_string(), "a 4-char value, far under the cap"),
        (
            "zzzzzzzz".to_string(),
            "an 8-char value, exactly at the cap",
        ),
        ("aabbccdd".repeat(5) + "zz", "a 42-char value, over the cap"),
    ] {
        let r = decode_error(
            &c,
            "superseded_node_id_hex",
            Some(&value),
            "restore_node_event",
        )
        .await;
        let msg = &r.message;
        assert!(
            !msg.contains(&value),
            "{why}: the refusal must not echo the whole value; got: {msg}"
        );
        assert!(
            msg.contains(&format!("{} chars", value.len())),
            "{why}: the refusal must report the value's length so a truncation is obvious; \
             got: {msg}"
        );
        assert!(
            msg.contains("..."),
            "{why}: the refusal must mark that the value was elided; got: {msg}"
        );
    }

    // The prefix still has to be worth printing: enough leading characters survive to
    // identify the value and to spot a wrong encoding at a glance.
    let r = decode_error(
        &c,
        "peer_node_id_hex",
        Some("0xABCDEF"),
        "submit_node_event",
    )
    .await;
    assert!(
        r.message.contains("0x"),
        "the prefix must keep enough to see a 0x-style encoding error; got: {}",
        r.message
    );
}

/// NULL fails closed with a legible "missing" message rather than returning NULL.
///
/// This arm is deliberately NOT reachable from db/007's two doors, which keep their own
/// richer NULL guards (they can name the authoring peer, which the helper cannot). It is
/// reachable from db/009 — whose guard used to sit AFTER the decode and could only say
/// "missing subject node id", not which field — and it is what stops the helper from being
/// declared `STRICT`, which would silently return NULL on NULL input and hand the doors a
/// NULL subject for their NOT NULL column to reject opaquely.
#[tokio::test]
async fn a_null_value_fails_closed_and_names_the_field() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let r = decode_error(&c, "peer_node_id_hex", None, "restore_node_event").await;
    let msg = &r.message;
    // Both halves matter: naming the helper's caller alone would also be satisfied by
    // Postgres's own "function … does not exist", i.e. it would pass before the helper is
    // written at all.
    assert!(
        msg.contains("peer_node_id_hex") && msg.contains("missing"),
        "a NULL value must be refused by field name; got: {msg}"
    );
}

/// On the happy path the helper is byte-for-byte `decode(v, 'hex')`.
///
/// The whole change is a no-op for every well-formed event on the wire, and this is the
/// assertion that says so: six doors swapped their decode for a call, and a difference
/// here would mean the subject node-id they store had changed. Mixed case is included
/// because peers do not agree on it (`identity::author_supersede` lowercases; a hand-built
/// payload may not) and `decode` accepts both.
#[tokio::test]
async fn valid_hex_decodes_exactly_as_before() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    for value in ["", "deadbeef", "1220AABB", &("ab".repeat(33))] {
        let same: bool = c
            .query_one(
                "SELECT cairn_decode_hex_or_raise('f', $1, 'd') = decode($1, 'hex')",
                &[&value],
            )
            .await
            .unwrap_or_else(|e| panic!("valid hex {value:?} must decode, not raise: {e}"))
            .get(0);
        assert!(
            same,
            "cairn_decode_hex_or_raise must agree with decode() on {value:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. End-to-end: the legible message reaches a door's caller.
// ---------------------------------------------------------------------------

/// Mint a signed `peer.added` event for an arbitrary key (no DB), with a caller-chosen
/// `peer_node_id_hex` so the malformed case can be built. Local to this suite, mirroring
/// `restore.rs`'s `synth_peer` — the test crates are separate, and a shared helper in
/// `tests/common` carries its own upkeep (the derivation pin in
/// `identity_scaffolding_shared.rs`).
fn synth_peer(sk: &SigningKey, name: &str, peer_node_id_hex: &str) -> Vec<u8> {
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "peer.added".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 2,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({
            "peer_node_id_hex": peer_node_id_hex, "peer_pubkey": kid,
            "fingerprint": "fp", "role": "peer"
        }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Mint a signed `node.superseded` event for an arbitrary key (no DB), with a
/// caller-chosen `superseded_node_id_hex`. The supersede arm reads a DIFFERENT payload
/// field from the peer arm, so it needs its own fixture to be exercised end-to-end.
fn synth_supersede(sk: &SigningKey, name: &str, superseded_node_id_hex: &str) -> Vec<u8> {
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.superseded".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 3,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid,
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "superseded_node_id_hex": superseded_node_id_hex }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Mint a signed `node.enrolled` event for an arbitrary key (no DB), so the restore door's
/// non-enroll branch has an author to resolve. Mirrors `identity::provision`'s genesis, so
/// its content-address IS the node-id.
fn synth_enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid,
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// The PEER-ADMISSION door refuses a malformed `peer_node_id_hex` legibly AND with the
/// skip-and-advance code — the case that matters most in this file.
///
/// `apply_remote_node_event` is the only one of the three doors on the peer path, so it is
/// the only one whose refusal is consumed by a *program* rather than read by an operator:
/// `sync.rs`'s pull loop branches on the SQLSTATE (see
/// `refuses_malformed_hex_with_the_skip_and_advance_code` for the full argument). It is
/// also where the original defect actually bit — a buggy peer's event, not an operator's
/// typo — and the P0001 contract is unobservable anywhere except here and in the helper.
///
/// The setup is the real trust path, because that is the only way to reach the malformed
/// field at all: A provisions, A pairs with B out-of-band, B's genesis is admitted (so B's
/// key resolves to a node), and only THEN does B's malformed `peer.added` get past the
/// deny-all gate and reach the decode. An un-peered author is refused earlier, for a
/// different reason, and would prove nothing about this change.
#[tokio::test]
async fn apply_remote_node_event_refuses_malformed_hex_legibly_and_skippably() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7940")
        .await
        .unwrap();

    // B's genesis: its content-address IS B's node-id, which is what A pins when pairing.
    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let genesis_b = synth_enroll(&sk_b, "B");
    let b_node_id = hex::encode(event_address(&genesis_b));

    // A pairs with B out-of-band, then admits B's genesis so kid_b resolves to a node.
    let bundle = PairingBundle {
        node_id_hex: b_node_id.clone(),
        pubkey_hex: kid_b.clone(),
        address: "127.0.0.1:7941".into(),
        fingerprint: short_fingerprint(&kid_b).unwrap(),
        nonce: "n".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: b_node_id.clone(),
        },
    };
    identity::author_peer(&a, &sk_a, &kid_a, "A", &bundle, Some("peer"))
        .await
        .unwrap();
    a.execute("SELECT apply_remote_node_event($1)", &[&genesis_b])
        .await
        .expect("B's genesis is admitted once B is a confirmed peer");

    // The door's OTHER arm reads a different payload field, so it is a separate call site
    // and gets its own case: a trusted peer's node.superseded with a malformed subject.
    let sup = synth_supersede(&sk_b, "B", "not-hex");
    let err = a
        .execute("SELECT apply_remote_node_event($1)", &[&sup])
        .await
        .expect_err("a malformed superseded_node_id_hex must be refused");
    let sup_err = err
        .as_db_error()
        .expect("the refusal must be a database error, not a transport failure");
    assert!(
        sup_err.message().contains("apply_remote_node_event")
            && sup_err.message().contains("superseded_node_id_hex")
            && sup_err.message().contains("not valid hex"),
        "the supersede arm's refusal must name itself, the field and the reason; got: {}",
        sup_err.message()
    );
    assert_eq!(
        sup_err.code(),
        &SqlState::RAISE_EXCEPTION,
        "the supersede arm must be skippable too — it is on the same pull path"
    );

    // Now the peer/revoke arm: a TRUSTED peer ships a malformed node-id.
    let ev = synth_peer(&sk_b, "B", "0xABC");
    let err = a
        .execute("SELECT apply_remote_node_event($1)", &[&ev])
        .await
        .expect_err("a malformed peer_node_id_hex must be refused");
    let db_err = err
        .as_db_error()
        .expect("the refusal must be a database error, not a transport failure");
    let msg = db_err.message().to_string();
    assert!(
        msg.contains("apply_remote_node_event")
            && msg.contains("peer_node_id_hex")
            && msg.contains("not valid hex"),
        "the admission door's refusal must name itself, the field and the reason; got: {msg}"
    );
    assert_eq!(
        db_err.code(),
        &SqlState::RAISE_EXCEPTION,
        "the admission door's refusal must be P0001 — sync.rs skips past a P0001 and \
         FREEZES the peer's cursor on anything else, so a 22-class code here stalls \
         node-plane sync from that peer permanently; got: {msg}"
    );
}

/// The LOCAL authoring door refuses a malformed `peer_node_id_hex` by name.
///
/// This is the wiring proof for db/007: the helper could be declared, correct and entirely
/// unreferenced, and every behavioural assertion above would still pass. Here the event
/// travels the real path — verify, derive op, reach the peer/revoke arm — and the caller
/// gets a message it can act on.
#[tokio::test]
async fn submit_node_event_refuses_malformed_hex_by_name() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    // submit_node_event only accepts events authored by THIS node's current key, so the
    // node must be provisioned and the event signed with its key.
    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7930")
        .await
        .unwrap();

    let ev = synth_peer(&sk, "A", "0xABC");
    let err = a
        .execute("SELECT submit_node_event($1)", &[&ev])
        .await
        .expect_err("a malformed peer_node_id_hex must be refused");
    let msg = err
        .as_db_error()
        .map(|e| e.message().to_string())
        .unwrap_or_default();
    assert!(
        msg.contains("submit_node_event")
            && msg.contains("peer_node_id_hex")
            && msg.contains("not valid hex"),
        "the local door's refusal must name itself, the field and the reason; got: {msg}"
    );
}

/// The RESTORE door refuses a malformed `peer_node_id_hex` by name.
///
/// db/009 is the site whose behaviour changes most: its guard used to run AFTER the decode
/// and could only report "missing subject node id" — so the malformed case never reached
/// it at all (the bare `decode` raised first) and the missing case named no field. Both
/// now go through the helper.
///
/// Restore is fenced to an un-enrolled node, so this case deliberately does NOT provision:
/// it restores a foreign genesis first (which is how the author key resolves), then feeds
/// the malformed peer event from the same key.
#[tokio::test]
async fn restore_node_event_refuses_malformed_hex_by_name() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let (sk, _kid) = cairn_event::generate_key().unwrap();
    let genesis = synth_enroll(&sk, "Restored");
    a.execute("SELECT restore_node_event($1)", &[&genesis])
        .await
        .expect("the medium's own genesis restores first, so its key resolves");

    let ev = synth_peer(&sk, "Restored", "not-hex-at-all");
    let err = a
        .execute("SELECT restore_node_event($1)", &[&ev])
        .await
        .expect_err("a malformed peer_node_id_hex must be refused");
    let msg = err
        .as_db_error()
        .map(|e| e.message().to_string())
        .unwrap_or_default();
    assert!(
        msg.contains("restore_node_event")
            && msg.contains("peer_node_id_hex")
            && msg.contains("not valid hex"),
        "the restore door's refusal must name itself, the field and the reason; got: {msg}"
    );
}
