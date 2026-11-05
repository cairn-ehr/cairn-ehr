//! A note and all its later parts, grouped as ONE "compound" note (sign-in slice 3c-1).
//!
//! A multi-writer draft signs as one event per writer's part, each `addendum-to` the part before;
//! a clinician can also add to an existing note (an addendum naming the note's ROOT). The window
//! shows all of those as a single note. This module is the pure grouping — no database, no
//! window — so any UI can reuse it.
use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::read::NoteRow;
use super::view::NoteGrade;

/// One note with all its parts. `parts[0]` is the root; the rest follow in
/// `(recording_started, event_id)` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompoundNote {
    pub root: Uuid,
    /// The root's chart.
    pub chart: Uuid,
    /// The root names a note this read does not contain (withheld on a doubted linked chart, or
    /// kept off the screen by a newer profile ruling): it is shown as a root, marked.
    pub orphan: bool,
    pub parts: Vec<NoteRow>,
}

/// Group `notes` into compounds, newest root `t_effective` first (ties by root id, so the order
/// is deterministic). Every input row lands in exactly one compound.
pub fn compounds(notes: Vec<NoteRow>) -> Vec<CompoundNote> {
    let index: HashMap<Uuid, usize> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.event_id, i))
        .collect();
    // For each row: (root index, root is an orphan).
    let resolved: Vec<(usize, bool)> = (0..notes.len())
        .map(|start| {
            let mut at = start;
            // Bounded by the read's length: a cycle cannot be signed, but a bound costs nothing.
            for _ in 0..notes.len() {
                match notes[at].addendum_to {
                    None => return (at, false),
                    Some(target) => match index.get(&target) {
                        Some(&next) => at = next,
                        None => return (at, true), // target not in the read
                    },
                }
            }
            (start, true) // still unresolved after the bound: this row is its own root
        })
        .collect();
    let mut roots: Vec<(usize, bool)> = Vec::new();
    let mut seen_root: HashSet<usize> = HashSet::new();
    for &(r, orphan) in &resolved {
        if seen_root.insert(r) {
            roots.push((r, orphan));
        }
    }
    let part_of: Vec<usize> = resolved.iter().map(|&(r, _)| r).collect();
    let mut out: Vec<CompoundNote> = roots
        .iter()
        .map(|&(r, orphan)| {
            let mut rest: Vec<NoteRow> = (0..notes.len())
                .filter(|&i| i != r && part_of[i] == r)
                .map(|i| notes[i].clone())
                .collect();
            rest.sort_by_key(|n| (n.recording_started, n.event_id));
            let mut parts = vec![notes[r].clone()];
            parts.extend(rest);
            CompoundNote {
                root: notes[r].event_id,
                chart: notes[r].chart,
                orphan,
                parts,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.parts[0]
            .t_effective
            .cmp(&a.parts[0].t_effective)
            .then(a.root.cmp(&b.root))
    });
    out
}

/// What the parts' grades say about the note as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundState {
    OnePart,
    EverySigned,
    IncludesUnsigned,
    IncludesUnverified,
}

/// The compound's state. A one-part note is `OnePart` whatever its grade (its byline already
/// says it); an empty slice cannot come out of [`compounds`] and also reads `OnePart`.
pub fn compound_state(parts: &[NoteRow]) -> CompoundState {
    if parts.len() < 2 {
        return CompoundState::OnePart;
    }
    // Exhaustive, no `_` arm: a new grade must break the build here (#412).
    let (mut unsigned, mut unverified) = (false, false);
    for p in parts {
        match p.grade {
            NoteGrade::Signed { .. } => {}
            NoteGrade::Claimed => unverified = true,
            NoteGrade::WentInUnsigned | NoteGrade::WriterNotKnown => unsigned = true,
        }
    }
    if unsigned {
        CompoundState::IncludesUnsigned
    } else if unverified {
        CompoundState::IncludesUnverified
    } else {
        CompoundState::EverySigned
    }
}

/// The words the window shows for a state; `None` for a one-part note.
pub fn state_words(s: CompoundState) -> Option<&'static str> {
    match s {
        CompoundState::OnePart => None,
        CompoundState::EverySigned => Some("every part signed"),
        CompoundState::IncludesUnsigned => Some("includes an unsigned part"),
        CompoundState::IncludesUnverified => {
            Some("includes a part whose signature this node cannot verify")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::view::PartWriter;
    use chrono::{Duration, TimeZone, Utc};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// A row: event `n`, adding to `to`, recorded `rec` minutes after a fixed origin, effective
    /// `eff` minutes after it.
    fn row(n: u128, to: Option<u128>, rec: i64, eff: i64, grade: NoteGrade) -> NoteRow {
        let origin = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        NoteRow {
            event_id: id(n),
            chart: id(900),
            text: format!("t{n}"),
            t_effective: origin + Duration::minutes(eff),
            effective_basis: "stated".into(),
            recording_started: origin + Duration::minutes(rec),
            grade,
            addendum_to: to.map(id),
            writer: PartWriter::TheSigner,
        }
    }

    fn signed() -> NoteGrade {
        NoteGrade::Signed { by: "Dr A".into() }
    }

    fn ids(c: &CompoundNote) -> Vec<Uuid> {
        c.parts.iter().map(|p| p.event_id).collect()
    }

    #[test]
    fn a_note_without_addenda_is_one_compound() {
        let c = compounds(vec![row(1, None, 0, 0, signed())]);
        assert_eq!(c.len(), 1);
        assert_eq!(
            (c[0].root, c[0].orphan, c[0].parts.len()),
            (id(1), false, 1)
        );
        assert_eq!(c[0].chart, id(900));
    }

    #[test]
    fn a_multi_part_sign_is_one_compound_in_writing_order() {
        let c = compounds(vec![
            row(3, Some(2), 3, 0, signed()),
            row(2, Some(1), 2, 0, signed()),
            row(1, None, 1, 0, signed()),
        ]);
        assert_eq!(c.len(), 1);
        assert_eq!(ids(&c[0]), vec![id(1), id(2), id(3)]);
    }

    #[test]
    fn two_addenda_to_one_root_join_it_in_recording_order() {
        // Equal recording_started: ordered by event id.
        let c = compounds(vec![
            row(9, Some(1), 5, 0, signed()),
            row(1, None, 0, 0, signed()),
            row(4, Some(1), 5, 0, signed()),
            row(7, Some(1), 2, 0, signed()),
        ]);
        assert_eq!(c.len(), 1);
        assert_eq!(ids(&c[0]), vec![id(1), id(7), id(4), id(9)]);
    }

    #[test]
    fn an_addendum_whose_note_is_not_read_is_an_orphan_root() {
        let c = compounds(vec![
            row(2, Some(77), 0, 0, signed()),
            row(3, Some(2), 1, 0, signed()),
        ]);
        assert_eq!(c.len(), 1);
        assert!(c[0].orphan);
        assert_eq!(ids(&c[0]), vec![id(2), id(3)]);
    }

    #[test]
    fn compounds_are_newest_root_first() {
        let c = compounds(vec![
            row(1, None, 0, 10, signed()),
            row(2, None, 0, 30, signed()),
            row(3, Some(1), 0, 50, signed()), // a later part does not move its root
            row(4, None, 0, 20, signed()),
        ]);
        let roots: Vec<Uuid> = c.iter().map(|c| c.root).collect();
        assert_eq!(roots, vec![id(2), id(4), id(1)]);
    }

    #[test]
    fn a_cycle_is_bounded() {
        let c = compounds(vec![
            row(1, Some(2), 0, 0, signed()),
            row(2, Some(1), 1, 0, signed()),
        ]);
        let all: Vec<Uuid> = c.iter().flat_map(ids).collect();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&id(1)) && all.contains(&id(2)));
    }

    #[test]
    fn the_state_words_are_exact() {
        let w = |gs: Vec<NoteGrade>| {
            let parts: Vec<NoteRow> = gs
                .into_iter()
                .enumerate()
                .map(|(i, g)| row(i as u128 + 1, None, 0, 0, g))
                .collect();
            state_words(compound_state(&parts))
        };
        assert_eq!(w(vec![NoteGrade::Claimed]), None);
        assert_eq!(w(vec![signed(), signed()]), Some("every part signed"));
        assert_eq!(
            w(vec![signed(), NoteGrade::WentInUnsigned]),
            Some("includes an unsigned part")
        );
        assert_eq!(
            w(vec![NoteGrade::WriterNotKnown, NoteGrade::Claimed]),
            Some("includes an unsigned part")
        );
        assert_eq!(
            w(vec![signed(), NoteGrade::Claimed]),
            Some("includes a part whose signature this node cannot verify")
        );
    }
}
