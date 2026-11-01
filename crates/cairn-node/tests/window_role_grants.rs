//! #898 — the window's whole day, run under the window's OWN login, never the schema owner's.
//!
//! # Why this file exists
//!
//! db/062 gives the reference window a least-privilege group role, `cairn_window` (it inherits
//! `cairn_agent`, the advisory-actor role, plus the narrow grants db/062 lists one by one). Every other suite connects as the
//! schema OWNER, so until now nothing showed which grants that role actually lacks: a missing
//! one would surface only in production, as a clinician's command failing with
//! `permission denied`. Here each `cairn_node` entry point the window calls (the list is
//! `grep -rhoE 'cairn_node::[a-z_]+(::[a-z_]+)+' cairn-gui/cairn-gui-{tauri,live}/src`, plus
//! the few SQL statements the window sends itself) runs on a REAL login granted
//! `cairn_window` ([`common::window_login`]).
//!
//! # What is asserted, and what is not
//!
//! Only "this call was not refused for want of a privilege". Setup (truncation, enrolment,
//! the chart and medication fixtures) is owner work and runs on the owner connection; only the
//! entry point under test runs on the window's. Most calls are driven to SUCCEED, because a
//! call refused early by the floor (an ordinary `P0001`) never reaches the statements after
//! it, and a grant those statements need would stay hidden. Where a call is expected to
//! succeed it is `expect`ed, so a fixture that stops reaching the deep path fails loudly.
//!
//! The vault doors the window calls are walked the same way in `window_role_vault_doors.rs`
//! (split out to keep each file under 500 lines); the shared checks live in `tests/common`.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Key material and
//! passphrases are minted at runtime.
mod common;

use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{
    canonical_pair, confirm_link_as_shown, link_charts_as_shown, standing_link,
    unlink_charts_as_shown_from, LinkVerb, RecordsShown, Reviewer,
};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::signoff::sign_off_medication_list;
use cairn_node::medication::{
    assert_medication, cease_medication, release_medication_holds, withhold_medication,
    AssertMedicationInput, AttestParams, AuthorParams, CeaseMedicationInput,
    WithholdMedicationInput,
};
use cairn_patient_search::SearchQuery;
use common::{
    assert_not_denied, cs, enroll_human, link_assertion_event, owner_and_window, seed_proposal,
    setup,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// The identity and proposal tables the link/duplicate walks write; cleared by `setup`.
/// `patient_name` too: the search walk expects "nothing found" before it registers, which a
/// previous run's registered name would otherwise break (#583's shape).
const TABLES: [&str; 7] = [
    "patient_name",
    "patient_link",
    "person_member",
    "identity_projection_flag",
    "link_veto_flag",
    "chart_identity_state",
    "match_proposal",
];

const ORIGIN: &str = "window-role-grants";

/// [`assert_not_denied`], and then the call must have SUCCEEDED. A read that fails after its
/// first statement would skip the statements after it, and hide any grant they need; so every
/// read on a fixture that should satisfy it goes through here, not through the bare check.
fn expect_as_window<T>(entry: &str, result: anyhow::Result<T>) -> T {
    assert_not_denied(entry, &result);
    match result {
        Ok(value) => value,
        Err(_) => panic!("{entry}: failed on the window's login (not a privilege refusal)"),
    }
}

/// The window really is connected as its own role, not as the owner: a walk that silently ran
/// as the owner would prove nothing.
async fn assert_is_the_window(window: &Client) {
    let row = window
        .query_one(
            "SELECT current_user::text, pg_has_role(current_user, 'cairn_window', 'MEMBER'), \
                    (SELECT rolsuper FROM pg_roles WHERE rolname = current_user)",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "cairn_window_probe");
    assert!(row.get::<_, bool>(1), "the probe login holds cairn_window");
    assert!(!row.get::<_, bool>(2), "the probe login is no superuser");
}

/// The connection-string trick `window_login` relies on: a repeated key's LAST value wins.
/// If tokio-postgres ever kept the first, every "window" test would quietly run as the owner.
#[test]
fn a_repeated_user_keeps_the_last_value() {
    let cfg: tokio_postgres::Config = "host=127.0.0.1 user=owner dbname=d user=cairn_window_probe"
        .parse()
        .unwrap();
    assert_eq!(cfg.get_user(), Some("cairn_window_probe"));
}

/// The search funnel, the chart's reads, the duplicate banner and worklist, the gesture
/// timer and the node's own standing — every READ the window makes, plus registration.
#[tokio::test]
async fn the_windows_reads_and_registration_run_under_its_own_login() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, mut window) = owner_and_window(&base).await;
    let (sk, kid) = setup(&owner, &TABLES).await;
    assert_is_the_window(&window).await;

    // The window's own SQL: the database's date for the funnel and worklist, its backend for
    // a reopen (#807), the node it is on.
    let today: String = window
        .query_one("SELECT current_date::text", &[])
        .await
        .expect("current_date")
        .get(0);
    let backend = window
        .query_one(
            "SELECT pid, backend_start::text FROM pg_stat_activity WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .map_err(anyhow::Error::from);
    expect_as_window("reopen: own backend", backend);
    // `load_local` errors on a node `cairn-node init` never provisioned (this test database);
    // `load_local_opt` runs the SAME query and answers `None` instead, so it can be expected.
    let r = cairn_node::identity::load_local_opt(&window).await;
    expect_as_window("identity::load_local", r);
    let r = cairn_node::actor_enrolment::device_actor_standing(&window, &kid).await;
    expect_as_window("actor_enrolment::device_actor_standing", r);
    let r = cairn_node::actor_enrolment::require_device_actor(&window, &kid).await;
    expect_as_window("actor_enrolment::require_device_actor", r);

    // The funnel: search (nothing yet), then register what was searched.
    let query = SearchQuery::new("Window Probe", None, &[]);
    let found = cairn_node::patient::search::search_patients(&window, &query, &today).await;
    let found = expect_as_window("patient::search::search_patients (empty)", found);
    assert!(found.people.is_empty(), "nothing is registered yet");
    let registered = cairn_node::patient::register::register_patient(
        &mut window,
        &sk,
        &kid,
        ORIGIN,
        Some("Window Probe"),
        &query,
        &found,
    )
    .await;
    let p = expect_as_window("patient::register::register_patient", registered);

    // A second chart, a machine link between them and an open proposal, so the chart reads
    // below have an edge, a standing link and a duplicate entry to find.
    let q = Uuid::now_v7();
    common::submit_registration(&owner, &sk, &kid, q, 1).await;
    let ev = link_assertion_event(&kid, p, q, LinkVerb::Link, 50, 0, "peer", false);
    common::apply_remote_raw(&owner, &sk, ev).await.unwrap();
    let r_other = Uuid::now_v7();
    common::submit_registration(&owner, &sk, &kid, r_other, 2).await;
    seed_proposal(&owner, p, r_other, "pending").await;

    // Search again: now the match runs its deep path (the person's charts, their doubt mark,
    // display facts and retained names), with the linked chart q joining p's row.
    let found = cairn_node::patient::search::search_patients(&window, &query, &today).await;
    let found = expect_as_window("patient::search::search_patients (a match)", found);
    assert_eq!(found.people.len(), 1, "the registered patient is found");
    assert_eq!(
        found.people[0].members().len(),
        2,
        "p's row carries its linked chart q"
    );

    let r = cairn_node::patient::candidate_read::candidates_by_id(&window, &[p], &today).await;
    assert!(!expect_as_window("patient::candidate_read::candidates_by_id", r).is_empty());
    let r = cairn_node::patient::person::person_charts(&window, p).await;
    let charts = expect_as_window("patient::person::person_charts", r);
    assert_eq!(
        charts,
        ChartSet::new([p, q]).unwrap(),
        "p's record is p and q"
    );
    let other = ChartSet::single(r_other);
    let r = cairn_node::patient::person::chart_identities(&window, &charts).await;
    expect_as_window("patient::person::chart_identities", r);
    let r = cairn_node::patient::compare::chart_facts(&window, &charts).await;
    expect_as_window("patient::compare::chart_facts", r);
    let r = cairn_node::patient::compare::cross_vetoes(&window, &charts, &other).await;
    expect_as_window("patient::compare::cross_vetoes", r);
    let r = cairn_node::patient::edges::record_edges(&window, &charts).await;
    assert!(!expect_as_window("patient::edges::record_edges", r).is_empty());
    let r = cairn_node::patient::edges::record_doubts(&window, &charts).await;
    expect_as_window("patient::edges::record_doubts", r);
    let r = cairn_node::patient::edges::record_unverified_links(&window, &charts).await;
    expect_as_window("patient::edges::record_unverified_links", r);
    let r = cairn_node::patient::edges::record_is_doubted(&window, &charts).await;
    expect_as_window("patient::edges::record_is_doubted", r);
    let (lo, hi) = canonical_pair(p, q);
    let r = standing_link(&window, lo, hi).await;
    assert!(expect_as_window("chart_link::standing_link", r).is_some());
    // (`list_patient_medications` over a chart WITH medications is walked in the medication test.)

    // The duplicate banner and the worklist.
    let r = cairn_node::duplicate_review::possible_duplicates(&window, &charts).await;
    assert!(!expect_as_window("duplicate_review::possible_duplicates", r).is_empty());
    let r = cairn_node::duplicate_review::open_pairs_between(&window, &charts, &other).await;
    assert!(!expect_as_window("duplicate_review::open_pairs_between", r).is_empty());
    let r = cairn_node::duplicate_review::worklist::worklist(&window, 25).await;
    let list = expect_as_window("duplicate_review::worklist::worklist", r);
    assert!(
        !list.items.is_empty(),
        "the open proposal is on the worklist"
    );
    // `worklist` folds each side's record read into a per-item String, so a denial there never
    // reaches the outer Result: both sides must have READ.
    for item in &list.items {
        assert!(
            item.newer.is_ok() && item.older.is_ok(),
            "duplicate_review::worklist::worklist: a side's record failed on the window's login"
        );
    }
    let r = cairn_node::duplicate_review::worklist::worklist_count(&window).await;
    assert!(expect_as_window("duplicate_review::worklist::worklist_count", r) >= 1);
    let r = cairn_node::duplicate_check::read_snapshot(&window).await;
    expect_as_window("duplicate_check::read_snapshot", r);
    let r = cairn_node::duplicate_check::chart_check_pending(&window, p).await;
    expect_as_window("duplicate_check::chart_check_pending", r);

    // The paper-parity timer every gesture feeds.
    let r = cairn_node::ui_timing::record_gesture(&window, "signoff", 3, 120).await;
    expect_as_window("ui_timing::record_gesture", r);
}

/// "Same person", the review panel's confirm and unlink, and "Different people" — the four
/// judgements the window signs over an identity link.
#[tokio::test]
async fn the_windows_link_judgements_run_under_its_own_login() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, mut window) = owner_and_window(&base).await;
    let (sk, kid) = setup(&owner, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&owner).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let (a, b, x) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for (chart, wall) in [(a, 1), (b, 2), (x, 3)] {
        common::submit_registration(&owner, &sk, &kid, chart, wall).await;
    }

    // Confirm a machine's link of A–X, read through the window as its review panel reads it.
    let ev = link_assertion_event(&kid, a, x, LinkVerb::Link, 40, 0, "peer", false);
    common::apply_remote_raw(&owner, &sk, ev).await.unwrap();
    let (lo, hi) = canonical_pair(a, x);
    let record = ChartSet::new([a, x]).unwrap();
    let ca = standing_link(&window, lo, hi)
        .await
        .unwrap()
        .expect("the machine's link stands")
        .content_address;
    let r = confirm_link_as_shown(&mut window, a, x, &record, &ca, &who, ORIGIN).await;
    assert_not_denied("chart_link::confirm_link_as_shown", &r);
    r.expect("the confirm is recorded");

    // Unlink it again from the review panel, opened on A.
    let ca = standing_link(&window, lo, hi)
        .await
        .unwrap()
        .expect("the confirmed link stands")
        .content_address;
    let r = unlink_charts_as_shown_from(&mut window, a, x, a, &record, &ca, &who, ORIGIN).await;
    assert_not_denied("chart_link::unlink_charts_as_shown_from", &r);
    r.expect("the unlink is recorded");

    // "Same person" over A and B, each its own record.
    let (left, right) = (ChartSet::single(a), ChartSet::single(b));
    let shown = RecordsShown {
        this: &left,
        other: &right,
        pair: None,
    };
    let r = link_charts_as_shown(&mut window, a, b, shown, &who, ORIGIN).await;
    assert_not_denied("chart_link::link_charts_as_shown", &r);
    r.expect("the link is recorded");

    // "Different people" over {A, B} and {X}, with an open proposal between them.
    seed_proposal(&owner, b, x, "pending").await;
    let mine = ChartSet::new([a, b]).unwrap();
    let theirs = ChartSet::single(x);
    let r = cairn_node::duplicate_review::record_different_people(
        &mut window,
        &mine,
        &theirs,
        &who,
        ORIGIN,
    )
    .await;
    assert_not_denied("duplicate_review::record_different_people", &r);
    r.expect("the judgement is recorded");
}

/// The medication chart's writes: sign off the list, hold a line, release the hold, stop it.
#[tokio::test]
async fn the_windows_medication_writes_run_under_its_own_login() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (mut owner, mut window) = owner_and_window(&base).await;
    let (sk, kid, sk_h, kid_h) = common::medication_setup(&owner).await;
    let patient = Uuid::now_v7();
    common::submit_registration(&owner, &sk, &kid, patient, 0).await;
    let med = assert_medication(
        &mut owner,
        &sk,
        &kid,
        ORIGIN,
        patient,
        &AssertMedicationInput {
            term: "metoprolol",
            coding: None,
            formulation: None,
            dose_amount: Some("25"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    // The window signs every write with the node's key AND a human author (ADR-0053).
    let author = AuthorParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    let attest = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: None,
        note: None,
    };
    // The chart's medication list over a chart that HAS a line, so the group, conflict and
    // stray-chart reads behind it run (an empty chart returns before them).
    let r = list_patient_medications(&window, patient).await;
    let list = expect_as_window("medication::read::list_patient_medications", r);
    assert!(!list.rows.is_empty(), "the asserted line is listed");

    let shown = ChartSet::single(patient);
    let r =
        sign_off_medication_list(&mut window, &sk, ORIGIN, &attest, patient, Some(&shown)).await;
    assert_not_denied("medication::signoff::sign_off_medication_list", &r);
    r.expect("the sign-off is recorded");

    let hold = WithholdMedicationInput {
        reason: "BP 85/50",
        from: None,
        until: cairn_event::medication::HoldUntil::Reviewed,
    };
    let r = withhold_medication(
        &mut window,
        &sk,
        &kid,
        ORIGIN,
        patient,
        med,
        &hold,
        Some(&author),
    )
    .await;
    assert_not_denied("medication::withhold_medication", &r);
    let withhold = r.expect("the hold is recorded");

    let r = release_medication_holds(
        &mut window,
        &sk,
        &kid,
        ORIGIN,
        patient,
        med,
        &[withhold],
        Some("BP recovered"),
        Some(&author),
    )
    .await;
    assert_not_denied("medication::release_medication_holds", &r);
    r.expect("the release is recorded");

    let stop = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: None,
    };
    let r = cease_medication(
        &mut window,
        &sk,
        &kid,
        ORIGIN,
        patient,
        med,
        &stop,
        Some(&author),
        None,
    )
    .await;
    assert_not_denied("medication::cease_medication", &r);
    r.expect("the cessation is recorded");
}
