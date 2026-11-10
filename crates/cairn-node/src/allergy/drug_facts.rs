//! What the drug read door (db/076) says, read by the node (allergy slice 3).
//!
//! The door is SQL, in Cairn's own database, and is the only place the optional drug database is
//! read. It keeps no drug logic of its own: it passes the drug database's allergy check through,
//! one pair at a time (M4). This module only turns its rows into plain Rust values the medication
//! check can use, and says "unavailable" when the door has nothing to say. It decides nothing
//! clinical: which pair becomes a mark or a "not checked" tag is `check::check_medications`'s job.
use anyhow::Context;
use std::collections::HashMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// One way a medication met an allergen. `kind` is the door's wire word: `same-drug`,
/// `same-group`, or a newer word a later drug database may add (kept as text so it still reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchPath {
    pub kind: String,
    /// The group the two shared, for a `same-group` path.
    pub group_name: Option<String>,
}

/// The drug database's answer for one (allergen, medication) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// At least one path joined them. One entry per `match` row (a pair can meet by several).
    Match(Vec<MatchPath>),
    /// Checked, and they do not meet.
    NoMatch,
    /// It could not place one side (any reason), so nothing was verified.
    Unplaced,
}

/// Every answer the door gave, or the fact that there is no drug database to ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrugFacts {
    Unavailable,
    /// Keyed `(allergen code, medication code)`. A pair the door gave no row for is absent.
    Available(HashMap<(Uuid, Uuid), Answer>),
}

/// One thing the allergy pick can offer: a drug or a drug group, ready to be coded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrugOption {
    pub system: String,
    pub code: String,
    pub display: String,
}

/// One row of the door's answer for a pair: `(outcome, match_kind, group_name)`.
type FactRow = (String, Option<String>, Option<String>);

/// Uuids as text, for binding as `$n::text[]::uuid[]` (this crate's idiom: no uuid feature on
/// the postgres driver).
fn texts(ids: &[Uuid]) -> Vec<String> {
    ids.iter().map(Uuid::to_string).collect()
}

/// Fold the door's rows for ONE pair into an [`Answer`]. Pure. Rows are `(outcome, kind, group)`.
/// Any `match` row makes the pair a match (and every match row's path is kept); otherwise the
/// pair is `no-match` if it has rows and every one says so; otherwise `unplaced` — a word this
/// build does not know, and no rows at all, included: "not verified" is the safe reading of an
/// answer we cannot interpret or were never given.
fn fold_rows(rows: &[FactRow]) -> Answer {
    let paths: Vec<MatchPath> = rows
        .iter()
        .filter(|(outcome, _, _)| outcome == "match")
        .map(|(_, kind, group)| MatchPath {
            kind: kind.clone().unwrap_or_default(),
            group_name: group.clone(),
        })
        .collect();
    if !paths.is_empty() {
        Answer::Match(paths)
    } else if !rows.is_empty() && rows.iter().all(|(outcome, _, _)| outcome == "no-match") {
        Answer::NoMatch
    } else {
        Answer::Unplaced
    }
}

/// Ask whether a drug database is there, and if so what it answers for every
/// (allergen, medication) pair. `allergens` and `meds` are the drug codes of the entries and the
/// current lines.
///
/// `Err` is NOT the same as `Unavailable`: the facts door can raise when the drug database is
/// installed but only partly readable (db/076's header). **Callers map `Err` to
/// `DrugFacts::Unavailable`**, so a drug database of unexpected shape degrades to "not checked"
/// rather than breaking the medication list (the window's `allergy_check::check_list` does this).
pub async fn read_drug_facts(
    c: &impl GenericClient,
    allergens: &[Uuid],
    meds: &[Uuid],
) -> anyhow::Result<DrugFacts> {
    let available: bool = c
        .query_one("SELECT cairn_drug_available()", &[])
        .await
        .context("asking whether a drug list is available")?
        .get(0);
    if !available {
        return Ok(DrugFacts::Unavailable);
    }
    let rows = c
        .query(
            "SELECT allergen::text, med::text, outcome, match_kind, group_name \
             FROM cairn_drug_facts($1::text[]::uuid[], $2::text[]::uuid[])",
            &[&texts(allergens), &texts(meds)],
        )
        .await
        .context("reading the drug list's allergy answers")?;
    let mut by_pair: HashMap<(Uuid, Uuid), Vec<FactRow>> = HashMap::new();
    for r in rows {
        let pair = (
            r.try_get::<_, String>(0)?.parse()?,
            r.try_get::<_, String>(1)?.parse()?,
        );
        by_pair
            .entry(pair)
            .or_default()
            .push((r.try_get(2)?, r.try_get(3)?, r.try_get(4)?));
    }
    Ok(DrugFacts::Available(
        by_pair
            .into_iter()
            .map(|(pair, rows)| (pair, fold_rows(&rows)))
            .collect(),
    ))
}

/// The review status the drug list gives each of `groups` (a group it does not know is absent;
/// with no drug list the map is empty). D11: callers treat anything but `"reviewed"` as not
/// reviewed.
pub async fn read_group_status(
    c: &impl GenericClient,
    groups: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let rows = c
        .query(
            "SELECT group_code::text, review_status \
             FROM cairn_drug_group_status($1::text[]::uuid[])",
            &[&texts(groups)],
        )
        .await
        .context("reading the drug list's group review status")?;
    rows.iter()
        .map(|r| Ok((r.try_get::<_, String>(0)?.parse()?, r.try_get(1)?)))
        .collect()
}

/// The groups and drugs matching `text` for the allergy pick (at most 12, groups first), or
/// `None` when this node has no drug list at all (the pick then records the name as typed).
pub async fn search_drugs(
    c: &impl GenericClient,
    text: &str,
) -> anyhow::Result<Option<Vec<DrugOption>>> {
    let available: bool = c
        .query_one("SELECT cairn_drug_available()", &[])
        .await
        .context("asking whether a drug list is available")?
        .get(0);
    if !available {
        return Ok(None);
    }
    let rows = c
        .query(
            "SELECT system, code, display FROM cairn_drug_search($1, 12)",
            &[&text],
        )
        .await
        .context("searching the drug list")?;
    rows.iter()
        .map(|r| {
            Ok(DrugOption {
                system: r.try_get(0)?,
                code: r.try_get(1)?,
                display: r.try_get(2)?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(outcome: &str, kind: Option<&str>) -> FactRow {
        (outcome.into(), kind.map(Into::into), None)
    }

    #[test]
    fn any_match_row_wins_the_pair() {
        let a = fold_rows(&[row("no-match", None), row("match", Some("same-drug"))]);
        assert!(matches!(a, Answer::Match(p) if p.len() == 1 && p[0].kind == "same-drug"));
        assert_eq!(fold_rows(&[row("no-match", None)]), Answer::NoMatch);
        assert_eq!(fold_rows(&[row("unplaced", None)]), Answer::Unplaced);
        assert_eq!(fold_rows(&[row("a-newer-word", None)]), Answer::Unplaced);
    }

    /// No rows is no answer: only a `no-match` the door actually said is a verified negative.
    #[test]
    fn no_rows_is_not_a_verified_negative() {
        assert_eq!(fold_rows(&[]), Answer::Unplaced);
    }
}
