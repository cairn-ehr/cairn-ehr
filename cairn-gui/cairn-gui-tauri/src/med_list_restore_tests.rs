//! Pure tests for `med_list_restore.rs`: the restore line's prefill, what its Sign stops, its
//! refusal when the version is gone, and the source and press it signs. No database; the
//! DB-gated walks are `med_list_restore_db_tests.rs`.
use super::*;
use cairn_event::medication::ALLERGY_SOURCE;
use cairn_node::allergy::check::MOIETY_SYSTEM;
use cairn_node::medication::line::read_directions;

const COMPANION: u128 = 0xC0;
const VERSION: u128 = 0x7E;

/// A version of metoprolol 25 mg po as a GP letter reported it, its companion current.
fn version(sig: Option<&str>) -> VersionForRestore {
    VersionForRestore {
        alternative_to: Uuid::from_u128(COMPANION),
        companion_current: true,
        standing: vec![Uuid::from_u128(COMPANION)],
        standing_restored: vec![],
        companion_charted_as: Some("started-here".into()),
        source: "GP letter".into(),
        term: "metoprolol".into(),
        coding: Some((
            MOIETY_SYSTEM.into(),
            Uuid::from_u128(0x3E70).to_string(),
            "metoprolol".into(),
        )),
        formulation: None,
        dose_amount: Some("25".into()),
        dose_unit: Some("mg".into()),
        sig: sig.map(Into::into),
        route: Some("po".into()),
        strength: None,
        restored: false,
    }
}

/// A stored list line, restoring `restores` when given.
fn line(restores: Option<u128>, info_source: Option<&str>) -> DraftLine {
    DraftLine {
        line_id: Uuid::from_u128(0x11),
        terminal_id: "pc-1".into(),
        seq: 1,
        charted_as: "started-here".into(),
        line_text: "metoprolol 25mg po bd".into(),
        parts: serde_json::json!([]),
        reason: None,
        hand_ticked: false,
        ticked_against: vec![],
        finished: true,
        saved_at: "2026-11-14 10:00:00+00".into(),
        typed_drug: "metoprolol".into(),
        directions: "25mg po bd".into(),
        held: false,
        info_source: info_source.map(Into::into),
        source_detail: None,
        why: None,
        set_aside_of: None,
        restores: restores.map(Uuid::from_u128),
    }
}

/// The prefill reads back, through the reader Sign uses, as the version's dose, route and sig:
/// a sig typed in this window already holds them and is taken as it was typed (never doubled,
/// which would read as two doses); a sig without them is completed from the version's fields.
#[test]
fn the_prefill_reads_back_as_the_versions_dose_sig_and_route() {
    for (sig, expected) in [
        (Some("25mg po bd"), "25mg po bd"),
        (Some("bd"), "25 mg po bd"),
        (None, "25 mg po"),
    ] {
        let v = version(sig);
        let (typed_drug, directions, line_text, parts) = restore_prefill(&v);
        assert_eq!(typed_drug, "metoprolol");
        assert_eq!(directions, expected, "{sig:?}");
        assert_eq!(line_text, format!("metoprolol {expected}"));
        let read = read_directions(&directions);
        assert_eq!(read.dose_amount.as_deref(), Some("25"), "{sig:?}");
        assert_eq!(read.dose_unit.as_deref(), Some("mg"), "{sig:?}");
        assert_eq!(read.route.as_deref(), Some("po"), "{sig:?}");
        if sig.is_some() {
            assert_eq!(read.frequency.as_deref(), Some("twice daily"), "{sig:?}");
        }
        assert!(read.unread.is_empty(), "{sig:?}: {:?}", read.unread);
        let parts: Vec<PartArg> = serde_json::from_value(parts).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].term, "metoprolol");
        let coding = parts[0].coding.as_ref().expect("the version's coding");
        assert_eq!(coding.system, MOIETY_SYSTEM);
        assert_eq!(coding.code, Uuid::from_u128(0x3E70).to_string());
    }
}

/// R3 + R4: a restore stops every line standing for the companion on the fresh read, each with
/// the plan's reason; none when nothing stands; an ordinary line is no restore at all.
#[test]
fn restore_to_sign_stops_only_a_current_companion() {
    let reason = replaced_by("GP letter");
    assert_eq!(reason, "replaced by the GP letter version");
    let restoring = line(Some(VERSION), None);
    let v = version(Some("bd"));
    let r = restore_to_sign(&restoring, Some(&v), &reason)
        .unwrap()
        .expect("a restore");
    assert_eq!(r.version_id, Uuid::from_u128(VERSION));
    assert_eq!(r.stop, [(Uuid::from_u128(COMPANION), reason.as_str())]);
    let stopped = VersionForRestore {
        companion_current: false,
        standing: vec![],
        ..v.clone()
    };
    let r = restore_to_sign(&restoring, Some(&stopped), &reason)
        .unwrap()
        .expect("still a restore");
    assert!(
        r.stop.is_empty(),
        "a stopped companion with nothing standing"
    );
    let replaced = VersionForRestore {
        companion_current: false,
        standing: vec![Uuid::from_u128(0xB1)],
        ..v.clone()
    };
    let r = restore_to_sign(&restoring, Some(&replaced), &reason)
        .unwrap()
        .unwrap();
    assert_eq!(r.stop, [(Uuid::from_u128(0xB1), reason.as_str())]);
    assert!(restore_to_sign(&line(None, None), None, &reason)
        .unwrap()
        .is_none());
}

/// Review Focus 4: a restore whose version is not on the opened chart's fresh read is refused
/// for that line alone, with the plan's words.
#[test]
fn a_gone_version_refuses_the_line() {
    assert_eq!(
        restore_to_sign(&line(Some(VERSION), None), None, "").err(),
        Some(RESTORE_VERSION_GONE)
    );
    assert_eq!(
        RESTORE_VERSION_GONE,
        "the version it restores can no longer be read"
    );
    assert_eq!(
        VERSION_GONE,
        "That version is no longer on this chart; read it again"
    );
}

/// The maintainer's ruling: a restore line naming no source of its own signs `unknown`, never
/// the press's default; one that names a source keeps it; an ordinary line keeps the press's.
#[test]
fn a_restore_line_with_no_source_signs_unknown() {
    assert_eq!(
        restore_source(&line(Some(VERSION), None), "clinician-observed"),
        "unknown"
    );
    let own = line(Some(VERSION), Some("external-record"));
    assert_eq!(restore_source(&own, "external-record"), "external-record");
    assert_eq!(
        restore_source(&line(None, None), "clinician-observed"),
        "clinician-observed"
    );
}

/// The maintainer's ruling: the restored line takes the companion's press; a companion charted
/// before slice 1a (none) gives `pre-existing`.
#[test]
fn the_restore_line_inherits_the_companions_charted_as() {
    let v = version(None);
    assert_eq!(restore_charted_as(&v), "started-here");
    let old = VersionForRestore {
        companion_charted_as: None,
        ..v
    };
    assert_eq!(restore_charted_as(&old), "pre-existing");
}

/// Ruling R11: the wire admits `unknown` and a peer may send any word, but a list line takes only
/// the two presses (db/078's CHECK), so anything but `started-here` restores as `pre-existing`.
#[test]
fn a_companion_charted_unknown_or_any_peer_word_restores_as_pre_existing() {
    for word in ["unknown", "pre-existing", "a-peers-new-word", ""] {
        let v = VersionForRestore {
            companion_charted_as: Some(word.into()),
            ..version(None)
        };
        assert_eq!(restore_charted_as(&v), "pre-existing", "{word:?}");
    }
}

/// Ruling R9 (a): among the ticked lines in Sign order, a restore of a version whose companion an
/// EARLIER ticked restore line also replaces is refused alone, legibly; the first is not.
#[test]
fn a_later_restore_of_the_same_line_is_refused() {
    let mut versions = Versions::new();
    versions.insert(Uuid::from_u128(VERSION), version(None));
    versions.insert(Uuid::from_u128(0x7F), version(None));
    let first = line(Some(VERSION), None);
    let mut second = line(Some(0x7F), None);
    second.line_id = Uuid::from_u128(0x12);
    assert_eq!(restore_holds(&first, [], &versions), Ok(()));
    assert_eq!(
        restore_holds(&second, [&first], &versions),
        Err(SAME_LINE_RESTORED)
    );
    assert_eq!(
        SAME_LINE_RESTORED,
        "another restore of the same line is signed first; signed after it, this one replaces it"
    );
    let ordinary = line(None, None);
    assert_eq!(restore_holds(&ordinary, [&first], &versions), Ok(()));
    let mut other = version(None);
    other.alternative_to = Uuid::from_u128(0xC1);
    other.standing = vec![Uuid::from_u128(0xC1)];
    versions.insert(Uuid::from_u128(0x7F), other);
    assert_eq!(restore_holds(&second, [&first], &versions), Ok(()));
    assert_eq!(
        restore_holds(&line(Some(0x99), None), [], &versions),
        Err(RESTORE_VERSION_GONE)
    );
}

/// Ruling R9 (b): a restore replacing a line that is itself a restore is never default-ticked;
/// one replacing the original line may be.
#[test]
fn a_restore_replacing_a_restore_withholds_the_default_tick() {
    let mut versions = Versions::new();
    versions.insert(Uuid::from_u128(VERSION), version(None));
    let restoring = line(Some(VERSION), None);
    assert!(!replaces_a_restore(&restoring, &versions));
    let replaced = VersionForRestore {
        standing: vec![Uuid::from_u128(0xB1)],
        standing_restored: vec![Uuid::from_u128(0xB1)],
        ..version(None)
    };
    versions.insert(Uuid::from_u128(VERSION), replaced);
    assert!(replaces_a_restore(&restoring, &versions));
    assert!(!replaces_a_restore(&line(None, None), &versions));
}

/// The restore door's words are never shown raw.
#[test]
fn a_refused_add_is_said_in_words() {
    for word in [
        "elsewhere",
        "moved",
        "no-such-writer",
        "deleted",
        "no-chart",
        "surprise",
    ] {
        let said = not_added(word);
        assert!(!said.contains(word), "{word}: {said}");
        assert!(said.starts_with("The line was not added"), "{said}");
    }
    assert_eq!(not_added("no-such-version"), VERSION_GONE);
}

/// Ruling R10: two restores conflict when they would stop a line in common even under different
/// companions (a version set aside beside a restore line, beside a sibling of the original's);
/// two restores of a stopped companion (both standing empty) still conflict by companion; two
/// restores of different lines with nothing in common do not.
#[test]
fn restores_of_one_lineage_conflict_through_what_they_stop() {
    let l1 = Uuid::from_u128(0xB1);
    let under = |companion: u128, standing: Vec<Uuid>| VersionForRestore {
        alternative_to: Uuid::from_u128(companion),
        standing,
        ..version(None)
    };
    let mut versions = Versions::new();
    versions.insert(Uuid::from_u128(VERSION), under(COMPANION, vec![l1]));
    versions.insert(Uuid::from_u128(0x7F), under(0xB1, vec![l1]));
    let sibling = line(Some(VERSION), None);
    let mut chain = line(Some(0x7F), None);
    chain.line_id = Uuid::from_u128(0x12);
    assert_eq!(
        restore_holds(&chain, [&sibling], &versions),
        Err(SAME_LINE_RESTORED)
    );
    versions.insert(Uuid::from_u128(VERSION), under(COMPANION, vec![]));
    versions.insert(Uuid::from_u128(0x7F), under(COMPANION, vec![]));
    assert_eq!(
        restore_holds(&chain, [&sibling], &versions),
        Err(SAME_LINE_RESTORED),
        "a stopped companion: equal companions still conflict"
    );
    versions.insert(
        Uuid::from_u128(0x7F),
        under(0xC2, vec![Uuid::from_u128(0xC2)]),
    );
    assert_eq!(restore_holds(&chain, [&sibling], &versions), Ok(()));
}

/// Ruling R12: a restore whose line was stopped since the press (nothing stands for its
/// companion) is never default-ticked, and its mark says so; one whose line stands is neither.
#[test]
fn a_restore_whose_line_was_stopped_since_withholds_the_tick_and_says_so() {
    let mut versions = Versions::new();
    versions.insert(Uuid::from_u128(VERSION), version(None));
    let restoring = line(Some(VERSION), None);
    assert!(!restores_a_stopped_line(&restoring, &versions));
    assert_eq!(
        restores_words(&versions[&Uuid::from_u128(VERSION)]),
        "restores the GP letter version"
    );
    let stopped = VersionForRestore {
        companion_current: false,
        standing: vec![],
        ..version(None)
    };
    assert_eq!(
        restores_words(&stopped),
        "restores the GP letter version — its line has since been stopped"
    );
    versions.insert(Uuid::from_u128(VERSION), stopped);
    assert!(restores_a_stopped_line(&restoring, &versions));
    assert!(!restores_a_stopped_line(&line(None, None), &versions));
    assert!(!restores_a_stopped_line(&line(Some(0x99), None), &versions));
}

/// A restore line's hand tick vouches for what stood for its companion when it was ticked (the
/// refs drawn in its `against`, sent back as `seen`): it holds at Sign while the fresh standing
/// is exactly that set, and fails (so Sign hands it back unticked) once a colleague's stop or
/// restore changed it, never signing a stop or restart the clinician did not see. A tick given
/// AFTER the "since been stopped" mark vouches for the empty set, and is a deliberate restart.
#[test]
fn a_restore_hand_tick_holds_only_over_the_standing_it_was_given_against() {
    let mut versions = Versions::new();
    versions.insert(Uuid::from_u128(VERSION), version(None));
    let mut restoring = line(Some(VERSION), None);
    restoring.hand_ticked = true;
    let drawn = restore_seen(&restoring, &versions);
    assert_eq!(
        drawn.len(),
        2,
        "the version and its one standing line: {drawn:?}"
    );
    restoring.ticked_against = restore_vouched(&drawn);
    assert!(restore_tick_holds(&restoring, &versions));

    // A colleague stops the companion: nothing stands, so the tick no longer holds.
    let stopped = VersionForRestore {
        companion_current: false,
        standing: vec![],
        ..version(None)
    };
    versions.insert(Uuid::from_u128(VERSION), stopped.clone());
    assert!(!restore_tick_holds(&restoring, &versions));

    // A colleague restores a sibling version: a restore line stands now instead.
    let replaced = VersionForRestore {
        standing: vec![Uuid::from_u128(0xB1)],
        standing_restored: vec![Uuid::from_u128(0xB1)],
        ..version(None)
    };
    versions.insert(Uuid::from_u128(VERSION), replaced);
    assert!(!restore_tick_holds(&restoring, &versions));

    // Ticked again with the mark drawn: a deliberate restart holds.
    versions.insert(Uuid::from_u128(VERSION), stopped);
    restoring.ticked_against = restore_vouched(&restore_seen(&restoring, &versions));
    assert!(restore_tick_holds(&restoring, &versions));

    // A tick that vouched for nothing of the restore (no marker) never holds. The version gone,
    // and an ordinary line, are not this rule's business: Sign refuses the first line alone.
    restoring.ticked_against = vec![];
    assert!(!restore_tick_holds(&restoring, &versions));
    assert!(restore_tick_holds(&restoring, &Versions::new()));
    assert!(restore_tick_holds(&line(None, None), &versions));
}

/// Only the restore's own refs are kept from `seen`: allergy refs are `vouched`'s, and a ref
/// that is not a uuid is dropped (it vouches for nothing).
#[test]
fn restore_vouched_keeps_only_the_restore_refs() {
    let seen = vec![
        WarningRef {
            source: ALLERGY_SOURCE.into(),
            reference: Uuid::from_u128(1).to_string(),
        },
        WarningRef {
            source: RESTORE_OF.into(),
            reference: Uuid::from_u128(2).to_string(),
        },
        WarningRef {
            source: RESTORE_STANDS.into(),
            reference: "not-a-uuid".into(),
        },
    ];
    assert_eq!(
        restore_vouched(&seen),
        vec![(RESTORE_OF.to_string(), Uuid::from_u128(2))]
    );
}

/// The default tick is withheld from a restore line that replaces a restore (R9b), restores a
/// stopped line (R12), or whose version no longer reads (Sign would refuse it); an ordinary line
/// and a plain restore are not withheld.
#[test]
fn the_default_tick_is_withheld_from_a_restore_sign_would_not_take_as_seen() {
    let mut versions = Versions::new();
    versions.insert(Uuid::from_u128(VERSION), version(None));
    let restoring = line(Some(VERSION), None);
    assert!(!restore_withholds_default_tick(&restoring, &versions));
    assert!(!restore_withholds_default_tick(
        &line(None, None),
        &versions
    ));
    assert!(restore_withholds_default_tick(&restoring, &Versions::new()));
    let stopped = VersionForRestore {
        standing: vec![],
        ..version(None)
    };
    versions.insert(Uuid::from_u128(VERSION), stopped);
    assert!(restore_withholds_default_tick(&restoring, &versions));
    let replaced = VersionForRestore {
        standing: vec![Uuid::from_u128(0xB1)],
        standing_restored: vec![Uuid::from_u128(0xB1)],
        ..version(None)
    };
    versions.insert(Uuid::from_u128(VERSION), replaced);
    assert!(restore_withholds_default_tick(&restoring, &versions));
}

/// A press on a version restored since the pane was drawn is refused in words (the pane offers
/// neither settle act on a restored version); one not restored is not.
#[test]
fn a_version_restored_since_is_refused() {
    assert_eq!(not_restored(&version(None)), Ok(()));
    let restored = VersionForRestore {
        restored: true,
        ..version(None)
    };
    assert_eq!(not_restored(&restored), Err(VERSION_RESTORED));
}

/// An add that landed answers Ok even when the list could not be read again: an error would
/// offer the press again, and a second press adds a second line.
#[test]
fn a_landed_add_whose_reread_failed_still_answers_ok() {
    let line_id = Uuid::from_u128(5);
    let opened = restore_opened(Err(refused("the lock was lost")), line_id);
    assert!(opened.list.is_none());
    assert_eq!(opened.line_id, line_id.to_string());
    let said = opened.said.expect("words for the failed read");
    assert!(
        said.contains("was added") && said.contains("the lock was lost"),
        "{said}"
    );
}

/// Review Focus 2: a restore line is judged against the chart without the rows made WHOLLY of
/// what it will stop; a row that also holds a thread it will not stop stays (still current after
/// Sign), and so does a row with no members.
#[test]
fn a_restore_is_judged_without_only_the_rows_it_wholly_stops() {
    use cairn_medication_view::fixtures::sample_chart;
    use cairn_medication_view::{MedicationStatus, MemberVouch};
    let row = |code: u128, members: &[u128]| {
        let mut r = sample_chart().rows[0].clone();
        r.status = MedicationStatus::Active;
        r.coding_system = Some(MOIETY_SYSTEM.into());
        r.coding_code = Some(Uuid::from_u128(code).to_string());
        r.coding_conflict = false;
        let vouch = r.members[0].clone();
        r.members = members
            .iter()
            .map(|m| MemberVouch {
                medication_id: Uuid::from_u128(*m),
                ..vouch.clone()
            })
            .collect();
        r
    };
    let rows = [
        row(1, &[COMPANION]),
        row(2, &[COMPANION, 0xD1]),
        row(3, &[]),
    ];
    let mut versions = Versions::new();
    versions.insert(Uuid::from_u128(VERSION), version(None));
    let restoring = line(Some(VERSION), None);
    let kept = current_besides_stops(&rows, &[restoring.clone(), line(None, None)], &versions);
    let codes: Vec<String> = kept[&restoring.line_id]
        .iter()
        .map(|(c, _)| c.clone())
        .collect();
    assert_eq!(
        codes,
        [2, 3].map(|c| Uuid::from_u128(c).to_string()),
        "only the row wholly stopped is left out"
    );
    assert_eq!(
        kept.len(),
        1,
        "an ordinary line is judged against the whole chart"
    );
}
