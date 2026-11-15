//! [`merge`]: two components' checks of one combination line, as one answer. Split out of
//! `check.rs` (its parent module, whose types it uses) to keep that file under 500 lines.
use super::LineCheck;
use uuid::Uuid;

/// Two answers for ONE medication line, as one (charting slice 1a: a combination is checked
/// one component at a time, every part sharing the line's `group_id`). `a`'s words come first.
///
/// - A mark of `b` is kept unless its words are already there or its allergy record already has
///   a mark: one warning per allergy record, and `against` stays one record per mark.
/// - Notes are unioned by their words; a note kept keeps its record beside it, and the notes
///   with a record stay ahead of those without (`adverse_against`'s alignment rule), so a
///   record never slides onto another note's words.
/// - The worse accounts (#1131) are unioned whole, even beside a mark dropped as already said.
/// - "Not checked" if either part was, but only while there is no mark (ADR-0098's invariant:
///   the loud tag already sends the clinician to the allergy list).
/// - The threshold is `a`'s (the caller stamps it after merging).
pub fn merge(a: LineCheck, b: LineCheck) -> LineCheck {
    let mut out = a;
    for (mark, head) in b.marks.into_iter().zip(b.against) {
        if !out.marks.contains(&mark) && !out.against.contains(&head) {
            out.marks.push(mark);
            out.against.push(head);
        }
    }
    // Split each side into (note, record) pairs and record-less notes, union, then rejoin.
    let split = |notes: Vec<String>, records: Vec<Uuid>| {
        let mut notes = notes.into_iter();
        let paired: Vec<(String, Uuid)> = notes.by_ref().zip(records).collect();
        (paired, notes.collect::<Vec<String>>())
    };
    let (mut paired, mut bare) = split(std::mem::take(&mut out.notes), out.adverse_against);
    let (b_paired, b_bare) = split(b.notes, b.adverse_against);
    let said = |w: &String, p: &[(String, Uuid)], n: &[String]| {
        p.iter().any(|(x, _)| x == w) || n.contains(w)
    };
    for (note, record) in b_paired {
        if !said(&note, &paired, &bare) {
            paired.push((note, record));
        }
    }
    for note in b_bare {
        if !said(&note, &paired, &bare) {
            bare.push(note);
        }
    }
    (out.notes, out.adverse_against) = paired.into_iter().unzip();
    out.notes.extend(bare);
    out.worse_accounts.extend(b.worse_accounts);
    out.adverse_worse_accounts.extend(b.adverse_worse_accounts);
    out.unverified = out.marks.is_empty() && (out.unverified || b.unverified);
    out
}
