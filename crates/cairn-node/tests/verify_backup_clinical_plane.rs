//! `verify-backup` asks about the CLINICAL plane (#567), driven through the real binary.
//!
//! Before #567 this command printed one line about the federation plane and nothing about the
//! clinical plane a restore now applies. These tests pin the three things the design
//! (`docs/superpowers/specs/2026-09-13-verify-backup-clinical-plane-design.md`) decided:
//!
//! 1. an older copy put back at the path the nightly backup writes to fails `backup SHORT` —
//!    the node's own sidecar proves the path held more;
//! 2. a clinical chain break with every record signature intact still fails, and fails BEFORE
//!    any clinical all-clear is printed (the reason `untrusted_clinical_notice` is not wired);
//! 3. a sidecar about ANOTHER path is not evidence, even when it records clinical events.
//!
//! The pure policy is unit-tested in `src/backup/clinical_verdict.rs`; these prove the arm
//! gathers the right facts and acts on the verdict.

mod common;

#[path = "common/clinic_kit.rs"]
mod clinic_kit;

use cairn_medium::{assess, parse_any, serialize_v3, MediumImage, Plane};
use cairn_node::backup;
use clinic_kit::{author_sealed_clinical_event, establish_clinic, Clinic};

/// Run a `cairn-node` command that must succeed, panicking with its stderr if it does not.
fn run_ok(cmd: &mut std::process::Command, what: &str) -> std::process::Output {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{what} must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn verify(cl: &Clinic, medium: &std::path::Path) -> std::process::Output {
    cl.cli()
        .args(["verify-backup", "--from"])
        .arg(medium)
        .output()
        .unwrap()
}

fn backup_to(cl: &Clinic, medium: &std::path::Path, what: &str) {
    run_ok(cl.cli().args(["backup", "--to"]).arg(medium), what);
}

/// The failure #567 exists for. Night 1 backs up a node with no charts; the file is copied
/// aside; a chart is written and night 2 captures it; then the night-1 copy is put back at the
/// same path — what a same-mount-point rotation, or a restored-from-an-old-copy drive, looks
/// like. Before #567 this verified green (the kit verdict saw an empty clinical plane as nothing
/// to cover) and a restore from it would have brought back no charts.
#[tokio::test]
async fn an_older_copy_at_the_backed_up_path_fails_as_short() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    backup_to(&cl, &cl.medium(), "night 1's backup (no charts yet)");
    let night_one = cl.dir.path().join("night-1.medium");
    std::fs::copy(cl.medium(), &night_one).unwrap();

    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    backup_to(&cl, &cl.medium(), "night 2's backup (one chart)");

    // Positive control: the evidence this test depends on really exists. Without it a green
    // run could mean "no sidecar was written", not "the rule works".
    let health = backup::read_health(&backup::health_path_for(&cl.key()))
        .expect("night 2 wrote backup-status.json");
    let recorded = health
        .clinical_watermark
        .expect("night 2's sidecar records the clinical capture");
    assert!(backup::health_describes_medium(
        &health.medium_path,
        &cl.medium()
    ));

    std::fs::copy(&night_one, cl.medium()).unwrap();
    let v = verify(&cl, &cl.medium());
    let stdout = String::from_utf8_lossy(&v.stdout);
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(
        !v.status.success(),
        "an older copy at the backed-up path must fail the health check; stdout:\n{stdout}"
    );
    assert!(stderr.contains("backup SHORT"), "named as SHORT: {stderr}");
    assert!(
        stderr.contains(&format!("through seq {recorded}")),
        "naming what the last backup recorded: {stderr}"
    );
    assert!(
        stdout.contains("clinical plane: EMPTY"),
        "and the plane line printed before the refusal says what the copy holds: {stdout}"
    );
}

/// A clinical segment whose chain link is broken while every record's signature still
/// verifies. `verify_backup_scope.rs`'s corrupt-clinical test flips a byte INSIDE a record,
/// which is the signature path; this is the chain path. It must fail as UNSOUND, and no
/// clinical all-clear may have printed before it.
#[tokio::test]
async fn a_broken_clinical_chain_link_fails_before_any_clinical_all_clear() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    backup_to(&cl, &cl.medium(), "the backup");

    let MediumImage::V3(mut m) = parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap() else {
        panic!("backup writes CAIRNB3")
    };
    assert!(
        assess(&m).sound(),
        "positive control: sound before the break"
    );
    let target = m
        .segments
        .iter()
        .position(|s| s.plane == Plane::Clinical)
        .expect("the backup captured a clinical segment");
    m.segments[target].prev_commitment = "deadbeef".into();
    std::fs::write(cl.medium(), serialize_v3(&m.segments).unwrap()).unwrap();

    // Prove the break is on the CHAIN path, not the signature path.
    let MediumImage::V3(broken) = parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap() else {
        panic!("still CAIRNB3")
    };
    let h = assess(&broken);
    assert!(
        h.records.all_intact(),
        "every record signature still verifies"
    );
    assert!(!h.chain.chain_intact(), "and the chain does not");

    let v = verify(&cl, &cl.medium());
    let stdout = String::from_utf8_lossy(&v.stdout);
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(
        !v.status.success(),
        "a broken chain must fail; stdout:\n{stdout}"
    );
    // UNSOUND, not SHORT. Both would be true of this file: the break gates the clinical records
    // out, so the trusted set is empty while the sidecar recorded a clinical watermark for this
    // path. Soundness is checked first because its remedy ("locate another copy") is the right
    // one for a damaged medium; "run backup again" would append to a broken chain.
    assert!(stderr.contains("backup UNSOUND"), "{stderr}");
    assert!(!stderr.contains("backup SHORT"), "{stderr}");
    assert!(
        !stdout.contains("clinical"),
        "no clinical-plane line may precede the refusal: {stdout}"
    );
}

/// A sidecar about ANOTHER path is not evidence, even when it records clinical events.
///
/// This pins a GREEN over a drive that lacks this node's charts, and it is deliberate: the
/// sidecar is node-global and `verify-backup` does not bind a medium to `--key`'s node, so a
/// sidecar about drive A may describe a different node than drive B. Closing it needs a kit
/// identity the sidecar can bind to (#551). A non-empty drive at another path already fails
/// COVERAGE-UNKNOWN; that asymmetry predates #567 (design §7).
#[tokio::test]
async fn a_sidecar_about_another_path_is_not_evidence_even_with_charts() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let drive_a = cl.dir.path().join("drive-a.medium");
    let drive_b = cl.dir.path().join("drive-b.medium");
    backup_to(&cl, &drive_b, "drive B's backup (no charts yet)");
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    backup_to(&cl, &drive_a, "drive A's backup (one chart)");

    let health = backup::read_health(&backup::health_path_for(&cl.key())).expect("sidecar");
    assert!(
        health.clinical_watermark.is_some(),
        "positive control: the sidecar DOES record clinical events"
    );
    assert!(backup::health_describes_medium(
        &health.medium_path,
        &drive_a
    ));
    assert!(!backup::health_describes_medium(
        &health.medium_path,
        &drive_b
    ));

    let v = verify(&cl, &drive_b);
    let stdout = String::from_utf8_lossy(&v.stdout);
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(
        v.status.success(),
        "a sidecar about drive A is not evidence about drive B; stderr:\n{stderr}"
    );
    assert!(!stderr.contains("backup SHORT"), "{stderr}");
    assert!(stdout.contains("clinical plane: EMPTY"), "{stdout}");
}
