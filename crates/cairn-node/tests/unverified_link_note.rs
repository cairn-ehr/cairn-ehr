//! R7 (#743 part 2): an un-attested link over a pair this node cannot verify.
//!
//! db/016 has two veto severities. `hard_veto` is a trustworthy clash, and it makes a standing
//! un-attested link a db/054 *doubt* (R1b's sign-off hold, R6's "in doubt" mark). `degrade_hold`
//! means two identifiers in one system differ while at least one side has no format profile, so
//! this node cannot tell a real mismatch from formatting. db/016 says a `degrade_hold` must NOT
//! demote an existing link, so it never becomes a doubt. Before R7, a peer's un-attested link over
//! such a pair was flagged nowhere: the pair reads as one record, so db/057 drops it from the banner
//! and the tray.
//!
//! db/054's `cairn_chart_set_unverified_links` lists those links, computed from the identifiers as
//! they stand NOW (never from a stored proposal), for a non-holding note in the links list. These
//! tests pin: it lists exactly that case, it is not a doubt (the hold stays false), and a clinician's
//! confirm ends it.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via `db::test_serial_guard`. Keys minted at runtime.
mod common;
use cairn_event::demographics::{
    identifier_assertion_body, render_identifier_twin, IdentifierAssertion,
};
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{link_charts, LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::patient::edges::{
    record_doubts, record_is_doubted, record_unverified_links, UnverifiedLink,
};
use common::{
    apply_remote_raw, cs, link_assertion_event, medication_setup as setup, register_pair,
    submit_signed, EventSpec,
};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "r7-unverified";
const SYSTEM: &str = "local-mrn";

async fn clean(c: &Client) {
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag, patient_identifier")
        .await
        .unwrap();
}

/// One identifier assertion with NO normalized form (a node without this system's format profile):
/// the comparison falls back to the raw value, and a difference there is only a `degrade_hold`.
async fn assert_unprofiled_identifier(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    value: &str,
    wall: i64,
) {
    let a = IdentifierAssertion {
        value,
        system: SYSTEM,
        provenance: "patient-stated",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: "demographic.identifier.asserted",
            schema_version: "demographic.identifier/1",
            payload: identifier_assertion_body(&a),
            plaintext_twin: Some(render_identifier_twin(&a)),
            wall,
        },
    )
    .await
    .expect("a valid identifier assertion is accepted");
}

/// A peer's un-attested link joins the pair while nothing differs; the differing identifiers
/// arrive after. The link is listed then, naming the system — so the read is computed now, not
/// at the link's arrival — and it is still NOT a doubt: the sign-off hold stays false.
#[tokio::test]
async fn an_unattested_link_over_a_soft_veto_is_listed_and_holds_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 40, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    let set = ChartSet::new([a, b]).unwrap();
    assert!(
        record_unverified_links(&c, &set).await.unwrap().is_empty(),
        "positive control: no identifier differs yet"
    );

    assert_unprofiled_identifier(&c, &sk, &kid, a, "00123", 50).await;
    assert_unprofiled_identifier(&c, &sk, &kid, b, "123", 51).await;

    assert_eq!(
        record_unverified_links(&c, &set).await.unwrap(),
        vec![UnverifiedLink {
            low: a.min(b),
            high: a.max(b),
            system: SYSTEM.to_string(),
        }]
    );
    assert!(
        record_doubts(&c, &set).await.unwrap().is_empty(),
        "a soft veto is never a db/054 doubt"
    );
    assert!(
        !record_is_doubted(&c, &set).await.unwrap(),
        "a soft veto never holds sign-off (db/016: it must not demote a link)"
    );
}

/// A clinician's confirm is the decision the note asks for: an attested link carries none.
#[tokio::test]
async fn a_confirmed_link_is_not_listed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    assert_unprofiled_identifier(&c, &sk, &kid, a, "00777", 20).await;
    assert_unprofiled_identifier(&c, &sk, &kid, b, "777", 21).await;
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 40, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    let set = ChartSet::new([a, b]).unwrap();
    assert_eq!(
        record_unverified_links(&c, &set).await.unwrap().len(),
        1,
        "positive control: the un-attested link is listed"
    );
    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    assert!(record_unverified_links(&c, &set).await.unwrap().is_empty());
}

/// A TRUSTWORTHY identifier clash (both sides profiled, normalized forms disjoint) is a hard veto:
/// db/054 already doubts that link (a clash), so it is not an "unverified" one. Pins the severity
/// filter: listing every finding would put a second note on a line already marked "in doubt".
#[tokio::test]
async fn a_trustworthy_identifier_clash_is_a_doubt_not_unverified() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    for (p, value, wall) in [(a, "4411", 20), (b, "5522", 21)] {
        let id = IdentifierAssertion {
            value,
            system: SYSTEM,
            provenance: "patient-stated",
            normalized: Some(value),
            profile: Some("test-profile@stub"),
            use_: None,
        };
        submit_signed(
            &c,
            &sk,
            &kid,
            EventSpec {
                patient: p,
                event_type: "demographic.identifier.asserted",
                schema_version: "demographic.identifier/1",
                payload: identifier_assertion_body(&id),
                plaintext_twin: Some(render_identifier_twin(&id)),
                wall,
            },
        )
        .await
        .unwrap();
    }
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 40, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    let set = ChartSet::new([a, b]).unwrap();
    assert!(
        record_is_doubted(&c, &set).await.unwrap(),
        "positive control: a trustworthy clash is a doubt"
    );
    assert!(record_unverified_links(&c, &set).await.unwrap().is_empty());
}

/// The window reads the list as `cairn_agent`, as it reads db/054's other two functions — over
/// a seeded finding, so the read proves the rows reach the role, not only that the call is
/// granted (random charts would return nothing either way).
#[tokio::test]
async fn the_agent_role_reads_the_finding() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = unverified_pair(&c, &sk, &kid).await;
    let set = ChartSet::new([a, b]).unwrap();
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let read = record_unverified_links(&c, &set).await;
    c.batch_execute("RESET ROLE").await.unwrap();
    assert_eq!(
        read.expect("cairn_agent may read the unverified links")
            .len(),
        1
    );
}

/// Registers two charts, joins them by a peer's UN-attested link, then gives them differing
/// identifiers in [`SYSTEM`] with no normalized form: one `degrade_hold`, one listed row.
async fn unverified_pair(c: &Client, sk: &cairn_event::SigningKey, kid: &str) -> (Uuid, Uuid) {
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(c, sk, kid, a, b).await;
    let ev = link_assertion_event(kid, a, b, LinkVerb::Link, 40, 0, "peer-matcher", false);
    apply_remote_raw(c, sk, ev).await.unwrap();
    assert_unprofiled_identifier(c, sk, kid, a, "00123", 50).await;
    assert_unprofiled_identifier(c, sk, kid, b, "123", 51).await;
    (a, b)
}

/// Only links with BOTH ends in the asked-for set are listed. A soft-vetoed link elsewhere, or
/// one with a single end in the set, must not appear: the window drops a row naming no standing
/// line of the record, so a widened filter (AND to OR) would be invisible on screen — this
/// guards the floor itself.
#[tokio::test]
async fn a_link_with_an_end_outside_the_set_is_not_listed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = unverified_pair(&c, &sk, &kid).await;
    let (d, _e) = unverified_pair(&c, &sk, &kid).await;
    let both = record_unverified_links(&c, &ChartSet::new([a, b]).unwrap())
        .await
        .unwrap();
    assert_eq!(both.len(), 1, "positive control: {both:?}");
    assert_eq!((both[0].low, both[0].high), (a.min(b), a.max(b)));
    let straddling = ChartSet::new([a, d]).unwrap();
    assert!(
        record_unverified_links(&c, &straddling)
            .await
            .unwrap()
            .is_empty(),
        "neither a-b nor d-e has both ends in {{a, d}}"
    );
}

/// A link no longer standing — a later peer unlink won — is not listed: there is no line left
/// to carry the note.
#[tokio::test]
async fn an_unlinked_pair_is_not_listed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = unverified_pair(&c, &sk, &kid).await;
    let set = ChartSet::new([a, b]).unwrap();
    assert_eq!(
        record_unverified_links(&c, &set).await.unwrap().len(),
        1,
        "positive control: listed while linked"
    );
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Unlink, 60, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    assert!(record_unverified_links(&c, &set).await.unwrap().is_empty());
}
