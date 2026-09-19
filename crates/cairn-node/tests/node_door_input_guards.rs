//! #621 — the guards that keep the doors' input validation WIRED.
//!
//! `node_door_refusals_are_p0001.rs` proves the behaviour: every malformed field is refused with
//! the code the node puller reads as a verdict. This file guards the two ways that behaviour can
//! be lost while every behavioural test it has stays green:
//!
//! 1. **a helper's declaration moves or is duplicated** — the #198 late-binding trap. cairn-sync
//!    loads a SUBSET of the migrations containing db/001 but not db/007 or db/009, and PL/pgSQL
//!    binds a call at first EXECUTION, so a shared helper declared beside today's callers is a
//!    first-write outage for tomorrow's;
//! 2. **a call site quietly reverts to a bare cast** — one expression, unchanged shape, invisible
//!    in review, and the event is still *refused*: just with `22P02` instead of `P0001`, which is
//!    a permanently frozen sync link rather than a skipped event. No behavioural test outside
//!    the sibling suite would notice, and the sibling suite cannot cover a door nobody thought to
//!    add to it.
//!
//! Both are asked of **`pg_proc`, the catalogue — what actually runs** — rather than of the source
//! text, the rule `late_custody_guards.rs` (#584) and `substitution_guard_covers_every_writer.rs`
//! (#619) established: a door loaded from a file nobody thought to grep is still in the catalogue.
//! Comments are stripped with the shared `sql_text` kit, so PROSE about `::uuid` (this file's own
//! db/007 comment says the words) can never stand in for code, in either direction.
//!
//! The one source-level check is the declaration guard, which is a question about FILES.

#[path = "common/sql_text.rs"]
mod sql_text;

use cairn_node::db;
use std::fs;
use std::path::PathBuf;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The three doors that write `node_event` from caller-supplied signed bytes.
const DOORS: [&str; 3] = [
    "submit_node_event",
    "apply_remote_node_event",
    "restore_node_event",
];

/// Repo-root `db/`. `CARGO_MANIFEST_DIR` is `crates/cairn-node`.
fn db_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db")
        .canonicalize()
        .expect("db/ dir")
}

/// Every `*.sql` migration directly under `db/`, as (file name, code-with-comments-stripped).
/// Not recursive, so the SQL mirrors under `db/tests/` are correctly excluded. Stripping comments
/// is what makes the declaration guard fail CLOSED: these files discuss the helpers by name in
/// prose, and a quoted example call must never be able to stand in for a real declaration.
fn migrations() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        out.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            sql_text::without_sql_comments(&sql),
        ));
    }
    out.sort();
    out
}

/// One door's body as the server holds it, with comments stripped.
async fn door_body(c: &Client, door: &str) -> String {
    let src: String = c
        .query_one("SELECT prosrc FROM pg_proc WHERE proname = $1", &[&door])
        .await
        .unwrap_or_else(|e| panic!("{door} must exist in the catalogue: {e}"))
        .get(0);
    sql_text::without_sql_comments(&src)
}

/// The two SHARED helpers must be declared exactly once, and in db/001.
///
/// Not a tidiness rule: cairn-sync replays a subset of the migrations that includes db/001 and
/// not db/007, so a helper declared beside its node-plane callers would let cairn-sync's schema
/// load cleanly and then fail on the first clinical call site that reached for it (#198). The
/// clinical door has the same raw casts today (#626), so that call site is a matter of when.
#[test]
fn the_shared_helpers_are_declared_only_in_db001() {
    for helper in ["cairn_uuid_or_raise", "cairn_hlc_nonneg_or_raise"] {
        let needle = format!("CREATE OR REPLACE FUNCTION {helper}(");
        let declaring: Vec<String> = migrations()
            .into_iter()
            .filter(|(_, sql)| sql.contains(&needle))
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            declaring,
            vec!["001_envelope.sql".to_string()],
            "{helper} must be declared ONLY in db/001 — a subset load that omits the declaring \
             migration turns PL/pgSQL late binding into a first-write outage (#198). Found in: \
             {declaring:?}"
        );
    }
}

/// Every door still CALLS each guard.
///
/// The mirror-image failure of the guard above, and the one a refactor actually causes: a call
/// site reverts to `(b ->> 'event_id')::uuid` and nothing goes red, because the malformed event
/// is still refused — under the SQLSTATE that freezes the link. Asked of the catalogue, so a
/// door added in a file this test never heard of is still covered by the next test, and this one
/// stays honest about the three that exist.
#[tokio::test]
async fn every_node_door_calls_every_input_guard() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for door in DOORS {
        let body = door_body(&c, door).await;
        for helper in [
            "cairn_uuid_or_raise(",
            "cairn_hlc_nonneg_or_raise(",
            "cairn_node_role_or_raise(",
        ] {
            assert!(
                body.contains(helper),
                "{door} no longer calls {helper} — the field it validated is back to raising \
                 PostgreSQL's own code, which freezes that peer's pull cursor permanently (#621). \
                 If the door genuinely stopped writing that column, delete it from this list \
                 deliberately; never let the call vanish silently."
            );
        }
    }
}

/// No BARE `::uuid` cast survives in any door body.
///
/// This is the rule that covers the cast nobody has written yet — the reason #228 stayed open for
/// casts after it closed for hex. A new field read out of a signed body is exactly the moment a
/// bare cast gets typed, and the author has no reason to suspect the SQLSTATE matters.
///
/// Comments are stripped first, which is load-bearing in BOTH directions here: db/007's own
/// comment explains why a bare `::uuid` is wrong and therefore contains the words, and a guard
/// that counted them would be red on correct code; a guard that did not strip could also be
/// GREEN on a body whose only `::uuid` had moved into a comment. The kit is shared with the other
/// catalogue guards so there is one stripper, never two (#608's shape).
#[tokio::test]
async fn no_node_door_casts_to_uuid_bare() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for door in DOORS {
        let body = door_body(&c, door).await;
        assert!(
            !body.contains("::uuid"),
            "{door} contains a bare ::uuid cast. On caller-supplied bytes that raises 22P02, \
             which the node puller cannot tell from a deadlock — so it freezes that peer's \
             cursor below the event, forever, with nothing penned and no ack remedy (#621). \
             Route it through cairn_uuid_or_raise, which gates on pg_input_is_valid and so \
             accepts exactly what the cast would."
        );
    }
}

/// The peer-role vocabulary has ONE source, and the table's CHECK reads it.
///
/// Re-inlining the list into the constraint is the tidy-up this exists to stop: the CHECK and the
/// door would then hold two lists, and the day one is widened without the other, an older node
/// meets `23514` — a frozen link — where it should have seen a legible P0001 it could skip until
/// its own upgrade. Asked of the LIVE constraint definition, not the source text, because an
/// existing database keeps the constraint it was created with unless the paired ALTER ran.
#[tokio::test]
async fn the_role_check_reads_the_one_vocabulary() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let def: String = c
        .query_one(
            "SELECT pg_get_constraintdef(oid) FROM pg_constraint
              WHERE conrelid = 'node_event'::regclass AND conname = 'node_event_role_check'",
            &[],
        )
        .await
        .expect("node_event_role_check must still exist — it is the floor under the door")
        .get(0);
    assert!(
        def.contains("cairn_node_roles()"),
        "node_event_role_check must read the vocabulary function, not a list of its own: {def}"
    );
}

/// The CHECK is still a FLOOR, not a formality.
///
/// The door's refusal is the legible, skippable one; the constraint is what still holds for a
/// caller with raw SQL — principle 12's gradient (the door is a privilege, the floor is the
/// guarantee). A reader of the guard above could reasonably conclude the CHECK is now decorative,
/// so this says otherwise in the only way that cannot rot.
#[tokio::test]
async fn the_role_check_still_refuses_a_raw_insert() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let e = c
        .execute(
            "INSERT INTO node_event
                 (node_event_id, op, author_node_id, subject_node_id, signer_key_id, role,
                  hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
             VALUES (gen_random_uuid(), 'peer', '\\x00', '\\x00', 'k', 'overlord', 1, 0, 'n',
                     '\\x01'::bytea, '\\x1220'::bytea || digest('\\x01'::bytea, 'sha256'))",
            &[],
        )
        .await
        .expect_err("the table's own CHECK must still refuse an unknown role");
    let code = e.as_db_error().map(|d| d.code().code().to_string());
    assert_eq!(
        code.as_deref(),
        Some("23514"),
        "a raw INSERT bypassing the door must still meet the CHECK: {e}"
    );
}
