//! The INPUTS to the order `search_patients` shows candidates in (ADR-0075, #671).
//!
//! Split out of `search.rs` because it answers a different question from that file: not
//! "what is displayed for each candidate?" but "which candidate is likeliest to be the
//! person at the desk?". The ranking RULE itself is pure and lives in
//! `cairn_patient_search::rank`; this module only reads what that rule needs — every retained
//! name (callsigns apart) and the query's tokens, both normalised by Postgres — and assembles
//! one `RankKey` per candidate. Nothing read here is ever displayed.
use cairn_patient_search::{
    callsign_typed_whole, is_dob_near_miss, tokens_exactly_matched, tokens_matched, RankKey,
};
use std::collections::HashMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// What db/046 said about ONE candidate: how many of its passes found the chart, and whether
/// the identifier pass is among them. Read by `search::read_candidate_passes`; the first two
/// ranking keys come straight from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MatchedPasses {
    pub(super) id: Uuid,
    /// Distinct passes matched (1..=3).
    pub(super) passes: u32,
    /// True when the identifier pass found this chart.
    pub(super) identifier_matched: bool,
}

/// db/046's per-candidate pass count, checked into the `1..=3` its three passes allow.
///
/// A plain `as u32` would wrap a negative count to ~4 billion — a chart ranked FIRST in every
/// prompt, with nothing on screen to say why. A count outside the contract means db/046 changed
/// under this code, and that must fail the search loudly (the funnel then says "the search
/// FAILED — this is NOT a 'no match'"), never reorder it silently.
pub(super) fn passes_from_db(count: i64) -> anyhow::Result<u32> {
    match u32::try_from(count) {
        Ok(n @ 1..=3) => Ok(n),
        _ => anyhow::bail!("db/046 reported {count} passes for one candidate; expected 1..=3"),
    }
}

/// Build one [`RankKey`] per candidate from the reads `search_patients` already made. Pure:
/// the keys and their order are `cairn_patient_search::rank`'s; this only gathers inputs.
///
/// # Why ranking exists (funnel UI slice 2c, 2026-09-23; widened by ADR-0075, 2026-09-26)
///
/// `cairn_search_candidates` is a DISJUNCTION of three passes, so a full-name-plus-DOB search
/// returns everyone sharing ANY name token or the birth date. The funnel's step-3 prompt
/// shows — and a registration then signs — only the first `PROMPT_CAP` of them
/// (`cairn_gui_funnel::bound_for_prompt`), and in plain id order those were the OLDEST charts:
/// the recently-registered duplicate the prompt exists to catch sat hundreds of rows down.
/// Slice 2c ranked by pass count; that fixed an EXACTLY-typed duplicate (500/500) but not one
/// typed with a wrong date of birth (100/500), because the name pass counts ONCE however many
/// name tokens matched. ADR-0075 added five keys under it: an identifier match, a §5.4 callsign
/// typed whole, name tokens matched (exactly or by a ≥3-byte prefix), a DOB near-miss, and
/// exactly-matched tokens as a tie-break (all in `cairn_patient_search::rank`'s module doc).
/// Ranking only reorders — the drift invariant *sweep-paired ⊆ search-found* is
/// untouched — and it needs no new `Candidate` field.
///
/// One asymmetry, deliberate: name tokens are counted over EVERY retained name (repudiated
/// included, #349), but the near-miss compares against the ONE date of birth `read_dob`
/// projects, so a chart whose DOB was later corrected is scored against the corrected value
/// only. Ordering only; widening it would mean reading the DOB assertion history.
///
/// `query_tokens` MUST come from [`normalise_query_tokens`], not `SearchQuery::name_tokens`:
/// the stored side was lowercased and NFC-normalised by Postgres, and Rust's lowercasing differs
/// from it for some scripts (Turkish "İ"). Only the query's birth date is taken otherwise, so the
/// un-normalised token list is not a same-typed argument sitting next to the right one.
pub(super) fn rank_keys(
    passes: &[MatchedPasses],
    query_tokens: &[String],
    birth_date: Option<&str>,
    retained: &RetainedNames,
    dobs: &HashMap<Uuid, (String, String)>,
) -> Vec<RankKey> {
    passes
        .iter()
        .map(|p| {
            // A candidate with no retained name (a DOB- or identifier-only match) matches no
            // tokens — it is ranked lower, never dropped. Likewise a chart with no callsign.
            let names = retained
                .names
                .get(&p.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let callsigns = retained
                .callsigns
                .get(&p.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            RankKey {
                id: p.id,
                passes: p.passes,
                identifier_matched: p.identifier_matched,
                callsign_matched: callsign_typed_whole(query_tokens, callsigns),
                tokens_matched: tokens_matched(query_tokens, names),
                dob_near_miss: match (birth_date, dobs.get(&p.id)) {
                    (Some(typed), Some((stored, _provenance))) => is_dob_near_miss(typed, stored),
                    _ => false,
                },
                tokens_exact: tokens_exactly_matched(query_tokens, names),
            }
        })
        .collect()
}

/// Each candidate's retained names, split the way the ranking must treat them: ordinary names
/// (counted token by token, prefixes included) and §5.4 callsigns (matched only whole).
#[derive(Debug, Default)]
pub(super) struct RetainedNames {
    pub(super) names: HashMap<Uuid, Vec<String>>,
    pub(super) callsigns: HashMap<Uuid, Vec<String>>,
}

/// Every RETAINED name of each candidate, lowercased and NFC-normalised BY POSTGRES — the
/// same normalisation db/046 applies before matching — for the ranking's name keys.
///
/// Reads `patient_name`, NOT `patient_name_current`, for db/046's own reason (#349): a
/// repudiated alias is exactly how a fabricated persona's chart is FOUND, so it must also
/// count toward how strongly it matched. A candidate with no row simply gets no entry.
///
/// §5.4 CALLSIGNS ARE READ APART, mirroring db/046: its name pass never splits a callsign into
/// parts and refuses it the prefix arm, so "Ed" matches no John Doe's "unknown-ed-site1-…" —
/// but it DOES match a callsign typed back whole, which is how a clerk re-finds the John Doe in
/// front of them. Mixed in with the ordinary names, `name_tokens` would split it into
/// "unknown", "ed", "site1" and count what db/046 refuses; left out entirely (the first version
/// of this read), the exact John Doe scored zero while every plain "Ed …" scored one (review of
/// #678). So it goes to `RetainedNames::callsigns`, for `callsign_typed_whole`.
pub(super) async fn read_retained_names<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<RetainedNames> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT patient_id::text AS patient_id, lower(normalize(value, NFC)) AS value, \
                      use_key = 'callsign' AS is_callsign \
               FROM patient_name \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out = RetainedNames::default();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.try_get::<_, String>("patient_id")?.parse()?;
        let value: String = row.try_get("value")?;
        let bucket = if row.try_get::<_, bool>("is_callsign")? {
            &mut out.callsigns
        } else {
            &mut out.names
        };
        bucket.entry(id).or_default().push(value);
    }
    Ok(out)
}

/// The query's name tokens, lowercased and NFC-normalised by Postgres, so both sides of the
/// ranking's token comparison went through the same server-side normalisation.
pub(super) async fn normalise_query_tokens<C: GenericClient + Sync>(
    client: &C,
    tokens: &[String],
) -> anyhow::Result<Vec<String>> {
    let row = client
        .query_one(
            "SELECT coalesce(array_agg(lower(normalize(t, NFC))), '{}') \
             FROM unnest($1::text[]) AS t",
            &[&tokens],
        )
        .await?;
    Ok(row.get::<_, Vec<String>>(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matched(n: u128, passes: u32, identifier_matched: bool) -> MatchedPasses {
        MatchedPasses {
            id: Uuid::from_u128(n),
            passes,
            identifier_matched,
        }
    }

    /// db/046 has three passes, so a count outside 1..=3 is a broken contract. `as u32` wrapped a
    /// negative count to ~4 billion — a chart that would rank FIRST in every prompt, silently.
    #[test]
    fn a_pass_count_outside_one_to_three_is_refused_not_wrapped() {
        assert_eq!(passes_from_db(1).unwrap(), 1);
        assert_eq!(passes_from_db(3).unwrap(), 3);
        for bad in [-1, 0, 4] {
            assert!(passes_from_db(bad).is_err(), "{bad} must be refused");
        }
    }

    fn tokens(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn a_query_without_a_dob_marks_no_near_miss() {
        let dobs = HashMap::from([(
            Uuid::from_u128(1),
            ("1980-07-03".to_string(), "patient-stated".to_string()),
        )]);
        let keys = rank_keys(
            &[matched(1, 1, false)],
            &tokens(&["john", "smith"]),
            None,
            &RetainedNames::default(),
            &dobs,
        );
        assert!(!keys[0].dob_near_miss);
    }

    #[test]
    fn a_candidate_with_no_dob_or_name_row_is_keyed_not_dropped() {
        let keys = rank_keys(
            &[matched(1, 1, true)],
            &tokens(&["john", "smith"]),
            Some("1980-03-07"),
            &RetainedNames::default(),
            &HashMap::new(),
        );
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].tokens_matched, 0);
        assert!(!keys[0].callsign_matched);
        assert!(!keys[0].dob_near_miss);
        assert!(keys[0].identifier_matched, "carried from db/046's passes");
    }

    #[test]
    fn keys_read_the_retained_names_and_the_stored_dob() {
        let retained = RetainedNames {
            names: HashMap::from([(Uuid::from_u128(1), tokens(&["alexander nguyen"]))]),
            callsigns: HashMap::new(),
        };
        let dobs = HashMap::from([(
            Uuid::from_u128(1),
            ("1980-07-03".to_string(), "patient-stated".to_string()),
        )]);
        let keys = rank_keys(
            &[matched(1, 1, false)],
            &tokens(&["alex", "nguyen"]),
            Some("1980-03-07"),
            &retained,
            &dobs,
        );
        assert_eq!(keys[0].tokens_matched, 2, "one exact, one by prefix");
        assert_eq!(keys[0].tokens_exact, 1);
        assert!(keys[0].dob_near_miss);
    }

    /// Review of #678: a callsign is matched whole and ONLY whole, and it never feeds the
    /// token count — so its parts cannot lift a John Doe for "Ed", nor sink it for its callsign.
    #[test]
    fn a_callsign_is_matched_whole_and_never_counted_as_name_tokens() {
        let call = "unknown-ed-site1-2026-09-26-a1b2c3d4";
        let retained = RetainedNames {
            names: HashMap::new(),
            callsigns: HashMap::from([(Uuid::from_u128(1), tokens(&[call]))]),
        };
        let typed_whole = cairn_patient_search::name_tokens(call);
        let keys = rank_keys(
            &[matched(1, 1, false)],
            &typed_whole,
            None,
            &retained,
            &HashMap::new(),
        );
        assert!(keys[0].callsign_matched);
        assert_eq!(
            keys[0].tokens_matched, 0,
            "the callsign's parts are not name tokens"
        );

        let keys = rank_keys(
            &[matched(1, 1, false)],
            &tokens(&["ed", "smith"]),
            None,
            &retained,
            &HashMap::new(),
        );
        assert!(!keys[0].callsign_matched);
        assert_eq!(keys[0].tokens_matched, 0);
    }
}
