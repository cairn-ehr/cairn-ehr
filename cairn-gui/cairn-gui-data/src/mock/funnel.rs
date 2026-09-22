//! The funnel ports, against fixtures — how `--mock` walks the whole front door with no
//! database and no signing key.
//!
//! # THE MATCHING RULE HERE IS NOT `db/046`'s, AND MUST NOT PRETEND TO BE
//!
//! Read this before trusting anything the mock finds. The real §5.8 semantics live in
//! `cairn_search_candidates` (`db/046`) — three blocking passes deliberately mirroring the
//! advisory matcher's, a byte-counted prefix minimum, callsign guards on both name arms, and
//! Unicode normalisation on both sides — and they are pinned by the DB-gated tests in
//! `crates/cairn-node/tests/patient_search.rs`.
//!
//! What follows is a *fixture*: a case-insensitive substring over the display name, an exact
//! birth-date compare, and an exact identifier compare. It is enough to walk the workflow and
//! deliberately not a second implementation of the real rule. A fixture that quietly claimed
//! to be the matcher would teach an accessibility or timing pass expectations the real search
//! does not meet — which is why the §1.2 measurement slice 2b owes must be taken against a
//! database, never against this.
//!
//! Its recall is *wider* than the real search in one direction (substring, not prefix) and
//! *narrower* in another (no normalisation, no part projection). Neither is a bug here. Both
//! are reasons not to generalise from it.
//!
//! # Registering in `--mock`
//!
//! It succeeds, minting a patient into the in-memory set so the next browse finds it — which
//! is the only thing that makes the *browse → nothing fits → register → prompt → commit* walk
//! mean anything. It signs nothing and touches no record, because in `--mock` there is no
//! record: the population lives in this process and vanishes with the window.

use crate::mock::fixtures::FixturePatient;
use crate::mock::MockData;
use crate::port::{DataError, PatientRegistration, PatientSearch};
use cairn_gui_funnel::AttestedSearch;
use cairn_patient_search::{age_years, Age, Candidate, CandidateList, SearchQuery};
use uuid::Uuid;

/// Does this fixture match the query, under the fixture rule?
///
/// A disjunction over the three passes, exactly as `db/046` is a disjunction — an extra
/// candidate is something a clerk dismisses, a missed one is the dangerous direction.
/// **Pure**, so the rule is testable without a `MockData` at all.
fn matches(patient: &FixturePatient, query: &SearchQuery) -> bool {
    // Pass 1 — an identifier, compared exactly. Both sides are already trimmed:
    // `SearchQuery::new` trims the query side, and the fixtures carry no stray whitespace.
    let by_identifier = query.identifiers.iter().any(|(system, value)| {
        patient
            .identifiers
            .iter()
            .any(|(s, v)| s == system && v == value)
    });

    // Pass 2 — the birth date, compared exactly as a string. A reduced-precision query
    // ("1975") matches a reduced-precision stored value and nothing else, which is narrower
    // recall than a range search but never a WRONG match — the same trade `db/046` makes.
    let by_birth_date = query
        .birth_date
        .as_deref()
        .is_some_and(|d| d == patient.birth_date);

    // Pass 3 — a name token as a substring. See the module doc: wider than the real prefix
    // rule, and not a claim about it.
    let haystack = patient.display_name.to_lowercase();
    let by_name = query
        .name_tokens
        .iter()
        .any(|token| haystack.contains(token));

    by_identifier || by_birth_date || by_name
}

/// Render a fixture as the candidate a clerk sees.
///
/// `today` is the caller's clock, so the displayed age stays pure and this is testable
/// against a fixed date. An unparseable or reduced-precision birth date yields **no age**
/// rather than a confident-looking number — `age_years` makes that decision, and it is
/// principle 4 on a wrong-chart-prevention surface.
fn as_candidate(patient: &FixturePatient, today: &str) -> Option<Candidate> {
    Some(Candidate {
        // A fixture with an unparseable uuid is a defect in the fixture table, not a
        // runtime condition, so it drops out rather than panicking the window.
        patient_id: Uuid::parse_str(&patient.uuid).ok()?,
        display_name: patient.display_name.clone(),
        age: age_years(&patient.birth_date, today).map(|years| Age {
            years,
            basis: "dob".to_string(),
        }),
        trust: patient.trust,
        last_activity: None,
        locale: None,
        photo_ref: None,
    })
}

impl MockData {
    /// The search, computed synchronously. Split out from the trait method so no lock guard
    /// is ever held across an `await` — which would make the returned future non-`Send` and
    /// break the port's contract.
    fn search_now(&self, query: &SearchQuery, today: &str) -> CandidateList {
        // An empty query short-circuits before anything is scanned, mirroring
        // `search_patients`: "found nothing" for an empty query is a true, exhaustive
        // answer, so the list is COMPLETE rather than partial.
        if query.is_empty() {
            return CandidateList {
                candidates: vec![],
                incomplete: false,
                incomplete_reason: None,
            };
        }
        let patients = self.patients.lock().expect("fixture population");
        CandidateList {
            candidates: patients
                .iter()
                .filter(|p| matches(p, query))
                .filter_map(|p| as_candidate(p, today))
                .collect(),
            // The fixture population is entirely readable by construction, so there is
            // nothing to report as withheld. `bound_for_prompt` adds the display-side
            // partiality later, if any.
            incomplete: false,
            incomplete_reason: None,
        }
    }

    /// The registration, computed synchronously. Same no-guard-across-await reason.
    fn register_now(&self, attested: &AttestedSearch, name: Option<&str>) -> Uuid {
        let id = Uuid::now_v7();
        let query = attested.query();
        // Blank-after-trim is "nothing supplied", the same rule `register_patient` applies —
        // an empty name field must never assert an empty-string name (principle 4).
        let display_name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or("(no name recorded)")
            .to_string();
        self.patients
            .lock()
            .expect("fixture population")
            .push(FixturePatient {
                uuid: id.to_string(),
                display_name,
                // The port's `Demographics.sex` is a bare `String` and cannot say *unknown*
                // distinctly from a recorded value. The funnel asks for no sex at all
                // (design decision 4's negative limb), so this is literally true.
                sex: "not recorded".to_string(),
                birth_date: query.birth_date.clone().unwrap_or_default(),
                identifiers: query.identifiers.clone(),
                // A chart nobody has confirmed the identity of yet. Claiming `Confirmed`
                // here would put a trust state on screen that no act has earned.
                trust: cairn_patient_search::TrustState::Unconfirmed,
            });
        id
    }
}

impl PatientSearch for MockData {
    fn search(
        &self,
        query: &SearchQuery,
        today: &str,
    ) -> impl std::future::Future<Output = Result<CandidateList, DataError>> + Send {
        // Computed BEFORE the async block, so the mutex guard is released before any await
        // point exists. A guard held across one would make this future non-`Send`.
        let list = self.search_now(query, today);
        async move { Ok(list) }
    }
}

impl PatientRegistration for MockData {
    fn register(
        &self,
        attested: &AttestedSearch,
        name: Option<&str>,
    ) -> impl std::future::Future<Output = Result<Uuid, DataError>> + Send {
        let id = self.register_now(attested, name);
        async move { Ok(id) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::fixtures::FIXTURE_UUID;
    use crate::port::ClinicalData;
    use cairn_gui_funnel::TokenStore;

    /// A fixed clock, so an age never changes under the test suite.
    const TODAY: &str = "2026-09-22";

    fn found(list: &CandidateList) -> Vec<String> {
        list.candidates
            .iter()
            .map(|c| c.display_name.clone())
            .collect()
    }

    async fn browse(data: &MockData, name: &str) -> CandidateList {
        data.search(&SearchQuery::new(name, None, &[]), TODAY)
            .await
            .expect("the fixture search does not fail")
    }

    #[tokio::test]
    async fn browsing_a_fragment_finds_the_chart_it_is_a_fragment_of() {
        // #636's headline case, one layer up: a clerk types `mich` to find Michaelowski.
        let data = MockData::with_fixtures();
        assert_eq!(
            found(&browse(&data, "mich").await),
            ["Michaelowski, Samantha"]
        );
    }

    #[tokio::test]
    async fn either_half_of_a_hyphenated_compound_finds_the_chart() {
        let data = MockData::with_fixtures();
        for half in ["fyodorowksi", "eschenbacher"] {
            assert_eq!(
                found(&browse(&data, half).await),
                ["Fyodorowksi-Eschenbacher, Katarzyna"],
                "typing {half} must find the compound"
            );
        }
    }

    #[tokio::test]
    async fn a_short_name_is_found_by_typing_it_whole() {
        // The byte minimum in db/046 gates PREFIXES, never short NAMES (#638). A rule that
        // hid two-character names would make Mei Wu permanently unfindable — and a clerk who
        // cannot find her creates a second chart for her.
        let data = MockData::with_fixtures();
        assert_eq!(found(&browse(&data, "wu").await), ["Wu, Mei"]);
    }

    #[tokio::test]
    async fn a_john_doe_chart_is_browsable_and_shows_as_identity_pending() {
        // §5.7, and load-bearing rather than decorative: a John Doe registered an hour ago
        // is precisely the chart a clerk must find when the family arrives with a name. A
        // search that hid identity-pending charts would manufacture a duplicate every time.
        let data = MockData::with_fixtures();
        let list = browse(&data, "unknown-ed-site1-2026-07-03-00ab").await;
        assert_eq!(list.candidates.len(), 1);
        assert_eq!(
            list.candidates[0].trust,
            cairn_patient_search::TrustState::Unconfirmed
        );
    }

    #[tokio::test]
    async fn a_search_that_matches_nobody_is_a_complete_answer_not_a_partial_one() {
        // A genuine zero is exhaustive and true. Marking it partial would teach a clerk to
        // distrust the one result the funnel most needs them to trust before creating a
        // chart.
        let list = browse(&MockData::with_fixtures(), "nobodyatall").await;
        assert!(list.candidates.is_empty());
        assert!(!list.incomplete);
    }

    #[tokio::test]
    async fn an_empty_query_finds_nobody_and_calls_that_a_complete_answer() {
        // Mirrors `search_patients`' short-circuit, but be honest about what this pins: the
        // ANSWER, not the short-circuit. With three empty key sets nothing would match
        // anyway, so deleting the early return leaves this green — the early return exists
        // so that a future change turning "nothing typed" into a full scan cannot write the
        // whole population into a permanent signed attestation, and only a reviewer, not
        // this test, is watching for that.
        let data = MockData::with_fixtures();
        let list = data
            .search(&SearchQuery::new("   ", None, &[]), TODAY)
            .await
            .unwrap();
        assert!(list.candidates.is_empty());
        assert!(!list.incomplete);
    }

    #[tokio::test]
    async fn every_candidate_a_clerk_can_see_can_have_its_chart_opened() {
        // A candidate a clerk can see and cannot open is a dead end — and the old
        // single-patient mock produced exactly that for every row but one. Browsing by the
        // shared birth-date pass is not possible here, so sweep by identifier instead:
        // every fixture must answer `demographics`.
        let data = MockData::with_fixtures();
        for name in ["amina", "mich", "fyodorowksi", "brien", "wu", "unknown-ed"] {
            for candidate in browse(&data, name).await.candidates {
                data.demographics(&candidate.patient_id.to_string())
                    .unwrap_or_else(|e| {
                        panic!(
                            "{} is listed but cannot be opened: {e:?}",
                            candidate.display_name
                        )
                    });
            }
        }
    }

    #[tokio::test]
    async fn an_age_is_shown_only_when_it_can_be_said_honestly() {
        // The John Doe's birth date is a YEAR. `age_years` refuses to invent a month and a
        // day for it, so no age is displayed — a year-only date silently becoming
        // "1 January" is the precise untruth principle 4 forbids, and this one would sit
        // beside a patient's name on a wrong-chart-prevention surface.
        let data = MockData::with_fixtures();
        let doe = browse(&data, "unknown-ed").await;
        assert!(doe.candidates[0].age.is_none(), "a year is not a birthday");

        let amina = browse(&data, "amina").await;
        assert_eq!(amina.candidates[0].age.as_ref().unwrap().years, 42);
    }

    #[tokio::test]
    async fn a_fixture_registration_is_findable_by_the_next_browse() {
        // The whole point of letting `--mock` register: the walk means nothing if the chart
        // the clerk just created cannot then be found.
        let data = MockData::with_fixtures();
        let typed = "Bakhtiyarov Ruslan";
        assert!(browse(&data, "bakhtiyarov").await.candidates.is_empty());

        let mut store = TokenStore::new();
        let query = SearchQuery::new(typed, Some("1988-05-05"), &[]);
        let displayed = data.search(&query, TODAY).await.unwrap();
        let token = store.record(query, displayed).unwrap();
        let attested = store.take(token).unwrap();

        let id = data.register(&attested, Some(typed)).await.unwrap();
        let again = browse(&data, "bakhtiyarov").await;
        assert_eq!(
            again
                .candidates
                .iter()
                .map(|c| c.patient_id)
                .collect::<Vec<_>>(),
            vec![id]
        );
        // …and it can be opened, like any other candidate.
        assert!(data.demographics(&id.to_string()).is_ok());
    }

    #[tokio::test]
    async fn a_registration_with_no_name_asserts_no_name_rather_than_an_empty_one() {
        // Principle 4 at the boundary, mirroring `register_patient`: a blank field must not
        // become an empty-string name. An identifier-only registration is legitimate.
        let data = MockData::with_fixtures();
        let mut store = TokenStore::new();
        let query = SearchQuery::new("", None, &[("MRN".into(), "77777".into())]);
        let displayed = data.search(&query, TODAY).await.unwrap();
        let token = store.record(query, displayed).unwrap();
        let attested = store.take(token).unwrap();

        let id = data.register(&attested, Some("   ")).await.unwrap();
        let found = data
            .search(
                &SearchQuery::new("", None, &[("MRN".into(), "77777".into())]),
                TODAY,
            )
            .await
            .unwrap();
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].patient_id, id);
        assert!(
            !found.candidates[0].display_name.trim().is_empty(),
            "an absent name must read as absent, never as a blank a clerk cannot see: {:?}",
            found.candidates[0].display_name
        );
    }

    #[test]
    fn nothing_in_the_search_path_can_narrow_on_sex() {
        // DESIGN DECISION 4's NEGATIVE LIMB, which is the safety half of it: a candidate
        // whose sex is unrecorded — or recorded wrongly, a common entry error — must never
        // become invisible because a clerk typed one. That is the matcher's
        // no-data-is-never-disagreement rule (principle 4) applied to search.
        //
        // Structural, not behavioural. `SearchQuery` carries name tokens, a birth date and
        // identifiers, and NOTHING else, so there is no field to narrow on and no argument
        // through which one could be smuggled. The display/rank half of decision 4 needs an
        // additive `Candidate.sex` and is deferred to #645.
        let q = SearchQuery::new("Wu Mei", Some("2001-07-14"), &[("MRN".into(), "1".into())]);
        let json = serde_json::to_value(&q).unwrap();
        // `serde_json::Value` keys an object by a sorted map, so this list is alphabetical
        // rather than in declaration order. The SET is what the claim is about.
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["birth_date", "identifiers", "name_tokens"],
            "a new SearchQuery field is a deliberate decision — if it narrows on sex or \
             gender, decision 4's negative limb is broken and a chart can be hidden from \
             the clerk who is about to duplicate it"
        );
    }

    #[test]
    fn the_fixture_population_is_multi_script_and_carries_an_identity_pending_chart() {
        // The mock is what the operator accessibility pass and the timing runbook use on a
        // laptop, so it must exercise the interesting shapes rather than six bland rows.
        let population = crate::mock::fixtures::starting_population();
        assert!(population.iter().any(|p| p.display_name.contains('阿')));
        assert!(population
            .iter()
            .any(|p| p.trust == cairn_patient_search::TrustState::Unconfirmed));
        assert!(
            population.iter().any(|p| p.uuid == FIXTURE_UUID),
            "the long-standing fixture id must survive, or the demographics and note tabs \
             lose the patient their tests name"
        );
    }
}
