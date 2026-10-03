//! The person-grouping half of the search-before-create funnel (R3, ADR-0076 decision 6).
//!
//! `search.rs` finds and ranks the CHARTS a typed query matched. This module supplies the
//! pieces that turn that chart list into PERSON rows: reading each matched chart's link
//! component, and deciding what to show for a member the search itself did not match (and
//! which this node may not even hold). The grouping rule proper is the pure
//! `cairn_patient_search::group_by_person`; this file feeds it and renders its members.
//!
//! WHY A NEW FILE. `search.rs` was at its size ceiling, and these helpers answer a distinct
//! question ("who else is this person?") from the ones it already owns ("who matched?").
use cairn_patient_search::TrustState;
use std::collections::{HashMap, HashSet};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use super::person::trust_of;

/// Read the link component of every chart in `ids`: for each chart, every chart of the same
/// person (itself included), sorted ascending (UUIDv7, so oldest first).
///
/// ONE statement, not one call per candidate. `cairn_person_charts` is `LANGUAGE sql STABLE`
/// (db/054), so a single query reads every component from one snapshot: two members of the
/// same person can never be seen with different link states half-way through a search, and
/// the search pays one round trip however many charts it matched. A chart that was never
/// linked comes back as its own set of one, so the caller never needs a fallback for a
/// missing entry - and must not invent one: [`assemble_components`] refuses an answer that
/// does not contain the chart it was asked about.
pub(super) async fn read_components<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, Vec<Uuid>>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let rows = client
        .query(
            "SELECT m::text AS chart, c::text AS member \
             FROM unnest($1::text[]::uuid[]) AS m, LATERAL cairn_person_charts(m) AS c",
            &[&id_strs],
        )
        .await?;
    let pairs: Vec<(Uuid, Uuid)> = rows
        .iter()
        .map(|r| {
            Ok((
                r.try_get::<_, String>("chart")?.parse()?,
                r.try_get::<_, String>("member")?.parse()?,
            ))
        })
        .collect::<anyhow::Result<_>>()?;
    assemble_components(pairs)
}

/// Group `(chart, member-of-its-component)` pairs into one sorted member list per chart, and
/// REFUSE any list that does not contain the chart it belongs to.
///
/// `group_by_person` trusts that precondition: a component missing its own anchor would be
/// placed in a row without it, and the chart would vanish from every row - the silent
/// duplicate-creating failure the front door exists to prevent. db/054 always unions the
/// chart itself in, so this cannot fire today; it is refused (the same refusal
/// `person::person_charts` makes) rather than guessed around should that function change.
/// Pure, so the rule is tested without a database.
fn assemble_components(pairs: Vec<(Uuid, Uuid)>) -> anyhow::Result<HashMap<Uuid, Vec<Uuid>>> {
    let mut out: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (chart, member) in pairs {
        out.entry(chart).or_default().push(member);
    }
    for (chart, members) in &mut out {
        members.sort();
        members.dedup();
        if !members.contains(chart) {
            anyhow::bail!(
                "cairn_person_charts did not return {chart} itself; expected a set containing it"
            );
        }
    }
    Ok(out)
}

/// What the name column of one member reads, and whether that counts against the search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisplayName {
    /// The §4.2 display-winner name (or John Doe callsign).
    Known(String),
    /// Every name ever asserted was struck as known-false (db/025): withheld on purpose, so
    /// not a read failure.
    Withheld,
    /// This node does not hold the chart's registration and has no name for it - a link can
    /// reach a chart that synced ahead or lies outside this node's scope (ADR-0004). The
    /// search is not partial because of it: nothing here could have been read.
    NotReceived,
    /// A chart this node holds, with no name ever asserted: a genuine read gap, and the one
    /// case that makes the search `incomplete` (ADR-0060 decision 2).
    Unreadable,
}

impl DisplayName {
    /// The text a clerk reads (a candidate's `display_name` is a plain `String`).
    pub(super) fn text(&self) -> String {
        match self {
            DisplayName::Known(name) => name.clone(),
            DisplayName::Withheld => "(name withheld)".to_string(),
            DisplayName::NotReceived => "(registration not yet received here)".to_string(),
            DisplayName::Unreadable => "(name unavailable)".to_string(),
        }
    }

    /// Only a genuine read gap counts toward `incomplete`; a withheld name and a chart not
    /// held here are honest, by-design absences.
    pub(super) fn is_unreadable(&self) -> bool {
        matches!(self, DisplayName::Unreadable)
    }
}

/// Decide the name cell for `id` from the three reads `search_patients` makes.
///
/// A name that arrived is shown whether or not the chart is held. With no name: struck-only
/// reads `Withheld`; otherwise a chart held here is `Unreadable` and one not held here is
/// `NotReceived`. Pure, so the four-way rule is tested without a database.
///
/// A HELD chart with no readable name is `Unreadable`, and that sets `incomplete` whether or
/// not the search matched it: it is on screen and signed as displayed, and the node could
/// not read it (the spec's "keeps today's rule"). This errs toward warning, which is the
/// safe direction for a registration attestation.
pub(super) fn display_name_for(
    id: Uuid,
    names: &HashMap<Uuid, String>,
    ever_named: &HashSet<Uuid>,
    held: &HashSet<Uuid>,
) -> DisplayName {
    match names.get(&id) {
        Some(name) => DisplayName::Known(name.clone()),
        None if ever_named.contains(&id) => DisplayName::Withheld,
        None if held.contains(&id) => DisplayName::Unreadable,
        None => DisplayName::NotReceived,
    }
}

/// The trust state a member shows: `person::trust_of`'s answer mapped to the shared enum.
///
/// A `chart_trust` row (db/024) is `'unconfirmed'` or `'under-review'`; no row on a held
/// chart is `confirmed`; no row on a chart NOT held here is `unknown` (principle 4: "no
/// row" about a chart this node has never received is not evidence of anything). An
/// unrecognised string can only be a future trust source this code was not taught about, and
/// fails toward `UnderReview`, the more cautious known state - an uncertain read must never
/// look more confident than it is.
pub(super) fn trust_state_for(held: bool, row: Option<&str>) -> TrustState {
    match trust_of(held, row).as_str() {
        "confirmed" => TrustState::Confirmed,
        "unknown" => TrustState::Unknown,
        "unconfirmed" => TrustState::Unconfirmed,
        _ => TrustState::UnderReview,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chart_not_held_here_reads_unknown_never_confirmed() {
        assert_eq!(trust_state_for(false, None), TrustState::Unknown);
        assert_eq!(trust_state_for(true, None), TrustState::Confirmed);
        // A positive claim wins whether or not the chart is held (person::trust_of's rule).
        assert_eq!(
            trust_state_for(false, Some("under-review")),
            TrustState::UnderReview
        );
        assert_eq!(
            trust_state_for(true, Some("unconfirmed")),
            TrustState::Unconfirmed
        );
        assert_eq!(
            trust_state_for(true, Some("a-future-state")),
            TrustState::UnderReview
        );
    }

    #[test]
    fn a_missing_name_on_a_chart_not_held_here_is_not_a_partial_search() {
        let p = Uuid::from_u128(1);
        let none = HashMap::new();
        let nobody = HashSet::new();
        let name = display_name_for(p, &none, &nobody, &nobody);
        assert_eq!(name, DisplayName::NotReceived);
        assert!(!name.is_unreadable());
        assert_eq!(name.text(), "(registration not yet received here)");
    }

    #[test]
    fn a_held_chart_with_no_name_ever_is_still_unreadable() {
        let p = Uuid::from_u128(1);
        let held: HashSet<Uuid> = [p].into();
        let name = display_name_for(p, &HashMap::new(), &HashSet::new(), &held);
        assert!(name.is_unreadable());
        assert_eq!(name.text(), "(name unavailable)");
    }

    #[test]
    fn a_struck_only_name_reads_withheld_held_or_not() {
        let p = Uuid::from_u128(1);
        let ever: HashSet<Uuid> = [p].into();
        assert_eq!(
            display_name_for(p, &HashMap::new(), &ever, &HashSet::new()),
            DisplayName::Withheld
        );
        assert!(!DisplayName::Withheld.is_unreadable());
        assert_eq!(DisplayName::Withheld.text(), "(name withheld)");
    }

    #[test]
    fn a_known_name_is_shown_whether_or_not_the_chart_is_held() {
        let p = Uuid::from_u128(1);
        let names: HashMap<Uuid, String> = [(p, "Mary SMYTHE".to_string())].into();
        assert_eq!(
            display_name_for(p, &names, &HashSet::new(), &HashSet::new()),
            DisplayName::Known("Mary SMYTHE".into())
        );
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn components_are_grouped_per_chart_and_sorted() {
        let pairs = vec![
            (id(2), id(2)),
            (id(2), id(1)),
            (id(1), id(2)),
            (id(1), id(1)),
        ];
        let out = assemble_components(pairs).unwrap();
        assert_eq!(out[&id(1)], vec![id(1), id(2)]);
        assert_eq!(out[&id(2)], vec![id(1), id(2)]);
    }

    #[test]
    fn a_component_that_omits_its_own_chart_is_refused() {
        // Chart 2's answer names only chart 1: grouping would drop chart 2 from every row.
        let err = assemble_components(vec![(id(2), id(1))]).unwrap_err();
        assert!(err.to_string().contains("did not return"), "{err}");
    }
}
