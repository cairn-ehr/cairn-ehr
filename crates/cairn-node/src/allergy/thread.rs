//! Folds a patient's allergy rows into THREADS and decides what each thread says now (allergy
//! slice 2). Pure: no database, no clock, no words a clinician reads except `ignored_overlays`.
//!
//! Nothing in the allergy record is ever edited. A correction is a NEW entry naming the line it
//! replaces (`AllergyRow::corrects`); a de-label (`ChallengeRow`) or an entered-in-error
//! withdrawal (`RetractionRow`) is a new event naming its target. So the current state is a fold
//! over every row, and this module is that fold. Reading order for a junior, matching
//! [`resolve`]'s steps:
//!
//! 1. **Which overlays count.** A de-label or withdrawal signed by a since-repudiated key counts
//!    for nothing (plan D4: such a key never takes a warning away), and neither does one filed on
//!    another chart than the record it names (D2; only a lenient remote door admits it). Each is
//!    listed in `ignored_overlays`, with why.
//! 2. **Which de-labels stand**: those no counted withdrawal names (M3: withdrawing a de-label
//!    puts the allergy back).
//! 3. **Threads.** An entry and every correction of it, transitively, are one thread. A
//!    repudiated key's correction, or one naming another chart's entry, links to nothing: it is
//!    an account of its own and supersedes nothing (D4, D2).
//! 4. **Status, per thread** (D1: an overlay on any version covers them all): *withdrawn* if a
//!    counted withdrawal names any member, else *historical* if a standing de-label does, else
//!    *live*.
//! 5. **Heads**: the members no other member corrects (two corrections of one line are two
//!    heads, M2). Each head's `earlier` is the line of versions it replaced.
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

/// One current account of an allergy: a live thread's head.
#[derive(Debug)]
pub struct Head<'a> {
    pub row: &'a AllergyRow,
    /// The versions this head replaced, newest first.
    pub earlier: Vec<&'a AllergyRow>,
    /// Another live head of the same allergen disagrees with this one.
    pub differing: bool,
}

/// A de-labelled thread, read as its newest head.
#[derive(Debug)]
pub struct Historical<'a> {
    pub head: &'a AllergyRow,
    /// Every other member of the thread (older versions and a fork's older heads), newest first.
    pub earlier: Vec<&'a AllergyRow>,
    /// The newest standing de-label on any member.
    pub challenge: &'a ChallengeRow,
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
    /// Withdrawn threads, each read as its newest head with the newest withdrawal naming it.
    pub withdrawn: Vec<Withdrawn<'a, AllergyRow>>,
    /// Withdrawn de-labels (their allergy is back in force unless another de-label stands).
    pub withdrawn_challenges: Vec<Withdrawn<'a, ChallengeRow>>,
    /// Withdrawn "none known" statements: they stand for no list.
    pub withdrawn_none: Vec<&'a NoneRow>,
    /// "Not counted: …" lines for the overlays step 1 set aside.
    pub ignored_overlays: Vec<String>,
}

/// The marker a byline carries for a key's standing (empty when nothing is in doubt).
pub fn key_marker(key: KeyState) -> &'static str {
    match key {
        KeyState::Standing => "",
        KeyState::Unconfirmed => SIGNER_UNCONFIRMED,
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

/// Why an overlay counts for nothing, as the end of its "Not counted" line; `None` when it
/// counts. `charts` maps every readable row's event id to its chart. A repudiated key's reason is
/// its byline marker, already in the line, so it adds nothing here.
fn not_counted_reason(
    charts: &HashMap<Uuid, Uuid>,
    key: KeyState,
    own_chart: Uuid,
    target: Uuid,
) -> Option<&'static str> {
    if key == KeyState::Repudiated {
        Some("")
    } else if charts.get(&target).is_some_and(|c| *c != own_chart) {
        Some(" — names a record on another chart")
    } else {
        None
    }
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

/// The entry `e` replaces, when that link counts (step 3): not from a repudiated key, not to
/// itself, and to an entry on `e`'s own chart.
fn link<'a>(e: &AllergyRow, by_id: &HashMap<Uuid, &'a AllergyRow>) -> Option<&'a AllergyRow> {
    let t = e
        .corrects
        .filter(|t| *t != e.event_id && e.key != KeyState::Repudiated)?;
    by_id.get(&t).copied().filter(|t| t.chart == e.chart)
}

/// Union-find root, the smallest event id of the set (so a thread's key is deterministic).
fn root(parent: &HashMap<Uuid, Uuid>, mut x: Uuid) -> Uuid {
    while parent[&x] != x {
        x = parent[&x];
    }
    x
}

/// One thread: every version of one entry, and its current heads.
struct Thread<'a> {
    members: Vec<&'a AllergyRow>,
    heads: Vec<Head<'a>>,
}

/// Steps 3 and 5 for every entry: the threads, in order of their smallest event id, each with
/// its heads and every head's `earlier`.
fn threads(entries: &[AllergyRow]) -> Vec<Thread<'_>> {
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
    sets.into_values()
        .map(|members| {
            let mut heads: Vec<&AllergyRow> = members
                .iter()
                .copied()
                .filter(|m| !replaced.contains(&m.event_id))
                .collect();
            // A forged cycle leaves no member that nothing corrects: show every member rather
            // than lose the allergy (over-warning is the safe error).
            if heads.is_empty() {
                heads = members.clone();
            }
            let head_ids: HashSet<Uuid> = heads.iter().map(|h| h.event_id).collect();
            let heads = heads
                .into_iter()
                .map(|row| Head {
                    row,
                    earlier: earlier_of(row, &by_id, &head_ids),
                    differing: false,
                })
                .collect();
            Thread { members, heads }
        })
        .collect()
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
    let charts: HashMap<Uuid, Uuid> = a
        .entries
        .iter()
        .map(|e| (e.event_id, e.chart))
        .chain(a.none.iter().map(|n| (n.event_id, n.chart)))
        .chain(a.challenges.iter().map(|c| (c.event_id, c.chart)))
        .collect();

    // Step 1: which overlays count; the newest counted withdrawal naming each target.
    let mut ignored_overlays = Vec::new();
    let mut challenges = Vec::new();
    for c in &a.challenges {
        match not_counted_reason(&charts, c.key, c.chart, c.target) {
            Some(why) => ignored_overlays.push(format!(
                "Not counted: de-label (tolerated {}){}{why}",
                c.tolerated,
                by_words(c.signer.as_deref(), c.key)
            )),
            None => challenges.push(c),
        }
    }
    let mut withdrawal: HashMap<Uuid, &RetractionRow> = HashMap::new();
    for w in &a.retractions {
        match not_counted_reason(&charts, w.key, w.chart, w.target) {
            Some(why) => ignored_overlays.push(format!(
                "Not counted: entered in error{}{why}",
                by_words(w.signer.as_deref(), w.key)
            )),
            None => keep_newest(&mut withdrawal, w.target, w, retraction_age),
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

    // Steps 3–5: each thread's status; withdrawn outranks historical, which outranks live.
    let mut live = Vec::new();
    let mut historical = Vec::new();
    let mut withdrawn = Vec::new();
    for t in threads(&a.entries) {
        let newest_head = t
            .heads
            .iter()
            .map(|h| h.row)
            .max_by_key(|r| age(r.hlc, r.event_id))
            .expect("every thread has a head");
        let withdrawn_by = t
            .members
            .iter()
            .filter_map(|r| withdrawal.get(&r.event_id).copied())
            .max_by_key(|w| retraction_age(w));
        if let Some(by) = withdrawn_by {
            withdrawn.push(Withdrawn {
                row: newest_head,
                by,
            });
            continue;
        }
        let de_label = t
            .members
            .iter()
            .filter_map(|r| standing.get(&r.event_id).copied())
            .max_by_key(|c| challenge_age(c));
        if let Some(challenge) = de_label {
            let mut earlier: Vec<&AllergyRow> = t
                .members
                .iter()
                .copied()
                .filter(|r| r.event_id != newest_head.event_id)
                .collect();
            newest_first(&mut earlier);
            historical.push(Historical {
                head: newest_head,
                earlier,
                challenge,
            });
            continue;
        }
        live.extend(t.heads);
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
    }
}

#[cfg(test)]
#[path = "thread_tests.rs"]
mod tests;
