//! Folds a patient's allergy rows into THREADS and decides what each head says now (allergy
//! slice 2). Pure: no database, no clock, no words a clinician reads except `ignored_overlays`.
//!
//! Nothing in the allergy record is ever edited. A correction is a NEW entry naming the line it
//! replaces (`AllergyRow::corrects`); a de-label (`ChallengeRow`) or an entered-in-error
//! withdrawal (`RetractionRow`) is a new event naming its target. So the current state is a fold
//! over every row, and this module is that fold. Reading order for a junior, matching
//! [`resolve`]'s steps:
//!
//! 1. **Which overlays count.** A SUSPECT overlay — signed by a since-repudiated or doubtful
//!    key (plan D4, review I1), or filed on another chart than the record it names (D2; only a
//!    lenient remote door admits it) — counts only when it ADDS a warning (`overlay_counts`,
//!    ruling R-4b). The rest are listed in `ignored_overlays`, with why.
//! 2. **Which de-labels stand**: those no counted withdrawal names (M3: withdrawing a de-label
//!    puts the allergy back).
//! 3. **Threads.** An entry and every correction of it, transitively, are one thread. A
//!    suspect key's correction, or one naming another chart's entry, links to nothing: it is
//!    an account of its own and supersedes nothing (D4, D2).
//! 4. **Heads**: the members no other member corrects (two corrections of one line are two
//!    heads, M2). Each head's `earlier` is the line of versions it replaced.
//! 5. **Status, per head** (ruling R-4a): an overlay covers the version it NAMES and the versions
//!    that one replaced, never a later correction of it. So a head is *withdrawn* if a counted
//!    withdrawal names it, else *historical* if a standing de-label names it, else *live*. An
//!    overlay naming an older version only hides that version under a live head's `earlier`: a
//!    correction signed concurrently with (or after) a withdrawal or de-label of its target is
//!    never struck by it.
//! 6. **Differing accounts**: two live heads of the same allergen ([`same_allergen`]) that
//!    disagree ([`accounts_differ`]) are both marked.
//!
//! Every tie breaks on (HLC, event id), so the fold is deterministic on every node.
use super::read::{AllergyRow, ChallengeRow, KeyState, NoneRow, PatientAllergies, RetractionRow};
use cairn_event::allergy::{EntryKind, ReactionCategory};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// The byline marker of an unconfirmed key, the note view's words (`note::view::signer_words`).
pub const SIGNER_UNCONFIRMED: &str = " (signer unconfirmed)";
/// The byline marker of a since-repudiated key, the note view's words.
pub const KEY_REPUDIATED: &str = " (key had been repudiated)";

/// One current account of an allergy: a thread's head.
#[derive(Debug)]
pub struct Head<'a> {
    pub row: &'a AllergyRow,
    /// The versions this head replaced, newest first.
    pub earlier: Vec<&'a AllergyRow>,
    /// Another live head of the same allergen disagrees with this one.
    pub differing: bool,
}

/// A de-labelled head.
#[derive(Debug)]
pub struct Historical<'a> {
    pub head: &'a AllergyRow,
    /// The versions this head replaced, newest first.
    pub earlier: Vec<&'a AllergyRow>,
    /// The newest standing de-label naming the head.
    pub challenge: &'a ChallengeRow,
}

/// A withdrawn head: the entry, the versions it replaced (newest first), and the withdrawal.
#[derive(Debug)]
pub struct WithdrawnEntry<'a> {
    pub row: &'a AllergyRow,
    pub earlier: Vec<&'a AllergyRow>,
    pub by: &'a RetractionRow,
}

/// A row struck by a counted withdrawal, with the withdrawal (its day and signer are worded).
#[derive(Debug)]
pub struct Withdrawn<'a, T> {
    pub row: &'a T,
    pub by: &'a RetractionRow,
}

/// The fold's answer. Every list is newest first except `live` (the view sorts it) and
/// `ignored_overlays` (in the read's order).
#[derive(Debug)]
pub struct Resolved<'a> {
    pub live: Vec<Head<'a>>,
    pub historical: Vec<Historical<'a>>,
    /// Withdrawn heads, each with the newest counted withdrawal naming it.
    pub withdrawn: Vec<WithdrawnEntry<'a>>,
    /// Withdrawn de-labels (their allergy is back in force unless another de-label stands).
    pub withdrawn_challenges: Vec<Withdrawn<'a, ChallengeRow>>,
    /// Withdrawn "none known" statements: they stand for no list.
    pub withdrawn_none: Vec<&'a NoneRow>,
    /// "Not counted: …" lines for the overlays step 1 set aside.
    pub ignored_overlays: Vec<String>,
    /// The newest COUNTED withdrawal naming each event id (an entry, de-label or "none known"
    /// statement), so the view can say it on an older version's "Earlier:" line (ruling R-4c).
    pub withdrawals: HashMap<Uuid, &'a RetractionRow>,
    /// The newest STANDING de-label naming each entry, for the same reason.
    pub de_labels: HashMap<Uuid, &'a ChallengeRow>,
}

/// The marker a byline carries for a key's standing (empty when nothing is in doubt).
pub fn key_marker(key: KeyState) -> &'static str {
    match key {
        KeyState::Standing => "",
        KeyState::Unconfirmed | KeyState::Doubtful => SIGNER_UNCONFIRMED,
        KeyState::Repudiated => KEY_REPUDIATED,
    }
}

/// `" by Dr Ada"` plus the key's marker. With no name this node can give, the marker alone: a
/// reader must still see why a signature carries no weight.
pub fn by_words(signer: Option<&str>, key: KeyState) -> String {
    let name = signer.map(|s| format!(" by {s}")).unwrap_or_default();
    format!("{name}{}", key_marker(key))
}

/// Plan D5: two terms name the same allergen when they match after trimming, collapsing inner
/// whitespace and Unicode lowercasing. Spelling is not forgiven ("amoxycillin" is not
/// "amoxicillin"); coded equality is slice 3's.
pub fn same_allergen(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// Plan D5: two accounts (of the same allergen) differ when their kinds differ (allergy vs
/// adverse effect), or when both reactions are known and neither is `not-known` and they
/// differ. A row carrying a kind or reaction word this build does not know never differs, and the
/// same account twice does not.
pub fn accounts_differ(a: &AllergyRow, b: &AllergyRow) -> bool {
    let (Some(ka), Some(kb)) = (EntryKind::parse(&a.kind), EntryKind::parse(&b.kind)) else {
        return false;
    };
    let (Some(ca), Some(cb)) = (
        ReactionCategory::parse(&a.category),
        ReactionCategory::parse(&b.category),
    ) else {
        return false;
    };
    let not_known = ReactionCategory::NotKnown;
    ka != kb || (ca != cb && ca != not_known && cb != not_known)
}

/// What kind of row an overlay names, as far as this node can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    Entry,
    NoneStatement,
    Challenge,
    /// Not a row this node holds readably (unreadable, or a peer's newer type).
    Unknown,
}

/// Why an overlay is SUSPECT, as the end of its "Not counted" line; `None` when it is not.
/// `rows` maps every readable row's event id to its chart and kind. A repudiated key's reason is
/// its byline marker, already in the line, so it adds nothing here; a doubtful key's marker is
/// an unconfirmed one's (which counts), so its reason is said.
fn suspicion(
    rows: &HashMap<Uuid, (Uuid, Named)>,
    key: KeyState,
    own_chart: Uuid,
    target: Uuid,
) -> Option<&'static str> {
    if key == KeyState::Repudiated {
        Some("")
    } else if key == KeyState::Doubtful {
        Some(" — its signer was enrolled by a key since repudiated")
    } else if rows.get(&target).is_some_and(|(c, _)| *c != own_chart) {
        Some(" — names a record on another chart")
    } else {
        None
    }
}

/// Ruling R-4b: does an overlay count? A trusted one always does. A SUSPECT one (repudiated key,
/// or naming another chart's record) counts only when it ADDS a warning: a withdrawal of a
/// de-label (the allergy is back) or of a "none known" statement (the reassurance is gone). A
/// suspect de-label, or a suspect withdrawal of an entry, would TAKE a warning away on the word of
/// a key nobody can vouch for, so it counts for nothing. Over-warning is the safe error.
fn overlay_counts(suspect: bool, is_withdrawal: bool, named: Named) -> bool {
    let adds_a_warning = is_withdrawal && matches!(named, Named::Challenge | Named::NoneStatement);
    !suspect || adds_a_warning
}

/// The (HLC, event id) sort key every "newest" in this module uses.
fn age(hlc: (i64, i32), id: Uuid) -> ((i64, i32), Uuid) {
    (hlc, id)
}

fn challenge_age(c: &ChallengeRow) -> ((i64, i32), Uuid) {
    age(c.hlc, c.event_id)
}

fn retraction_age(w: &RetractionRow) -> ((i64, i32), Uuid) {
    age(w.hlc, w.event_id)
}

/// Sort rows newest first.
fn newest_first(rows: &mut [&AllergyRow]) {
    rows.sort_by_key(|r| std::cmp::Reverse(age(r.hlc, r.event_id)));
}

/// The entry `e` replaces, when that link counts (step 3): not from a suspect (repudiated or
/// doubtful) key, not to itself, and to an entry on `e`'s own chart.
fn link<'a>(e: &AllergyRow, by_id: &HashMap<Uuid, &'a AllergyRow>) -> Option<&'a AllergyRow> {
    let t = e
        .corrects
        .filter(|t| *t != e.event_id && !e.key.is_suspect())?;
    by_id.get(&t).copied().filter(|t| t.chart == e.chart)
}

/// Union-find root, the smallest event id of the set (so a thread's key is deterministic).
fn root(parent: &HashMap<Uuid, Uuid>, mut x: Uuid) -> Uuid {
    while parent[&x] != x {
        x = parent[&x];
    }
    x
}

/// Steps 3 and 4 for every entry: every thread's heads, threads in order of their smallest
/// event id, each head with its `earlier`.
fn heads(entries: &[AllergyRow]) -> Vec<Head<'_>> {
    let by_id: HashMap<Uuid, &AllergyRow> = entries.iter().map(|e| (e.event_id, e)).collect();
    // Union-find over the counted links; the smaller root wins, so a root is its set's smallest id.
    let mut parent: HashMap<Uuid, Uuid> = by_id.keys().map(|k| (*k, *k)).collect();
    for e in entries {
        if let Some(t) = link(e, &by_id) {
            let (a, b) = (root(&parent, e.event_id), root(&parent, t.event_id));
            parent.insert(a.max(b), a.min(b));
        }
    }
    let replaced: HashSet<Uuid> = entries
        .iter()
        .filter_map(|e| link(e, &by_id))
        .map(|t| t.event_id)
        .collect();
    let mut sets: BTreeMap<Uuid, Vec<&AllergyRow>> = BTreeMap::new();
    for e in entries {
        sets.entry(root(&parent, e.event_id)).or_default().push(e);
    }
    let mut out = Vec::new();
    for members in sets.into_values() {
        let mut heads: Vec<&AllergyRow> = members
            .iter()
            .copied()
            .filter(|m| !replaced.contains(&m.event_id))
            .collect();
        // A forged cycle leaves no member that nothing corrects: show every member rather than
        // lose the allergy (over-warning is the safe error).
        if heads.is_empty() {
            heads = members;
        }
        let head_ids: HashSet<Uuid> = heads.iter().map(|h| h.event_id).collect();
        out.extend(heads.into_iter().map(|row| Head {
            row,
            earlier: earlier_of(row, &by_id, &head_ids),
            differing: false,
        }));
    }
    out
}

/// The versions `head` replaced: walk back along its links, newest first, stopping at another
/// head or a version already seen (only a forged cycle can bring either).
fn earlier_of<'a>(
    head: &AllergyRow,
    by_id: &HashMap<Uuid, &'a AllergyRow>,
    heads: &HashSet<Uuid>,
) -> Vec<&'a AllergyRow> {
    let mut earlier: Vec<&AllergyRow> = Vec::new();
    let mut at = link(head, by_id);
    while let Some(e) = at {
        if heads.contains(&e.event_id) || earlier.iter().any(|x| x.event_id == e.event_id) {
            break;
        }
        earlier.push(e);
        at = link(e, by_id);
    }
    newest_first(&mut earlier);
    earlier
}

/// Keep `row` under `key` in `map` when it is newer than what is there.
fn keep_newest<'a, T>(
    map: &mut HashMap<Uuid, &'a T>,
    key: Uuid,
    row: &'a T,
    age_of: fn(&T) -> ((i64, i32), Uuid),
) {
    let kept = map.entry(key).or_insert(row);
    if age_of(row) > age_of(kept) {
        *kept = row;
    }
}

/// Fold the record (see the module doc for the six steps).
pub fn resolve(a: &PatientAllergies) -> Resolved<'_> {
    let rows: HashMap<Uuid, (Uuid, Named)> = a
        .entries
        .iter()
        .map(|e| (e.event_id, (e.chart, Named::Entry)))
        .chain(
            a.none
                .iter()
                .map(|n| (n.event_id, (n.chart, Named::NoneStatement))),
        )
        .chain(
            a.challenges
                .iter()
                .map(|c| (c.event_id, (c.chart, Named::Challenge))),
        )
        .collect();
    let named = |t: Uuid| rows.get(&t).map_or(Named::Unknown, |(_, n)| *n);

    // Step 1: which overlays count (a suspect one only to add a warning); the newest counted
    // withdrawal naming each target.
    let mut ignored_overlays = Vec::new();
    let mut challenges = Vec::new();
    for c in &a.challenges {
        let why = suspicion(&rows, c.key, c.chart, c.target);
        if overlay_counts(why.is_some(), false, named(c.target)) {
            challenges.push(c);
        } else {
            ignored_overlays.push(format!(
                "Not counted: de-label (tolerated {}){}{}",
                c.tolerated,
                by_words(c.signer.as_deref(), c.key),
                why.unwrap_or_default()
            ));
        }
    }
    let mut withdrawal: HashMap<Uuid, &RetractionRow> = HashMap::new();
    for w in &a.retractions {
        let why = suspicion(&rows, w.key, w.chart, w.target);
        if overlay_counts(why.is_some(), true, named(w.target)) {
            keep_newest(&mut withdrawal, w.target, w, retraction_age);
        } else {
            ignored_overlays.push(format!(
                "Not counted: entered in error{}{}",
                by_words(w.signer.as_deref(), w.key),
                why.unwrap_or_default()
            ));
        }
    }

    // Step 2: the standing de-labels, newest per target; the withdrawn ones are listed.
    let mut standing: HashMap<Uuid, &ChallengeRow> = HashMap::new();
    let mut withdrawn_challenges = Vec::new();
    for c in challenges {
        match withdrawal.get(&c.event_id) {
            Some(by) => withdrawn_challenges.push(Withdrawn { row: c, by }),
            None => keep_newest(&mut standing, c.target, c, challenge_age),
        }
    }

    // Steps 3–5: each head's status; withdrawn outranks historical, which outranks live.
    let mut live = Vec::new();
    let mut historical = Vec::new();
    let mut withdrawn = Vec::new();
    for h in heads(&a.entries) {
        if let Some(by) = withdrawal.get(&h.row.event_id) {
            withdrawn.push(WithdrawnEntry {
                row: h.row,
                earlier: h.earlier,
                by,
            });
        } else if let Some(challenge) = standing.get(&h.row.event_id) {
            historical.push(Historical {
                head: h.row,
                earlier: h.earlier,
                challenge,
            });
        } else {
            live.push(h);
        }
    }

    // Step 6: differing accounts, over every live head of every list and chart.
    let differing: Vec<bool> = live
        .iter()
        .map(|h| {
            live.iter().any(|o| {
                o.row.event_id != h.row.event_id
                    && same_allergen(&o.row.term, &h.row.term)
                    && accounts_differ(o.row, h.row)
            })
        })
        .collect();
    for (h, d) in live.iter_mut().zip(differing) {
        h.differing = d;
    }

    let mut withdrawn_none: Vec<&NoneRow> = a
        .none
        .iter()
        .filter(|n| withdrawal.contains_key(&n.event_id))
        .collect();
    withdrawn_none.sort_by_key(|n| std::cmp::Reverse(age(n.hlc, n.event_id)));
    historical.sort_by_key(|h| std::cmp::Reverse(challenge_age(h.challenge)));
    withdrawn.sort_by_key(|w| std::cmp::Reverse(retraction_age(w.by)));
    withdrawn_challenges.sort_by_key(|w| std::cmp::Reverse(retraction_age(w.by)));
    Resolved {
        live,
        historical,
        withdrawn,
        withdrawn_challenges,
        withdrawn_none,
        ignored_overlays,
        withdrawals: withdrawal,
        de_labels: standing,
    }
}

#[cfg(test)]
#[path = "thread_tests.rs"]
mod tests;
