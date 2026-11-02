//! Tests for `superuser_panel.rs`. The pure ones need no database; the DB-gated ones
//! (`a_superuser_grants_and_an_enroller_then_rekeys`, `a_repudiated_key_leaves_the_worklist_for_good`)
//! self-skip without `CAIRN_TEST_PG`.
use super::*;
use crate::signer::Credential;
use zeroize::Zeroizing;

fn pass(word: &str) -> String {
    [word, "horse", "battery"].join("-")
}

fn credential(staff_id: &str, passphrase: &str) -> Credential {
    Credential {
        staff_id: staff_id.into(),
        passphrase: Zeroizing::new(passphrase.to_string()),
    }
}

fn row(approver: &str, doubtful: bool) -> Unconfirmed {
    Unconfirmed {
        staff_id: "nurse1".into(),
        display_name: "Nia Nurse".into(),
        kid: "kid-1".into(),
        approver_staff_id: approver.into(),
        approved_at: "2026-11-01T04:05:00Z".into(),
        doubtful,
        stops_at: None,
    }
}

fn utc() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-11-01T10:00:00Z")
        .unwrap()
        .to_utc()
}

#[test]
fn a_worklist_row_says_who_approved_it_and_when() {
    assert_eq!(
        unconfirmed_text(&row("enrol1", false), &utc()),
        "Nia Nurse (nurse1): new key approved by enrol1 on Sun 1 Nov 2026 04:05."
    );
}

#[test]
fn a_doubtful_row_says_why() {
    let said = unconfirmed_text(&row("enrol1", true), &utc());
    assert!(
        said.ends_with(" Doubtful: approved by a repudiated enroller."),
        "{said}"
    );
}

/// §5.5 I1: under a site expiry the superuser sees when each unconfirmed key stops signing —
/// the one person who can prevent it, warned even if its holder signs nothing in its last week.
#[test]
fn a_row_under_a_site_expiry_says_when_it_stops_signing() {
    let mut r = row("enrol1", false);
    assert!(!unconfirmed_text(&r, &utc()).contains("signing"));
    r.stops_at = Some("2026-11-20T04:05:00Z".into());
    let said = unconfirmed_text(&r, &utc());
    assert!(
        said.ends_with(" Stops signing on Fri 20 Nov 2026 04:05."),
        "{said}"
    );
    r.stops_at = Some("2026-11-01T09:00:00Z".into());
    let said = unconfirmed_text(&r, &utc());
    assert!(
        said.ends_with(
            " Stopped signing on Sun 1 Nov 2026 09:00. Confirm it to let it sign again."
        ),
        "{said}"
    );
}

#[test]
fn a_gone_approver_is_said_so() {
    let said = unconfirmed_text(&row("", false), &utc());
    assert!(
        said.contains("approved by someone no longer in the vault on"),
        "{said}"
    );
}

fn grant_row(enroller: bool, kid: Option<&str>, key_repudiated: bool) -> EnrollerRow {
    EnrollerRow {
        staff_id: "enrol1".into(),
        display_name: "Eve".into(),
        enroller,
        kid: kid.map(String::from),
        key_repudiated,
    }
}

#[test]
fn a_grant_row_says_what_the_member_can_do() {
    let r = |enroller| grant_row(enroller, Some("k1"), false);
    // "has the role", never "can enrol": the floor also wants their own key confirmed and current.
    assert_eq!(
        enroller_text(&r(true)),
        "Eve (enrol1): has the enroller role."
    );
    assert_eq!(
        enroller_text(&r(false)),
        "Eve (enrol1): does not have the enroller role."
    );
}

/// §5.5: a confirmed key (an impostor found later) or an enroller's own key is never on the
/// worklist, so each grant row offers its member's CURRENT key to Repudiate. A key already
/// repudiated is said so and offered no more (repudiation is final; the remedy is a re-key).
#[test]
fn a_grant_row_offers_its_members_key_to_repudiate_unless_already_repudiated() {
    let live = enroller_view(&grant_row(true, Some("k1"), false));
    assert_eq!(live.kid.as_deref(), Some("k1"));
    assert_eq!(live.staff_id, "enrol1");
    assert!(live.enroller);
    let gone = enroller_view(&grant_row(true, Some("k1"), true));
    assert_eq!(gone.kid, None, "nothing left to repudiate");
    assert_eq!(
        gone.text,
        "Eve (enrol1): has the enroller role. Their key is repudiated: give them a new key from \
         the re-key form."
    );
    assert_eq!(enroller_view(&grant_row(false, None, false)).kid, None);
}

/// `enrollers()` lists non-superusers only, so a listed ID is refused in one plain sentence.
#[test]
fn a_listed_member_is_told_only_a_superuser_may() {
    let rows = vec![grant_row(true, None, false)];
    assert_eq!(
        superuser_refusal(&rows, " ENROL1 "),
        Some(ONLY_SUPERUSER.to_string())
    );
    assert_eq!(superuser_refusal(&rows, "boss"), None);
    assert!(!ONLY_SUPERUSER.contains('\u{2014}'));
}

/// A staff ID alone never reaches the vault (it would count as a wrong guess); fixture state says so.
#[tokio::test]
async fn no_passphrase_and_no_database_are_refused_plainly() {
    let state = AppState::mock(None);
    let none = set_enroller_impl(&state, credential("boss", ""), "x", true).await;
    assert_eq!(none.unwrap_err(), NEED_SUPERUSER);
    let fixture = judge_key_impl(
        &state,
        credential("boss", &pass("s")),
        "k",
        Judgement::Repudiate,
    )
    .await;
    assert_eq!(fixture.unwrap_err(), FIXTURE);
}

/// Source guard (no DB-free behavioural test can see the ORDER): wipe, then the shared release;
/// never the held-session door.
#[test]
fn a_superuser_act_wipes_then_releases_once_and_holds_nothing() {
    let body: String =
        crate::source_guard::rust_fn(include_str!("superuser_panel.rs"), "superuser_key<'a>")
            .split_whitespace()
            .collect();
    let order = [
        "state.forget_held_key().await;",
        "enrollers(&db)",
        "release_enrol_key(",
    ];
    let at: Vec<usize> = order
        .iter()
        .map(|s| body.find(s).unwrap_or_else(|| panic!("{s}\n{body}")))
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{body}");
    assert!(
        !body.contains("signer_for_act") && !body.contains("session"),
        "{body}"
    );
    for f in ["set_enroller_impl", "judge_key_impl"] {
        let b = crate::source_guard::rust_fn(include_str!("superuser_panel.rs"), f);
        assert!(b.contains("superuser_key(state, &credential)"), "{f}");
    }
}

/// Principle 3: Repudiate opens no dialog; the page never calls `confirm(`.
#[test]
fn repudiate_asks_no_question() {
    let js = include_str!("../src-ui/superuser.js");
    assert!(
        !js.contains("confirm("),
        "no dialog on Repudiate (principle 3)"
    );
    assert!(!js.contains("alert("));
}

/// The grant row's Repudiate acts on the kid the ROW carries (never one typed or looked up again)
/// and is drawn only when the row has one; it goes through the same in-flight-guarded button and
/// the same act-then-refresh path as the worklist's.
#[test]
fn a_grant_rows_repudiate_uses_the_rows_own_key() {
    let js = include_str!("../src-ui/superuser.js");
    let body = crate::source_guard::js_fn(js, "superuserRefresh");
    assert!(
        body.contains("superuserAct(\"judge_key\", { kid: m.kid, judgement: \"repudiate\" })"),
        "{body}"
    );
    assert!(body.contains("m.kid ?"), "drawn only with a key: {body}");
}

/// DB-gated setup: `boss` (superuser), `enrol1` and `nurse1` (clinicians), as the window's login.
async fn world() -> Option<(impl Sized, AppState, String, String)> {
    use cairn_node::vault::{bootstrap_superuser, NewMember, Purpose, Release};
    let Ok(cs) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    owner
        .batch_execute(
            "TRUNCATE vault_attempt, vault_member, vault_terminal, vault_policy, event_log, actor_event, \
             patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
        )
        .await
        .unwrap();
    let (su_p, en_p) = (pass("su"), pass("en"));
    let member = |staff_id, role, passphrase| NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: None,
    };
    bootstrap_superuser(&owner, &member("boss", "superuser", &su_p))
        .await
        .unwrap();
    owner
        .batch_execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_window_probe') \
             THEN CREATE ROLE cairn_window_probe LOGIN; END IF; \
             EXCEPTION WHEN duplicate_object THEN NULL; END $$; \
             GRANT cairn_window TO cairn_window_probe;",
        )
        .await
        .unwrap();
    let window = cairn_node::db::connect(&format!("{cs} user=cairn_window_probe"))
        .await
        .unwrap();
    let su_key = match cairn_node::vault::release_for_act(&window, "boss", &su_p, Purpose::Enrol)
        .await
        .unwrap()
    {
        Release::Released { key, .. } => key,
        other => panic!("{other:?}"),
    };
    for (id, p) in [("enrol1", &en_p), ("nurse1", &pass("nu"))] {
        cairn_node::vault::enrol_member(&owner, &member(id, "clinician", p), "boss", &su_key)
            .await
            .unwrap();
    }
    let mut state = AppState::mock(None);
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        window,
    )));
    Some((guard, state, su_p, en_p))
}

/// DB-gated helper: `boss` grants `enrol1` (superuser-approved, so confirmed at once, which db/061
/// requires of an enroller), then `enrol1` re-keys `nurse1`. An enroller-approved key stays
/// UNCONFIRMED (db/061 `cairn_vault_enrol`), so `nurse1`'s new key is on the worklist.
async fn nurse_rekeyed_by_an_enroller(state: &AppState, su_p: &str, en_p: &str) {
    set_enroller_impl(state, credential("boss", su_p), "enrol1", true)
        .await
        .expect("granted");
    let new_p = pass("new");
    let form = crate::rekey::RekeyForm {
        staff_id: "nurse1".into(),
        role: "clinician".into(),
        display_name: String::new(),
        phone: String::new(),
        email: String::new(),
        passphrase: Zeroizing::new(new_p.clone()),
        passphrase_again: Zeroizing::new(new_p),
        hint: String::new(),
    };
    crate::rekey::rekey_member_impl(state, credential("enrol1", en_p), form)
        .await
        .expect("an enroller re-keys a clinician");
}

/// DB-gated (run locally): a non-superuser is refused, a grant is listed, the enroller's re-key
/// lands on the worklist naming the enroller, and Confirm takes it off.
#[tokio::test]
async fn a_superuser_grants_and_an_enroller_then_rekeys() {
    let Some((_g, state, su_p, en_p)) = world().await else {
        return;
    };
    // A non-superuser is told so before the vault is asked.
    let refused = set_enroller_impl(&state, credential("enrol1", &en_p), "nurse1", true).await;
    assert_eq!(refused.unwrap_err(), ONLY_SUPERUSER);
    nurse_rekeyed_by_an_enroller(&state, &su_p, &en_p).await;
    let granted = list_enrollers_impl(&state).await.unwrap();
    assert!(granted.iter().any(|r| r.staff_id == "enrol1" && r.enroller));
    let work = list_unconfirmed_impl(&state).await.unwrap();
    let mine = work
        .iter()
        .find(|u| {
            u.text.starts_with("Test Person (nurse1)") && u.text.contains("approved by enrol1")
        })
        .expect("on the worklist");
    judge_key_impl(
        &state,
        credential("boss", &su_p),
        &mine.kid,
        Judgement::Confirm,
    )
    .await
    .expect("confirmed");
    assert!(list_unconfirmed_impl(&state)
        .await
        .unwrap()
        .iter()
        .all(|u| u.kid != mine.kid));
}

/// DB-gated: a repudiated key leaves the worklist, and cannot then be confirmed.
#[tokio::test]
async fn a_repudiated_key_leaves_the_worklist_for_good() {
    let Some((_g, state, su_p, en_p)) = world().await else {
        return;
    };
    nurse_rekeyed_by_an_enroller(&state, &su_p, &en_p).await;
    let work = list_unconfirmed_impl(&state).await.unwrap();
    let kid = work
        .iter()
        .find(|u| u.text.starts_with("Test Person (nurse1)"))
        .expect("on the worklist")
        .kid
        .clone();
    judge_key_impl(
        &state,
        credential("boss", &su_p),
        &kid,
        Judgement::Repudiate,
    )
    .await
    .expect("repudiated");
    assert!(list_unconfirmed_impl(&state)
        .await
        .unwrap()
        .iter()
        .all(|u| u.kid != kid));
    let again = judge_key_impl(&state, credential("boss", &su_p), &kid, Judgement::Confirm).await;
    assert_eq!(
        again.unwrap_err(),
        REFUSED,
        "confirm after repudiate is the floor's refusal, not a fault"
    );
}

/// A final act is never behind a bare `false`: the page names the judgement, and anything else
/// (a boolean, a typo) is refused by serde before a key is released.
#[test]
fn a_judgement_is_named_never_a_boolean() {
    let j = |s: &str| serde_json::from_str::<Judgement>(s);
    assert_eq!(j("\"confirm\"").unwrap(), Judgement::Confirm);
    assert_eq!(j("\"repudiate\"").unwrap(), Judgement::Repudiate);
    for bad in ["false", "true", "\"Repudiate\"", "\"\""] {
        assert!(j(bad).is_err(), "{bad}");
    }
    let js = include_str!("../src-ui/superuser.js");
    assert!(
        !js.contains("confirm: "),
        "no boolean judgement left in the page"
    );
}

/// Three answers to a failed write, three sentences: the floor refused on purpose (the row was
/// out of date, or the act no longer applies), the node failed with an answer, or no answer came.
#[test]
fn a_failed_write_says_which_kind_of_failure() {
    assert_eq!(write_failure_sentence(true, true), REFUSED);
    assert_eq!(write_failure_sentence(false, true), NOT_RECORDED);
    assert_eq!(write_failure_sentence(false, false), OUTCOME_UNKNOWN);
    assert!(REFUSED.contains("nothing was recorded") && REFUSED.contains("lists"));
}

/// A list read happens AFTER a write; if it fails, the write's outcome must not be contradicted
/// by "Nothing was recorded" (the superuser would press again).
#[test]
fn a_failed_list_read_never_says_nothing_was_recorded() {
    assert!(!READ_FAILED.contains("recorded"), "{READ_FAILED}");
    for f in ["list_unconfirmed_impl", "list_enrollers_impl"] {
        let b = crate::source_guard::rust_fn(include_str!("superuser_panel.rs"), f);
        assert!(
            b.contains("read_fault(") && !b.contains(" fault("),
            "{f}: {b}"
        );
    }
    // The page drops rows it could not re-read, so no stale key is left to press.
    let js = include_str!("../src-ui/superuser.js");
    let body = crate::source_guard::js_fn(js, "superuserRefresh");
    let catch = &body[body.find("catch").expect("a catch")..];
    assert!(
        catch.contains("grants.replaceChildren()") && catch.contains("keys.replaceChildren()"),
        "{catch}"
    );
}

/// Whatever the act's outcome, the superuser's passphrase leaves the strip: the clear runs
/// AFTER the try/catch, never inside it, or a failed act would leave it for the next person.
#[test]
fn a_superuser_act_clears_the_strip_both_ways() {
    let js = include_str!("../src-ui/superuser.js");
    let body = crate::source_guard::js_fn(js, "superuserAct");
    let clear = body.find("clearSigner(false)").expect("clears the strip");
    let catch_end = body.rfind("failureText(failure)").expect("the catch");
    assert!(clear > catch_end, "{body}");
    assert!(body[..clear].matches("clearSigner").count() == 0, "{body}");
}

/// DB-gated (run locally, #905): the race the floor now closes. The superuser reads the grant
/// list (nurse1's key K1), then an enroller re-keys nurse1 (K2), then the superuser presses
/// Repudiate on the stale row. The floor refuses K1, the page says the lists were out of date,
/// and K2 is still on the worklist, unjudged.
#[tokio::test]
async fn repudiate_on_a_stale_row_is_refused_and_says_so() {
    let Some((_g, state, su_p, en_p)) = world().await else {
        return;
    };
    let stale = list_enrollers_impl(&state)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.staff_id == "nurse1")
        .and_then(|r| r.kid)
        .expect("nurse1's key, as listed");
    nurse_rekeyed_by_an_enroller(&state, &su_p, &en_p).await;
    let said = judge_key_impl(
        &state,
        credential("boss", &su_p),
        &stale,
        Judgement::Repudiate,
    )
    .await
    .unwrap_err();
    assert_eq!(said, REFUSED);
    let work = list_unconfirmed_impl(&state).await.unwrap();
    assert!(
        work.iter()
            .any(|u| u.text.starts_with("Test Person (nurse1)") && u.kid != stale),
        "the new key is still waiting: {work:?}"
    );
}
