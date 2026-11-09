//! Pure tests for the thread resolver: which rows are one entry's versions, which version is
//! current, and which accounts differ. No database. The words a clinician reads are pinned in
//! `view_overlay_tests.rs`; only `ignored_overlays` (worded here) is pinned in this file.
use super::*;
use crate::allergy::read::KeyState;
use crate::allergy::test_fixtures::*;

/// The live heads' event ids, sorted.
fn live_ids(r: &Resolved) -> Vec<Uuid> {
    let mut v: Vec<Uuid> = r.live.iter().map(|h| h.row.event_id).collect();
    v.sort();
    v
}
fn head<'r, 'a>(r: &'r Resolved<'a>, n: u128) -> &'r Head<'a> {
    r.live.iter().find(|h| h.row.event_id == id(n)).unwrap()
}
fn earlier_ids(h: &Head) -> Vec<Uuid> {
    h.earlier.iter().map(|e| e.event_id).collect()
}

#[test]
fn same_allergen_ignores_case_and_spacing_not_spelling() {
    assert!(same_allergen("Penicillin  ", "penicillin"));
    assert!(same_allergen(
        " Co-Amoxiclav \t tablets",
        "co-amoxiclav tablets"
    ));
    assert!(same_allergen("ÉTHER", "éther"));
    assert!(!same_allergen("amoxycillin", "amoxicillin"));
    assert!(!same_allergen("penicillin", "penicillin v"));
}

#[test]
fn a_chain_of_corrections_shows_only_the_newest() {
    let p = pa(
        vec![
            pen(1, "rash", 8),
            correcting(pen(2, "urticaria", 9), 1),
            correcting(pen(3, "anaphylaxis", 10), 2),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(3)]);
    assert_eq!(earlier_ids(head(&r, 3)), vec![id(2), id(1)], "newest first");
    assert!(!head(&r, 3).differing);
}

/// Review Focus 5: two concurrent corrections of one line are both current, neither lost.
#[test]
fn two_corrections_of_one_line_are_two_accounts() {
    let p = pa(
        vec![
            pen(1, "rash", 8),
            correcting(pen(2, "rash", 9), 1),
            correcting(pen(3, "anaphylaxis", 9), 1),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(2), id(3)]);
    assert_eq!(earlier_ids(head(&r, 2)), vec![id(1)]);
    assert_eq!(earlier_ids(head(&r, 3)), vec![id(1)]);
    assert!(head(&r, 2).differing && head(&r, 3).differing);

    // The same two corrections agreeing on the reaction are two accounts that do not differ.
    let p = pa(
        vec![
            pen(1, "rash", 8),
            correcting(pen(2, "anaphylaxis", 9), 1),
            correcting(pen(3, "anaphylaxis", 9), 1),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(2), id(3)]);
    assert!(!head(&r, 2).differing && !head(&r, 3).differing);
}

/// Two concurrent corrections of one line that rename what it names are still competing
/// versions of ONE record: both are flagged, though the names no longer match (review finding:
/// a fork that fixed the spelling two ways, or turned one branch into an adverse effect, read as
/// two unrelated allergies with nothing saying they compete).
#[test]
fn a_fork_that_renames_the_allergen_is_still_differing_accounts() {
    let p = pa(
        vec![
            entry(1, "drug", "amoxicilin", "allergy", "rash", 8),
            correcting(entry(2, "drug", "amoxicillin", "allergy", "rash", 9), 1),
            correcting(
                entry(3, "drug", "amoxycillin", "adverse-effect", "rash", 9),
                1,
            ),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(2), id(3)]);
    assert!(head(&r, 2).differing && head(&r, 3).differing);

    // A fork whose branches differ only in what KIND of allergen it names.
    let p = pa(
        vec![
            pen(1, "rash", 8),
            correcting(entry(2, "drug", "penicillin", "allergy", "rash", 9), 1),
            correcting(entry(3, "other", "penicillin", "allergy", "rash", 9), 1),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert!(head(&r, 2).differing && head(&r, 3).differing);

    // Two unrelated entries that merely name different allergens are NOT differing accounts.
    let p = pa(
        vec![
            entry(1, "drug", "amoxicillin", "allergy", "rash", 8),
            entry(2, "drug", "aspirin", "adverse-effect", "gi-upset", 9),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert!(!head(&r, 1).differing && !head(&r, 2).differing);
}

/// A fork whose branches each grew: a head's `earlier` is its own line of versions, not the
/// other branch's.
#[test]
fn each_fork_head_keeps_only_its_own_earlier_versions() {
    let p = pa(
        vec![
            pen(1, "rash", 8),
            correcting(pen(2, "rash", 9), 1),
            correcting(pen(3, "rash", 10), 2),
            correcting(pen(4, "rash", 9), 1),
            correcting(pen(5, "rash", 11), 4),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(3), id(5)]);
    assert_eq!(earlier_ids(head(&r, 3)), vec![id(2), id(1)]);
    assert_eq!(earlier_ids(head(&r, 5)), vec![id(4), id(1)]);
}

/// D2: only a lenient remote door admits this; the reader treats it as a fresh entry
/// (over-warns: both lines stay).
#[test]
fn a_correction_naming_another_charts_entry_is_its_own_root() {
    let mut b = correcting(pen(2, "anaphylaxis", 9), 1);
    b.chart = id(2000);
    let p = pa(vec![pen(1, "rash", 8), b], vec![], vec![], vec![]);
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(1), id(2)]);
    assert!(head(&r, 1).earlier.is_empty() && head(&r, 2).earlier.is_empty());
}

/// D4: a since-repudiated key's correction supersedes nothing — the anaphylaxis it would have
/// downgraded stays, beside it.
#[test]
fn a_repudiated_keys_correction_is_its_own_account() {
    let mut b = correcting(
        entry(2, "drug", "penicillin", "adverse-effect", "gi-upset", 9),
        1,
    );
    b.key = KeyState::Repudiated;
    let p = pa(vec![pen(1, "anaphylaxis", 8), b], vec![], vec![], vec![]);
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(1), id(2)]);
    assert!(head(&r, 1).earlier.is_empty());
    assert!(head(&r, 1).differing && head(&r, 2).differing);

    // A standing key's correction OF the repudiated one does supersede it.
    let mut b = pen(2, "rash", 9);
    b.key = KeyState::Repudiated;
    let p = pa(
        vec![b, correcting(pen(3, "urticaria", 10), 2)],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(3)]);
}

/// A forged `corrects` cycle (or an entry correcting itself) must not make a thread vanish:
/// every member of a headless thread is a head.
#[test]
fn a_forged_cycle_keeps_every_member_on_the_list() {
    let p = pa(
        vec![
            correcting(pen(1, "rash", 8), 2),
            correcting(pen(2, "rash", 9), 1),
            correcting(entry(3, "drug", "latex", "allergy", "rash", 8), 3),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(1), id(2), id(3)]);
    assert!(r.live.iter().all(|h| h.earlier.is_empty()));
}

#[test]
fn not_known_never_differs_and_duplicates_do_not_differ() {
    let differs = |a, b| accounts_differ(&a, &b);
    assert!(!differs(pen(1, "not-known", 8), pen(2, "anaphylaxis", 8)));
    assert!(!differs(pen(1, "rash", 8), pen(2, "rash", 8)));
    assert!(differs(pen(1, "rash", 8), pen(2, "anaphylaxis", 8)));
    // An unknown kind or reaction word (a newer peer's) never differs.
    let intolerance = entry(2, "drug", "penicillin", "intolerance", "rash", 8);
    assert!(!differs(pen(1, "rash", 8), intolerance));
    assert!(!differs(pen(1, "weird", 8), pen(2, "rash", 8)));
    let ae_weird = entry(2, "drug", "penicillin", "adverse-effect", "weird", 8);
    assert!(!differs(pen(1, "rash", 8), ae_weird));

    let p = pa(
        vec![
            pen(1, "not-known", 8),
            pen(2, "anaphylaxis", 9),
            pen(3, "anaphylaxis", 9),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert!(r.live.iter().all(|h| !h.differing));
}

#[test]
fn an_allergy_and_an_adverse_effect_on_one_allergen_differ() {
    let mut ae = entry(2, "drug", "Penicillin ", "adverse-effect", "gi-upset", 9);
    ae.chart = id(2000);
    let p = pa(
        vec![
            pen(1, "rash", 8),
            ae,
            entry(3, "other", "latex", "allergy", "rash", 8),
        ],
        vec![],
        vec![],
        vec![],
    );
    let r = resolve(&p);
    assert!(head(&r, 1).differing && head(&r, 2).differing);
    assert!(!head(&r, 3).differing);
    // Kinds differing differ even when neither reaction is known.
    assert!(accounts_differ(
        &pen(1, "not-known", 8),
        &entry(2, "drug", "penicillin", "adverse-effect", "not-known", 8)
    ));
}

/// R-P4: a de-label or withdrawal filed on another chart than the record it names (only a
/// lenient remote door admits it) counts for nothing, and says so.
#[test]
fn an_overlay_naming_another_charts_event_is_ignored() {
    let mut c = challenge(5, 1, 9);
    c.chart = id(2000);
    let mut w = retraction(6, 1, 10);
    w.chart = id(2000);
    w.key = KeyState::Unconfirmed;
    let p = pa(vec![pen(1, "anaphylaxis", 8)], vec![], vec![c], vec![w]);
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(1)]);
    assert!(r.historical.is_empty() && r.withdrawn.is_empty());
    assert_eq!(
        r.ignored_overlays,
        vec![
            "Not counted: de-label (tolerated amoxicillin) by Dr Ada — names a record on another chart",
            "Not counted: entered in error by Dr Ada (signer unconfirmed) — names a record on another chart",
        ]
    );
}

/// D4: a since-repudiated key's de-label or withdrawal takes no warning away; it is listed.
#[test]
fn a_repudiated_keys_overlays_count_for_nothing() {
    let mut c = challenge(5, 1, 9);
    c.key = KeyState::Repudiated;
    let mut w = retraction(6, 2, 10);
    w.key = KeyState::Repudiated;
    w.signer = None;
    let p = pa(
        vec![
            pen(1, "anaphylaxis", 8),
            entry(2, "drug", "latex", "allergy", "rash", 8),
        ],
        vec![],
        vec![c],
        vec![w],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(1), id(2)]);
    assert_eq!(
        r.ignored_overlays,
        vec![
            "Not counted: de-label (tolerated amoxicillin) by Dr Ada (key had been repudiated)",
            // No name this node can give: the reason still shows.
            "Not counted: entered in error (key had been repudiated)",
        ]
    );
}

/// R-4a: status is per head — an overlay covers the head it names (and what that head
/// replaced), never a sibling branch; withdrawn outranks de-labelled.
#[test]
fn status_is_per_head_and_withdrawn_outranks_historical() {
    let p = pa(
        vec![
            pen(1, "rash", 8),
            correcting(pen(2, "rash", 9), 1),
            correcting(pen(3, "urticaria", 10), 1),
        ],
        vec![],
        vec![challenge(5, 1, 11), challenge(6, 2, 12)],
        vec![],
    );
    let r = resolve(&p);
    // The de-label of 1 names an ancestor only; 2's own de-label covers 2; 3 is untouched.
    assert_eq!(live_ids(&r), vec![id(3)]);
    assert_eq!(r.historical.len(), 1);
    let h = &r.historical[0];
    assert_eq!(h.head.event_id, id(2));
    assert_eq!(h.challenge.event_id, id(6));
    let ids: Vec<Uuid> = h.earlier.iter().map(|e| e.event_id).collect();
    assert_eq!(ids, vec![id(1)]);

    let mut p = p;
    p.retractions = vec![retraction(7, 2, 13)];
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(3)]);
    assert!(r.historical.is_empty());
    assert_eq!(r.withdrawn.len(), 1);
    assert_eq!(r.withdrawn[0].row.event_id, id(2));
    assert_eq!(r.withdrawn[0].by.event_id, id(7));
}

/// Review I1: a DOUBTFUL key (unconfirmed, and an approver of its enrolment since repudiated) is
/// suspect as a repudiated one is: its withdrawal of an entry and its correction take nothing
/// away, but its withdrawal of a de-label adds a warning, so it counts.
#[test]
fn a_doubtful_keys_overlays_count_only_to_add_a_warning() {
    let mut w = retraction(6, 1, 10);
    w.key = KeyState::Doubtful;
    let mut back = retraction(7, 5, 10);
    back.key = KeyState::Doubtful;
    let mut c = correcting(
        entry(3, "drug", "latex", "adverse-effect", "gi-upset", 9),
        2,
    );
    c.key = KeyState::Doubtful;
    let p = pa(
        vec![
            pen(1, "anaphylaxis", 8),
            entry(2, "drug", "latex", "allergy", "anaphylaxis", 8),
            c,
            entry(4, "drug", "codeine", "allergy", "rash", 8),
        ],
        vec![],
        vec![challenge(5, 4, 9)],
        vec![w, back],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(1), id(2), id(3), id(4)]);
    assert!(
        head(&r, 2).earlier.is_empty(),
        "the correction links to nothing"
    );
    assert!(r.withdrawn.is_empty() && r.historical.is_empty());
    assert_eq!(r.withdrawn_challenges.len(), 1, "the de-label is withdrawn");
    assert_eq!(
        r.ignored_overlays,
        vec![
            "Not counted: entered in error by Dr Ada (signer unconfirmed) \
             — its signer was enrolled by a key since repudiated"
        ]
    );
}

/// Final check (review, test gap 3): overlays that name another overlay are refused at the
/// authoring door but ADMITTED at the remote door, so they reach the fold. A withdrawal of a
/// withdrawal must NOT undo the first (ADR-0097 D3): honouring it would re-hide an allergy, a
/// warning taken away. A de-label naming a "none known" statement lifts nothing.
#[test]
fn an_overlay_naming_an_overlay_changes_nothing() {
    let p = pa(
        vec![pen(1, "anaphylaxis", 8)],
        vec![none(20, "drug", 8)],
        vec![challenge(5, 1, 9), challenge(8, 20, 9)],
        vec![retraction(6, 5, 10), retraction(7, 6, 11)],
    );
    let r = resolve(&p);
    assert_eq!(live_ids(&r), vec![id(1)], "the allergy is back in force");
    assert!(r.historical.is_empty());
    assert_eq!(r.withdrawn_challenges.len(), 1);
    assert_eq!(r.withdrawn_challenges[0].row.event_id, id(5));
    assert_eq!(r.withdrawn_challenges[0].by.event_id, id(6));
    assert!(
        r.withdrawn_none.is_empty(),
        "the none statement still stands"
    );
}

/// Final check (review, test gap 4): two de-labels of one line (two clinicians, perhaps across a
/// partition). The line rests on the NEWEST standing one — the one "Withdraw de-label" names —
/// and withdrawing it leaves the line historical on the older one until that is withdrawn too.
/// (That the window does not yet say a second de-label stands is #1048.)
#[test]
fn a_line_with_two_delabels_rests_on_the_newest_standing_one() {
    let resolve_with = |retractions: Vec<RetractionRow>| {
        let p = pa(
            vec![pen(1, "anaphylaxis", 8)],
            vec![],
            vec![challenge(5, 1, 9), challenge(6, 1, 10)],
            retractions,
        );
        let r = resolve(&p);
        (
            live_ids(&r),
            r.historical
                .iter()
                .map(|h| h.challenge.event_id)
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(resolve_with(vec![]), (vec![], vec![id(6)]));
    assert_eq!(
        resolve_with(vec![retraction(7, 6, 11)]),
        (vec![], vec![id(5)])
    );
    assert_eq!(
        resolve_with(vec![retraction(7, 6, 11), retraction(8, 5, 12)]),
        (vec![id(1)], vec![])
    );
}
