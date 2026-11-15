//! Tests for `supervision_panel.rs`. The pure ones and the source guards need no database; the
//! DB-gated ones (`an_assigner_links_and_a_supervisor_lifts_from_the_window`,
//! `a_superuser_key_is_released_for_enrol_after_a_refused_clinical`) self-skip without
//! `CAIRN_TEST_PG`. CI does not run the window's DB-gated tests (#905): run them locally.
use super::*;
use crate::signer::Credential;
use crate::whos_here::Claim;
use zeroize::Zeroizing;

const JS: &str = include_str!("../src-ui/supervision.js");

fn pass(word: &str) -> String {
    [word, "horse", "battery"].join("-")
}

fn credential(staff_id: &str, passphrase: &str) -> Credential {
    Credential {
        staff_id: staff_id.into(),
        passphrase: Zeroizing::new(passphrase.to_string()),
    }
}

fn member(role: &str, needs: bool, supervisors: &[&str]) -> StaffSupervision {
    StaffSupervision {
        staff_id: "ada1".into(),
        display_name: "Dr Ada Notewriter".into(),
        role: role.into(),
        needs_cosigning: needs,
        assigner: false,
        supervisors: supervisors.iter().map(|s| s.to_string()).collect(),
    }
}

/// Design §3.4: a requirement with nobody to meet it is said on the row, so the gap has an owner.
#[test]
fn the_gap_row_reads_no_supervisor_recorded() {
    let gap = staff_view(&member("clinician", true, &[]));
    assert!(
        gap.text
            .contains("needs co-signing · no supervisor recorded"),
        "{}",
        gap.text
    );
    let student = staff_view(&member("student", true, &[]));
    assert!(
        student
            .text
            .contains("needs co-signing (student) · no supervisor recorded"),
        "{}",
        student.text
    );
    assert!(student.student);
    let covered = staff_view(&member("clinician", true, &["jsmith"]));
    assert!(!covered.text.contains("no supervisor"), "{}", covered.text);
    assert!(
        covered.text.contains("supervised by jsmith"),
        "{}",
        covered.text
    );
    let free = staff_view(&member("clinician", false, &[]));
    assert!(!free.text.contains("no supervisor"), "{}", free.text);
}

/// "My supervisees" is read for the person named in who's here, by the vault's own staff-ID key;
/// with nobody named it lists nobody and says why, before the database is asked.
#[tokio::test]
async fn my_supervisees_needs_a_named_claim() {
    let state = AppState::mock(None);
    assert_eq!(
        list_my_supervisees_impl(&state).await.unwrap_err(),
        NOBODY_NAMED
    );
    let rows = vec![
        member("clinician", true, &["JSmith ", "bwong"]),
        StaffSupervision {
            staff_id: "other".into(),
            ..member("clinician", false, &["bwong"])
        },
    ];
    let mine = supervisees(&rows, " jsmith");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].staff_id, "ada1");
    // Narrowed to the row's OWN spelling of me: what "Step away" sends back to the door.
    assert_eq!(mine[0].supervisors, vec!["JSmith ".to_string()]);
    assert!(supervisees(&rows, "nobody").is_empty());
}

/// Source guard (no DB-free behavioural test can see the ORDER): the passphrase check, the wipe,
/// the lock, then the release (clinical first, enrol only on a refused purpose), every other
/// answer through `from_release_noting`; never the held-session door.
#[test]
fn a_supervision_act_wipes_then_releases_and_holds_nothing() {
    let src = include_str!("supervision_panel.rs");
    let body: String = crate::source_guard::rust_fn(src, "supervision_key<'a>")
        .split_whitespace()
        .collect();
    let order = [
        "has_passphrase()",
        "state.forget_held_key().await;",
        ".lock()",
        "Purpose::Clinical",
        "Release::RefusedPurpose",
        "Purpose::Enrol",
        "from_release_noting(",
    ];
    let at: Vec<usize> = order
        .iter()
        .map(|s| body.find(s).unwrap_or_else(|| panic!("{s}\n{body}")))
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{body}");
    assert!(
        !body.contains("signer_for_act")
            && !body.contains("session")
            && !body.contains("from_release("),
        "{body}"
    );
    let act = crate::source_guard::rust_fn(src, "supervision_act_impl");
    assert!(act.contains("supervision_key(state, &credential)"), "{act}");
}

/// A list read happens AFTER a write; its failure must not say "Nothing was recorded".
#[test]
fn a_failed_supervision_read_never_says_nothing_was_recorded() {
    let src = include_str!("supervision_panel.rs");
    let b = crate::source_guard::rust_fn(src, "read_staff");
    assert!(b.contains("read_fault(") && !b.contains(" fault("), "{b}");
    for f in ["supervisionRefresh", "superviseesRefresh"] {
        let body = crate::source_guard::js_fn(JS, f);
        let catch = &body[body.find("catch").expect("a catch")..];
        assert!(catch.contains(".replaceChildren()"), "{f}: {catch}");
    }
}

/// Whatever the act's outcome, the passphrase leaves the strip: the clear runs AFTER the
/// try/catch, never inside it, or a refused act would leave it for the next person.
#[test]
fn supervision_act_clears_the_strip_both_ways() {
    let body = crate::source_guard::js_fn(JS, "supervisionAct");
    let clear = body.find("clearSigner(false)").expect("clears the strip");
    let catch_end = body.rfind("failureText(failure)").expect("the catch");
    assert!(clear > catch_end, "{body}");
    assert_eq!(body[..clear].matches("clearSigner").count(), 0, "{body}");
}

/// Principle 3: no act here asks a question.
#[test]
fn no_dialog() {
    for d in ["confirm(", "alert(", "prompt("] {
        assert!(!JS.contains(d), "{d}");
    }
}

/// A blank supervisor, an unknown op word, or no passphrase is refused before the vault is asked.
#[tokio::test]
async fn a_malformed_act_is_refused_before_any_release() {
    let state = AppState::mock(None);
    let act = |op: &str, sup: Option<&str>, p: &str| {
        supervision_act_impl(
            &state,
            credential("boss", p),
            op.to_string(),
            "ada1".into(),
            sup.map(String::from),
        )
    };
    assert_eq!(
        act("link-supervisor", Some("  "), &pass("s"))
            .await
            .unwrap_err(),
        NEED_SUPERVISOR
    );
    assert_eq!(
        act("Link", None, &pass("s")).await.unwrap_err(),
        NOT_RECORDED
    );
    assert_eq!(
        act("lift-cosign", None, "").await.unwrap_err(),
        NEED_CREDENTIAL
    );
    assert_eq!(
        act("lift-cosign", None, &pass("s")).await.unwrap_err(),
        FIXTURE
    );
}

/// DB-gated setup: `boss` (superuser), and the clinicians `assign1`, `sup1` and `trainee1`, all
/// enrolled by the superuser (so confirmed at once, which every non-superuser approver needs),
/// reached through the window's own login role.
async fn world() -> Option<(impl Sized, AppState, String, String)> {
    use cairn_node::vault::{bootstrap_superuser, release_for_act, NewMember, Purpose, Release};
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
    let (su_p, cl_p) = (pass("su"), pass("cl"));
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
    let su_key = match release_for_act(&window, "boss", &su_p, Purpose::Enrol)
        .await
        .unwrap()
    {
        Release::Released { key, .. } => key,
        other => panic!("{other:?}"),
    };
    for id in ["assign1", "sup1", "trainee1"] {
        cairn_node::vault::enrol_member(&owner, &member(id, "clinician", &cl_p), "boss", &su_key)
            .await
            .unwrap();
    }
    let mut state = AppState::mock(None);
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        window,
    )));
    Some((guard, state, su_p, cl_p))
}

async fn act(
    state: &AppState,
    who: &str,
    p: &str,
    op: &str,
    sup: Option<&str>,
) -> Result<String, String> {
    supervision_act_impl(
        state,
        credential(who, p),
        op.into(),
        "trainee1".into(),
        sup.map(String::from),
    )
    .await
}

/// DB-gated: a superuser's key serves only the enrol purpose, so the clinical refusal is retried
/// once with enrol (that refusal costs no try). A wrong passphrase then reads as wrong, never as
/// "a superuser cannot sign a clinical act".
#[tokio::test]
async fn a_superuser_key_is_released_for_enrol_after_a_refused_clinical() {
    let Some((_g, state, su_p, _)) = world().await else {
        return;
    };
    supervision_act_impl(
        &state,
        credential("boss", &su_p),
        "grant-assigner".into(),
        "assign1".into(),
        None,
    )
    .await
    .expect("a superuser grants the right");
    let wrong = supervision_act_impl(
        &state,
        credential("boss", &pass("nope")),
        "withdraw-assigner".into(),
        "assign1".into(),
        None,
    )
    .await
    .unwrap_err();
    assert!(
        wrong.starts_with("That passphrase is not right."),
        "{wrong}"
    );
}

/// DB-gated (run locally, #905): the whole flow through the window's `_impl` fns. The superuser
/// grants the right; the assigner links a supervisor and requires co-signing; the supervisor,
/// named in who's here, sees their supervisee and lifts it. A member without the right is refused
/// in `REFUSED`'s sentence.
#[tokio::test]
async fn an_assigner_links_and_a_supervisor_lifts_from_the_window() {
    let Some((_g, state, su_p, cl_p)) = world().await else {
        return;
    };
    supervision_act_impl(
        &state,
        credential("boss", &su_p),
        "grant-assigner".into(),
        "assign1".into(),
        None,
    )
    .await
    .expect("granted");
    // sup1 holds no right, so the door refuses: said plainly, nothing recorded.
    let refused = act(&state, "sup1", &cl_p, "require-cosign", None).await;
    assert_eq!(refused.unwrap_err(), REFUSED);
    act(&state, "assign1", &cl_p, "link-supervisor", Some("sup1"))
        .await
        .expect("linked");
    act(&state, "assign1", &cl_p, "require-cosign", None)
        .await
        .expect("required");
    let all = list_supervision_impl(&state).await.unwrap();
    let t = all
        .iter()
        .find(|r| r.staff_id == "trainee1")
        .expect("listed");
    assert!(
        t.needs_cosigning && t.supervisors == vec!["sup1".to_string()],
        "{t:?}"
    );
    assert!(all.iter().any(|r| r.staff_id == "assign1" && r.assigner));
    state.whos_here.lock().await.pick(Claim::Named {
        staff_id: "sup1".into(),
        display_name: "Test Person".into(),
    });
    let mine = list_my_supervisees_impl(&state).await.unwrap();
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(mine[0].staff_id, "trainee1");
    let said = act(&state, "sup1", &cl_p, "lift-cosign", None)
        .await
        .expect("a supervisor lifts it");
    assert!(said.contains("trainee1"), "{said}");
    let after = list_my_supervisees_impl(&state).await.unwrap();
    assert!(!after[0].needs_cosigning, "{after:?}");
    act(&state, "sup1", &cl_p, "unlink-supervisor", Some("sup1"))
        .await
        .expect("a supervisor steps away");
    assert!(list_my_supervisees_impl(&state).await.unwrap().is_empty());
}

/// The lift's success line claims no waiting notes exist (there may be none, or all co-signed).
#[test]
fn the_lift_sentence_claims_no_waiting_notes() {
    assert_eq!(
        done_sentence(SupervisionOp::LiftCosign, "trainee1", None),
        "trainee1 no longer needs notes co-signed. Notes signed while it was required still need \
         a co-signature if they have none."
    );
}

/// The success branch of a JS refresh: from its `try {` to its `} catch`.
fn success_branch<'a>(js: &'a str, f: &str) -> &'a str {
    let body = crate::source_guard::js_fn(js, f);
    &body[body.find("try {").expect("a try")..body.find("} catch").expect("a catch")]
}

/// A good re-read clears an earlier failure's sentence (NOBODY_NAMED, READ_FAILED), which would
/// otherwise stay on screen beside a full list and be false; an act's own outcome is kept.
#[test]
fn a_good_read_clears_a_stale_status() {
    for (f, id) in [
        ("supervisionRefresh", "supervision-status"),
        ("superviseesRefresh", "my-supervisees-status"),
    ] {
        let ok = success_branch(JS, f);
        let clear = format!("if (!outcome) setMessage(el(\"{id}\"), \"\");");
        assert!(ok.contains(&clear), "{f}: {ok}");
    }
}
