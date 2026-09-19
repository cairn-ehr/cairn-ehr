//! PR #623 review, finding 1 — the pull loop reads an `event_id` EXACTLY as the door does.
//!
//! # What went wrong, and what is pinned
//!
//! The node-plane doors read a signed body's `event_id` with Postgres's `::uuid` cast. The pull
//! loop's substitution question (`sync/substitution.rs::held_content_address`) used to read it
//! with the `uuid` crate, whose grammar is NARROWER: Postgres also accepts a hyphen after any group
//! of four hex digits. So a rival spelled `a0ee-bc99-…` was refused by the door — which read it as
//! the held id — while the loop's lookup found "nothing held", and the rival was skipped as routine
//! scoping instead of penned. Anyone minting a rival could switch the pen off by choosing a spelling.
//!
//! The fix is `uuid_as_postgres_reads_it`, a mirror of Postgres's own parser. Its unit tests pin it
//! against answers copied from a live server; THIS file asks the live server again, over a
//! generated corpus, so a parser drift on either side fails here rather than silently. The pull-loop
//! consequence — the oddly spelled rival is penned — is pinned in `node_substitution_is_penned.rs`.

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_node::db;
use cairn_node::sync::substitution::{held_content_address, uuid_as_postgres_reads_it};
use node_plane_kit::{address_of, call, cs, fresh_node, node_id_hex, peer_event, spelled_oddly};
use tokio_postgres::Client;
use uuid::Uuid;

/// What the live server makes of `spelling`: its canonical text, or `None` where `::uuid` raises.
///
/// Asked in two statements on purpose. A single `CASE WHEN pg_input_is_valid($1,'uuid') THEN
/// $1::uuid END` may be constant-folded when planned for its actual value, raising on exactly the
/// inputs the guard was meant to skip.
async fn postgres_reads(db: &Client, spelling: &str) -> Option<String> {
    let valid: bool = db
        .query_one("SELECT pg_input_is_valid($1, 'uuid')", &[&spelling])
        .await
        .unwrap()
        .get(0);
    if !valid {
        return None;
    }
    Some(
        db.query_one("SELECT $1::text::uuid::text", &[&spelling])
            .await
            .unwrap()
            .get(0),
    )
}

/// Every spelling worth asking about for `id`: each of the 2^7 ways to place the optional hyphens
/// Postgres allows (after every second byte but the last), three ways each — lowercase, uppercase,
/// and braced lowercase (the parser treats case and braces independently) — plus the malformed
/// neighbours it must reject.
fn corpus(id: Uuid) -> Vec<String> {
    let hex = id.simple().to_string();
    let groups: Vec<&str> = (0..8).map(|g| &hex[g * 4..g * 4 + 4]).collect();
    let mut out = Vec::new();
    for mask in 0u8..128 {
        let mut s = String::new();
        for (g, group) in groups.iter().enumerate() {
            s.push_str(group);
            if g < 7 && mask & (1 << g) != 0 {
                s.push('-');
            }
        }
        out.push(format!("{{{s}}}"));
        out.push(s.to_uppercase());
        out.push(s);
    }
    out.extend([
        format!("{hex}-"),
        format!("-{hex}"),
        format!("{}--{}", &hex[..8], &hex[8..]),
        format!("{}-{}", &hex[..3], &hex[3..]),
        format!(" {hex}"),
        format!("{hex} "),
        format!("{{{hex}"),
        format!("{hex}}}"),
        format!("urn:uuid:{id}"),
        hex[..30].to_string(),
        format!("{hex}0"),
        String::new(),
        "not-a-uuid".into(),
    ]);
    out
}

#[tokio::test]
async fn the_lookup_reads_every_spelling_exactly_as_postgres_does() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let db = db::connect(&base).await.unwrap();
    let id = Uuid::now_v7();
    let spellings = corpus(id);
    let mut accepted = 0;
    for spelling in &spellings {
        let pg = postgres_reads(&db, spelling).await;
        let ours = uuid_as_postgres_reads_it(spelling).map(|u| u.to_string());
        assert_eq!(
            ours, pg,
            "the pull loop and the door must read {spelling:?} the same way"
        );
        if pg.is_some() {
            accepted += 1;
        }
    }
    // Anti-vacuity: the corpus must exercise BOTH directions, or agreement proves nothing.
    assert_eq!(accepted, 3 * 128, "every hyphen placement is accepted");
    assert!(
        accepted < spellings.len(),
        "and the malformed neighbours are there to be rejected"
    );
}

/// The lookup itself, end to end: an oddly spelled id FINDS the row the door holds, and a string
/// that is no UUID answers `None` without a query error (which the pull loop would have to treat
/// as a freeze — a permanent wedge for an event refused before the door parsed its id).
#[tokio::test]
async fn the_lookup_finds_a_held_id_however_it_is_spelled() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;
    let contested = Uuid::now_v7();
    let held = peer_event(&a.sk, "peer.added", contested, &node_id_hex(1));
    call(&a.db, "submit_node_event", &held)
        .await
        .expect("A holds the event");

    assert_eq!(
        held_content_address(&a.db, &spelled_oddly(contested))
            .await
            .expect("the lookup must not fail on a spelling Postgres accepts"),
        Some(address_of(&held)),
        "the oddly spelled id names the row the door holds"
    );
    assert_eq!(
        held_content_address(&a.db, "not-a-uuid")
            .await
            .expect("a malformed id is answered, not an error"),
        None
    );
}
