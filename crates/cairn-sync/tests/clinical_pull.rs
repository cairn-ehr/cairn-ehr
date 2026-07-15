//! Issue #199 (review finding B4) — the clinical-plane A→B pull, end to end.
//!
//! The set-union convergence guarantee ("author on A, sync to B, identical
//! projections") is Cairn's flagship property, and until this test it was verified
//! only by hand on the walking-skeleton rigs. This drives the REAL binary — `serve`
//! on node A, `pull` on node B — over real TCP, with events authored through the
//! production medication orchestrators on A, and then asserts the two nodes read
//! byte-identically through every medication projection. It also proves the slice-4
//! attestation token TRAVELS the wire (issue #91's parallel arrays) and re-verifies
//! at B's apply door, so a human vouch replicates without weakening.
//!
//! Skips unless BOTH `CAIRN_TEST_PG` (node A) and `CAIRN_TEST_PG2` (node B) are set.
//! Serialized cluster-wide via cairn-node's `db::test_serial_guard` (both DBs live on
//! the same cluster in CI, and this file TRUNCATEs shared tables on both).
use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, cease_medication, change_dose, reconcile_medications, AssertMedicationInput,
    AttestParams, CeaseMedicationInput, ChangeDoseInput, ReconcileInput,
};
use std::process::{Child, Command};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs_a() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}
fn cs_b() -> Option<String> {
    std::env::var("CAIRN_TEST_PG2").ok()
}

/// Fixed local port for A's serve loop. Safe from clashes: the serial guard means at
/// most one DB-gated test runs at a time, and the guard below kills the child on drop.
const LISTEN: &str = "127.0.0.1:39717";

/// Kill the spawned `serve` child when the test ends (pass or panic) so a leaked
/// listener can never wedge a later run on the fixed port.
struct ServeGuard(Child);
impl Drop for ServeGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Truncate the log + the medication projections + the sync bookkeeping on one node
/// and reset its HLC, so each run starts from a genuinely empty pair of nodes.
async fn reset(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, medication_statement, \
         medication_cessation, medication_dose_event, medication_dose_correction, \
         medication_reconciliation, medication_group_member, medication_projection_flag, \
         medication_attestation, medication_patient_conflict_flag, \
         sync_state, sync_quarantine CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
}

/// Enroll the same device + human keys on one node. B must know both actors before
/// the pull: actor enrollment does not travel the clinical plane (the registry's
/// sync semantics are #205), so the test performs the owner ceremony on each node —
/// exactly what a real two-site deployment does today.
async fn enroll_actors(c: &Client, kid_device: &str, kid_human: &str) {
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"ward-terminal\"}', $1)",
        &[&kid_device],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_human],
    )
    .await
    .unwrap();
}

/// One node's complete medication read-state, rendered to comparable strings:
/// the event set (ids + content addresses + whether an attestation is held), the
/// current/past medication views, the group collapse, and the standing vouches.
/// Two nodes holding the same event set MUST render this identically (set-union
/// convergence + ADR-0045 deterministic winners).
async fn snapshot(c: &Client) -> Vec<String> {
    let mut out = Vec::new();
    for row in c
        .query(
            "SELECT event_id::text, encode(content_address, 'hex'), \
                    coalesce(encode(attestation, 'hex'), '-'), \
                    coalesce(encode(attester_key, 'hex'), '-') \
             FROM event_log ORDER BY event_id",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "event {} {} att={} akey={}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, String>(2),
            row.get::<_, String>(3)
        ));
    }
    for row in c
        .query(
            "SELECT patient_id::text, medication_id::text, term, \
                    coalesce(dose_amount, '-'), coalesce(dose_unit, '-') \
             FROM patient_medication_current ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "current {} {} {} {}/{}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, String>(2),
            row.get::<_, String>(3),
            row.get::<_, String>(4)
        ));
    }
    for row in c
        .query(
            "SELECT patient_id::text, medication_id::text, term \
             FROM patient_medication_past ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "past {} {} {}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, String>(2)
        ));
    }
    for row in c
        .query(
            "SELECT medication_id::text, group_id::text \
             FROM medication_group_member ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "group {} -> {}",
            row.get::<_, String>(0),
            row.get::<_, String>(1)
        ));
    }
    for row in c
        .query(
            "SELECT medication_id::text, attester_kid, stale \
             FROM medication_thread_attestation ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "vouch {} by {} stale={}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, bool>(2)
        ));
    }
    out
}

/// Wait until A's serve loop accepts TCP (readiness poll, bounded).
fn wait_listening(addr: &str) {
    for _ in 0..50 {
        if std::net::TcpStream::connect(addr).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("serve did not start listening on {addr}");
}

#[tokio::test]
async fn a_to_b_pull_converges_projections_and_ships_the_attestation() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    // --- provision both nodes and enroll the same actors on each ---
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_d, kid_d) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    enroll_actors(&a, &kid_d, &kid_h).await;
    enroll_actors(&b, &kid_d, &kid_h).await;

    // --- author a realistic little chart on A through the production orchestrators ---
    let mut a = a; // the orchestrators take &mut Client (they open transactions)
    let patient = Uuid::now_v7();
    let metformin = AssertMedicationInput {
        term: "metformin",
        inn_code: None,
        formulation: Some("tablet"),
        dose_amount: Some("500"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2023"),
        started_precision: Some("year"),
    };
    let med1 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &metformin, None)
        .await
        .unwrap();
    // Dose change WITH an atomic human vouch: the attestation covers assert+change,
    // so the standing vouch must read non-stale on BOTH nodes after sync.
    let vouch = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("ward-round review"),
        note: None,
    };
    change_dose(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        med1,
        &ChangeDoseInput {
            dose_amount: Some("1000"),
            dose_unit: Some("mg"),
            effective: Some("2026-06"),
            effective_precision: Some("month"),
            info_source: "clinician",
            reason: Some("HbA1c above target"),
        },
        Some(&vouch),
    )
    .await
    .unwrap();
    // A ceased thread (exercises the past view across the wire).
    let atorva = AssertMedicationInput {
        term: "atorvastatin",
        dose_amount: Some("40"),
        ..metformin
    };
    let med2 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &atorva, None)
        .await
        .unwrap();
    cease_medication(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        med2,
        &CeaseMedicationInput {
            stopped: Some("2026-05"),
            stopped_precision: Some("month"),
            reason: Some("myalgia"),
        },
        None,
    )
    .await
    .unwrap();
    // A reconciled duplicate pair (exercises the group collapse across the wire).
    let ibu = AssertMedicationInput {
        term: "ibuprofen",
        dose_amount: Some("400"),
        ..metformin
    };
    let med3 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &ibu, None)
        .await
        .unwrap();
    let med4 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &ibu, None)
        .await
        .unwrap();
    reconcile_medications(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        med3,
        med4,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
    )
    .await
    .unwrap();

    // --- the wire: serve on A, one pull on B, through the real binary ---
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args(["serve", "--conn", &base_a, "--listen", LISTEN])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN);
    let pull = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN,
            "--peer-name",
            "node-a",
        ])
        .output()
        .expect("run pull");
    assert!(
        pull.status.success(),
        "pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&pull.stdout),
        String::from_utf8_lossy(&pull.stderr)
    );

    // --- the flagship assertion: A and B read identically, projection by projection ---
    let snap_a = snapshot(&a).await;
    let snap_b = snapshot(&b).await;
    assert_eq!(
        snap_a, snap_b,
        "A and B must render identical medication read-state after one pull"
    );

    // Non-vacuous floor: the snapshot really carries the chart we authored.
    let joined = snap_a.join("\n");
    assert!(
        joined.contains("metformin 1000/mg"),
        "the synced dose change drives B's current dose:\n{joined}"
    );
    assert!(
        joined.contains("past") && joined.contains("atorvastatin"),
        "the ceased thread reads as past on both nodes:\n{joined}"
    );
    assert!(
        joined.contains(&format!("vouch {med1} by {kid_h} stale=false")),
        "the human vouch travelled, re-verified, and reads current:\n{joined}"
    );
    let ibu_groups: Vec<&str> = snap_a
        .iter()
        .filter(|l| l.starts_with("group"))
        .map(|s| s.as_str())
        .collect();
    assert_eq!(
        ibu_groups.len(),
        2,
        "both duplicate threads joined one reconciled group:\n{joined}"
    );

    // The quarantine stayed empty: nothing on this wire needed penning.
    let penned: i64 = b
        .query_one("SELECT count(*) FROM sync_quarantine", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(penned, 0, "a clean pull pens nothing");
}
