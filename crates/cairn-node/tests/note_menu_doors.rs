//! DB-gated: sign-in slice 3c-3c's two person-menu read doors (db/073).
//!
//! The window's login has no grant on `vault_member` or on `actor_event`'s actor columns (db/062),
//! so the menu learns WHO a key or a draft's writer is only through these two definer doors:
//! `cairn_note_people()` (everyone the menu may name, with every key each ever held) and
//! `cairn_note_drafts_by_writer()` (open drafts by the staff id that wrote a non-blank part).
//! Staff are enrolled through the real doors by `common/vault_enroller_kit.rs`.
mod common;
#[path = "common/vault_enroller_kit.rs"]
#[macro_use]
mod kit;

use kit::*;
use tokio_postgres::Client;
use uuid::Uuid;

// uuid and timestamps cross the wire as text (the repo's `::text::uuid` idiom): no tokio-postgres feature needed.

/// boss (superuser) plus clinicians jsmith and bwong (enrollers), as the kit's `staff` makes them.
async fn team(c: &Client) {
    staff(c).await;
}

/// One people row, by staff id: (actor hex, display name, keys).
async fn person(c: &Client, staff: &str) -> (String, Option<String>, Vec<String>) {
    let r = c
        .query_one(
            "SELECT actor_id, display_name, keys FROM cairn_note_people() WHERE staff_id = $1",
            &[&staff],
        )
        .await
        .unwrap_or_else(|e| panic!("no people row for {staff}: {e}"));
    (r.get(0), r.get(1), r.get(2))
}

/// Insert an open draft on `chart` with one part per `(seq, writer, narrative)`, directly as the
/// owner (the window's save door is not what is under test). Returns the draft id.
async fn draft(c: &Client, chart: Uuid, parts: &[(i32, Option<&str>, &str)]) -> String {
    let id = Uuid::now_v7().to_string();
    let chart = chart.to_string();
    c.execute(
        "INSERT INTO note_draft (draft_id, patient_id, terminal_id, recording_started, \
                                 last_keystroke) VALUES ($1::text::uuid, $2::text::uuid, 'pc-menu', now(), now())",
        &[&id, &chart],
    )
    .await
    .unwrap();
    for (seq, writer, text) in parts {
        c.execute(
            "INSERT INTO note_draft_part (draft_id, seq, writer, narrative, recording_started, \
                                          last_keystroke) VALUES ($1::text::uuid, $2, $3, $4, now(), now())",
            &[&id, seq, writer, text],
        )
        .await
        .unwrap();
    }
    id
}

#[tokio::test]
async fn people_lists_a_member_with_every_key_it_ever_held() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    rekey(
        &c,
        "jsmith",
        "clinician",
        None,
        None,
        JSMITH2,
        "bwong",
        BWONG,
    )
    .await
    .unwrap();
    let rows = c
        .query(
            "SELECT 1 FROM cairn_note_people() WHERE staff_id = 'jsmith'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "one row per actor, not per key");
    let (actor, name, keys) = person(&c, "jsmith").await;
    assert_eq!(actor, hex::encode(actor_of(&c, "jsmith").await));
    assert_eq!(name.as_deref(), Some("Dr jsmith"));
    let mut want = vec![kid_of(JSMITH), kid_of(JSMITH2)];
    want.sort();
    assert_eq!(keys, want, "the old key and the new one, sorted");
}

#[tokio::test]
async fn people_lists_a_supervisor_whose_member_row_is_gone() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    // jsmith is linked under bwong, then bwong's member row is dropped (as after a restore).
    acted(&c, "link-supervisor", "jsmith", Some("bwong"), "boss", BOSS).await;
    let a = hex::encode(actor_of(&c, "bwong").await);
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'bwong'; \
         DELETE FROM vault_member WHERE staff_id = 'bwong'",
    )
    .await
    .unwrap();
    let r = c
        .query_one(
            "SELECT staff_id, display_name FROM cairn_note_people() WHERE actor_id = $1",
            &[&a],
        )
        .await
        .expect("the supervisor stays nameable by the record");
    assert_eq!(
        r.get::<_, Option<String>>(0).as_deref(),
        Some("bwong"),
        "the pinned handle"
    );
    assert_eq!(
        r.get::<_, Option<String>>(1),
        None,
        "no member row, no display name"
    );
}

#[tokio::test]
async fn drafts_by_writer_lists_each_writer_once_and_skips_blank_parts() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let d = draft(
        &c,
        Uuid::now_v7(),
        &[
            (1, Some("jsmith"), "chest pain"),
            (2, Some("jsmith"), "settled"), // same writer twice: still one row
            (3, Some("bwong"), "   "),      // blank: names nobody
            (4, None, "typed by someone unknown"),
        ],
    )
    .await;
    let rows = c
        .query(
            "SELECT writer FROM cairn_note_drafts_by_writer() WHERE draft_id = $1::text::uuid",
            &[&d],
        )
        .await
        .unwrap();
    let writers: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
    assert_eq!(writers, ["jsmith"]);
}

#[tokio::test]
async fn drafts_by_writer_due_agrees_with_the_per_chart_door() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let chart = Uuid::now_v7();
    let chart = &chart.to_string();
    // One draft whose writer holds a vault key, one whose writer has none (so it is stuck).
    let ok = draft(&c, chart.parse().unwrap(), &[(1, Some("jsmith"), "fine")]).await;
    let stuck = draft(&c, chart.parse().unwrap(), &[(1, Some("ghost"), "no key")]).await;
    for (id, want_stuck) in [(&ok, false), (&stuck, true)] {
        let by: (String, bool) = {
            let r = c
                .query_one(
                    "SELECT due::text, stuck FROM cairn_note_drafts_by_writer() \
                      WHERE draft_id = $1::text::uuid",
                    &[&id],
                )
                .await
                .unwrap();
            (r.get(0), r.get(1))
        };
        let per_chart = c
            .query_one(
                "SELECT due::text, stuck FROM cairn_note_drafts_due($1::text::uuid) \
                  WHERE draft_id = $2::text::uuid",
                &[&chart, &id],
            )
            .await
            .unwrap();
        assert_eq!(by.0, per_chart.get::<_, String>(0));
        assert_eq!(by.1, per_chart.get::<_, bool>(1));
        assert_eq!(by.1, want_stuck);
    }
}

#[tokio::test]
async fn the_window_may_call_both_and_public_may_not() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let base = cs().unwrap();
    let window = cairn_node::db::connect(&common::window_login(&c, &base).await)
        .await
        .expect("the window's login connects");
    for call in [
        "SELECT * FROM cairn_note_people()",
        "SELECT * FROM cairn_note_drafts_by_writer()",
    ] {
        window
            .query(call, &[])
            .await
            .unwrap_or_else(|e| panic!("the window must run {call}: {e}"));
    }
    // A role holding no grant at all (not even cairn_window): refused with 42501.
    c.batch_execute(
        "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_menu_nogrant') \
         THEN CREATE ROLE cairn_menu_nogrant NOLOGIN; END IF; END $$; \
         GRANT cairn_menu_nogrant TO CURRENT_USER;",
    )
    .await
    .unwrap();
    for call in [
        "SELECT * FROM cairn_note_people()",
        "SELECT * FROM cairn_note_drafts_by_writer()",
    ] {
        c.batch_execute("SET ROLE cairn_menu_nogrant")
            .await
            .unwrap();
        let r = c.query(call, &[]).await;
        c.batch_execute("RESET ROLE").await.unwrap();
        let e = r.expect_err(call);
        assert_eq!(
            e.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
            "{call}"
        );
    }
}
