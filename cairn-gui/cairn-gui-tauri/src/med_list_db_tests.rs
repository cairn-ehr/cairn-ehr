//! DB-gated walks for the medication list (on `CAIRN_TEST_PG`, run locally — #905: CI never runs
//! these), on `supervised_note`'s fixture: the window's own login, a John Doe chart, and Drs Bea,
//! Sue and Zed in the vault. Who's here names Bea, whose list it is. There is no drug list on the
//! test database, so an allergy mark here is by code, and an uncoded line reads "not checked".
use super::*;
use crate::allergies::{allergy_none_impl, allergy_record_impl, EntryInput};
use crate::allergy_check::CodingArg;
use crate::med_chart::NAME_IT;
use crate::med_list_sign::{med_list_sign_impl, MedListSigned, MED_LIST_STALE};
use crate::notes::review::db_tests::{credential, supervised_note};
use crate::signer::Credential;
use crate::state::Now;
use cairn_node::allergy::check::MOIETY_SYSTEM;
use cairn_node::allergy::view::{build_view, ListStatus};
use tokio_postgres::Client;

/// Name `staff` in who's here, as the person picker does.
pub(super) async fn here(state: &AppState, staff: &str, name: &str) {
    state.whos_here.lock().await.pick(Claim::Named {
        staff_id: staff.into(),
        display_name: name.into(),
    });
}

/// The fixture with an empty draft list and Bea named; `None` without `CAIRN_TEST_PG`.
pub(super) async fn bea_list() -> Option<(tokio_postgres::Client, Client, AppState, Uuid)> {
    let (guard, owner, state, chart, _) = supervised_note().await?;
    owner
        .batch_execute("TRUNCATE medication_draft_line, medication_draft_line_deleted")
        .await
        .unwrap();
    here(&state, "bea", "Dr Bea").await;
    Some((guard, owner, state, chart))
}

/// A drugref moiety pick (no drug list is installed: the code alone is what the check reads).
pub(super) fn pick(term: &str, code: u128) -> Vec<PartArg> {
    vec![PartArg {
        term: term.into(),
        coding: Some(CodingArg {
            system: MOIETY_SYSTEM.into(),
            code: Uuid::from_u128(code).to_string(),
            display: term.into(),
        }),
    }]
}

/// One finished Start line (`typed` + " " + `directions`), new, having seen no warning.
pub(super) fn arg(
    (typed, directions): (&str, &str),
    parts: Vec<PartArg>,
    hand_ticked: bool,
) -> LineArg {
    LineArg {
        line_id: Uuid::now_v7().to_string(),
        charted_as: "started-here".into(),
        line_text: format!("{typed} {directions}"),
        parts,
        typed_drug: typed.into(),
        directions: directions.into(),
        reason: None,
        hand_ticked,
        finished: true,
        seen: vec![],
        held: false,
    }
}

/// Save `line` under `writer` (as the webview sends the writer its list was read under); the
/// door's word.
pub(super) async fn save_as(
    state: &AppState,
    chart: Uuid,
    line: LineArg,
    writer: Option<&str>,
) -> String {
    let patient = chart.to_string();
    med_list_save_impl(
        state,
        &patient,
        std::slice::from_ref(&patient),
        line,
        writer,
    )
    .await
    .unwrap()
    .answer
}

/// Save one finished line for whoever is named; its id.
pub(super) async fn save(
    state: &AppState,
    chart: Uuid,
    typed: (&str, &str),
    parts: Vec<PartArg>,
    hand_ticked: bool,
) -> Uuid {
    let line = arg(typed, parts, hand_ticked);
    let id = line.line_id.parse().unwrap();
    let writer = claim_writer(state).await.0;
    assert_eq!(
        save_as(state, chart, line, writer.as_deref()).await,
        "saved"
    );
    id
}

pub(super) async fn read(state: &AppState, chart: Uuid) -> MedListView {
    let patient = chart.to_string();
    med_list_read_impl(state, &patient, Some(std::slice::from_ref(&patient)))
        .await
        .unwrap()
}

/// The ids the pane shows ticked, as the webview sends them to Sign.
pub(super) fn ticked(view: &MedListView) -> Vec<String> {
    view.lines
        .iter()
        .filter(|l| l.ticked)
        .map(|l| l.line_id.clone())
        .collect()
}

pub(super) async fn sign(
    state: &AppState,
    chart: Uuid,
    ticked: &[String],
    credential: Option<Credential>,
) -> Result<MedListSigned, ErrorView> {
    let patient = chart.to_string();
    med_list_sign_impl(state, &patient, vec![patient.clone()], ticked, credential).await
}

/// How many medication assertions the chart holds (read as the owner).
pub(super) async fn assertions(owner: &Client, chart: Uuid) -> i64 {
    owner
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication.asserted' \
               AND patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0)
}

/// How many times `staff`'s key has been released (db/061's `success_mark`: the attempt number of
/// their last success, one step per release with the right passphrase).
pub(super) async fn releases(owner: &Client, staff: &str) -> i64 {
    owner
        .query_one(
            "SELECT success_mark FROM vault_attempt WHERE staff_id = $1",
            &[&staff],
        )
        .await
        .unwrap()
        .get(0)
}

/// The draft rows left on the chart, by id (read as the owner, whoever is named).
pub(super) async fn left(owner: &Client, chart: Uuid) -> Vec<Uuid> {
    owner
        .query(
            "SELECT line_id::text FROM medication_draft_line WHERE patient_id = $1::text::uuid \
              ORDER BY seq",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<_, String>(0).parse().unwrap())
        .collect()
}

/// A drug allergy to amoxicillin coded to `moiety`, signed by Bea.
pub(super) async fn amoxicillin_allergy(state: &AppState, chart: Uuid, moiety: u128) {
    let patient = chart.to_string();
    let entry = EntryInput {
        allergen_kind: Some("drug".into()),
        term: "amoxicillin".into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: None,
        source: Some("patient-stated".into()),
        corrects: None,
    };
    let coding = pick("amoxicillin", moiety).remove(0).coding;
    allergy_record_impl(
        state,
        &patient,
        vec![patient.clone()],
        entry,
        coding,
        credential("bea"),
    )
    .await
    .unwrap();
}

pub(super) fn done(
    signed: MedListSigned,
) -> (String, usize, usize, Vec<(String, String)>, Vec<String>) {
    match signed {
        MedListSigned::Done {
            outcome,
            charted,
            of,
            not_charted,
            unknown,
            ..
        } => (outcome, charted, of, not_charted, unknown),
        other => panic!("expected Done, got {other:?}"),
    }
}

/// Two complete coded lines on a chart whose allergies are known take the default tick, a third
/// is ticked by hand, and ONE Sign charts all three: the vault is asked once, each line leaves the
/// list, and the outcome counts them.
#[tokio::test]
async fn one_sign_charts_every_ticked_line_with_one_key_release() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let patient = chart.to_string();
    allergy_none_impl(
        &state,
        &patient,
        vec![patient.clone()],
        "all",
        credential("bea"),
    )
    .await
    .unwrap();
    let complete = ("amoxicillin", "500mg cap tds");
    save(
        &state,
        chart,
        complete,
        pick("amoxicillin", 0xA40C_7001),
        false,
    )
    .await;
    save(
        &state,
        chart,
        ("metformin", "500mg tab bd"),
        pick("metformin", 0x3E7F_7001),
        false,
    )
    .await;
    save(&state, chart, ("paracetamol", "1g prn"), vec![], true).await;
    let view = read(&state, chart).await;
    let defaults: Vec<bool> = view.lines.iter().map(|l| l.default_ticked).collect();
    assert!(!defaults[2], "an uncoded line is ticked by hand only");
    assert!(
        defaults[0],
        "complete, coded, checked: {:?} {:?}",
        view.lines[0].read, view.lines[0].check
    );
    assert_eq!(ticked(&view).len(), 2 + defaults[1] as usize);

    let before = releases(&owner, "bea").await;
    let ids = ticked(&view);
    let (outcome, charted, of, not_charted, unknown) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!((charted, of), (ids.len(), ids.len()), "{outcome}");
    assert!(not_charted.is_empty() && unknown.is_empty(), "{outcome}");
    assert_eq!(outcome, format!("Charted {} of {}", ids.len(), ids.len()));
    assert_eq!(releases(&owner, "bea").await, before + 1, "one key release");
    assert_eq!(assertions(&owner, chart).await, ids.len() as i64);
    assert_eq!(left(&owner, chart).await.len(), 3 - ids.len());
}

/// Both lines hand-ticked, then an amoxicillin allergy recorded: Sign refuses, signs nothing,
/// asks no key, and unticks only the amoxicillin line. Returns its id and the other's.
async fn an_allergy_after_the_tick(state: &AppState, owner: &Client, chart: Uuid) -> (Uuid, Uuid) {
    let amox = save(
        state,
        chart,
        ("amox", "500mg tds"),
        pick("amoxicillin", 0xA40C_7002),
        true,
    )
    .await;
    let met = save(state, chart, ("metformin", "500mg bd"), vec![], true).await;
    amoxicillin_allergy(state, chart, 0xA40C_7002).await;
    // M1: the pane already shows the tick gone, before any Sign.
    let shown = read(state, chart).await;
    let amox_row = shown
        .lines
        .iter()
        .find(|l| l.line_id == amox.to_string())
        .unwrap();
    assert!(
        amox_row.hand_ticked && !amox_row.ticked,
        "a tick that no longer holds is not shown"
    );
    let before = releases(owner, "bea").await;
    let ids = [amox.to_string(), met.to_string()];
    match sign(state, chart, &ids, credential("bea")).await.unwrap() {
        MedListSigned::Changed { lines, message } => {
            assert_eq!(lines, [amox.to_string()]);
            assert_eq!(
                message,
                "1 ticked line(s) changed since they were ticked: nothing was signed. Look at them again"
            );
        }
        other => panic!("expected Changed, got {other:?}"),
    }
    assert_eq!(
        releases(owner, "bea").await,
        before,
        "the vault was not asked"
    );
    assert_eq!(assertions(owner, chart).await, 0, "nothing was signed");
    let view = read(state, chart).await;
    let by = |id: Uuid| {
        view.lines
            .iter()
            .find(|l| l.line_id == id.to_string())
            .unwrap()
    };
    assert!(
        !by(amox).hand_ticked && !by(amox).ticked,
        "the warned line lost its tick"
    );
    assert!(by(amox).check.reason_needed, "and shows its warning");
    assert!(by(met).hand_ticked, "the other line keeps its tick");
    (amox, met)
}

#[tokio::test]
async fn an_allergy_entered_after_the_tick_refuses_and_unticks_only_that_line() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    an_allergy_after_the_tick(&state, &owner, chart).await;
}

#[tokio::test]
async fn the_second_press_signs_the_rest() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (amox, met) = an_allergy_after_the_tick(&state, &owner, chart).await;
    let ids = ticked(&read(&state, chart).await);
    assert_eq!(ids, [met.to_string()]);
    let (outcome, charted, of, ..) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!((charted, of), (1, 1), "{outcome}");
    assert_eq!(
        left(&owner, chart).await,
        [amox],
        "the warned line stays, unsigned"
    );
}

/// ADR-0060 decision 1: a hand-ticked line with no name is skipped with slice 1a's words; the
/// other ticked line signs, and the skipped one stays in the list.
#[tokio::test]
async fn a_blank_named_line_is_skipped_and_the_rest_sign() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let blank = save(&state, chart, ("  ", "500mg tds"), vec![], true).await;
    let met = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    let ids = [blank.to_string(), met.to_string()];
    let (outcome, charted, of, not_charted, _) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!((charted, of), (1, 2));
    assert_eq!(not_charted, [(blank.to_string(), NAME_IT.to_string())]);
    assert_eq!(
        outcome,
        "Charted 1 of 2\n   500mg tds — not charted: Name the medication."
    );
    assert_eq!(left(&owner, chart).await, [blank]);
    let _ = met;
}

/// Review Focus 3: Bea's list, but the key held is Sue's. A Sign with no staff id typed releases
/// nothing new and writes nothing: the key is not the writer's. A typed id for Sue is refused
/// before the vault is asked.
#[tokio::test]
async fn a_held_key_of_another_member_writes_nothing() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let met = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    state
        .signer_for_act(credential("sue"), Now::read())
        .await
        .expect("Sue's key is held");
    let ids = [met.to_string()];
    let not_yours = "This list was written by Dr Bea; only they can sign it";
    assert_eq!(
        sign(&state, chart, &ids, None).await.unwrap_err().text,
        not_yours
    );
    let before = releases(&owner, "sue").await;
    assert_eq!(
        sign(&state, chart, &ids, credential("sue"))
            .await
            .unwrap_err()
            .text,
        not_yours
    );
    assert_eq!(
        releases(&owner, "sue").await,
        before,
        "refused before the key"
    );
    assert_eq!(assertions(&owner, chart).await, 0);
    assert_eq!(left(&owner, chart).await, [met]);
}

/// Review Focus 5: allergies never asked. No line is default-ticked, however complete; a hand
/// tick signs; and the drug-allergy status is still "not yet asked".
#[tokio::test]
async fn not_yet_asked_hand_ticks_sign() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let complete = ("amoxicillin", "500mg cap tds");
    save(
        &state,
        chart,
        complete,
        pick("amoxicillin", 0xA40C_7003),
        false,
    )
    .await;
    let view = read(&state, chart).await;
    assert!(!view.lines[0].default_ticked && !view.lines[0].ticked);
    let line = save(
        &state,
        chart,
        complete,
        pick("amoxicillin", 0xA40C_7004),
        true,
    )
    .await;
    let ids = [line.to_string()];
    let (outcome, charted, ..) = done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!(charted, 1, "{outcome}");
    let allergies = cairn_node::allergy::read::list_patient_allergies(&owner, chart)
        .await
        .unwrap();
    let status = build_view(&allergies, chrono::FixedOffset::east_opt(0).unwrap()).drug;
    assert_eq!(status, ListStatus::NotYetAsked);
}

/// Review Focus 3: who's here switches from Bea to Sue. Bea's lines leave the screen, and a Sign
/// by Sue naming them is stale; back to Bea, they are there, untouched.
#[tokio::test]
async fn a_who_is_here_switch_hides_the_list() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let met = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    here(&state, "sue", "Dr Sue").await;
    let view = read(&state, chart).await;
    assert!(view.lines.is_empty());
    assert_eq!(view.writer_name.as_deref(), Some("Dr Sue"));
    let ids = [met.to_string()];
    let err = sign(&state, chart, &ids, credential("sue"))
        .await
        .unwrap_err();
    assert_eq!(err.text, MED_LIST_STALE);
    assert_eq!(assertions(&owner, chart).await, 0);
    here(&state, "bea", "Dr Bea").await;
    let view = read(&state, chart).await;
    assert_eq!(view.lines[0].line_id, met.to_string());
    assert!(view.lines[0].hand_ticked);
}
