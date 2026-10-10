//! The display half of a candidate: name, age, trust, last activity, locale, photo. Shared by
//! the search (`search.rs`) and the possible-duplicate worklist (R5b), so the two show one
//! person the same way - a worklist row must read exactly as the search would have shown that
//! chart, not a second, drifting rendering of it.
//!
//! Several small reads, joined in Rust - see `search.rs`'s module doc for why not one join.
//! The UUID binding convention is the same: ids are bound as text and cast in SQL, and every
//! UUID column is cast back to text and parsed on the Rust side.

use super::person;
use super::search_person::{display_name_for, trust_state_for};
use cairn_patient_search::{age_years, Age, Candidate};
use std::collections::{HashMap, HashSet};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// Every display read for a set of charts, done once. `dobs` is exposed because the search's
/// ranking reads it too, so one query serves both display and ranking.
pub struct DisplayFacts {
    names: HashMap<Uuid, String>,
    ever_named: HashSet<Uuid>,
    held: HashSet<Uuid>,
    dobs: HashMap<Uuid, (String, String)>,
    trust_states: HashMap<Uuid, String>,
    last_activity: HashMap<Uuid, String>,
    locales: HashMap<Uuid, String>,
    photo_refs: HashMap<Uuid, String>,
}

/// Run every display read for `ids` (the query count the search path was budgeted for, §5.11:
/// the "names ever asserted" read happens only when some chart has no display name, #344 N4).
pub async fn read_display_facts<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<DisplayFacts> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let names = read_display_names(client, ids).await?;
    // Only pay for the repudiated-vs-never-named distinction when a name is missing: most
    // reads have a name for every chart and this path is latency-budgeted.
    let missing: Vec<Uuid> = ids
        .iter()
        .filter(|id| !names.contains_key(id))
        .copied()
        .collect();
    let ever_named = if missing.is_empty() {
        HashSet::new()
    } else {
        read_names_ever_asserted(client, &missing).await?
    };
    Ok(DisplayFacts {
        names,
        ever_named,
        // Whether this node holds each chart's registration: "no trust row" means `confirmed`
        // only for a chart that exists here.
        held: person::read_held(client, &id_strs).await?,
        dobs: read_dob(client, ids).await?,
        trust_states: read_trust_states(client, ids).await?,
        last_activity: read_last_activity(client, ids).await?,
        locales: read_locale(client, ids).await?,
        photo_refs: read_photo_refs(client, ids).await?,
    })
}

impl DisplayFacts {
    /// Each chart's dob value with its winning assertion's provenance (the ranking reads it too).
    pub fn dobs(&self) -> &HashMap<Uuid, (String, String)> {
        &self.dobs
    }

    /// One candidate, never dropped (a silently-dropped row is precisely the duplicate-creating
    /// failure the funnel exists to prevent). The flag is `true` when its name could not be read;
    /// the search counts those into `incomplete`. Every other field is `Option`-typed in the
    /// shared model, so a missing dob/trust-row/activity/locale/photo is an honest "unknown",
    /// not a read failure.
    pub fn candidate(&self, id: Uuid, today: &str) -> (Candidate, bool) {
        let name = display_name_for(id, &self.names, &self.ever_named, &self.held);
        let age = self.dobs.get(&id).and_then(|(dob, basis)| {
            age_years(dob, today).map(|years| Age {
                years,
                basis: basis.clone(),
            })
        });
        let unreadable = name.is_unreadable();
        (
            Candidate {
                patient_id: id,
                display_name: name.text(),
                age,
                trust: trust_state_for(
                    self.held.contains(&id),
                    self.trust_states.get(&id).map(String::as_str),
                ),
                last_activity: self.last_activity.get(&id).cloned(),
                locale: self.locales.get(&id).cloned(),
                photo_ref: self.photo_refs.get(&id).cloned(),
            },
            unreadable,
        )
    }
}

/// One candidate per id, in `ids`' order - what the search would show for each. A chart this
/// node has never heard of is still returned, never dropped: named "(registration not yet
/// received here)", its trust unknown.
pub async fn candidates_by_id<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
    today: &str,
) -> anyhow::Result<Vec<Candidate>> {
    let facts = read_display_facts(client, ids).await?;
    Ok(ids.iter().map(|id| facts.candidate(*id, today).0).collect())
}

/// The §4.2 display-winner name for each candidate, or the John Doe callsign — whichever
/// `patient_name_current` (db/012) currently picks. A candidate with no row here has never
/// had ANY name asserted (possible: a chart matched by identifier or dob alone); such a
/// candidate is never dropped by the caller. Only a HELD nameless chart is reported
/// `incomplete` (see `search_person::display_name_for`); one not held here reads
/// "(registration not yet received here)" and is not a partial search.
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
/// to be missing from `read_display_names`'s result (see `missing` in `read_display_facts`).
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
/// suites' helper) reads. What a MISSING row means depends on whether the chart is held here
/// (`confirmed` if so, `unknown` if not): the caller applies `search_person::trust_state_for`
/// rather than this function inventing a fabricated row for it.
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
/// Since #345 the registration act itself creates this row (`patient_chart_apply` is
/// registered for `identity.registration.asserted`, `patient.amended` and `note.added`), so a
/// chart registered moments ago reports its registration date rather than nothing — the birth
/// of a chart is activity, and an empty column read as "nothing ever happened here".
///
/// A row can still be legitimately absent: a chart known only from a peer's replicated
/// demographic or clinical events, whose registration has not synced yet (the remote door is
/// lenient by design, ADR-0061 decision 3). That absence is read here as `None` exactly like
/// every other candidate with no matching row, NOT distinguished as an error: `last_activity`
/// is an honest "no activity recorded yet", consistent with `Candidate::last_activity` being
/// `Option`-typed in the shared model.
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
