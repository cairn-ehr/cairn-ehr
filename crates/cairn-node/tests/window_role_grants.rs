//! #898 — the window's whole day, run under the window's OWN login, never the schema owner's.
//!
//! # Why this file exists
//!
//! db/062 gives the reference window a least-privilege group role, `cairn_window` (it inherits
//! `cairn_agent`, the runtime role, plus two node reads). Every other suite connects as the
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
use cairn_node::vault::{self, NewMember, Purpose, Release};
use cairn_patient_search::{CandidateList, SearchQuery};
use common::{cs, enroll_human, link_assertion_event, seed_proposal, setup, window_login};
use tokio_postgres::error::SqlState;
use tokio_postgres::Client;
use uuid::Uuid;

/// The identity and proposal tables the link/duplicate walks write; cleared by `setup`.
const TABLES: [&str; 6] = [
    "patient_link",
    "person_member",
    "identity_projection_flag",
    "link_veto_flag",
    "chart_identity_state",
    "match_proposal",
];

const ORIGIN: &str = "window-role-grants";

/// Fail if `result` is a privilege refusal. Any other outcome — success or an ordinary floor
/// refusal — is fine here; the caller asserts more where the walk depends on success.
///
/// Two checks, because not every `cairn_node` path keeps the `tokio_postgres::Error` in its
/// chain: some render the database's message into a string (`db_text`, `legible_db_error`),
/// which drops the SQLSTATE. Postgres words every 42501 "permission denied for …", so the
/// text check catches those. The message names only `entry` — never the error, which can carry
/// identity-built or credential-adjacent text. To see the object, rerun with `-- --nocapture`
/// after adding an `eprintln!` locally, or `SET ROLE cairn_window` in psql and run the SQL.
fn assert_not_denied<T>(entry: &str, result: &anyhow::Result<T>) {
    let Err(e) = result else { return };
    let code = e.chain().find_map(|cause| {
        cause
            .downcast_ref::<tokio_postgres::Error>()
            .and_then(|pg| pg.code().cloned())
    });
    assert_ne!(
        code,
        Some(SqlState::INSUFFICIENT_PRIVILEGE),
        "{entry}: the window's role lacks a grant this command needs (SQLSTATE 42501)"
    );
    assert!(
        !format!("{e:#}").contains("permission denied"),
        "{entry}: the window's role lacks a grant this command needs (permission denied)"
    );
}

/// The owner connection (schema loaded, setup runs here) and the window's own connection,
/// opened with the NON-loading `db::connect` exactly as the window opens it.
async fn owner_and_window(base: &str) -> (Client, Client) {
    let owner = db::connect_and_load_schema(base).await.unwrap();
    let window = db::connect(&window_login(&owner, base).await)
        .await
        .expect("the window's login connects");
    (owner, window)
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
    assert_not_denied("reopen: own backend", &backend);
    let r = cairn_node::identity::load_local(&window).await;
    assert_not_denied("identity::load_local", &r);
    let r = cairn_node::actor_enrolment::device_actor_standing(&window, &kid).await;
    assert_not_denied("actor_enrolment::device_actor_standing", &r);
    let r = cairn_node::actor_enrolment::require_device_actor(&window, &kid).await;
    assert_not_denied("actor_enrolment::require_device_actor", &r);

    // The funnel: search, then register what was searched.
    let query = SearchQuery::new("window-probe", None, &[]);
    let found = cairn_node::patient::search::search_patients(&window, &query, &today).await;
    assert_not_denied("patient::search::search_patients", &found);
    let nothing = CandidateList {
        people: vec![],
        incomplete: false,
        incomplete_reason: None,
    };
    let registered = cairn_node::patient::register::register_patient(
        &mut window,
        &sk,
        &kid,
        ORIGIN,
        None,
        &query,
        &nothing,
    )
    .await;
    assert_not_denied("patient::register::register_patient", &registered);
    let p = registered.expect("registration is admitted");

    // A second chart, a machine link between them and an open proposal, so the chart reads
    // below have an edge, a standing link and a duplicate entry to find.
    let q = Uuid::now_v7();
    common::submit_registration(&owner, &sk, &kid, q, 1).await;
    let ev = link_assertion_event(&kid, p, q, LinkVerb::Link, 50, 0, "peer", false);
    common::apply_remote_raw(&owner, &sk, ev).await.unwrap();
    let r_other = Uuid::now_v7();
    common::submit_registration(&owner, &sk, &kid, r_other, 2).await;
    seed_proposal(&owner, p, r_other, "pending").await;

    let r = cairn_node::patient::candidate_read::candidates_by_id(&window, &[p], &today).await;
    assert_not_denied("patient::candidate_read::candidates_by_id", &r);
    let charts = cairn_node::patient::person::person_charts(&window, p).await;
    assert_not_denied("patient::person::person_charts", &charts);
    let charts = charts.unwrap();
    let other = ChartSet::single(r_other);
    let r = cairn_node::patient::person::chart_identities(&window, &charts).await;
    assert_not_denied("patient::person::chart_identities", &r);
    let r = cairn_node::patient::compare::chart_facts(&window, &charts).await;
    assert_not_denied("patient::compare::chart_facts", &r);
    let r = cairn_node::patient::compare::cross_vetoes(&window, &charts, &other).await;
    assert_not_denied("patient::compare::cross_vetoes", &r);
    let r = cairn_node::patient::edges::record_edges(&window, &charts).await;
    assert_not_denied("patient::edges::record_edges", &r);
    let r = cairn_node::patient::edges::record_doubts(&window, &charts).await;
    assert_not_denied("patient::edges::record_doubts", &r);
    let r = cairn_node::patient::edges::record_unverified_links(&window, &charts).await;
    assert_not_denied("patient::edges::record_unverified_links", &r);
    let r = cairn_node::patient::edges::record_is_doubted(&window, &charts).await;
    assert_not_denied("patient::edges::record_is_doubted", &r);
    let (lo, hi) = canonical_pair(p, q);
    let r = standing_link(&window, lo, hi).await;
    assert_not_denied("chart_link::standing_link", &r);
    let r = list_patient_medications(&window, p).await;
    assert_not_denied("medication::read::list_patient_medications", &r);

    // The duplicate banner and the worklist.
    let r = cairn_node::duplicate_review::possible_duplicates(&window, &charts).await;
    assert_not_denied("duplicate_review::possible_duplicates", &r);
    let r = cairn_node::duplicate_review::open_pairs_between(&window, &charts, &other).await;
    assert_not_denied("duplicate_review::open_pairs_between", &r);
    let r = cairn_node::duplicate_review::worklist::worklist(&window, 25).await;
    assert_not_denied("duplicate_review::worklist::worklist", &r);
    let r = cairn_node::duplicate_review::worklist::worklist_count(&window).await;
    assert_not_denied("duplicate_review::worklist::worklist_count", &r);
    let r = cairn_node::duplicate_check::read_snapshot(&window).await;
    assert_not_denied("duplicate_check::read_snapshot", &r);
    let r = cairn_node::duplicate_check::chart_check_pending(&window, p).await;
    assert_not_denied("duplicate_check::chart_check_pending", &r);

    // The paper-parity timer every gesture feeds.
    let r = cairn_node::ui_timing::record_gesture(&window, "signoff", 3, 120).await;
    assert_not_denied("ui_timing::record_gesture", &r);
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

/// The vault the window talks to: the terminal lock's stored hash, a staff member's hint, the
/// duplicate-ID check, an enrolment, and the release of a key for one act.
#[tokio::test]
async fn the_windows_vault_doors_run_under_its_own_login() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    setup(&owner, &[]).await;
    owner
        .batch_execute("TRUNCATE vault_attempt, vault_member, vault_terminal CASCADE")
        .await
        .unwrap();
    let su_pass = format!("su-{}", Uuid::now_v7());
    let member = |staff_id, role, passphrase| NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: Some("a hint"),
    };
    // The first superuser is bootstrapped by the owner's console verb, never by the window.
    vault::bootstrap_superuser(&owner, &member("boss", "superuser", &su_pass))
        .await
        .unwrap();

    let terminal = window
        .query_one("SELECT cairn_vault_terminal_phc()", &[])
        .await
        .map_err(anyhow::Error::from);
    assert_not_denied("terminal lock: cairn_vault_terminal_phc", &terminal);
    let hint = window
        .query_one("SELECT cairn_vault_hint($1)", &[&"boss"])
        .await
        .map_err(anyhow::Error::from);
    assert_not_denied("sign-in hint: cairn_vault_hint", &hint);
    let r = vault::staff_id_taken(&window, "jsmith").await;
    assert_not_denied("vault::staff_id_taken", &r);

    let r = vault::release_for_act(&window, "boss", &su_pass, Purpose::Enrol).await;
    assert_not_denied("vault::release_for_act", &r);
    let Release::Released { key: su, .. } = r.unwrap() else {
        panic!("the superuser's key is released for an enrolment")
    };
    let clin_pass = format!("clin-{}", Uuid::now_v7());
    let r = vault::enrol_member(
        &window,
        &member("jsmith", "clinician", &clin_pass),
        "boss",
        &su,
    )
    .await;
    assert_not_denied("vault::enrol_member", &r);
    r.expect("the enrolment is recorded");

    // Pure, but the window calls it: hashing the terminal passphrase touches no table.
    assert!(vault::terminal_phc(&clin_pass).is_ok());
}
