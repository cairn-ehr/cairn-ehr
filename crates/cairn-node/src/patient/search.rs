//! §5.8 search-before-create: the ONE mapping from this node's projections into the shared
//! `Candidate` model a clerk sees before a new chart may be created. The CLI reads through
//! this function today; the future picker window and the native API (ADR-0023) are expected
//! to wrap it rather than re-derive the joins — same discipline as `medication/read.rs` for
//! the drug chart.
//!
//! WHY SEVERAL SMALL QUERIES AND NOT ONE JOIN. A candidate's display fields come from seven
//! genuinely different projections (the display-winner name, the raw retained name set used
//! only to disambiguate a repudiated-away name from a never-asserted one, dob, trust, chart
//! activity, address, photo evidence), several of which are retained SETS rather than single
//! rows (an address has one row per USE; a chart may carry more than one photo-evidence
//! event over its life; a rendition set may carry more than one rendition per attachment).
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

use cairn_patient_search::{age_years, Age, Candidate, CandidateList, SearchQuery, TrustState};
use std::collections::{HashMap, HashSet};
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

    let ids = read_candidate_ids(client, query).await?;
    if ids.is_empty() {
        return Ok(empty_list());
    }

    let names = read_display_names(client, &ids).await?;
    // N4 (review round 2, #344): only pay for the repudiation-vs-never-asserted distinction
    // (below) when there is actually a gap to explain. Most searches return candidates that
    // ALL have a display name, and this read path is §5.11-budgeted at "no spinner" — a
    // second round trip nobody needs is not free just because it is small.
    let missing_names: Vec<Uuid> = ids
        .iter()
        .filter(|id| !names.contains_key(id))
        .copied()
        .collect();
    let ever_named = if missing_names.is_empty() {
        HashSet::new()
    } else {
        read_names_ever_asserted(client, &missing_names).await?
    };
    let dobs = read_dob(client, &ids).await?;
    let trust_states = read_trust_states(client, &ids).await?;
    let last_activity = read_last_activity(client, &ids).await?;
    let locales = read_locale(client, &ids).await?;
    let photo_refs = read_photo_refs(client, &ids).await?;

    // The one field a candidate cannot honestly render as `None`: `Candidate::display_name`
    // is a plain `String`, not `Option<String>`, because a nameless row on a search results
    // list is meaningless to a clerk. Every OTHER field is already `Option`-typed in the
    // shared model, so a missing dob/trust-row/last-activity/locale/photo degrades silently
    // and correctly to `None` — that is an honest "unknown", not a read failure, and must
    // NOT itself flip `incomplete` (see the John-Doe test: no `patient_chart` row is normal).
    let mut unreadable_names = 0usize;
    let candidates: Vec<Candidate> = ids
        .iter()
        .map(|id| {
            let display_name = match names.get(id) {
                Some(name) => name.clone(),
                // db/025: a chart whose ONLY asserted name(s) were struck as known-false has
                // NO winner row in `patient_name_current` BY DESIGN — showing the known-false
                // name back would be a precise untruth (principle 4), so the view withholds
                // it on purpose. That is an honest "name withheld", not a failed read, and
                // must NOT count toward `incomplete` the way a genuine read failure does.
                // `ever_named` answers this precisely — see `read_names_ever_asserted`'s doc.
                None if ever_named.contains(id) => "(name withheld)".to_string(),
                None => {
                    unreadable_names += 1;
                    // Never drop the candidate: a silently-dropped row is precisely the
                    // duplicate-creating failure this funnel exists to prevent. An honest
                    // placeholder keeps the chart visible while `incomplete` (below) tells
                    // the clerk the read was not exhaustive.
                    "(name unavailable)".to_string()
                }
            };
            let age = dobs.get(id).and_then(|(dob, basis)| {
                age_years(dob, today).map(|years| Age {
                    years,
                    basis: basis.clone(),
                })
            });
            Candidate {
                patient_id: *id,
                display_name,
                age,
                trust: trust_states
                    .get(id)
                    .map_or(TrustState::Confirmed, |s| trust_state_from_db(s)),
                last_activity: last_activity.get(id).cloned(),
                locale: locales.get(id).cloned(),
                photo_ref: photo_refs.get(id).cloned(),
            }
        })
        .collect();

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
        candidates,
        incomplete,
        incomplete_reason,
    })
}

/// The "found nothing, and that is the whole truth" list — shared by both short-circuits
/// above (empty query; a real search that genuinely matched no chart).
fn empty_list() -> CandidateList {
    CandidateList {
        candidates: vec![],
        incomplete: false,
        incomplete_reason: None,
    }
}

/// `chart_trust.trust_state` is a closed, DB-defined vocabulary (db/024): a row present
/// there is, by construction, either `'unconfirmed'` or `'under-review'` — never
/// `'confirmed'`, which the view represents by ABSENCE of a row (mirrored by
/// `read_trust_states`'s `map_or` above). A string this match does not recognise can only
/// mean a future db/0xx trust source this Rust code has not been taught about yet; failing
/// toward `UnderReview` (the more cautious of the two known states — the same "sharper
/// caution wins" rule `chart_trust`'s own view comment states) is the safe direction,
/// exactly as principle 4 asks: an uncertain read must never silently look more confident
/// than it is.
fn trust_state_from_db(trust_state: &str) -> TrustState {
    match trust_state {
        "unconfirmed" => TrustState::Unconfirmed,
        "under-review" => TrustState::UnderReview,
        _ => TrustState::UnderReview,
    }
}

/// Call `cairn_search_candidates` once and return the DISTINCT patient ids it names, sorted
/// for determinism (UUIDv7 sorts close to chart-creation order, so this is a stable and
/// meaningful order, not an arbitrary one — the same reasoning `medication/read.rs` gives
/// for sorting query results in Rust rather than depending on database order).
///
/// One chart can legitimately appear on more than one row (matched by more than one pass —
/// see `a_chart_matching_two_passes_returns_one_row_per_pass` in this crate's `db/046`
/// tests), so this is where the query -> ONE candidate collapse happens; every read below
/// operates on this already-deduplicated id list.
async fn read_candidate_ids<C: GenericClient + Sync>(
    client: &C,
    query: &SearchQuery,
) -> anyhow::Result<Vec<Uuid>> {
    let identifiers: Vec<serde_json::Value> = query
        .identifiers
        .iter()
        .map(|(system, value)| serde_json::json!({"system": system, "value": value}))
        .collect();
    let identifiers_json = serde_json::to_string(&identifiers)?;
    let birth_date: Option<&str> = query.birth_date.as_deref();

    let rows = client
        .query(
            "SELECT DISTINCT patient_id::text AS patient_id \
             FROM cairn_search_candidates($1, $2, $3::text::jsonb)",
            &[&query.name_tokens, &birth_date, &identifiers_json],
        )
        .await?;

    let mut ids: Vec<Uuid> = rows
        .iter()
        .map(|row| row.get::<_, String>("patient_id").parse::<Uuid>())
        .collect::<Result<_, _>>()?;
    ids.sort();
    Ok(ids)
}

/// The §4.2 display-winner name for each candidate, or the John Doe callsign — whichever
/// `patient_name_current` (db/012) currently picks. A candidate with no row here has never
/// had ANY name asserted (possible: a chart matched by identifier or dob alone); such a
/// candidate is never dropped by the caller, only reported `incomplete`.
async fn read_display_names<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT patient_id::text AS patient_id, value \
               FROM patient_name_current \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(id, row.get::<_, String>("value"));
    }
    Ok(out)
}

/// Which of `ids` have EVER had a `patient_name` row asserted (struck or not) — used to tell
/// "this chart's only name(s) were repudiated" (an honest, by-design absence from
/// `patient_name_current`) apart from "this chart never had a name asserted at all" (a
/// genuine read gap the caller reports via `incomplete`). Called ONLY for ids already known
/// to be missing from `read_display_names`'s result (see `missing_names` in the caller).
///
/// PROVABLY SUFFICIENT, not a heuristic: `patient_name_current` (db/025) is `patient_name`
/// filtered by exactly ONE condition — an anti-join against `name_repudiation` on
/// `(patient_id, value)` — and nothing else. So a candidate already known to be ABSENT from
/// `patient_name_current` that nonetheless HAS a `patient_name` row must have had every one
/// of its rows individually struck; the view has no other mechanism to drop it. That makes
/// "does `patient_name` have any row for this id" the exact question, not an approximation.
///
/// Review round 2 (N3): the PRIOR version of this check asked "does `patient_alias_pool`
/// (db/025's cross-patient known-alias view) have any row naming this patient", which is the
/// WRONG question — the alias pool has no requirement that a struck value ever belonged to
/// THIS chart's own `patient_name` rows (it exists so the matcher can recognise a returning
/// fabricated persona on ANY chart), so a chart with zero names ever asserted plus one
/// unrelated repudiation would false-positive into "(name withheld)" and be silently dropped
/// from `incomplete` — converting a genuine read gap into a claimed by-design absence, and
/// suppressing exactly the ADR-0060 decision-2 signal that tells the clerk the search was not
/// exhaustive. Reading `patient_name` (broadly granted, db/012) instead of `name_repudiation`
/// or `patient_alias_pool` also sidesteps db/025's deliberate no-broad-grant on the base
/// table (`reason` is forensic free text, ADR-0006) without needing the alias view at all.
async fn read_names_ever_asserted<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashSet<Uuid>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT DISTINCT patient_id::text AS patient_id \
               FROM patient_name \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashSet::new();
    for row in client.query(sql, &[&id_strs]).await? {
        out.insert(row.get::<_, String>("patient_id").parse()?);
    }
    Ok(out)
}

/// Each candidate's dob VALUE together with the WINNING assertion's own `provenance` —
/// carried through as `Age::basis` (principle 4: an age derived from a document-verified dob
/// and one derived from a clinician's estimate are different claims, and a clerk comparing
/// candidates needs to know which is which). Absent for a candidate with no dob on file —
/// `age_years` is never even called for it, so `Candidate::age` degrades to `None`, not to a
/// guess.
async fn read_dob<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, (String, String)>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT patient_id::text AS patient_id, value, provenance \
               FROM patient_demographic \
               WHERE field = 'dob' AND patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(
            id,
            (
                row.get::<_, String>("value"),
                row.get::<_, String>("provenance"),
            ),
        );
    }
    Ok(out)
}

/// `chart_trust` for each candidate — the same view `common::trust_of` (the identity test
/// suites' helper) reads. A candidate with NO row here is, by that view's own construction
/// (db/024's header comment), in the default `confirmed` state; the caller's `map_or`
/// applies that default rather than this function inventing a fabricated row for it.
async fn read_trust_states<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT patient_id::text AS patient_id, trust_state \
               FROM chart_trust \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(id, row.get::<_, String>("trust_state"));
    }
    Ok(out)
}

/// ISO `YYYY-MM-DD` of `patient_chart.last_activity` for each candidate that HAS a
/// `patient_chart` row with one set.
///
/// A registration event alone (§5.4 John Doe included) never creates this row —
/// `patient_chart_apply` is registered only for `patient.created` / `patient.amended` /
/// `note.added` — so a fresh registration-only chart legitimately has none. That absence is
/// read here as `None` exactly like every other candidate with no matching row, NOT
/// distinguished as an error: `last_activity` is an honest "no activity recorded yet",
/// consistent with `Candidate::last_activity` being `Option`-typed in the shared model.
async fn read_last_activity<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT patient_id::text AS patient_id, last_activity::date::text AS last_activity \
               FROM patient_chart \
               WHERE patient_id = ANY($1::text[]::uuid[]) AND last_activity IS NOT NULL";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(id, row.get::<_, String>("last_activity"));
    }
    Ok(out)
}

/// One locale one-liner per candidate, reduced from `patient_address_current`'s per-USE rows
/// (home/work/… each has its own row there) to the single freshest assertion across every
/// use — same recency-first tiebreak `patient_address_current` itself already applies within
/// a use (db/014), just carried one level further to collapse across uses.
///
/// KNOWN LIMITATION, tracked as issue #347 (not fixed here — a data-model gap, not a bug in
/// this query): this reads the address's `display` value verbatim, which is the mandatory
/// FULL address string the §4.3 structural floor requires (`structured.parts` is
/// deliberately culture-neutral and carries no guaranteed "suburb"/"town" key to extract
/// instead — inventing one would be exactly the cultural-capture ADR-0014 forbids elsewhere
/// in this codebase). So today's "locale one-liner" can be a full address, not only a
/// suburb hint. Issue #347 tracks the new address facet a true locale-only projection needs.
async fn read_locale<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT DISTINCT ON (patient_id) patient_id::text AS patient_id, display \
               FROM patient_address_current \
               WHERE patient_id = ANY($1::text[]::uuid[]) \
               ORDER BY patient_id, last_hlc_wall DESC, last_hlc_count DESC, \
                        provenance_rank DESC, asserted_origin COLLATE \"C\" DESC, \
                        use_key COLLATE \"C\" DESC";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(id, row.get::<_, String>("display"));
    }
    Ok(out)
}

/// The digest of the candidate's `original` rendition, from their most recent
/// `identity.evidence.asserted` PHOTO event THAT CARRIES ONE — a content-addressed
/// reference only, never bytes. Fetching the image is byte-tier work (ADR-0013) and must
/// not sit on the search latency path §5.11 budgets at "type a few chars and enter, no
/// spinner". NOT simply "the most recent photo event": a newer photo event whose attachment
/// carries only a preview rendition (no `original` yet — e.g. mid-upload) is silently
/// skipped rather than blanking out an older original that IS still the best evidence on
/// file (N2, review round 2 — the opening line above used to claim otherwise).
///
/// Reads `event_log` directly rather than through a projection: `identity.evidence.asserted`
/// is additive (db/028) and carries no dedicated "current" view the way a demographic field
/// does, so the freshest row by HLC IS the read.
///
/// SELECTS BY `role = 'original'`, NEVER BY POSITION (review round 1, #344 Important 2).
/// ADR-0042 exists precisely so one attachment can carry N renditions (a thumbnail preview
/// alongside the original, say) — `renditions -> 0` is whichever the AUTHOR happened to
/// list first, not necessarily the original, so indexing positionally would let a future
/// preview-adding change silently swap what the picker displays. `jsonb_array_elements`
/// over each attachment's rendition set, filtered on the named role, is index-order-proof.
///
/// TOTAL ORDER, INCLUDING TIES (N1, review round 2): `role` is an open string with NO
/// uniqueness constraint in the wire shape (`cairn_event::attachment::Rendition`) or the DB,
/// so two attachments on the SAME event — or, in principle, two renditions both marked
/// "original" within one attachment's own set — can tie on `(hlc_wall, hlc_counter)`. Without
/// a further tiebreak, `DISTINCT ON`'s pick among tied rows is Postgres's to make, not this
/// query's, and could differ between two runs of the identical search: exactly the kind of
/// silent non-determinism a wrong-chart-prevention surface cannot tolerate (a clerk must see
/// the SAME photo every time they search the same name). `digest_hex` is a content hash, so
/// ordering by it last makes the whole ORDER BY total and therefore the pick stable.
/// `COLLATE "C"` (review round 3, ADR-0045/#69 — the same fix `patient_address_current` and
/// `patient_name_current` already carry, db/014/db/024): a TEXT tiebreak that trusts the
/// node's DEFAULT collation could rank the SAME two hex strings differently on two nodes
/// with different default collations, converging to different photos on the same data — the
/// exact class of bug this crate's other tiebreaks already guard against, so this one must
/// too rather than being the one silent exception.
async fn read_photo_refs<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT DISTINCT ON (patient_id) patient_id, digest_hex \
               FROM ( \
                 SELECT e.patient_id::text AS patient_id, e.hlc_wall, e.hlc_counter, \
                        rendition ->> 'digest_hex' AS digest_hex \
                   FROM event_log e \
                   CROSS JOIN LATERAL jsonb_array_elements(e.attachments) AS attachment \
                   CROSS JOIN LATERAL jsonb_array_elements(attachment -> 'renditions') AS rendition \
                  WHERE e.patient_id = ANY($1::text[]::uuid[]) \
                    AND e.event_type = 'identity.evidence.asserted' \
                    AND e.body ->> 'kind' = 'photo' \
                    AND NOT e.sealed \
                    AND rendition ->> 'role' = 'original' \
               ) matched \
               ORDER BY patient_id, hlc_wall DESC, hlc_counter DESC, digest_hex COLLATE \"C\"";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        if let Some(digest) = row.get::<_, Option<String>>("digest_hex") {
            out.insert(id, digest);
        }
    }
    Ok(out)
}
