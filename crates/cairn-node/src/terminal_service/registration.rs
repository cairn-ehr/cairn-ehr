//! The terminal service serves ONE registration request (#909, design D6).
//!
//! # Where this fits
//!
//! The window no longer holds the node key, so it cannot sign a registration. It writes the
//! registration's inputs to db/083's queue instead (`cairn_registration_submit`): the name, the
//! search's query, the candidates its screen DISPLAYED, and the day. [`serve_one`] is the step
//! the terminal service repeats: take one request, register it (or refuse it), and record the
//! outcome, all in ONE transaction. The claim is a row lock, never a committed state, so a
//! service that dies part-way leaves the request pending, to be served later or expire.
//!
//! # Why the service re-runs the search
//!
//! The window's login can write any request by raw SQL, so its "I searched and was shown these
//! charts" is only a claim. The service runs the same search again, cuts it to the prompt the
//! window would have shown (`bound_for_prompt`, the window's own function), and refuses if that
//! prompt holds a chart the request does not name ([`unseen_candidates`]). That catches a forged
//! "saw no one", and also a near-match that appeared after the clerk's screen was drawn.
//!
//! # Which failures settle and which do not
//!
//! A request that can never succeed is settled `refused`, so the window can say why: one the
//! service cannot read, a new match, a refusal by the floor. Anything else (a lost connection, a
//! privilege error) is an accident, not a verdict: the transaction rolls back, the request stays
//! pending, and `serve_one` returns `Err` for its caller to log and retry. Nothing here logs; a
//! caller that does must log ids and causes only, never a name or a query (they are clinical).

use crate::db_diagnosis::{commit_failure, floor_refusal_message, Committed, DeliberateRefusal};
use crate::node_key::NodeKey;
use crate::patient::register_tx::register_patient_in_tx;
use crate::patient::search::search_patients;
use anyhow::Context;
use cairn_event::SigningKey;
use cairn_patient_search::{bound_for_prompt, name_tokens, CandidateList, SearchQuery};
use std::collections::HashSet;
use tokio_postgres::{Client, Transaction};
use uuid::Uuid;

/// The window's words when the re-run search finds a chart the clerk was not shown.
pub const NEW_MATCH_REFUSAL: &str =
    "a new possible match has appeared since the search — search again";

/// The advisory-lock key every [`serve_one`] takes first: the ASCII of "CARNRG" (CARN for Cairn,
/// RG for registration), the same naming as db/018's CARNLK and db/033's CARNMR.
///
/// WHY A LOCK ACROSS SERVICES (final review F1, ruling R15). Two requests for the same new
/// patient can be claimed by two services at once. Each re-runs its search before either has
/// committed, so neither sees the other's chart, and both register: a duplicate. The claim's row
/// lock cannot stop it (the two claims are two rows), and the node's HLC row lock comes only at
/// the first tick, after both searches. Taking this one lock as the transaction's FIRST
/// statement makes registrations take turns, node-wide: the second service waits until the
/// first has committed, so its search (a fresh snapshot, READ COMMITTED) finds the first's chart
/// and refuses. Taken before the claim, so a waiting service holds no request in the meantime.
pub const REGISTRATION_LOCK: i64 = 0x4341_524E_5247;

/// What one [`serve_one`] call did.
///
/// `reason` is written for the WINDOW (it is what the clerk reads) and may carry the floor's own
/// sentence, so it is not log text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Served {
    /// No pending, unexpired request was waiting (or another service holds the only one).
    Idle,
    /// The request made chart `patient`.
    Registered { request: Uuid, patient: Uuid },
    /// The request was settled `refused`, and made no chart.
    Refused { request: Uuid, reason: String },
}

/// The charts in `fresh` that the request's displayed list does not name: the ones the clerk
/// never had the chance to recognise. **Pure.**
///
/// `fresh` is the re-run search's BOUNDED prompt, never the whole search: the window shows and
/// attests only the closest `PROMPT_CAP` people (ADR-0075), so an uncut list would call the
/// sixth namesake "new" on nearly every common-name registration, and no re-search could cure it.
///
/// A chart that was displayed but is no longer found is NOT unseen: the clerk saw it and judged
/// it, so it costs them nothing (design D6).
pub fn unseen_candidates(fresh: &CandidateList, displayed: &CandidateList) -> Vec<Uuid> {
    let shown: HashSet<Uuid> = displayed.displayed_charts().into_iter().collect();
    fresh
        .displayed_charts()
        .into_iter()
        .filter(|chart| !shown.contains(chart))
        .collect()
}

/// Read a request's jsonb columns back into the types the window wrote them from, and refuse,
/// with the window's words, a request that could never register. **Pure.**
///
/// Three refusals, each decided before any SQL so the request settles instead of looping:
/// - a query or list that does not parse;
/// - a NUL character: Postgres text cannot hold one, and the error it raises is not a floor
///   refusal, so it would leave the request pending until it expired (controller ruling R7). The
///   request row's own columns cannot carry a NUL either today; this keeps the rule here, where
///   the decoded strings first exist, rather than trusting that;
/// - a name that is not the name the query was built from. The service re-runs the QUERY but
///   registers the NAME, so without this a forger could search for nobody and register
///   somebody. `register_patient`'s own precondition states the same rule.
fn decode_request(
    name: Option<&str>,
    query: &str,
    displayed: &str,
) -> Result<(SearchQuery, CandidateList), String> {
    // serde's own message is not passed on: it can quote the value it choked on.
    let query: SearchQuery = serde_json::from_str(query)
        .map_err(|_| "this registration request's search could not be read".to_string())?;
    let displayed: CandidateList = serde_json::from_str(displayed).map_err(|_| {
        "the candidates this registration request displayed could not be read".to_string()
    })?;
    let identifiers = query.identifiers.iter().flat_map(|(s, v)| [s, v]);
    let has_nul = name
        .into_iter()
        .chain(query.birth_date.as_deref())
        .chain(query.name_tokens.iter().map(String::as_str))
        .chain(identifiers.map(String::as_str))
        .any(|text| text.contains('\0'));
    if has_nul {
        return Err(
            "this registration request carries a NUL character, which the record cannot \
             hold — type the details again"
                .to_string(),
        );
    }
    if name_tokens(name.unwrap_or("")) != query.name_tokens {
        return Err(
            "this registration request's name is not the name it searched for — search again"
                .to_string(),
        );
    }
    Ok((query, displayed))
}

/// The window's words for a registration error that is a VERDICT, or `None` for an accident.
/// **Pure.**
///
/// Two kinds of verdict: the floor's bare `RAISE` (P0001, nothing was written) and a refusal this
/// crate decided in Rust before any SQL (a date of birth no precision fits). Both decide the
/// same way on every retry, so the request is settled rather than served again and again.
fn verdict_text(e: &anyhow::Error) -> Option<String> {
    floor_refusal_message(e).map(str::to_string).or_else(|| {
        e.chain()
            .find_map(|cause| cause.downcast_ref::<DeliberateRefusal>())
            .map(ToString::to_string)
    })
}

/// A request's outcome before it is settled: the chart made (and its registration event, which a
/// lost COMMIT must name), or the refusal's words.
enum Outcome {
    Registered(Committed<Uuid>),
    Refused(String),
}

/// Take one request off db/083's queue and serve it, in one transaction: lock → claim → decode →
/// re-run the search → register on a savepoint → settle → commit. See the module doc for which
/// failures settle `refused` and which return `Err` with the request left pending.
///
/// The transaction holds the node's HLC row from its first tick to its commit, so nothing else
/// runs inside it. `node_origin` stamps the events' clocks, as every node-authored write does.
pub async fn serve_one(
    client: &mut Client,
    sk: &SigningKey,
    node_origin: &str,
) -> anyhow::Result<Served> {
    let tx = client
        .transaction()
        .await
        .context("opening the registration transaction")?;
    // First, before the claim: see REGISTRATION_LOCK for why registrations take turns.
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&REGISTRATION_LOCK])
        .await
        .context("taking the registration lock")?;
    let Some(row) = tx
        .query_opt(
            "SELECT request_id::text, name, query::text, displayed::text, today \
               FROM cairn_registration_claim()",
            &[],
        )
        .await
        .context("claiming a registration request")?
    else {
        tx.rollback()
            .await
            .context("closing an idle registration transaction")?;
        return Ok(Served::Idle);
    };
    let request: Uuid = row.get::<_, String>(0).parse()?;
    let name: Option<String> = row.get(1);
    let today: String = row.get(4);

    let outcome = match decode_request(name.as_deref(), row.get(2), row.get(3)) {
        Err(reason) => Outcome::Refused(reason),
        Ok((query, displayed)) => {
            let fresh = search_patients(&tx, &query, &today)
                .await
                .context("re-running the request's search")?;
            // The same cut the window's prompt made. Safe at the cut only because the ranking is
            // total (`rank_candidates` breaks every tie on the chart id), so an unchanged record
            // yields the same five in the same order here as on the clerk's screen.
            let prompt = bound_for_prompt(&fresh);
            if unseen_candidates(prompt.as_list(), &displayed).is_empty() {
                register_on_savepoint(&tx, sk, node_origin, name.as_deref(), &query, &displayed)
                    .await?
            } else {
                Outcome::Refused(NEW_MATCH_REFUSAL.to_string())
            }
        }
    };

    settle(&tx, request, &outcome).await?;
    // A registration that was attempted may have landed if its COMMIT is lost: name its event.
    if let Err(e) = tx.commit().await {
        let e = anyhow::Error::from(e);
        return Err(match &outcome {
            Outcome::Registered(done) => commit_failure(done.event_id, e),
            Outcome::Refused(_) => e,
        }
        .context(format!("committing registration request {request}")));
    }
    Ok(match outcome {
        Outcome::Registered(done) => Served::Registered {
            request,
            patient: done.value,
        },
        Outcome::Refused(reason) => Served::Refused { request, reason },
    })
}

/// Register on a savepoint of the claim's transaction (controller ruling R2), so a refusal can
/// be undone while the claim survives to be settled. On a verdict the savepoint is rolled back
/// FIRST (a failed statement aborts it, 25P02) and the refusal returned; any other error is
/// returned as is, and the caller's `?` drops the transaction, which rolls it back.
///
/// Plain `SAVEPOINT` statements rather than `Transaction::savepoint`: the release would read as
/// a second commit to `tests/db_errors_stay_legible.rs`, which counts commit calls by shape.
async fn register_on_savepoint(
    tx: &Transaction<'_>,
    sk: &SigningKey,
    node_origin: &str,
    name: Option<&str>,
    query: &SearchQuery,
    displayed: &CandidateList,
) -> anyhow::Result<Outcome> {
    tx.batch_execute("SAVEPOINT registration")
        .await
        .context("opening the registration savepoint")?;
    let kid = NodeKey::Full(sk).kid();
    match register_patient_in_tx(tx, sk, &kid, node_origin, name, query, displayed).await {
        Ok(done) => {
            tx.batch_execute("RELEASE SAVEPOINT registration")
                .await
                .context("releasing the registration savepoint")?;
            Ok(Outcome::Registered(done))
        }
        Err(e) => {
            let rolled_back = tx.batch_execute("ROLLBACK TO SAVEPOINT registration").await;
            match (rolled_back, verdict_text(&e)) {
                (Ok(()), Some(words)) => Ok(Outcome::Refused(words)),
                // An accident, or a savepoint that would not roll back (then even a verdict
                // cannot be settled): return the registration's own error, the useful one.
                _ => Err(e),
            }
        }
    }
}

/// Record `outcome` on request `request` through db/083's settle door, inside the claim's
/// transaction (only the lock holder settles, so a settled row is never overwritten).
async fn settle(tx: &Transaction<'_>, request: Uuid, outcome: &Outcome) -> anyhow::Result<()> {
    let (status, patient, event, refusal) = match outcome {
        Outcome::Registered(done) => (
            "registered",
            Some(done.value.to_string()),
            Some(done.event_id.to_string()),
            None,
        ),
        Outcome::Refused(reason) => ("refused", None, None, Some(reason.as_str())),
    };
    tx.execute(
        "SELECT cairn_registration_settle($1::text::uuid, $2, $3::text::uuid, $4::text::uuid, $5)",
        &[&request.to_string(), &status, &patient, &event, &refusal],
    )
    .await
    .with_context(|| format!("settling registration request {request} as {status}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_patient_search::{Candidate, PersonRow, TrustState};

    /// A list showing exactly these charts, one person row each.
    fn list(ids: &[Uuid]) -> CandidateList {
        let candidates = ids
            .iter()
            .map(|&patient_id| Candidate {
                patient_id,
                display_name: "someone".into(),
                age: None,
                trust: TrustState::Unconfirmed,
                last_activity: None,
                locale: None,
                photo_ref: None,
            })
            .collect();
        CandidateList {
            people: PersonRow::each_alone(candidates),
            ..CandidateList::empty()
        }
    }

    fn ids(n: usize) -> Vec<Uuid> {
        (0..n).map(|_| Uuid::now_v7()).collect()
    }

    #[test]
    fn nothing_unseen_when_fresh_is_a_subset() {
        let shown = ids(3);
        assert!(unseen_candidates(&list(&shown[..2]), &list(&shown)).is_empty());
        assert!(unseen_candidates(&list(&shown), &list(&shown)).is_empty());
        assert!(unseen_candidates(&CandidateList::empty(), &CandidateList::empty()).is_empty());
    }

    #[test]
    fn a_chart_found_now_but_not_shown_is_unseen() {
        let shown = ids(2);
        let newcomer = Uuid::now_v7();
        let fresh = list(&[shown[0], newcomer, shown[1]]);
        assert_eq!(unseen_candidates(&fresh, &list(&shown)), vec![newcomer]);
        // "I searched and saw no one", forged, against a search that finds someone.
        assert_eq!(
            unseen_candidates(&list(&shown), &CandidateList::empty()),
            shown
        );
    }

    #[test]
    fn a_chart_shown_but_gone_is_not_unseen() {
        let shown = ids(2);
        assert!(unseen_candidates(&list(&shown[1..]), &list(&shown)).is_empty());
        assert!(unseen_candidates(&CandidateList::empty(), &list(&shown)).is_empty());
    }

    /// The JSON the window writes: the same serde the request row's jsonb is read back with.
    fn wire(name: &str) -> (String, String) {
        let query = SearchQuery::new(name, Some("1980-01-01"), &[("mrn".into(), "A1".into())]);
        (
            serde_json::to_string(&query).unwrap(),
            serde_json::to_string(&CandidateList::empty()).unwrap(),
        )
    }

    #[test]
    fn a_well_formed_request_decodes() {
        let (q, d) = wire("Ann Nguyen");
        let (query, displayed) = decode_request(Some("Ann Nguyen"), &q, &d).unwrap();
        assert_eq!(
            query,
            SearchQuery::new(
                "Ann Nguyen",
                Some("1980-01-01"),
                &[("mrn".into(), "A1".into())]
            )
        );
        assert_eq!(displayed, CandidateList::empty());
        // A nameless request (a search on dob and identifier alone) is well formed too.
        let (q, d) = wire("");
        assert!(decode_request(None, &q, &d).is_ok());
    }

    #[test]
    fn a_malformed_query_or_list_is_refused_with_a_reason() {
        let (q, d) = wire("Ann Nguyen");
        let bad_query = decode_request(Some("Ann Nguyen"), r#"{"bogus":1}"#, &d).unwrap_err();
        assert!(bad_query.contains("search"), "{bad_query}");
        let bad_list = decode_request(Some("Ann Nguyen"), &q, "[1]").unwrap_err();
        assert!(bad_list.contains("displayed"), "{bad_list}");
    }

    /// R7: Postgres text cannot hold a NUL, and the error it raises is not a floor refusal,
    /// so a NUL that reached the register would leave the request pending until it expired.
    #[test]
    fn a_nul_anywhere_the_record_would_store_it_is_refused() {
        let nul = "\u{0}";
        let (q, d) = wire("Ann");
        assert!(decode_request(Some(&format!("Ann{nul}")), &q, &d)
            .unwrap_err()
            .contains("NUL"));
        for query in [
            SearchQuery::new("Ann", Some(&format!("1980{nul}")), &[]),
            SearchQuery::new("Ann", None, &[("mrn".into(), format!("A{nul}1"))]),
            SearchQuery::new("Ann", None, &[(format!("m{nul}rn"), "A1".into())]),
        ] {
            let q = serde_json::to_string(&query).unwrap();
            assert!(decode_request(Some("Ann"), &q, &d)
                .unwrap_err()
                .contains("NUL"));
        }
    }

    /// The search the service re-runs is the request's QUERY, while the chart is registered
    /// under its NAME. A request whose name is not the name its query was built from would let
    /// a forger search for nobody and register somebody.
    #[test]
    fn a_name_that_is_not_the_one_searched_for_is_refused() {
        let (q, d) = wire("Zzyzx Qq");
        let e = decode_request(Some("Ann Nguyen"), &q, &d).unwrap_err();
        assert!(e.contains("name"), "{e}");
        let (q, d) = wire("Ann Nguyen");
        assert!(
            decode_request(None, &q, &d).is_err(),
            "a name searched for but not registered"
        );
    }
}
