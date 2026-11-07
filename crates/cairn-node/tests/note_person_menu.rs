//! Sign-in slice 3c-3c, Task 4 — `note::menu_read::read_menus`, against a real database.
//!
//! The person menu ("Waiting for you") is built from ONE read: the people door, the supervision
//! record, the drafts door and every part the people's keys wrote. These tests drive real
//! enrolments, real signatures, a real quarantine commit and a real repudiation, then ask the
//! menu what waits for whom. Pinned: Review Focus 1 (a lifted requirement leaves a part waiting),
//! 3 (a repudiated key's part is the REAL person's), 4 (the bar's read and the picker's read agree).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::note::draft::save_part;
use cairn_node::note::menu::{PersonMenu, Why};
use cairn_node::note::menu_read::read_menus;
use cairn_node::note::sign::{sign_note, SignNoteInput};
use cairn_node::note::vouch::{sign_off_note, SignOffInput};
use chrono::{DateTime, Utc};
use common::note_setup;
use note_vouch_kit::{member, timer_part_by, Member, ORIGIN};
use tokio_postgres::Client;
use uuid::Uuid;

async fn console(c: &Client, op: &str, subject: &str, supervisor: Option<&str>) {
    c.execute(
        "SELECT cairn_supervision_console_act($1, $2, $3)",
        &[&op, &subject, &supervisor],
    )
    .await
    .unwrap_or_else(|e| panic!("{op} {subject}: {}", common::db_msg(&e)));
}

async fn sign_as(c: &mut Client, node_sk: &SigningKey, chart: Uuid, m: &Member) -> Uuid {
    let input = SignNoteInput {
        draft: Uuid::now_v7(),
        patient: chart,
        terminal: "ward-3-pc",
        writer: Some(m.staff),
        narrative: "obs stable",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: None,
        also_vouches: &[],
    };
    sign_note(c, node_sk, &input, &m.author())
        .await
        .unwrap()
        .expect("the note signs")
        .event_id
}

/// `m` signs off the one-part note `p`.
async fn sign_off(c: &mut Client, m: &Member, chart: Uuid, p: Uuid) {
    let input = SignOffInput {
        patient: chart,
        root: p,
        shown_parts: &[p],
        node_origin: ORIGIN,
    };
    sign_off_note(c, &input, &m.author())
        .await
        .unwrap()
        .expect("the sign-off is filed");
}

/// Repudiate `m`'s key and give the same person (same actor) a fresh one (note_repudiated_signer.rs).
async fn repudiate_and_rekey(c: &Client, m: &Member) -> Member {
    let aid: Vec<u8> = c
        .query_one(
            "SELECT actor_id FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
            &[&m.kid],
        )
        .await
        .unwrap()
        .get(0);
    let (sk, kid) = cairn_event::generate_key().unwrap();
    c.execute(
        "SELECT cairn_actor_append($1, 'rotate-key', $2, NULL, NULL)",
        &[&aid, &kid],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT cairn_actor_append($1, 'repudiate', $2, 'console', NULL)",
        &[&aid, &m.kid],
    )
    .await
    .unwrap();
    Member {
        sk,
        kid,
        staff: m.staff,
    }
}

/// Let the clock pass a millisecond boundary (the record is compared at millisecond granularity).
async fn next_millisecond() {
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
}

/// `staff`'s menu from the all-people read.
async fn menu(c: &Client, staff: &str) -> PersonMenu {
    read_menus(c, None)
        .await
        .unwrap()
        .menu_for(staff, Utc::now())
        .unwrap_or_else(|| panic!("{staff} is a person"))
}

fn parts_of(items: &[cairn_node::note::menu::MenuItem]) -> Vec<Option<Uuid>> {
    items.iter().map(|i| i.part).collect()
}

macro_rules! db_test {
    ($name:ident, |$c:ident| $body:block) => {
        #[tokio::test]
        async fn $name() {
            let Some(base) = common::cs() else {
                eprintln!("skipped: set CAIRN_TEST_PG");
                return;
            };
            let _guard = db::test_serial_guard(&base).await.unwrap();
            #[allow(unused_mut)]
            let mut $c = db::connect_and_load_schema(&base).await.unwrap();
            $c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
                .await
                .unwrap();
            $body
        }
    };
}

db_test!(
    a_student_part_waits_in_each_supervisors_group_until_one_cosigns,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let sam = member(&c, "sam", "Sam", "student").await;
        let sue = member(&c, "sue", "Dr Sue", "registrar").await;
        let tom = member(&c, "tom", "Dr Tom", "registrar").await;
        console(&c, "link-supervisor", "sam", Some("sue")).await;
        console(&c, "link-supervisor", "sam", Some("tom")).await;
        let p = sign_as(&mut c, &node_sk, chart, &sam).await;

        for who in ["sue", "tom"] {
            let m = menu(&c, who).await;
            assert_eq!(parts_of(&m.cosign), vec![Some(p)], "{who}");
            assert_eq!(m.count(), 1, "{who}");
            match &m.cosign[0].why {
                Why::NeedsCoSign { others } => assert_eq!(others.len(), 1, "the other supervisor"),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            menu(&c, "sam").await.count(),
            0,
            "the writer is owed nothing"
        );

        sign_off(&mut c, &sue, chart, p).await;
        assert!(menu(&c, "sue").await.cosign.is_empty());
        assert!(
            menu(&c, "tom").await.cosign.is_empty(),
            "one co-sign clears it"
        );
        let _ = tom;
    }
);

db_test!(require_sign_lift_keeps_the_part_waiting_and_counted, |c| {
    let (node_sk, _, _, _, chart) = note_setup(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    let _sue = member(&c, "sue", "Dr Sue", "registrar").await;
    console(&c, "require-cosign", "bea", None).await;
    console(&c, "link-supervisor", "bea", Some("sue")).await;
    let p = sign_as(&mut c, &node_sk, chart, &bea).await;
    next_millisecond().await;
    console(&c, "lift-cosign", "bea", None).await;

    let m = menu(&c, "sue").await;
    assert_eq!(parts_of(&m.cosign), vec![Some(p)]);
    assert_eq!(m.count(), 1);
});

db_test!(
    a_part_gone_in_unsigned_is_in_its_writers_group_until_signed,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let bea = member(&c, "bea", "Dr Bea", "clinician").await;
        let zed = member(&c, "zed", "Dr Zed", "registrar").await;
        let p = timer_part_by(&mut c, &node_sk, chart, "bea", None).await;

        let m = menu(&c, "bea").await;
        assert_eq!(parts_of(&m.own), vec![Some(p)]);
        assert_eq!(m.own[0].why, Why::WentInUnsigned);
        assert_eq!(menu(&c, "zed").await.count(), 0);

        sign_off(&mut c, &zed, chart, p).await;
        assert!(
            menu(&c, "bea").await.own.is_empty(),
            "a colleague's sign-off clears it"
        );
        let _ = bea;
    }
);

db_test!(
    a_repudiated_part_is_in_the_real_persons_group_until_re_signed,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let bea = member(&c, "bea", "Dr Bea", "clinician").await;
        let zed = member(&c, "zed", "Dr Zed", "registrar").await;
        let p = sign_as(&mut c, &node_sk, chart, &bea).await;
        let bea2 = repudiate_and_rekey(&c, &bea).await;

        let m = menu(&c, "bea").await;
        assert_eq!(parts_of(&m.own), vec![Some(p)]);
        assert_eq!(m.own[0].why, Why::KeyRepudiated);
        assert!(menu(&c, "zed").await.cosign.is_empty());

        sign_off(&mut c, &zed, chart, p).await;
        assert_eq!(
            menu(&c, "bea").await.own.len(),
            1,
            "a colleague clears nothing"
        );

        sign_off(&mut c, &bea2, chart, p).await;
        assert!(
            menu(&c, "bea").await.own.is_empty(),
            "the real person re-signs"
        );
    }
);

db_test!(
    an_open_draft_is_in_its_writers_group_with_the_doors_due_time,
    |c| {
        let (_, _, _, _, chart) = note_setup(&c).await;
        let _dee = member(&c, "dee", "Dr Dee", "clinician").await;
        let draft = Uuid::now_v7();
        save_part(
            &c,
            draft,
            chart,
            "pc-menu",
            Some("dee"),
            "half a note",
            None,
            None,
        )
        .await
        .unwrap();
        let due: DateTime<Utc> = c
            .query_one(
                "SELECT due FROM cairn_note_drafts_by_writer() WHERE draft_id = $1::text::uuid",
                &[&draft.to_string()],
            )
            .await
            .unwrap()
            .get(0);

        let m = menu(&c, "dee").await;
        assert_eq!(m.own.len(), 1);
        match &m.own[0].why {
            Why::Draft {
                draft_id, due: d, ..
            } => {
                assert_eq!(*draft_id, draft);
                assert_eq!(*d, due);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m.own[0].chart, chart);
    }
);

db_test!(
    the_bar_read_and_the_all_read_give_the_same_menu_for_the_person,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let sam = member(&c, "sam", "Sam", "student").await;
        let _sue = member(&c, "sue", "Dr Sue", "registrar").await;
        console(&c, "link-supervisor", "sam", Some("sue")).await;
        sign_as(&mut c, &node_sk, chart, &sam).await;
        timer_part_by(&mut c, &node_sk, chart, "sue", None).await;
        save_part(
            &c,
            Uuid::now_v7(),
            chart,
            "pc",
            Some("sue"),
            "draft",
            None,
            None,
        )
        .await
        .unwrap();

        let now = Utc::now();
        let bar = read_menus(&c, Some("sue")).await.unwrap();
        let all = read_menus(&c, None).await.unwrap();
        let a = bar.menu_for("sue", now).unwrap();
        assert_eq!(a, all.menu_for("sue", now).unwrap());
        assert_eq!(a.count(), 3, "a draft, an unsigned part, a co-sign");
        assert!(bar.person("sam").is_some() || bar.person("sue").is_some());
        // Unknown staff id: no menu, no parts read.
        let none = read_menus(&c, Some("nobody")).await.unwrap();
        assert!(none.menu_for("nobody", now).is_none());
        assert!(none.parts.is_empty());
    }
);

db_test!(counts_equal_each_persons_menu_count, |c| {
    let (node_sk, _, _, _, chart) = note_setup(&c).await;
    let sam = member(&c, "sam", "Sam", "student").await;
    let _sue = member(&c, "sue", "Dr Sue", "registrar").await;
    console(&c, "link-supervisor", "sam", Some("sue")).await;
    sign_as(&mut c, &node_sk, chart, &sam).await;

    let now = Utc::now();
    let all = read_menus(&c, None).await.unwrap();
    let counts = all.counts(now);
    assert!(counts.len() >= 3, "every person with a staff id");
    for (staff, n) in &counts {
        assert_eq!(*n, all.menu_for(staff, now).unwrap().count(), "{staff}");
    }
    assert_eq!(counts.iter().find(|(s, _)| s == "sue").unwrap().1, 1);
    let sue_actor = all.person("sue").unwrap().actor.clone();
    assert_eq!(all.name_of(&sue_actor), Some("Dr Sue"));
    assert_eq!(all.name_of("no-such-actor"), None);
});

/// `staff`'s menu read both ways (the bar's narrowed keys, and every person's), asserted equal.
async fn bar_equals_all(c: &Client, staff: &str) -> PersonMenu {
    let now = Utc::now();
    let bar = read_menus(c, Some(staff))
        .await
        .unwrap()
        .menu_for(staff, now);
    let all = read_menus(c, None).await.unwrap().menu_for(staff, now);
    assert_eq!(bar, all, "{staff}: the bar's read and the picker's read");
    bar.unwrap_or_else(|| panic!("{staff} is a person"))
}

// Final review F6: the bar narrows the keys it reads (`keys_for_bar`); these are the cases where a
// narrowed read is likeliest to miss a row the all-people read finds.
db_test!(
    the_bar_read_and_the_all_read_agree_where_they_are_likeliest_to_differ,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        // A repudiated key's part, in the REAL person's group 1 (its key is no longer theirs).
        let zed = member(&c, "zed", "Dr Zed", "registrar").await;
        let zp = sign_as(&mut c, &node_sk, chart, &zed).await;
        let _zed2 = repudiate_and_rekey(&c, &zed).await;
        let z = bar_equals_all(&c, "zed").await;
        assert_eq!(parts_of(&z.own), vec![Some(zp)]);
        assert_eq!(z.own[0].why, Why::KeyRepudiated);

        // A review-group row: a supervisee's signed part that owes no co-sign.
        let bea = member(&c, "bea", "Dr Bea", "clinician").await;
        let _sue = member(&c, "sue", "Dr Sue", "registrar").await;
        console(&c, "link-supervisor", "bea", Some("sue")).await;
        let bp = sign_as(&mut c, &node_sk, chart, &bea).await;

        // A supervisor who stepped away: Tom sees nothing of Sam's; Sue still owes the co-sign.
        let sam = member(&c, "sam", "Sam", "student").await;
        let _tom = member(&c, "tom", "Dr Tom", "registrar").await;
        console(&c, "link-supervisor", "sam", Some("sue")).await;
        console(&c, "link-supervisor", "sam", Some("tom")).await;
        let sp = sign_as(&mut c, &node_sk, chart, &sam).await;
        next_millisecond().await;
        console(&c, "unlink-supervisor", "sam", Some("tom")).await;

        let s = bar_equals_all(&c, "sue").await;
        assert_eq!(parts_of(&s.review), vec![Some(bp)]);
        assert_eq!(parts_of(&s.cosign), vec![Some(sp)]);
        let t = bar_equals_all(&c, "tom").await;
        for group in [&t.own, &t.cosign, &t.review] {
            assert!(!parts_of(group).contains(&Some(sp)), "{t:?}");
        }
    }
);
