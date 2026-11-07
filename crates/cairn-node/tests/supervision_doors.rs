//! DB-gated: sign-in slice 3c-3a's supervision doors (db/072) — who may link a supervisor, end a
//! link, require or lift co-signing, and grant the "assigns supervision" right (design
//! 2026-11-06-sign-in-slice-3c-3 §3.1; the plan's Global Constraints matrix).
//!
//! The trust boundary under test: `cairn_supervision_act` is reachable by `cairn_agent`, so by any
//! raw-SQL client holding it. Every act must carry a SIGNED approval over its own address, by a
//! member whose key is current (and, unless a superuser, confirmed), and must pass the matrix:
//! one test per matrix row below, each allowed approver succeeding and each refusal firing with
//! its own words. Nobody acts on their own supervision as the supervisee (Review Focus 4).
//!
//! The fixtures (keys minted at runtime, enrolment through the real doors) are
//! `common/vault_enroller_kit.rs`.
mod common;
#[path = "common/vault_enroller_kit.rs"]
#[macro_use]
mod kit;

use kit::*;

// The six ops (db/004), by the names the door takes.
const LINK: &str = "link-supervisor";
const UNLINK: &str = "unlink-supervisor";
const REQUIRE: &str = "require-cosign";
const LIFT: &str = "lift-cosign";
const GRANT: &str = "grant-assigner";
const WITHDRAW: &str = "withdraw-assigner";
use tokio_postgres::Client;

/// boss (superuser); clinicians jsmith, bwong, plain and asg; a student, stu; an admin, adm —
/// all enrolled (and so confirmed) by boss. asg holds the assigns-supervision right.
async fn team(c: &Client) {
    boot(c).await;
    for (s, role, v) in [
        ("jsmith", "clinician", JSMITH),
        ("bwong", "clinician", BWONG),
        ("plain", "clinician", PLAIN),
        ("asg", "clinician", ASSIGNER),
        ("stu", "student", STUDENT),
        ("adm", "admin", ADMIN),
    ] {
        enrol(c, s, role, v, "boss", BOSS).await.unwrap();
    }
    acted(c, GRANT, "asg", None, "boss", BOSS).await;
}

/// (op, subject actor hex, related actor hex) for every supervision row, in order.
async fn record(c: &Client) -> Vec<(String, String, Option<String>)> {
    c.query(
        "SELECT op, actor_id, related_actor FROM cairn_supervision_record()",
        &[],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}

async fn hex_actor(c: &Client, staff: &str) -> String {
    hex::encode(actor_of(c, staff).await)
}

#[tokio::test]
async fn link_supervisor_by_a_superuser_or_an_assigner_with_every_refusal() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let link = |s: &'static str, p: &'static str, a: &'static str, by: u8| {
        let c = &c;
        async move { supervise(c, LINK, s, Some(p), a, by).await }
    };
    link("bwong", "jsmith", "boss", BOSS).await.unwrap();
    link("stu", "jsmith", "asg", ASSIGNER).await.unwrap();
    // An assigner may name themselves as someone ELSE's supervisor.
    link("bwong", "asg", "asg", ASSIGNER).await.unwrap();
    // An admin may hold the right and act with it.
    acted(&c, GRANT, "adm", None, "boss", BOSS).await;
    link("plain", "jsmith", "adm", ADMIN).await.unwrap();

    refused(
        link("plain", "bwong", "jsmith", JSMITH).await,
        "only a superuser or a holder of the assigns-supervision right",
    );
    refused(
        link("bwong", "jsmith", "boss", BOSS).await,
        "already supervises",
    );
    refused(
        link("bwong", "bwong", "boss", BOSS).await,
        "nobody is their own supervisor",
    );
    for (s, p) in [("bwong", "boss"), ("bwong", "adm"), ("adm", "jsmith")] {
        refused(
            link(s, p, "boss", BOSS).await,
            "a superuser or an admin is never a party to supervision",
        );
    }
    refused(
        link("bwong", "stu", "boss", BOSS).await,
        "a student cannot supervise",
    );
    refused(
        link("asg", "jsmith", "asg", ASSIGNER).await,
        "nobody acts on their own supervision",
    );
    refused(
        link("bwong", "nobody", "boss", BOSS).await,
        "the supervisor nobody is not an enrolled member",
    );
    refused(
        supervise(&c, LINK, "bwong", None, "boss", BOSS).await,
        "needs the supervisor's staff ID",
    );
}

#[tokio::test]
async fn unlink_supervisor_by_a_superuser_an_assigner_or_the_supervisor() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let act = |op: &'static str, s: &'static str, p: &'static str, a: &'static str, by: u8| {
        let c = &c;
        async move { supervise(c, op, s, Some(p), a, by).await }
    };
    acted(&c, LINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    refused(
        act(UNLINK, "bwong", "jsmith", "plain", PLAIN).await,
        "only a superuser, an assigner or the supervisor themself may end this link",
    );
    acted(&c, UNLINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    refused(
        act(UNLINK, "bwong", "jsmith", "boss", BOSS).await,
        "no link stands between bwong and supervisor jsmith",
    );
    acted(&c, LINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    acted(&c, UNLINK, "bwong", Some("jsmith"), "asg", ASSIGNER).await;
    // The supervisee may not end it, as the next test pins with the assigner right in hand.
    acted(&c, LINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    refused(
        act(UNLINK, "bwong", "jsmith", "bwong", BWONG).await,
        "nobody acts on their own supervision",
    );
}

#[tokio::test]
async fn require_cosign_by_a_superuser_or_an_assigner_with_every_refusal() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let req = |s: &'static str, a: &'static str, by: u8| {
        let c = &c;
        async move { supervise(c, REQUIRE, s, None, a, by).await }
    };
    req("bwong", "boss", BOSS).await.unwrap();
    req("jsmith", "asg", ASSIGNER).await.unwrap();
    refused(
        req("bwong", "boss", BOSS).await,
        "bwong already needs co-signing",
    );
    refused(
        req("stu", "boss", BOSS).await,
        "a student needs co-signing for as long as they are a student",
    );
    for s in ["adm", "boss"] {
        refused(
            req(s, "asg", ASSIGNER).await,
            "a superuser or an admin is never a party to supervision",
        );
    }
    refused(
        req("plain", "jsmith", JSMITH).await,
        "only a superuser or a holder of the assigns-supervision right",
    );
    refused(
        req("asg", "asg", ASSIGNER).await,
        "nobody acts on their own supervision",
    );
    refused(
        supervise(&c, REQUIRE, "plain", Some("jsmith"), "boss", BOSS).await,
        "require-cosign takes no supervisor",
    );
    refused(
        req("nobody", "boss", BOSS).await,
        "nobody is not an enrolled member",
    );
}

#[tokio::test]
async fn lift_cosign_by_a_superuser_an_assigner_or_a_current_supervisor() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let act = |op: &'static str, s: &'static str, a: &'static str, by: u8| {
        let c = &c;
        async move { supervise(c, op, s, None, a, by).await }
    };
    refused(
        act(LIFT, "bwong", "boss", BOSS).await,
        "bwong does not need co-signing",
    );
    refused(
        act(LIFT, "stu", "boss", BOSS).await,
        "a student needs co-signing for as long as they are a student",
    );
    act(REQUIRE, "bwong", "boss", BOSS).await.unwrap();
    act(LIFT, "bwong", "boss", BOSS).await.unwrap();
    act(REQUIRE, "bwong", "boss", BOSS).await.unwrap();
    act(LIFT, "bwong", "asg", ASSIGNER).await.unwrap();
    act(REQUIRE, "bwong", "boss", BOSS).await.unwrap();
    // jsmith is not (yet) bwong's supervisor.
    refused(
        act(LIFT, "bwong", "jsmith", JSMITH).await,
        "only a superuser, an assigner or a current supervisor of bwong may lift",
    );
    refused(
        act(LIFT, "bwong", "bwong", BWONG).await,
        "nobody acts on their own supervision",
    );
}

#[tokio::test]
async fn grant_and_withdraw_assigner_by_a_superuser_only() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let act = |op: &'static str, s: &'static str, a: &'static str, by: u8| {
        let c = &c;
        async move { supervise(c, op, s, None, a, by).await }
    };
    refused(
        act(GRANT, "jsmith", "asg", ASSIGNER).await,
        "only a superuser may grant or withdraw the assigns-supervision right",
    );
    refused(
        act(GRANT, "boss", "boss", BOSS).await,
        "boss is a superuser, who already holds every assigner power",
    );
    refused(
        act(GRANT, "asg", "boss", BOSS).await,
        "asg already holds the assigns-supervision right",
    );
    act(WITHDRAW, "asg", "boss", BOSS).await.unwrap();
    refused(
        act(WITHDRAW, "asg", "boss", BOSS).await,
        "asg does not hold the assigns-supervision right",
    );
    // Withdrawn, asg is a plain member again.
    refused(
        act(REQUIRE, "bwong", "asg", ASSIGNER).await,
        "only a superuser or a holder of the assigns-supervision right",
    );
    refused(
        supervise(&c, "promote", "asg", None, "boss", BOSS).await,
        "promote is not a supervision act",
    );
}

#[tokio::test]
async fn an_unconfirmed_member_cannot_approve() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    // jsmith, an enroller, enrols "unc": unconfirmed until a superuser confirms it.
    set_enroller(&c, "jsmith", true, "boss", BOSS)
        .await
        .unwrap();
    enrol(&c, "unc", "clinician", JSMITH2, "jsmith", JSMITH)
        .await
        .unwrap();
    acted(&c, GRANT, "unc", None, "boss", BOSS).await;
    refused(
        supervise(&c, REQUIRE, "bwong", None, "unc", JSMITH2).await,
        "unc's key is unconfirmed",
    );
    judge(&c, JSMITH2, true, "boss", BOSS).await.unwrap();
    acted(&c, REQUIRE, "bwong", None, "unc", JSMITH2).await;
}

/// Review Focus 4: a supervisee holding the assigner right cannot unlink their own supervisor,
/// lift their own requirement, or link themselves to themselves — even with a valid token.
#[tokio::test]
async fn a_supervisee_cannot_act_on_their_own_supervision_even_as_assigner() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    acted(&c, LINK, "asg", Some("jsmith"), "boss", BOSS).await;
    acted(&c, REQUIRE, "asg", None, "boss", BOSS).await;
    for (op, p) in [
        (UNLINK, Some("jsmith")),
        (LIFT, None),
        (LINK, Some("asg")),
        (LINK, Some("plain")),
    ] {
        refused(
            supervise(&c, op, "asg", p, "asg", ASSIGNER).await,
            "nobody acts on their own supervision",
        );
    }
}

#[tokio::test]
async fn a_supervisor_may_step_away_and_lift() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    acted(&c, LINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    acted(&c, REQUIRE, "bwong", None, "boss", BOSS).await;
    acted(&c, LIFT, "bwong", None, "jsmith", JSMITH).await;
    supervise(&c, UNLINK, "bwong", Some("jsmith"), "jsmith", JSMITH)
        .await
        .unwrap();
    // Stepped away, jsmith supervises bwong no more.
    acted(&c, REQUIRE, "bwong", None, "boss", BOSS).await;
    refused(
        supervise(&c, LIFT, "bwong", None, "jsmith", JSMITH).await,
        "a current supervisor of bwong",
    );
}

/// A supervisor acts only on THEIR OWN link and THEIR OWN supervisee: a co-supervisor may not end
/// another's link, and a supervisor of someone else may not lift this person's co-signing.
#[tokio::test]
async fn a_supervisor_acts_only_on_their_own_link_and_supervisee() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    acted(&c, LINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    acted(&c, LINK, "bwong", Some("plain"), "boss", BOSS).await;
    refused(
        supervise(&c, UNLINK, "bwong", Some("plain"), "jsmith", JSMITH).await,
        "only a superuser, an assigner or the supervisor themself may end this link",
    );
    acted(&c, LINK, "stu", Some("jsmith"), "boss", BOSS).await;
    acted(&c, REQUIRE, "bwong", None, "boss", BOSS).await;
    acted(&c, UNLINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    // jsmith still supervises stu, but no longer bwong.
    refused(
        supervise(&c, LIFT, "bwong", None, "jsmith", JSMITH).await,
        "a current supervisor of bwong",
    );
}

/// A link to a supervisor whose member row is gone (left, or re-enrolled as a new actor) can still
/// be ended: otherwise only that absent supervisor's co-sign would ever clear the supervisee's
/// notes. The prior and the apply resolve the supervisor the same way, so the token verifies.
#[tokio::test]
async fn a_link_to_a_departed_supervisor_can_still_be_ended() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    acted(&c, LINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    let jsmith = hex_actor(&c, "jsmith").await;
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'jsmith'; \
         DELETE FROM vault_member WHERE staff_id = 'jsmith'",
    )
    .await
    .unwrap();
    acted(&c, UNLINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    let sup: Vec<String> = c
        .query_one(
            "SELECT supervisors FROM cairn_supervision_staff() WHERE staff_id = 'bwong'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(sup.is_empty(), "no supervisor stands: {sup:?}");
    let last = record(&c).await.pop().unwrap();
    assert_eq!(
        last,
        (
            UNLINK.to_string(),
            hex_actor(&c, "bwong").await,
            Some(jsmith)
        )
    );
}

#[tokio::test]
async fn the_record_lists_every_act_in_order() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    acted(&c, LINK, "bwong", Some("jsmith"), "boss", BOSS).await;
    acted(&c, REQUIRE, "bwong", None, "asg", ASSIGNER).await;
    acted(&c, LIFT, "bwong", None, "jsmith", JSMITH).await;
    supervise(&c, UNLINK, "bwong", Some("jsmith"), "boss", BOSS)
        .await
        .unwrap();
    acted(&c, WITHDRAW, "asg", None, "boss", BOSS).await;
    let (asg, bwong, jsmith) = (
        hex_actor(&c, "asg").await,
        hex_actor(&c, "bwong").await,
        hex_actor(&c, "jsmith").await,
    );
    let row =
        |op: &str, a: &str, r: Option<&str>| (op.to_string(), a.to_string(), r.map(String::from));
    assert_eq!(
        record(&c).await,
        [
            row(GRANT, &asg, None),
            row(LINK, &bwong, Some(&jsmith)),
            row(REQUIRE, &bwong, None),
            row(LIFT, &bwong, None),
            row(UNLINK, &bwong, Some(&jsmith)),
            row(WITHDRAW, &asg, None),
        ]
    );
}

#[tokio::test]
async fn the_staff_list_says_who_supervises_whom_now() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    for p in ["jsmith", "asg", "plain"] {
        acted(&c, LINK, "bwong", Some(p), "boss", BOSS).await;
    }
    supervise(&c, UNLINK, "bwong", Some("plain"), "boss", BOSS)
        .await
        .unwrap();
    acted(&c, REQUIRE, "bwong", None, "boss", BOSS).await;
    acted(&c, LINK, "stu", Some("jsmith"), "boss", BOSS).await;
    let rows: Vec<(String, String, bool, bool, Vec<String>)> = c
        .query(
            "SELECT staff_id, role, needs_cosigning, assigner, supervisors FROM cairn_supervision_staff()",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4)))
        .collect();
    let row = |s: &str, role: &str, n: bool, a: bool, sup: &[&str]| {
        (
            s.to_string(),
            role.to_string(),
            n,
            a,
            sup.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
        )
    };
    // Every non-superuser member, by staff ID; a student needs co-signing by role alone.
    assert_eq!(
        rows,
        [
            row("adm", "admin", false, false, &[]),
            row("asg", "clinician", false, true, &[]),
            row("bwong", "clinician", true, false, &["asg", "jsmith"]),
            row("jsmith", "clinician", false, false, &[]),
            row("plain", "clinician", false, false, &[]),
            row("stu", "student", true, false, &["jsmith"]),
        ]
    );
    // A supervisor whose member row is gone (as after a restore) is named by the pinned handle.
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'jsmith'; \
         DELETE FROM vault_member WHERE staff_id = 'jsmith'",
    )
    .await
    .unwrap();
    let sup: Vec<String> = c
        .query_one(
            "SELECT supervisors FROM cairn_supervision_staff() WHERE staff_id = 'stu'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(sup, ["jsmith"]);
}

/// The console door applies the same party and state refusals, under its own name, but asks for
/// no approver: running it IS the authority (it has no grant).
#[tokio::test]
async fn the_console_door_shares_the_matrix_without_an_approver() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    team(&c).await;
    let console = "SELECT cairn_supervision_console_act($1, $2, $3)";
    c.execute(console, &[&LINK, &"bwong", &Some("jsmith")])
        .await
        .unwrap();
    let e = c
        .execute(console, &[&LINK, &"bwong", &Some("jsmith")])
        .await
        .unwrap_err();
    assert!(
        msg(&e).starts_with("cairn_supervision_console_act: ")
            && msg(&e).contains("already supervises"),
        "{}",
        msg(&e)
    );
    let by: String = c
        .query_one(
            "SELECT approved_by FROM actor_event WHERE op = 'link-supervisor'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(by, "console");
}
