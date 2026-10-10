//! §5.8 search-before-create: the search that maps this node's projections into the shared
//! `Candidate` model a clerk sees before a new chart may be created. Its DISPLAY half — the
//! per-chart reads and the rendering of one candidate — lives in `candidate_read.rs`, which the
//! possible-duplicate worklist calls too (`candidates_by_id`), so one person reads the same in
//! both; this file keeps the matching and the ADR-0075 ranking. The CLI reads through
//! this function today; the future picker window and the native API (ADR-0023) are expected
//! to wrap it rather than re-derive the joins — same discipline as `medication/read.rs` for
//! the drug chart.
//!
//! WHY SEVERAL SMALL QUERIES AND NOT ONE JOIN (the display reads themselves now live in
//! `candidate_read.rs`; the reasoning is kept here, where the search assembles them with its own
//! ranking reads). A candidate's display fields come from seven
//! genuinely different projections (the display-winner name, the raw retained name set used
//! only to disambiguate a repudiated-away name from a never-asserted one, dob, trust, chart
//! activity, address, photo evidence), several of which are retained SETS rather than single
//! rows (an address has one row per USE; a chart may carry more than one photo-evidence
//! event over its life; a rendition set may carry more than one rendition per attachment).
//! Two more reads feed only the ORDER, never a displayed field: every retained name
//! (`read_retained_names`) and the query's own tokens (`normalise_query_tokens`), both
//! normalised by Postgres; `rank_keys` assembles them into the ADR-0075 ranking's keys.
//! Folding all of that into one query would need multiple levels of aggregation and would be
//! far harder for a reviewer to check against each projection's own definition. Plain
//! queries plus an explicit assembly step in Rust is the reviewer-legible shape §9 asks for
//! — the same reasoning `medication/read.rs`'s module doc states for the drug chart.
//!
//! Generic over `GenericClient` so a caller can read through an open transaction (the
//! `medication/read.rs` precedent) — e.g. the future `register.rs` re-checking the list
//! inside the same transaction that writes the registration attestation.
//!
//! UUID BINDING. This crate does not enable tokio-postgres's `with-uuid-1` feature (see
//! `medication/read.rs`'s "UUID BINDING" note), so every UUID parameter is bound as text and
//! cast in SQL (`$1::text::uuid` / `$1::text[]::uuid[]`), and every UUID column is cast back
//! to text in the SELECT list and parsed on the Rust side.
//!
//! JSONB BINDING. Nor does it enable `with-serde_json-1` (mirrors `enroll.rs`'s
//! `$1::text::jsonb` idiom): `p_identifiers` is serialized to a JSON string in Rust and bound
//! as `&str`, cast to `jsonb` in SQL. This is a deliberate departure from the task brief's
//! literal "bind as a serde_json::Value" — that would not compile without the unenabled
//! driver feature, and every other jsonb parameter in this crate already uses the text-cast
//! idiom, so following it here keeps the binding convention uniform rather than one-off.

use super::candidate_read::read_display_facts;
use super::search_person::read_components;
use super::search_rank::{
    normalise_query_tokens, passes_from_db, rank_keys, read_retained_names, MatchedPasses,
};
use cairn_patient_search::{
    group_by_person, rank_candidates, Candidate, CandidateList, PersonRow, SearchQuery,
};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// Map this node's projections to the candidate list a clerk sees before creating a chart.
///
/// `today` is the caller's clock (ISO `YYYY-MM-DD`) — this function does no I/O to learn the
/// date, mirroring `cairn_patient_search::age_years` staying pure and letting the edge own
/// the clock.
pub async fn search_patients<C: GenericClient + Sync>(
    client: &C,
    query: &SearchQuery,
    today: &str,
) -> anyhow::Result<CandidateList> {
    // Short-circuit BEFORE touching the database. An empty query (no name, no dob, no
    // identifiers) has nothing to block on: `cairn_search_candidates` would legitimately
    // return zero rows for one anyway (db/046's three passes each require a non-null,
    // non-empty key), so this is not a correctness fix — it is a defence-in-depth guard
    // against a future change to that function ever turning "nothing typed" into a full
    // scan, which would write the entire patient population into a permanent signed
    // attestation the first time a registration search ran. "Found nothing" for an empty
    // query is a true, exhaustive answer, so the returned list is complete, not partial.
    if query.is_empty() {
        return Ok(empty_list());
    }

    let passes = read_candidate_passes(client, query).await?;
    let matched: Vec<Uuid> = passes.iter().map(|p| p.id).collect();
    if matched.is_empty() {
        return Ok(empty_list());
    }

    // R3 (ADR-0076 decision 6): the front door lists PEOPLE. Read each matched chart's link
    // component first, so every display read below covers EVERY member of every person
    // shown - a registration signs "displayed" over exactly what these reads return.
    let components = read_components(client, &matched).await?;
    let mut ids: Vec<Uuid> = components.values().flatten().copied().collect();
    ids.sort();
    ids.dedup();

    // Every display read, once (shared with the possible-duplicate worklist, so both show a
    // chart the same way). `facts.dobs()` also feeds the ranking below - one query, two uses.
    let facts = read_display_facts(client, &ids).await?;
    // ADR-0075: rank BEFORE assembling. Only the charts the search MATCHED are ranked (an
    // unmatched member has no rank keys); `group_by_person` then places each person at the
    // position of their best-ranked chart and appends the unmatched members after it.
    let retained = read_retained_names(client, &matched).await?;
    let query_tokens = normalise_query_tokens(client, &query.name_tokens).await?;
    let ranked_ids = rank_candidates(rank_keys(
        &passes,
        &query_tokens,
        query.birth_date.as_deref(),
        &retained,
        facts.dobs(),
    ));
    // A chart without its component would drop out of every row: fail the search loudly.
    let groups = group_by_person(&ranked_ids, &components)?;

    // The one field a candidate cannot honestly render as `None`: `Candidate::display_name`
    // is a plain `String`, not `Option<String>`, because a nameless row on a search results
    // list is meaningless to a clerk. Every OTHER field is already `Option`-typed in the
    // shared model, so a missing dob/trust-row/last-activity/locale/photo degrades silently
    // and correctly to `None` — that is an honest "unknown", not a read failure, and must
    // NOT itself flip `incomplete`. A chart's `patient_chart` row exists once its
    // registration has been received (#345), so "no row" now means "not held here".
    let mut unreadable_names = 0usize;
    let mut candidate_for = |id: &Uuid| -> Candidate {
        let (candidate, unreadable) = facts.candidate(*id, today);
        if unreadable {
            unreadable_names += 1;
        }
        candidate
    };
    let people: Vec<PersonRow> = groups
        .iter()
        .map(|group| {
            PersonRow::new(group.iter().map(&mut candidate_for).collect())
                .ok_or_else(|| anyhow::anyhow!("a person row was grouped with no chart in it"))
        })
        .collect::<anyhow::Result<_>>()?;

    let (incomplete, incomplete_reason) = if unreadable_names > 0 {
        (
            true,
            Some(format!(
                "{unreadable_names} candidate(s) could not be read: no display name on file"
            )),
        )
    } else {
        (false, None)
    };

    Ok(CandidateList {
        people,
        incomplete,
        incomplete_reason,
    })
}

/// The "found nothing, and that is the whole truth" list — shared by both short-circuits
/// above (empty query; a real search that genuinely matched no chart).
fn empty_list() -> CandidateList {
    CandidateList::empty()
}

/// Call `cairn_search_candidates` once and return each DISTINCT patient id it names with how
/// many passes it matched and whether the identifier pass is one of them, in id order. The DISPLAY order is decided later, in
/// [`search_patients`], by `cairn_patient_search::rank_candidates` over `search_rank::rank_keys` — in
/// Rust rather than depended on from the database, the same reasoning `medication/read.rs`
/// gives for sorting query results itself.
///
/// One chart can legitimately appear on more than one row (matched by more than one pass —
/// see `a_chart_matching_two_passes_returns_one_row_per_pass` in this crate's `db/046`
/// tests), so this is where the query -> ONE candidate collapse happens, and the number of
/// rows collapsed is exactly the strength the ranking reads first. Every read in
/// `search_patients` operates on this already-deduplicated id list.
async fn read_candidate_passes<C: GenericClient + Sync>(
    client: &C,
    query: &SearchQuery,
) -> anyhow::Result<Vec<MatchedPasses>> {
    let identifiers: Vec<serde_json::Value> = query
        .identifiers
        .iter()
        .map(|(system, value)| serde_json::json!({"system": system, "value": value}))
        .collect();
    let identifiers_json = serde_json::to_string(&identifiers)?;
    let birth_date: Option<&str> = query.birth_date.as_deref();

    let rows = client
        .query(
            // db/046's outer UNION dedups on (patient_id, matched_pass), so one patient comes
            // back once per pass it matched. Counting per patient therefore counts the DISTINCT
            // passes it matched (1..=3); the `DISTINCT` inside the count states that rather
            // than relying on it. `bool_or` asks the ranking's second question of the same rows:
            // did the IDENTIFIER pass find this chart (ADR-0075, review of #678)?
            "SELECT patient_id::text AS patient_id, count(DISTINCT matched_pass) AS passes, \
                    bool_or(matched_pass = 'identifier') AS identifier_matched \
             FROM cairn_search_candidates($1, $2, $3::text::jsonb) \
             GROUP BY patient_id",
            &[&query.name_tokens, &birth_date, &identifiers_json],
        )
        .await?;

    let mut rows: Vec<MatchedPasses> = rows
        .iter()
        .map(|row| {
            Ok(MatchedPasses {
                // `try_get`, not `get`: `get` PANICS on a NULL or a changed type, and this is
                // the read `passes_from_db` exists to make fail loudly as an error instead.
                id: row.try_get::<_, String>("patient_id")?.parse::<Uuid>()?,
                passes: passes_from_db(row.try_get::<_, i64>("passes")?)?,
                identifier_matched: row.try_get::<_, bool>("identifier_matched")?,
            })
        })
        .collect::<anyhow::Result<_>>()?;
    // Id order only, so `rank_keys` is fed a deterministic list (the reads themselves land in
    // maps and do not care). The order SHOWN is decided by the ranking, which ends on an id
    // tie-break anyway.
    rows.sort_by_key(|p| p.id);
    Ok(rows)
}
