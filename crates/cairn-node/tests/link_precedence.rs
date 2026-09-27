//! ADR-0076 decision 5 — an ATTESTED link assertion outranks an UN-ATTESTED one.
//!
//! `patient_link` used to be latest-HLC-wins, so a matcher's link (ours or a peer's) with a
//! later HLC silently displaced a reviewer's `unlink`, and an un-attested `unlink` (the
//! ADR-0030 agent writer can author one — unlinks are never veto-gated) could split a
//! reviewer's `link`. The winner order is now: attested first, then `(hlc_wall,
//! hlc_counter, origin)`, then `content_address`. Still a TOTAL order, so every node
//! converges — which is why every test here applies its events in BOTH arrival orders.
//!
//! "Attested" is the one definition db/018 already used for the #190 veto check: an
//! attester key is present and `cairn_attestation_vouched` holds.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::chart_link::LinkVerb;
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{
    apply_remote_attested, apply_remote_raw, link_assertion_event, register_pair, vetoed_pair,
};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Empty every identity projection this suite reads, then enroll one agent (the machine
/// writer) and one human (the reviewer). Returns (agent_sk, agent_kid, human_sk, human_kid).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.link_veto_flag') IS NOT NULL THEN TRUNCATE link_veto_flag; END IF; \
           IF to_regclass('public.match_proposal') IS NOT NULL THEN TRUNCATE match_proposal; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"prec-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"P\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Between arrival orders: empty the log and the projections (event_log is append-only —
/// a DELETE is refused, so TRUNCATE as `overlay_tiebreaker.rs::reset_between_orders` does),
/// keep the enrolled actors (`actor_event` is not truncated), and re-register the pair so
/// the second order starts from the same charts as the first.
async fn reset_links(c: &Client, sk_a: &SigningKey, kid_a: &str, a: Uuid, b: Uuid) {
    c.batch_execute(
        "TRUNCATE event_log, patient_chart, patient_link, person_member, link_veto_flag, \
         identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    register_pair(c, sk_a, kid_a, a, b).await;
}

/// The standing (state, attested) for the canonical pair.
async fn standing(c: &Client, a: Uuid, b: Uuid) -> (String, bool) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let r = c
        .query_one(
            "SELECT state, attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&lo.to_string(), &hi.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

/// Are a and b one person now? (Compare representatives; a missing row is "not linked".)
async fn same_person(c: &Client, a: Uuid, b: Uuid) -> bool {
    let r: Option<bool> = c
        .query_one(
            "SELECT (SELECT person_id FROM person_member WHERE patient_id = $1::text::uuid)
                  = (SELECT person_id FROM person_member WHERE patient_id = $2::text::uuid)",
            &[&a.to_string(), &b.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    r.unwrap_or(false)
}

/// One assertion to land: its body, and whether it travels with a human token.
struct Landing {
    body: EventBody,
    attested: bool,
}

/// Build a [`Landing`] — `attested` is stated ONCE and drives both the body (a
/// responsibility-bearing contributor) and the door it lands through (with a human token),
/// so the two can never disagree and quietly exercise a different path than a test names.
#[allow(clippy::too_many_arguments)]
fn landing(
    kid: &str,
    a: Uuid,
    b: Uuid,
    verb: LinkVerb,
    wall: i64,
    counter: i32,
    origin: &str,
    attested: bool,
) -> Landing {
    Landing {
        body: link_assertion_event(kid, a, b, verb, wall, counter, origin, attested),
        attested,
    }
}

/// Land `l` through the REMOTE door (apply_remote_event), attested or not.
async fn land_remote(c: &Client, l: &Landing, sk_a: &SigningKey, sk_h: &SigningKey, kid_h: &str) {
    if l.attested {
        apply_remote_attested(c, sk_h, l.body.clone(), sk_h, kid_h)
            .await
            .expect("attested lands");
    } else {
        apply_remote_raw(c, sk_a, l.body.clone())
            .await
            .expect("un-attested lands");
    }
}

/// Land `l` through the LOCAL door (submit_event), attested or not.
async fn land_local(c: &Client, l: &Landing, sk_a: &SigningKey, sk_h: &SigningKey, kid_h: &str) {
    if l.attested {
        common::submit_attested(c, sk_h, l.body.clone(), sk_h, kid_h)
            .await
            .expect("attested lands");
    } else {
        let signed = sign(&l.body, sk_a).unwrap();
        c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
            .await
            .expect("un-attested lands");
    }
}

/// Land `first` then `second`, read the standing edge; reset; land them the other way round;
/// read again. Both orders must agree (convergence) — the value returned is that agreement.
#[allow(clippy::too_many_arguments)]
async fn both_orders_remote(
    c: &Client,
    first: &Landing,
    second: &Landing,
    a: Uuid,
    b: Uuid,
    sk_a: &SigningKey,
    kid_a: &str,
    sk_h: &SigningKey,
    kid_h: &str,
) -> ((String, bool), bool) {
    land_remote(c, first, sk_a, sk_h, kid_h).await;
    land_remote(c, second, sk_a, sk_h, kid_h).await;
    let one = (standing(c, a, b).await, same_person(c, a, b).await);
    reset_links(c, sk_a, kid_a, a, b).await;
    land_remote(c, second, sk_a, sk_h, kid_h).await;
    land_remote(c, first, sk_a, sk_h, kid_h).await;
    let two = (standing(c, a, b).await, same_person(c, a, b).await);
    assert_eq!(
        one, two,
        "the two arrival orders must converge on one winner"
    );
    one
}

#[tokio::test]
async fn a_later_machine_link_does_not_displace_a_human_unlink() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 10, 0, "nodeH", true);
    let machine_link = landing(&kid_a, a, b, LinkVerb::Link, 20, 0, "nodeM", false);

    let ((state, attested), merged) = both_orders_remote(
        &c,
        &human_unlink,
        &machine_link,
        a,
        b,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
    )
    .await;
    assert_eq!(
        state, "unlink",
        "the human's judgement stands against a later machine link"
    );
    assert!(
        attested,
        "the standing row records that its winner was attested"
    );
    assert!(!merged, "the two charts stay two people");
}

#[tokio::test]
async fn a_later_machine_unlink_does_not_split_a_human_link() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_link = landing(&kid_h, a, b, LinkVerb::Link, 10, 0, "nodeH", true);
    let machine_unlink = landing(&kid_a, a, b, LinkVerb::Unlink, 20, 0, "nodeM", false);

    let ((state, _), merged) = both_orders_remote(
        &c,
        &human_link,
        &machine_unlink,
        a,
        b,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
    )
    .await;
    assert_eq!(
        state, "link",
        "an un-attested unlink never splits a human's link"
    );
    assert!(merged);
}

#[tokio::test]
async fn between_two_human_judgements_the_later_wins() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let link = landing(&kid_h, a, b, LinkVerb::Link, 10, 0, "nodeH", true);
    let unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 20, 0, "nodeH", true);

    let ((state, attested), merged) =
        both_orders_remote(&c, &link, &unlink, a, b, &sk_a, &kid_a, &sk_h, &kid_h).await;
    assert_eq!(
        state, "unlink",
        "a later human judgement reverses an earlier one"
    );
    assert!(attested);
    assert!(!merged);
}

#[tokio::test]
async fn an_attested_assertion_wins_even_an_hlc_triple_collision() {
    // Identical (wall, counter, origin) — the old order fell through to the
    // content address; the attested assertion must win whichever address sorts higher.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 30, 4, "same", true);
    let machine_link = landing(&kid_a, a, b, LinkVerb::Link, 30, 4, "same", false);

    let ((state, _), _) = both_orders_remote(
        &c,
        &human_unlink,
        &machine_link,
        a,
        b,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
    )
    .await;
    assert_eq!(state, "unlink");

    // The end-to-end part above cannot by itself prove the rank beats the address: the two
    // content addresses are uncontrolled, so against the OLD comparator it would pass
    // whenever the human's address happened to sort higher — a coin flip. Pin it directly
    // on the comparator with the addresses chosen AGAINST the attested side ('\x01' < '\xff'):
    // the attested new edge wins over an un-attested one with the higher address…
    let attested_new_wins: bool = c
        .query_one(
            "SELECT cairn_link_overlay_wins(TRUE, 30, 4, 'o', '\\x01'::bytea, \
                                            FALSE, 30, 4, 'o', '\\xff'::bytea)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        attested_new_wins,
        "attested beats un-attested even with the lower content address"
    );
    // …and the reverse comparison loses (antisymmetry: the two cannot both win).
    let unattested_new_wins: bool = c
        .query_one(
            "SELECT cairn_link_overlay_wins(FALSE, 30, 4, 'o', '\\xff'::bytea, \
                                            TRUE, 30, 4, 'o', '\\x01'::bytea)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !unattested_new_wins,
        "un-attested never displaces attested, whatever its address"
    );
}

#[tokio::test]
async fn the_local_door_ranks_the_same_way() {
    // Both doors reach the ONE applier; pin that the local door (submit_event) does not
    // take a different path. Non-vetoed pair, so the local door admits the agent's link.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 10, 0, "nodeH", true);
    let machine_link = landing(&kid_a, a, b, LinkVerb::Link, 20, 0, "nodeM", false);
    land_local(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;
    land_local(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;

    assert_eq!(standing(&c, a, b).await, ("unlink".to_string(), true));
    assert!(!same_person(&c, a, b).await);
}

#[tokio::test]
async fn an_older_human_unlink_clears_a_standing_vetoed_machine_link() {
    // A vetoed machine link lands first (remote door admits it, flags it,
    // both charts read under-review). A human unlink with an EARLIER HLC then arrives —
    // under the old order it lost; now it wins, the flag clears and the charts split.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let machine_link = landing(&kid_a, a, b, LinkVerb::Link, 20, 0, "nodeM", false);
    land_remote(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;
    assert_eq!(
        flag_count(&c).await,
        1,
        "precondition: the vetoed machine link is flagged"
    );

    let human_unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 10, 0, "nodeH", true);
    land_remote(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;

    assert_eq!(standing(&c, a, b).await, ("unlink".to_string(), true));
    assert_eq!(
        flag_count(&c).await,
        0,
        "the human decision clears the veto worklist row"
    );
    assert!(!same_person(&c, a, b).await);
}

#[tokio::test]
async fn a_vetoed_machine_link_after_a_human_unlink_raises_no_flag() {
    // The mirror: the human unlink is standing; a LATER vetoed machine link arrives. It loses,
    // so the flag lifecycle (derived from the standing winner) must not raise a phantom row.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let human_unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 10, 0, "nodeH", true);
    let machine_link = landing(&kid_a, a, b, LinkVerb::Link, 20, 0, "nodeM", false);
    land_remote(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;
    land_remote(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;

    assert_eq!(standing(&c, a, b).await.0, "unlink");
    assert_eq!(flag_count(&c).await, 0);
    assert!(!same_person(&c, a, b).await);
}

#[tokio::test]
async fn every_standing_row_records_its_winners_attestation_truthfully() {
    // The column must never disagree with the one definition, evaluated on the winning
    // event. Three pairs, ONE event each (attested, un-attested, attested), so this checks
    // the INSERT path of db/018's upsert; the UPDATE path's value (a later winner
    // displacing an earlier one) is pinned by the `standing()` reads in the tests above.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    for (i, attested) in [(0, true), (1, false), (2, true)] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, a, b).await;
        let kid = if attested { &kid_h } else { &kid_a };
        let l = landing(kid, a, b, LinkVerb::Link, 10 + i, 0, "n", attested);
        land_remote(&c, &l, &sk_a, &sk_h, &kid_h).await;
    }
    let disagreeing: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_link pl JOIN event_log el ON el.content_address = pl.content_address
              WHERE pl.attested IS DISTINCT FROM
                    (el.attester_key IS NOT NULL AND cairn_attestation_vouched(el.event_id))",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(disagreeing, 0);
    let rows: i64 = c
        .query_one("SELECT count(*) FROM patient_link", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        rows, 3,
        "positive control: the check above ran over three rows"
    );
}

#[tokio::test]
async fn an_upgraded_node_refolds_a_winner_the_old_order_chose() {
    // Recreate what a generation-54 node holds after the old order ran: a
    // human unlink (HLC 10) displaced by a machine link (HLC 20) — the machine link is the
    // stored winner and the charts are merged. The upgrade connect must leave the human's
    // unlink standing and the charts apart. Filling the new column alone would not: it
    // would mark the machine link "not attested" and keep it as the winner.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let (lo_s, hi_s) = (lo.to_string(), hi.to_string());

    let human_unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 10, 0, "nodeH", true);
    let machine_link = landing(&kid_a, a, b, LinkVerb::Link, 20, 0, "nodeM", false);
    land_remote(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;
    land_remote(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;

    // Forge the OLD order's outcome: the machine link as the stored winner, the pre-R2a
    // table shape (no column), the component merged, the generation one behind.
    c.batch_execute(&format!(
        "UPDATE patient_link pl SET state = 'link', hlc_wall = el.hlc_wall,
                hlc_counter = el.hlc_counter, origin = el.node_origin,
                content_address = el.content_address
           FROM event_log el
          WHERE el.event_type = 'identity.link.asserted'
            AND pl.low = '{lo_s}'::uuid AND pl.high = '{hi_s}'::uuid;
         ALTER TABLE patient_link DROP COLUMN attested CASCADE;
         SELECT cairn_recompute_component('{lo_s}'::uuid, NULL);
         UPDATE node_schema SET version = 54;"
    ))
    .await
    .unwrap();
    assert!(
        same_person(&c, a, b).await,
        "precondition: the forged old state is merged"
    );
    drop(c);

    // The upgrade: reconnect = replay every migration, then (generation 54 → 55) the heal.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(
        standing(&c, a, b).await,
        ("unlink".to_string(), true),
        "the human's unlink is re-decided as the winner"
    );
    assert!(
        !same_person(&c, a, b).await,
        "and the charts are two people again"
    );
    let recorded: i32 = c
        .query_one("SELECT version FROM node_schema", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(recorded, cairn_event::schema_generation::SCHEMA_GENERATION);
}

#[tokio::test]
async fn the_refold_happens_even_when_cairn_sync_stamped_the_generation_first() {
    // `cairn-sync init` shares this database and SCHEMA_GENERATION, runs the same
    // generation heal, and stamps 55 — but it loads no identity migration, so its heal
    // replays links through the OLD db/018 applier. cairn-node then connects, sees
    // 55 == 55 and skips its own heal. Recreate that: the old order's winner stands, the
    // column is absent (db/018 not yet loaded by the new binary), and the generation is
    // ALREADY current. db/055 itself must still re-decide the pair.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let (lo_s, hi_s) = (lo.to_string(), hi.to_string());

    let human_unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 10, 0, "nodeH", true);
    let machine_link = landing(&kid_a, a, b, LinkVerb::Link, 20, 0, "nodeM", false);
    land_remote(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;
    land_remote(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;

    c.batch_execute(&format!(
        "UPDATE patient_link pl SET state = 'link', hlc_wall = el.hlc_wall,
                hlc_counter = el.hlc_counter, origin = el.node_origin,
                content_address = el.content_address
           FROM event_log el
          WHERE el.event_type = 'identity.link.asserted'
            AND pl.low = '{lo_s}'::uuid AND pl.high = '{hi_s}'::uuid;
         ALTER TABLE patient_link DROP COLUMN attested CASCADE;
         SELECT cairn_recompute_component('{lo_s}'::uuid, NULL);
         UPDATE node_schema SET version = {gen};",
        gen = cairn_event::schema_generation::SCHEMA_GENERATION
    ))
    .await
    .unwrap();
    assert!(
        same_person(&c, a, b).await,
        "precondition: the forged old state is merged"
    );
    drop(c);

    // No generation change on this connect, so no heal — db/055 alone must do it.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(
        standing(&c, a, b).await,
        ("unlink".to_string(), true),
        "the human's unlink is re-decided without the generation heal"
    );
    assert!(!same_person(&c, a, b).await, "and the charts are two again");
}

#[tokio::test]
async fn the_backfill_never_marks_an_unvouched_token_attested() {
    // db/055's backfill writes `attested = TRUE` only through the ONE definition. An
    // unvouched token (a deferred event's carried, never-verified token — PR #302 F2) must
    // stay FALSE: a spurious TRUE is STICKY — replaying the same event (FALSE) loses to it,
    // and it would then outrank every real human judgement on the pair.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    // A human-token link lands, then its vouch is withdrawn the way db/020 marks a deferred
    // event's token (the marker row), and the stored column is forced back to FALSE — the
    // state a winner projected before the vouch was known would have.
    let link = landing(&kid_h, a, b, LinkVerb::Link, 20, 0, "nodeH", true);
    land_remote(&c, &link, &sk_a, &sk_h, &kid_h).await;
    c.batch_execute(
        "INSERT INTO event_attestation_unvouched (event_id)
           SELECT event_id FROM event_log WHERE event_type = 'identity.link.asserted';
         UPDATE patient_link SET attested = FALSE;",
    )
    .await
    .unwrap();
    drop(c);

    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(
        standing(&c, a, b).await,
        ("link".to_string(), false),
        "an unvouched token is not a vouch — the backfill leaves it un-attested"
    );
    // And an EARLIER real human unlink therefore still outranks it.
    let unlink = landing(&kid_h, a, b, LinkVerb::Unlink, 10, 0, "nodeH", true);
    land_remote(&c, &unlink, &sk_a, &sk_h, &kid_h).await;
    assert_eq!(standing(&c, a, b).await, ("unlink".to_string(), true));
}

async fn flag_count(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0)
}
