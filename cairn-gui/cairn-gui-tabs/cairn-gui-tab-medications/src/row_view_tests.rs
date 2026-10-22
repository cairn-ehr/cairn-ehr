//! Tests for `row_view.rs`, split out to keep that file readable.
use crate::test_rows::{chart, member, row};
use crate::view::build_view;
use cairn_medication_view::{MedicationStatus, VouchState};

#[test]
fn a_coded_drug_displays_its_coded_name_over_the_free_text_term() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.term = "little white pill".into();
    r.coding_display = Some("metformin hydrochloride".into());
    let view = build_view(&chart(vec![r]));
    assert_eq!(view.rows[0].primary, "metformin hydrochloride");
}

/// Principle 4: a vague term is a legitimate recorded value, never blanked out.
#[test]
fn an_uncoded_drug_displays_its_free_text_term_unaltered() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.term = "little white pill".into();
    let view = build_view(&chart(vec![r]));
    assert_eq!(view.rows[0].primary, "little white pill");
    assert!(view.rows[0].current, "an active drug is current");
}

#[test]
fn a_fresh_vouch_names_its_signatory() {
    let rows = vec![row(
        1,
        MedicationStatus::Active,
        vec![member(
            1,
            VouchState::Fresh {
                by: "abcdef0123456789".into(),
            },
        )],
    )];
    let view = build_view(&chart(rows));
    assert!(
        view.rows[0].vouch_label.contains("abcdef01"),
        "the clinician must see WHOSE signature it is: {}",
        view.rows[0].vouch_label
    );
}

#[test]
fn a_stale_vouch_says_so() {
    let rows = vec![row(
        1,
        MedicationStatus::Active,
        vec![member(
            1,
            VouchState::Stale {
                by: "abcdef0123456789".into(),
            },
        )],
    )];
    let view = build_view(&chart(rows));
    assert!(
        view.rows[0].vouch_label.contains("out of date"),
        "got: {}",
        view.rows[0].vouch_label
    );
}

#[test]
fn a_ceased_row_is_shown_marked_and_never_targeted() {
    let rows = vec![row(
        1,
        MedicationStatus::Ceased,
        vec![member(1, VouchState::Absent)],
    )];
    let view = build_view(&chart(rows));
    assert_eq!(view.rows.len(), 1, "a struck line stays on the chart");
    assert_eq!(view.rows[0].status_label, "ceased");
    assert!(
        !view.rows[0].current,
        "the typed fact agrees with the label"
    );
    assert!(!view.rows[0].will_be_signed);
    assert!(
        !view.rows[0].can_cease,
        "a ceased drug cannot be ceased again"
    );
}

#[test]
fn advisory_flags_are_surfaced_as_row_labels() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.reconciliation_flagged = true;
    r.coding_conflict = true;
    let view = build_view(&chart(vec![r]));
    assert_eq!(view.rows[0].flags.len(), 2, "got: {:?}", view.rows[0].flags);
}

#[test]
fn the_dose_reads_as_amount_and_unit() {
    let view = build_view(&chart(vec![row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    )]));
    assert_eq!(view.rows[0].dose, "500 mg");
}

/// Principle 4 again: an unknown dose is shown as unknown, never as a blank that reads
/// like "no dose" or as a fabricated default.
#[test]
fn an_absent_dose_is_shown_as_unknown() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.dose_amount = None;
    r.dose_unit = None;
    assert_eq!(
        build_view(&chart(vec![r])).rows[0].dose,
        "dose not recorded"
    );
}

/// #697 (b): a doubted-link line says WHY in its own words — not "shared with another
/// patient's record", which names the wrong cause.
#[test]
fn a_doubted_link_line_names_the_doubted_link_not_another_patient() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.cross_patient = true;
    r.wrong_chart.doubted_link = true;
    let flags = &build_view(&chart(vec![r])).rows[0].flags;
    assert!(flags.iter().any(|f| f.contains("in doubt")), "{flags:?}");
    assert!(
        flags.iter().any(|f| f.contains("cannot yet vouch")
            && f.contains("until the record's links are no longer in doubt")),
        "{flags:?}"
    );
    assert!(
        !flags.iter().any(|f| f.contains("another patient")),
        "{flags:?}"
    );
}

/// PR #717 review: a doubted line with nothing to sign — ceased, or signed before the
/// doubt arose — still says the doubt, but not "cannot be signed until …": that promises a
/// later signature for a line that needs none (and a ceased line is never signed).
#[test]
fn a_doubted_line_nothing_withholds_promises_no_later_signature() {
    let mut ceased = row(
        1,
        MedicationStatus::Ceased,
        vec![member(1, VouchState::Absent)],
    );
    let mut signed = row(
        2,
        MedicationStatus::Active,
        vec![member(2, VouchState::Fresh { by: "dr_b".into() })],
    );
    for r in [&mut ceased, &mut signed] {
        r.cross_patient = true;
        r.wrong_chart.doubted_link = true;
    }
    let view = build_view(&chart(vec![ceased, signed]));
    for r in &view.rows {
        assert!(
            r.flags.iter().any(|f| f.contains("cannot yet vouch")),
            "{:?}",
            r.flags
        );
        assert!(
            !r.flags.iter().any(|f| f.contains("cannot be signed")),
            "{:?}",
            r.flags
        );
    }
}

#[test]
fn a_line_with_both_reasons_says_both() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.cross_patient = true;
    r.wrong_chart = cairn_medication_view::WrongChartReasons {
        outside_set: true,
        doubted_link: true,
        patient_conflict: false,
    };
    let flags = &build_view(&chart(vec![r])).rows[0].flags;
    assert!(
        flags.iter().any(|f| f.contains("another patient")),
        "{flags:?}"
    );
    assert!(flags.iter().any(|f| f.contains("in doubt")), "{flags:?}");
}

/// The withheld line is SHOWN — hiding a drug is the worse failure — but it is not a
/// sign-off target, and the row has to say so where the clinician is looking.
#[test]
fn a_cross_patient_line_is_shown_flagged_and_not_signed() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.cross_patient = true;
    let view = build_view(&chart(vec![r]));
    assert_eq!(view.rows.len(), 1, "the drug must still be visible");
    assert!(!view.rows[0].will_be_signed);
    assert!(
        view.rows[0]
            .flags
            .iter()
            .any(|f| f.contains("another patient")),
        "the row must say why it cannot be signed: {:?}",
        view.rows[0].flags
    );
}

/// A group is not signed off until every member is: the summary names the WORST member
/// state, so a half-signed reconciled pair never reads as done.
#[test]
fn a_partly_signed_group_does_not_read_as_signed() {
    let reconciled = row(
        1,
        MedicationStatus::Active,
        vec![
            member(1, VouchState::Fresh { by: "dr_b".into() }),
            member(2, VouchState::Absent),
        ],
    );
    let view = build_view(&chart(vec![reconciled]));
    assert_eq!(view.rows[0].vouch_label, "not signed");
    assert!(view.rows[0].will_be_signed);
}

// ---- ADR-0076 decision 1: every row of a combined list names its source chart(s).

/// A linked list is two paper folders clipped together, and the clinician reads BOTH — so
/// every line says which folder it came from. A never-linked chart says nothing extra:
/// it reads exactly as it did before R1 (the plan's cognitive-load budget).
#[test]
fn a_linked_list_labels_each_row_with_its_source() {
    use cairn_medication_view::{fixtures::sample_chart, ChartSet};
    use uuid::Uuid;

    let opened = Uuid::from_u128(1); // the fixture chart
    let linked = Uuid::from_u128(0xB);

    // Unlinked: no row carries a label, whatever its sources.
    let single = build_view(&sample_chart());
    assert!(
        single.rows.iter().all(|r| r.source.is_none()),
        "a never-linked chart reads exactly as before: {:?}",
        single.rows
    );
    assert_eq!(single.charts, vec![opened.to_string()]);

    // Linked: the relabelled row names the OTHER chart; every row names its own.
    let mut list = sample_chart();
    list.charts = ChartSet::new([opened, linked]).unwrap();
    list.rows[0].source_charts = vec![linked];
    let view = build_view(&list);
    assert_eq!(view.rows[0].source, Some(linked.to_string()));
    for r in &view.rows[1..] {
        let source = r
            .source
            .as_deref()
            .expect("every row of a linked list is labelled");
        assert!(!source.contains(&linked.to_string()), "{source}");
        assert!(source.contains(&opened.to_string()), "{source}");
    }
    // A group spanning two charts names both, in the row's own (sorted) order.
    let cross = view.rows.last().unwrap();
    assert_eq!(
        cross.source,
        Some(format!("{}, {}", opened, Uuid::from_u128(2)))
    );
    assert_eq!(view.charts, vec![opened.to_string(), linked.to_string()]);
}

/// The race case named on `MedicationRow::source_charts` itself: a group-chart read
/// landing between the read model's several statements can leave a row on a LINKED
/// list with an empty `source_charts`. Before this fix, `linked.then(|| source_label(…))`
/// turned that into `Some("")` — an empty string `main.js` treats as falsy, so the row
/// silently lost its provenance label on screen with no visible sign anything was
/// wrong. Render an explicit marker instead, so the clinician sees "we don't know",
/// never a blank that reads as "nothing to say" (principle 4: acknowledged uncertainty
/// beats a silent gap).
#[test]
fn a_linked_row_with_no_source_charts_says_so_rather_than_going_silent() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.source_charts = vec![];
    let view = super::build_row(
        &r,
        &std::collections::HashSet::new(),
        true,
        &chrono::Local::now(),
    );
    assert_eq!(
        view.source,
        Some("(chart not read)".to_string()),
        "an empty source_charts on a linked list must say so explicitly, not render as \
         an empty (falsy) string"
    );
}

/// #836: a line a synced event naming another patient's chart landed on says so on the row.
#[test]
fn a_conflict_line_says_another_patients_event_landed_on_it() {
    let mut r = row(
        1,
        MedicationStatus::Active,
        vec![member(1, VouchState::Absent)],
    );
    r.cross_patient = true;
    r.wrong_chart.patient_conflict = true;
    let flags = &build_view(&chart(vec![r.clone()])).rows[0].flags;
    let said = |flags: &[String], text: &str| flags.iter().any(|f| f.contains(text));
    assert!(
        said(flags, "recorded for another patient's chart"),
        "{flags:?}"
    );
    assert!(said(flags, "cannot be signed"), "{flags:?}");
    // The remedy is on the row itself: the report beside the list covers withheld lines only.
    assert!(
        said(flags, cairn_medication_view::CONFLICT_INSTRUCTION),
        "{flags:?}"
    );
    assert!(!said(flags, "in doubt"), "{flags:?}");

    // Ceased: no signature is owed and "stop this line" is impossible (the #717 lesson).
    r.status = MedicationStatus::Ceased;
    let flags = &build_view(&chart(vec![r])).rows[0].flags;
    assert!(
        said(flags, cairn_medication_view::CONFLICT_CEASED_INSTRUCTION),
        "{flags:?}"
    );
    assert!(!said(flags, "cannot be signed"), "{flags:?}");
}

/// PR #845 review: a hazard line signed before the hazard arose is in no report, so its remedy
/// must be on the row — for a line reaching outside the record, `chart_pane` may also have
/// hidden its Release (#844) — and it must not claim it "cannot be signed" (the #717 lesson).
#[test]
fn a_signed_hazard_line_carries_its_remedy_on_the_row() {
    let said = |flags: &[String], text: &str| flags.iter().any(|f| f.contains(text));
    let signed = |pick: fn(&mut cairn_medication_view::WrongChartReasons)| {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Fresh { by: "dr_b".into() })],
        );
        r.cross_patient = true;
        pick(&mut r.wrong_chart);
        build_view(&chart(vec![r])).rows[0].flags.clone()
    };
    let outside = signed(|w| w.outside_set = true);
    assert!(
        said(&outside, cairn_medication_view::SEPARATION_INSTRUCTION),
        "{outside:?}"
    );
    assert!(!said(&outside, "cannot be signed"), "{outside:?}");
    let conflict = signed(|w| w.patient_conflict = true);
    assert!(
        said(&conflict, cairn_medication_view::CONFLICT_INSTRUCTION),
        "{conflict:?}"
    );
    assert!(!said(&conflict, "cannot be signed"), "{conflict:?}");
}
