//! ADR-0081 (#803) — a hold "from now" starts at the node's PHYSICAL now, never its HLC wall.
//!
//! The failure this pins: db/020 lets a peer whose clock runs fast push this node's
//! `hlc_state` up to 24 h ahead, and `node_hlc_tick` keeps issuing that wall. If a withhold
//! without `from` (or a release) were stamped with the HLC wall, a nurse's "W until reviewed"
//! recorded after syncing with a peer 2 h fast would START 2 h in the future — the fold would
//! call it pending and the drug would read as givable for 2 h. A 1 h timed hold would be
//! refused outright (over before it begins), and a release would seem to do nothing.
//!
//! So the orchestrators stamp `t_effective` with the physical clock, and db/059 resolves a
//! hold's start / a release's instant from it. Each test here ratchets the shared clock ~2 h
//! ahead, writes through the real orchestrator, and asks the fold at the physical now.
//!
//! `hlc_state` is SHARED by every suite on this database and survives TRUNCATE, so the ratchet
//! is undone by a Drop guard ([`ClockRestore`]) that runs even when an assertion panics.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::medication::HoldUntil;
use cairn_node::db;
use cairn_node::medication::{
    release_medication_holds, withhold_medication, WithholdMedicationInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// How far ahead of the physical clock the "fast peer" leaves this node's HLC.
const SKEW_MS: i64 = 2 * 3_600_000;

/// Puts the shared node clock back (`hlc_wall = 0`, PR #285's rule, as `chart_link.rs` does)
/// when dropped — including while unwinding from a failed assertion, which a plain call at
/// the end of the test would skip. `Drop` cannot `.await` and we are already inside the
/// test's runtime, so the reset runs on a fresh thread with its own small runtime, joined
/// before the drop returns. It never panics (a panic during unwinding would abort): a failed
/// reset is printed instead, so the operator still sees it.
struct ClockRestore(String);

impl Drop for ClockRestore {
    fn drop(&mut self) {
        let base = self.0.clone();
        let outcome = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("runtime: {e}"))?;
            rt.block_on(async {
                let c = db::connect(&base).await.map_err(|e| format!("{e:#}"))?;
                c.execute(
                    "UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0 WHERE id",
                    &[],
                )
                .await
                .map_err(|e| db_msg(&e))?;
                Ok::<(), String>(())
            })
        })
        .join();
        if !matches!(outcome, Ok(Ok(()))) {
            eprintln!("WARNING: could not reset the shared hlc_state: {outcome:?}");
        }
    }
}

/// Push this node's HLC `SKEW_MS` ahead of its physical clock — what syncing with a fast peer
/// does — and return the guard that undoes it.
async fn ratchet_clock_ahead(c: &Client, base: &str) -> ClockRestore {
    let guard = ClockRestore(base.to_string());
    c.execute(
        "UPDATE hlc_state SET hlc_wall = (extract(epoch FROM clock_timestamp()) * 1000)::bigint + $1, \
                hlc_counter = 0 WHERE id",
        &[&SKEW_MS],
    )
    .await
    .unwrap();
    guard
}

/// The fold for `med`'s line at the PHYSICAL now: (withhold event id, pending) per hold.
async fn holds_now(c: &Client, med: Uuid) -> Vec<(String, bool)> {
    c.query(
        "SELECT h.withhold_event_id::text, h.pending \
         FROM medication_thread_group g, \
              cairn_medication_group_holds(ARRAY[g.group_id], clock_timestamp()) h \
         WHERE g.medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

/// None of `events` tripped the bitemporal ceiling (db/005 step 1b'): the physical instant is
/// read before the HLC tick, so it is never after the event's own wall.
async fn assert_never_flagged(c: &Client, events: &[Uuid]) {
    let ids: Vec<String> = events.iter().map(Uuid::to_string).collect();
    let flagged: i64 = c
        .query_one(
            "SELECT count(*) FROM t_effective_ceiling_flag f \
             JOIN event_log e ON e.content_address = f.content_address \
             WHERE e.event_id = ANY ($1::text[]::uuid[])",
            &[&ids],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flagged, 0,
        "a physical-now t_effective is never a ceiling anomaly"
    );
}

/// The physical clock plus `minutes`, as an RFC 3339 instant with an explicit offset.
async fn physical_plus(c: &Client, minutes: i32) -> String {
    c.query_one(
        "SELECT to_char((clock_timestamp() + make_interval(mins => $1)) AT TIME ZONE 'UTC', \
                        'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
        &[&minutes],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_hold_from_now_is_in_force_now_even_when_the_hlc_runs_ahead() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _serial = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let _clock = ratchet_clock_ahead(&c, &base).await;

    let open = withhold(&mut c, &sk, &kid, p, med, "BP 85/50").await;
    assert_eq!(
        holds_now(&c, med).await,
        vec![(open.to_string(), false)],
        "a W with no `from` is in force at once — not pending until the HLC wall"
    );

    // A one-hour hold is shorter than the skew: stamped with the HLC wall it would be over
    // before it began and refused at the local door.
    let until = physical_plus(&c, 60).await;
    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: None,
        until: HoldUntil::At(&until),
    };
    let timed = withhold_medication(&mut c, &sk, &kid, "test-node", p, med, &input, None)
        .await
        .expect("a 1 h hold from now is admitted under a 2 h HLC skew");
    let rows = holds_now(&c, med).await;
    assert!(
        rows.contains(&(timed.to_string(), false)),
        "the 1 h hold is in force now: {rows:?}"
    );
    assert_never_flagged(&c, &[open, timed]).await;
}

#[tokio::test]
async fn a_release_ends_the_hold_now_even_when_the_hlc_runs_ahead() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _serial = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    // The hold is written BEFORE the ratchet, so it is unambiguously in force now.
    let w = withhold(&mut c, &sk, &kid, p, med, "HR 44").await;
    assert_eq!(holds_now(&c, med).await, vec![(w.to_string(), false)]);
    let _clock = ratchet_clock_ahead(&c, &base).await;

    let r = release_medication_holds(&mut c, &sk, &kid, "test-node", p, med, &[w], None, None)
        .await
        .expect("the release is admitted");
    assert!(
        holds_now(&c, med).await.is_empty(),
        "a release takes effect now — not 2 h from now at the HLC wall"
    );
    // `w` was written with the HLC in step with the physical clock — the closest case.
    assert_never_flagged(&c, &[w, r]).await;
}
