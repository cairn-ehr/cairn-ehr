//! Unit tests for [`super`]: the shortfall rule on each axis, the verdict an operator reads, and
//! the one impure evidence adapter. A sibling file, as `cairn-medium/src/chain.rs` does it.

use super::*;
use cairn_medium::MediumRecord;

/// Runtime-derived bytes for a fixture field — never a literal (house rule 6: a byte
/// literal in a custody field trips CodeQL's hard-coded-cryptographic-value query).
fn filler(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
}

/// One record at `seq`. `variant` changes the custody sidecar, so two records at one seq
/// with different variants are DIFFERENT records (a straddled re-capture).
fn rec(seq: i64, variant: Option<u8>) -> MediumRecord {
    MediumRecord {
        signed_bytes: filler(1, 8),
        attestation: None,
        attester_key: None,
        dek_wrapped: variant.map(|v| filler(v, 16)),
        source_seq: seq,
    }
}

fn plane(records: Vec<MediumRecord>, collapsed: usize) -> PlaneRecords {
    PlaneRecords {
        records,
        gated_out: 0,
        collapsed,
    }
}

/// Evidence as a v2 (or newer) sidecar yields it: both recorded facts.
fn recorded(newest_seq: Option<i64>, clinical_records: u64) -> Option<LastBackupEvidence> {
    Some(LastBackupEvidence {
        newest_seq,
        clinical_records: Some(clinical_records),
    })
}

/// Evidence as a pre-v2 sidecar yields it: a newest seq at most, never a record count.
fn recorded_seq_only(newest_seq: Option<i64>) -> Option<LastBackupEvidence> {
    Some(LastBackupEvidence {
        newest_seq,
        clinical_records: None,
    })
}

/// The verdict over `acc`, with the medium's RAW record count taken from the accounting's own
/// arithmetic (its three fields sum to the raw count).
fn verdict(
    acc: &PlaneRecords,
    legacy: bool,
    evidence: Option<LastBackupEvidence>,
) -> ClinicalPlaneVerdict {
    let raw = acc.records.len() + acc.collapsed + acc.gated_out;
    verdict_with_raw(acc, raw, legacy, evidence)
}

/// The verdict with an explicit raw count, so a count-axis test shows the number it tests.
fn verdict_with_raw(
    acc: &PlaneRecords,
    medium_clinical_records: usize,
    legacy: bool,
    evidence: Option<LastBackupEvidence>,
) -> ClinicalPlaneVerdict {
    clinical_plane_verdict(&ClinicalPlaneFacts {
        accounting: acc,
        medium_clinical_records,
        legacy,
        evidence,
    })
}

fn health_for(
    medium_path: &str,
    version: u8,
    clinical_watermark: Option<i64>,
    clinical_events: u64,
) -> BackupHealth {
    BackupHealth {
        version,
        last_backup_unix: 0,
        medium_path: medium_path.into(),
        medium_bytes: 0,
        node_events: 1,
        clinical_events,
        clinical_watermark,
        export_covers_seq: None,
        extra: serde_json::Map::new(),
    }
}

const V2: u8 = super::super::SUPPORTED_HEALTH_VERSION;

// --- the shortfall rule: the newest-seq axis ---------------------------------------------

#[test]
fn no_evidence_is_never_a_shortfall() {
    assert_eq!(shortfall(None, 0, None), None);
    assert_eq!(shortfall(Some(40), 3, None), None);
    let neither_fact = recorded_seq_only(None);
    assert_eq!(
        shortfall(None, 0, neither_fact),
        None,
        "no fact, no evidence"
    );
}

#[test]
fn an_empty_medium_against_evidence_is_short() {
    assert_eq!(
        shortfall(None, 0, recorded_seq_only(Some(40))),
        Some(Shortfall {
            newest_seq: Some(40),
            clinical_records: None,
        })
    );
}

#[test]
fn a_medium_behind_the_evidence_is_short() {
    assert_eq!(
        shortfall(Some(39), 5, recorded_seq_only(Some(40))),
        Some(Shortfall {
            newest_seq: Some(40),
            clinical_records: None,
        })
    );
}

/// The boundary: holding exactly what the last backup recorded is complete.
#[test]
fn a_medium_level_with_the_evidence_is_not_short() {
    assert_eq!(shortfall(Some(40), 5, recorded_seq_only(Some(40))), None);
}

/// A backup that wrote the medium durably and then failed to write its sidecar leaves the
/// medium AHEAD of the evidence. Holding more than recorded is not a shortfall.
#[test]
fn a_medium_ahead_of_the_evidence_is_not_short() {
    assert_eq!(shortfall(Some(41), 5, recorded_seq_only(Some(40))), None);
}

// --- the shortfall rule: the record-count axis -------------------------------------------

/// **The case the newest seq alone cannot see** (final review, 2026-09-14). Night 1 captures
/// seqs 1–100 while seq 97's transaction is still uncommitted (yielding 99 records); night 2
/// writes nothing new but backfills 97 below the watermark, and records 100 clinical records.
/// The night-1 copy put back has the SAME newest seq and one record fewer.
#[test]
fn the_same_newest_seq_with_fewer_records_is_short() {
    assert_eq!(
        shortfall(Some(100), 99, recorded(Some(100), 100)),
        Some(Shortfall {
            newest_seq: None,
            clinical_records: Some(100),
        })
    );
}

/// The count boundary: the same raw count is complete.
#[test]
fn a_medium_level_with_the_recorded_count_is_not_short() {
    assert_eq!(shortfall(Some(100), 101, recorded(Some(100), 101)), None);
}

/// More raw records than recorded is the failed-sidecar-write case on this axis too.
#[test]
fn more_records_than_recorded_is_not_short() {
    assert_eq!(shortfall(Some(100), 102, recorded(Some(100), 101)), None);
}

/// A v1 sidecar never recorded a count; serde defaults it to 0, which is a claim nobody made.
/// Its absence must never turn into a count-based refusal — nor, from a stray 0, into an
/// all-clear the sidecar did not state either. So the count axis is simply not consulted.
#[test]
fn a_pre_v2_sidecar_never_refuses_on_the_count() {
    assert_eq!(shortfall(Some(100), 3, recorded_seq_only(Some(100))), None);
}

#[test]
fn both_axes_short_are_reported_together() {
    assert_eq!(
        shortfall(Some(90), 50, recorded(Some(100), 101)),
        Some(Shortfall {
            newest_seq: Some(100),
            clinical_records: Some(101),
        })
    );
}

/// The axes are independent: AHEAD on one never excuses SHORT on the other.
#[test]
fn ahead_on_one_axis_does_not_excuse_the_other() {
    assert_eq!(
        shortfall(Some(120), 50, recorded(Some(100), 101)),
        Some(Shortfall {
            newest_seq: None,
            clinical_records: Some(101),
        }),
        "a newer seq with fewer records is still not what that backup wrote"
    );
    assert_eq!(
        shortfall(Some(90), 500, recorded(Some(100), 101)),
        Some(Shortfall {
            newest_seq: Some(100),
            clinical_records: None,
        }),
        "more records with an older newest seq is still missing the newest event"
    );
}

// --- the verdict ---------------------------------------------------------------------

#[test]
fn records_present_without_evidence_report_count_and_newest_seq() {
    let acc = plane(vec![rec(3, None), rec(5, None), rec(8, None)], 0);
    let v = verdict(&acc, false, None);
    assert_eq!(
        v.summary,
        "clinical-plane records OK: 3 verified, newest seq 8"
    );
    assert_eq!(v.advisory, None);
    assert_eq!(v.refusal, None);
}

#[test]
fn collapsed_re_captures_are_named_in_the_summary() {
    let acc = plane(vec![rec(1, None), rec(2, None)], 4);
    let v = verdict(&acc, false, None);
    assert_eq!(
        v.summary,
        "clinical-plane records OK: 2 verified, newest seq 2, 4 byte-identical \
         re-capture(s) collapsed"
    );
}

#[test]
fn a_straddled_duplicate_is_advisory_and_never_refuses() {
    let acc = plane(vec![rec(7, Some(4)), rec(7, None), rec(8, None)], 0);
    let v = verdict(&acc, false, recorded(Some(8), 3));
    let advisory = v
        .advisory
        .expect("two different records at one seq must be named");
    assert!(
        advisory.contains(": 7."),
        "the position is named: {advisory}"
    );
    assert!(
        advisory.contains("A restore would apply all of them"),
        "worded for a check that has applied nothing: {advisory}"
    );
    assert!(
        !advisory.contains("were applied"),
        "restore's past tense would be false here: {advisory}"
    );
    assert_eq!(v.refusal, None, "a straddle is not a restorability failure");
}

#[test]
fn an_empty_plane_without_evidence_says_empty_and_does_not_refuse() {
    let acc = plane(vec![], 0);
    let v = verdict(&acc, false, None);
    assert!(
        v.summary.starts_with("clinical plane: EMPTY — "),
        "{}",
        v.summary
    );
    assert!(
        v.summary.contains("restore NO patient data"),
        "{}",
        v.summary
    );
    assert_eq!(
        v.refusal, None,
        "a fresh clinic's empty plane is a correct backup"
    );
}

#[test]
fn a_legacy_medium_without_evidence_says_none_and_does_not_refuse() {
    let acc = plane(vec![], 0);
    let v = verdict(&acc, true, None);
    assert!(
        v.summary.starts_with("clinical plane: NONE — "),
        "{}",
        v.summary
    );
    assert!(v.summary.contains("CAIRNB1/CAIRNB2"), "{}", v.summary);
    assert_eq!(v.refusal, None);
}

#[test]
fn an_empty_plane_against_evidence_refuses_naming_both_sides() {
    let acc = plane(vec![], 0);
    let v = verdict(&acc, false, recorded_seq_only(Some(4812)));
    let refusal = v
        .refusal
        .expect("the sidecar proves this path held clinical events");
    assert!(refusal.starts_with("backup SHORT: "), "{refusal}");
    assert!(refusal.contains("newest clinical seq 4812"), "{refusal}");
    assert!(refusal.contains("no clinical records at all"), "{refusal}");
    assert!(
        refusal.contains("run `backup --to`"),
        "the remedy: {refusal}"
    );
    assert!(
        v.summary.starts_with("clinical plane: EMPTY"),
        "the plane line still prints first: {}",
        v.summary
    );
}

#[test]
fn a_plane_behind_evidence_refuses_naming_both_seqs() {
    let acc = plane(vec![rec(1, None), rec(30, None)], 0);
    let refusal = verdict(&acc, false, recorded_seq_only(Some(40)))
        .refusal
        .expect("30 < 40");
    assert!(refusal.contains("newest clinical seq 40"), "{refusal}");
    assert!(
        refusal.contains("this medium's newest clinical seq is 30"),
        "{refusal}"
    );
}

/// Review M3: the evidence is about a PATH, and in a rotation it was a different drive — so
/// the message must never say "this medium" recorded anything. And "through seq N" implied no
/// gaps below N, which is exactly the claim the count axis exists to stop relying on.
#[test]
fn the_refusal_names_the_path_and_never_claims_a_gapless_run() {
    let acc = plane(vec![rec(30, None)], 0);
    let refusal = verdict(&acc, false, recorded(Some(40), 2))
        .refusal
        .expect("30 < 40");
    assert!(
        refusal.contains("this node's last backup to this path"),
        "{refusal}"
    );
    assert!(!refusal.contains("backup to this medium"), "{refusal}");
    assert!(!refusal.contains("through seq"), "{refusal}");
    assert!(
        refusal.contains("Rotating drives through one mount point?"),
        "the rotation hint stays: {refusal}"
    );
}

/// A legacy file at a path where this node last wrote clinical events is not what that
/// backup wrote either: `backup` converts a legacy medium to CAIRNB3 on its next capture.
#[test]
fn a_legacy_medium_against_evidence_refuses() {
    let acc = plane(vec![], 0);
    assert!(verdict(&acc, true, recorded_seq_only(Some(12)))
        .refusal
        .is_some());
}

#[test]
fn a_plane_level_with_evidence_does_not_refuse() {
    let acc = plane(vec![rec(40, None)], 0);
    assert_eq!(verdict(&acc, false, recorded(Some(40), 1)).refusal, None);
}

/// The below-watermark backfill, as the operator reads it: the newest seq matches, so the
/// refusal must name the COUNTS — and must not claim a certain loss, because the missing
/// records could all have been byte-identical re-captures a restore collapses anyway.
#[test]
fn the_same_newest_seq_with_fewer_records_refuses_naming_the_counts() {
    let acc = plane(vec![rec(1, None), rec(2, None), rec(100, None)], 0);
    let refusal = verdict_with_raw(&acc, 3, false, recorded(Some(100), 4))
        .refusal
        .expect("3 raw records < the 4 that backup recorded");
    assert!(refusal.starts_with("backup SHORT: "), "{refusal}");
    assert!(
        refusal.contains("That backup recorded 4 clinical record(s); this medium holds 3."),
        "{refusal}"
    );
    assert!(
        !refusal.contains("newest clinical seq"),
        "the seq axis is level, so it is not reported as short: {refusal}"
    );
    assert!(
        refusal.contains("unless every missing record was a re-capture (byte-identical or with different custody)"),
        "a count shortfall alone is not a certain loss: {refusal}"
    );
    assert!(
        refusal.contains("run `backup --to`"),
        "the remedy: {refusal}"
    );
}

#[test]
fn more_raw_records_than_recorded_does_not_refuse() {
    let acc = plane(vec![rec(1, None), rec(100, None)], 3);
    let v = verdict_with_raw(&acc, 5, false, recorded(Some(100), 4));
    assert_eq!(v.refusal, None, "5 raw records > the 4 recorded");
}

/// v1 evidence carries no count, so a medium with fewer raw records than anything is not
/// refused on that axis — its level newest seq is all the evidence there is.
#[test]
fn a_pre_v2_sidecar_with_a_lower_medium_count_does_not_refuse() {
    let acc = plane(vec![rec(100, None)], 0);
    let v = verdict_with_raw(&acc, 1, false, recorded_seq_only(Some(100)));
    assert_eq!(v.refusal, None);
}

#[test]
fn both_axes_short_is_one_refusal_naming_both() {
    let acc = plane(vec![rec(1, None), rec(90, None)], 0);
    let refusal = verdict(&acc, false, recorded(Some(100), 5))
        .refusal
        .expect("90 < 100 and 2 < 5");
    assert_eq!(
        refusal.matches("backup SHORT").count(),
        1,
        "one refusal: {refusal}"
    );
    assert!(
        refusal.contains("That backup recorded newest clinical seq 100; this medium's newest clinical seq is 90."),
        "{refusal}"
    );
    assert!(
        refusal.contains("That backup recorded 5 clinical record(s); this medium holds 2."),
        "{refusal}"
    );
    assert!(
        !refusal.contains("unless every missing record"),
        "with the newest event missing, the loss is certain: {refusal}"
    );
}

/// Facts no sound medium can produce — raw records present, none verified — still get a TRUE
/// sentence: "no clinical records at all" would be false over three raw records.
#[test]
fn raw_records_with_none_verified_are_not_called_none_at_all() {
    let refusal = verdict_with_raw(&plane(vec![], 0), 3, false, recorded_seq_only(Some(40)))
        .refusal
        .expect("no verified seq against a recorded one");
    assert!(!refusal.contains("at all"), "{refusal}");
    assert!(
        refusal.contains("none of this medium's 3 clinical record(s) is verified"),
        "{refusal}"
    );
}

// --- the evidence adapter ------------------------------------------------------------

#[test]
fn a_sidecar_describing_this_medium_is_evidence() {
    let h = health_for("/nonexistent-567/cairn.medium", V2, Some(40), 7);
    assert_eq!(
        last_backup_evidence_for(Some(&h), Path::new("/nonexistent-567/cairn.medium")),
        recorded(Some(40), 7)
    );
}

/// The case only a node-global sidecar can create: a rotation drive at another path.
/// That sidecar is about some other artifact — possibly another node's — never this one.
#[test]
fn a_sidecar_describing_another_medium_is_not_evidence() {
    let h = health_for("/nonexistent-567/drive-a.medium", V2, Some(40), 7);
    assert_eq!(
        last_backup_evidence_for(Some(&h), Path::new("/nonexistent-567/drive-b.medium")),
        None
    );
}

#[test]
fn no_sidecar_is_not_evidence() {
    let medium = Path::new("/nonexistent-567/cairn.medium");
    assert_eq!(last_backup_evidence_for(None, medium), None);
}

/// A sidecar that recorded no watermark (a fresh clinic) gives no newest-seq evidence, and its
/// count of 0 can never make anything short — so an empty medium beside it stays green.
#[test]
fn a_sidecar_with_no_recorded_watermark_is_no_evidence_of_a_shortfall() {
    let medium = Path::new("/nonexistent-567/cairn.medium");
    let fresh = health_for("/nonexistent-567/cairn.medium", V2, None, 0);
    let evidence = last_backup_evidence_for(Some(&fresh), medium);
    assert_eq!(evidence, recorded(None, 0));
    assert_eq!(verdict(&plane(vec![], 0), false, evidence).refusal, None);
}

/// v1 sidecars carry no per-plane counts; serde defaults `clinical_events` to 0.
/// `describe_health` already refuses to render that 0 as a fact, and so does the evidence.
/// (A non-zero count is used here so the test cannot pass merely because 0 compares low.)
#[test]
fn a_pre_v2_sidecar_gives_no_count_evidence() {
    let medium = Path::new("/nonexistent-567/cairn.medium");
    let v1 = health_for("/nonexistent-567/cairn.medium", V2 - 1, None, 9);
    assert_eq!(
        last_backup_evidence_for(Some(&v1), medium),
        recorded_seq_only(None)
    );
}
