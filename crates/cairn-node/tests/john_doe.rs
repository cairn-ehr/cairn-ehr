//! Integration coverage for §5.4 unidentified ("John Doe") registration (slice A):
//! `john_doe::register_john_doe` composes a callsign name assertion + the C4
//! identity-pending marker through the real `submit_event` door, so a chart is created
//! that (a) renders *unconfirmed* on `chart_trust`, and (b) carries the system-generated
//! callsign as a placeholder-use name in `patient_name` / `patient_name_current`. Real
//! Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`.
//! Mirrors `identity_identify.rs` (the C4 slice this composes onto).

use cairn_node::{db, john_doe};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{cs, trust_of};

/// The tables this suite truncates on top of `common::setup`'s clinical core.
///
/// `patient_name` holds the callsign this slice asserts, and `chart_identity_state` (db/024)
/// the pending marker it composes. Both go through `setup`'s `to_regclass` guard, which is a
/// no-op for a table that already exists and keeps the helper correct on a DB migrated only
/// to an earlier stage. Nothing carries a foreign key to `patient_name`, so truncating it
/// alongside the core list rather than inside its `CASCADE` is equivalent.
///
/// `patient_registration` joins the set with #344: the John Doe chart now begins with an
/// `identity.registration.asserted` event (db/045), so this suite's TRUNCATE must reach that
/// projection too or a row left behind by an earlier test would make
/// `patient_registration_current` ambiguous for a reused patient_id (it never is reused in
/// practice — UUIDv7 — but the truncate-everything-this-suite-touches discipline is what
/// keeps every test here independent of run order).
const OVERLAY_TABLES: [&str; 3] = [
    "chart_identity_state",
    "patient_name",
    "patient_registration",
];

/// The standing identity state of a chart (db/024 overlay), or None if no row exists.
async fn identity_state(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// The (use_key, value) of every retained name on a chart.
async fn names_of(c: &Client, subject: Uuid) -> Vec<(String, String)> {
    let s_s = subject.to_string();
    c.query(
        "SELECT use_key, value FROM patient_name WHERE patient_id = $1::text::uuid ORDER BY value",
        &[&s_s],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

/// The display-winner name for a chart (patient_name_current), or None.
async fn current_name(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT value FROM patient_name_current WHERE patient_id = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

// --- registration behaviour ---

#[tokio::test]
async fn register_john_doe_creates_an_unconfirmed_chart() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;

    let (pid, _call, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .expect("registration accepted by the floor");

    // §5.4: identity-pending is an active workflow state → chart renders *unconfirmed*.
    assert_eq!(identity_state(&c, pid).await.as_deref(), Some("pending"));
    assert_eq!(trust_of(&c, pid).await.as_deref(), Some("unconfirmed"));
}

#[tokio::test]
async fn a_blank_basis_is_refused_before_anything_is_minted_or_ticked() {
    // Final review (minor). `register-john-doe --basis ""` used to hard-fail at the db/045
    // floor only AFTER minting a patient UUID and ticking three HLCs. The floor stays the
    // enforcement point (principle 12) — this pins the CHEAP refusal in front of it, the
    // same discipline `patient-register` already applies to `--birth-date`.
    //
    // Zero side effects is the assertion that matters, and it is checked against
    // `event_log` (the wire) rather than any projection: an empty log is only meaningful
    // because `common::setup` truncates it, so a partial write could not hide behind a
    // projection that happened to reject the row.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;

    for blank in ["", "   "] {
        let err =
            john_doe::register_john_doe(&mut c, &sk, &kid, "n", "ED", "site1", "2026-07-03", blank)
                .await
                .expect_err("a non-standard registration states why (§5.3/§5.4)");
        assert!(
            err.to_string().contains("--basis"),
            "the refusal must name the flag the operator got wrong, not surface a raw \
             Postgres exception: {err}"
        );
    }

    let total: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        total, 0,
        "a refused basis must mint NOTHING — not the registration, not the callsign, not \
         the pending marker"
    );
}

#[tokio::test]
async fn callsign_is_stored_as_a_placeholder_use_name_and_is_the_display_winner() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;

    let (pid, call, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .unwrap();

    // The callsign lives in patient_name under the reserved 'callsign' use — that use is
    // what the advisory matcher excludes on, and it is what marks the name a placeholder.
    let names = names_of(&c, pid).await;
    assert_eq!(names, vec![("callsign".to_string(), call.clone())]);
    // With no legal name, db/012's unidentified-patient fallback makes the callsign the
    // display winner — the chart header shows the obvious placeholder, never a fake name.
    assert_eq!(current_name(&c, pid).await.as_deref(), Some(call.as_str()));
    assert!(
        call.starts_with("Unknown-"),
        "the header is an obvious placeholder: {call}"
    );
}

#[tokio::test]
async fn two_john_does_coexist_as_distinct_pending_charts_with_distinct_callsigns() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;

    // Same site, same day — the partition-safe suffix must still keep them apart.
    let (p1, c1, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious, no ID",
    )
    .await
    .unwrap();
    let (p2, c2, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unresponsive trauma, no ID",
    )
    .await
    .unwrap();

    assert_ne!(p1, p2, "distinct UUIDs");
    assert_ne!(
        c1, c2,
        "distinct callsigns even at same site/day (suffix disambiguates)"
    );
    assert_eq!(trust_of(&c, p1).await.as_deref(), Some("unconfirmed"));
    assert_eq!(trust_of(&c, p2).await.as_deref(), Some("unconfirmed"));
}

// --- finisher 1: node-local friendly ordinal ---

/// Registration returns a per-node_origin ordinal (1, 2, …) and a foreign node_origin's
/// registrations form their OWN partition, never shifting this node's numbers. Proves the
/// VIEW is node-scoped without any `local_node` dependency, and that only callsign
/// registrations count (the count equals the number of John Does, not their events).
#[tokio::test]
async fn ordinal_numbers_registrations_per_node_origin() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;

    // Two John Does first-recorded on node "n" → ordinals 1 then 2, in registration order.
    let (_p1, _c1, o1) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "n", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    let (p2, _c2, o2) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "n", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    assert_eq!(o1, 1);
    assert_eq!(o2, 2);

    // A registration first-recorded on a DIFFERENT node_origin starts its own sequence at
    // 1 and does not shift node "n"'s ordinals.
    let (_p3, _c3, o3) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "m", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    assert_eq!(o3, 1, "a different node_origin is a separate partition");

    // node "n"'s second John Doe still reads ordinal 2 via the VIEW.
    let n2: i64 = c
        .query_one(
            "SELECT ordinal FROM john_doe_local_ordinal WHERE patient_id = $1::text::uuid",
            &[&p2.to_string()],
        )
        .await
        .unwrap()
        .get("ordinal");
    assert_eq!(n2, 2);

    // Only callsign registrations are counted (each register authors ONE callsign name +
    // one pending marker; the pending marker is not a name → excluded). Three John Does
    // total across both partitions.
    let total: i64 = c
        .query_one("SELECT count(*) FROM john_doe_local_ordinal", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        total, 3,
        "only callsign name registrations appear in the VIEW"
    );
}

// --- #344: the chart's birth act is a registration, same as every other class ---

#[tokio::test]
async fn a_john_doe_chart_begins_with_an_unidentified_registration() {
    // §5.3's three classes finally recorded. The registration is the chart's FIRST event
    // (lowest HLC of the three), which is what lets #345's precedence rule land later with
    // no carve-out for John Doe.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;

    let (pid, _call, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .unwrap();

    let pid_s = pid.to_string();

    // patient_registration_current.class == 'unidentified'.
    let reg = c
        .query_one(
            "SELECT class, registered_hlc_wall, registered_hlc_count \
             FROM patient_registration_current WHERE patient_id = $1::text::uuid",
            &[&pid_s],
        )
        .await
        .expect("a registration row must exist for every John Doe chart");
    let class: String = reg.get(0);
    let reg_wall: i64 = reg.get(1);
    let reg_count: i32 = reg.get(2);
    assert_eq!(class, "unidentified");

    // The registration's HLC must strictly precede the callsign name event's — the
    // registration is authored FIRST inside the transaction, at a strictly earlier tick.
    let name = c
        .query_one(
            "SELECT hlc_wall, hlc_counter FROM event_log \
             WHERE patient_id::text = $1 AND event_type = 'demographic.field.asserted'",
            &[&pid_s],
        )
        .await
        .expect("the callsign name event must exist");
    let name_wall: i64 = name.get(0);
    let name_count: i32 = name.get(1);

    assert!(
        (reg_wall, reg_count) < (name_wall, name_count),
        "the registration's HLC ({reg_wall}, {reg_count}) must strictly precede the \
         callsign name's ({name_wall}, {name_count}) — the registration is the chart's \
         FIRST event, not an afterthought bolted on beside the other two"
    );
}

#[tokio::test]
async fn the_unidentified_registration_carries_no_search_attestation() {
    // Structural absence, not empty: there is nothing to search an unconscious patient
    // with, and claiming otherwise would be a precise untruth (principle 4). The db/045
    // floor refuses a non-standard registration carrying a `search` key at all — this test
    // pins that this codepath never tries to hand it one.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;

    let (pid, _call, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .unwrap();

    let pid_s = pid.to_string();

    // `body ? 'search'` is jsonb KEY-PRESENCE, not a null check — an explicit
    // `"search": null` would still trip it. That is the same distinction db/045's own
    // check function draws (§5.4), and it is what a merely-`Option`-shaped absence in Rust
    // could get wrong without this test catching it: `None` must serialize to no key at
    // all, not to a present `null`.
    let has_search: bool = c
        .query_one(
            "SELECT body ? 'search' FROM event_log \
             WHERE patient_id::text = $1 AND event_type = 'identity.registration.asserted'",
            &[&pid_s],
        )
        .await
        .expect("the registration event must exist")
        .get(0);
    assert!(
        !has_search,
        "a John Doe registration must carry NO 'search' key — there is nothing to search \
         an unconscious patient with"
    );

    // And at the projection layer: `search_incomplete` is the honest NULL that can only
    // mean "no search ran" for a non-standard class (patient_registration.rs pins the same
    // invariant for the floor directly; this pins that register_john_doe's real output
    // lands the same way).
    let incomplete: Option<bool> = c
        .query_one(
            "SELECT search_incomplete FROM patient_registration_current \
             WHERE patient_id = $1::text::uuid",
            &[&pid_s],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        incomplete, None,
        "search_incomplete must be NULL — no search ran, so there is no completeness to state"
    );
}
