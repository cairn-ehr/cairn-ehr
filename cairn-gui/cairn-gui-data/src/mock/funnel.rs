//! The funnel ports, against fixtures — how `--mock` walks the whole front door with no
//! database and no signing key.
//!
//! # THE MATCHING RULE HERE IS NOT `db/046`'s, AND MUST NOT PRETEND TO BE
//!
//! Read this before trusting anything the mock finds. The real §5.8 semantics live in
//! `cairn_search_candidates` (`db/046`) — three blocking passes deliberately mirroring the
//! advisory matcher's, a prefix minimum counted in BYTES and gating the prefix arm only (so
//! short names stay findable by exact match, #638), callsign guards on the parts and prefix
//! sources but deliberately NOT on the whole-token one (a clerk must still find a John Doe by
//! typing the callsign back in full), and Unicode normalisation on both sides — and they are
//! pinned by the DB-gated tests in `crates/cairn-node/tests/patient_search.rs`.
//!
//! What follows is a *fixture*: a case-insensitive substring over the display name, an exact
//! birth-date compare, and an exact identifier compare. It is enough to walk the workflow and
//! deliberately not a second implementation of the real rule. A fixture that quietly claimed
//! to be the matcher would teach an accessibility or timing pass expectations the real search
//! does not meet — which is why the §1.2 measurement slice 2b owes must ALSO be taken against
//! a database, never against this alone. (The design asks for both: in `--mock` and against a
//! database.)
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
///
/// Total: every fixture yields a candidate. Nothing here can drop a row.
fn as_candidate(patient: &FixturePatient, today: &str) -> Candidate {
    Candidate {
        // Infallible: `FixturePatient.uuid` is a parsed `Uuid`. It used to be a String parsed
        // here, and the `Option` that produced let a typo'd fixture DROP OUT of a candidate
        // list that still called itself complete — a silently-missing chart on the one screen
        // whose job is stopping a duplicate. Moving the parse into the fixture builder makes
        // that a loud panic at window start instead of a wrong answer at the desk.
        patient_id: patient.uuid,
        display_name: patient.display_name.clone(),
        age: age_years(&patient.birth_date, today).map(|years| Age {
            years,
            basis: "dob".to_string(),
        }),
        trust: patient.trust,
        last_activity: None,
        locale: None,
        photo_ref: None,
    }
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
                .map(|p| as_candidate(p, today))
                // `map`, never `filter_map`: a row that matched must appear. This was a
                // `filter_map` over a fallible id parse, which could delete a matching chart
                // while the two lines below still swore the list was complete.
                .collect(),
            // Now literally true, and true BY CONSTRUCTION rather than by assertion: every
            // fixture that matches becomes a candidate, so there is no drop path that could
            // withhold one. `bound_for_prompt` adds the display-side partiality later, if any.
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
                uuid: id,
                display_name,
                // The port's `Demographics.sex` is a bare `String` and cannot say *unknown*
                // distinctly from a recorded value. The funnel asks for no sex at all
                // (design decision 4's negative limb), so this is literally true.
                sex: "not recorded".to_string(),
                // NAMED, not blank. `unwrap_or_default()` put an empty string here, which
                // renders as an empty field — indistinguishable from *not-yet-asked* or from
                // a rendering bug. Principle 4 wants those states distinct, and the two
                // fields either side of this one already name their absence.
                birth_date: query
                    .birth_date
                    .clone()
                    .unwrap_or_else(|| "not recorded".to_string()),
                identifiers: query.identifiers.clone(),
                // A chart nobody has confirmed the identity of yet. Claiming `Confirmed`
                // here would put a trust state on screen that no act has earned.
                trust: cairn_patient_search::TrustState::Unconfirmed,
            });
        id
    }
}

// Both ports do their work WHEN AWAITED, not when the future is built, and that is a
// correctness property rather than a style preference. These were once written as a
// synchronous call followed by `async move { Ok(value) }`, which performs the effect at CALL
// time: building a `register` future and dropping it still minted a patient, while a live
// implementation would have done nothing (its await IS the query). A 2b cancellation test
// passing against the mock would then have proven nothing about the real one.
//
// `async fn` in the impl of an RPITIT trait method is allowed and is what clippy asks for
// here; the declared `+ Send` bound is still checked against these bodies. They satisfy it
// because the mutex guard is taken and dropped with no await point in between — see the
// `search_now` / `register_now` split, which exists for exactly that reason.

impl PatientSearch for MockData {
    async fn search(&self, query: &SearchQuery, today: &str) -> Result<CandidateList, DataError> {
        // The armed failure (#660) is consumed INSIDE the async body, for the same reason
        // `search_now` was split out at all: this port must do its work when AWAITED, never
        // when the future is built. Taking it before the body would let a future that is
        // built and dropped spend the failure, so the call a test meant to fail would
        // quietly succeed.
        match self.armed_failure() {
            Some(e) => Err(e),
            None => Ok(self.search_now(query, today)),
        }
    }
}

impl PatientRegistration for MockData {
    async fn register(
        &self,
        attested: AttestedSearch,
        name: Option<&str>,
    ) -> Result<Uuid, (DataError, AttestedSearch)> {
        // The attestation goes BACK inside the error, exactly as the live port does — and it
        // is returned WITHOUT minting a patient, so a failed registration leaves no chart. A
        // mock that dropped the attestation here would let every `--mock` test of the
        // recovery walk pass while the real walk latched the token store, which is why #659
        // and #660 belong to the same slice.
        match self.armed_failure() {
            Some(e) => Err((e, attested)),
            None => Ok(self.register_now(&attested, name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::fixtures::FIXTURE_UUID;
    use crate::port::ClinicalData;
    use cairn_gui_funnel::{bound_for_prompt, TokenStore};

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
        // Bounded before recording, because `record` takes nothing else — which is what
        // stops a prompt showing five from signing that it displayed forty.
        let token = store.record(query, bound_for_prompt(&displayed)).unwrap();
        let attested = store.take(token).unwrap();

        let id = data.register(attested, Some(typed)).await.unwrap();
        store.commit();
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
        let token = store.record(query, bound_for_prompt(&displayed)).unwrap();
        let attested = store.take(token).unwrap();

        let id = data.register(attested, Some("   ")).await.unwrap();
        store.commit();
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

    #[tokio::test]
    async fn a_birth_date_alone_finds_the_chart_that_carries_it() {
        // PASS 2 WAS ENTIRELY UNEXERCISED: every search in this suite passed `None` for the
        // date, so deleting the birth-date arm of `matches` left the whole suite green. The
        // list this produces is what a registration attests to as its displayed set, so a
        // dead pass 2 understates what the clerk was shown.
        let data = MockData::with_fixtures();
        let list = data
            .search(&SearchQuery::new("", Some("1979-11-20"), &[]), TODAY)
            .await
            .unwrap();
        assert_eq!(found(&list), ["Michaelowski, Samantha"]);
    }

    #[tokio::test]
    async fn a_reduced_precision_birth_date_matches_only_a_reduced_precision_stored_value() {
        // The same trade `db/046` pass 2 makes: an exact string compare, so a year-only query
        // matches a year-only stored value and nothing else. Narrower recall than a range
        // search, never a WRONG match — and a registrar is frequently told only a year
        // (principle 4).
        let data = MockData::with_fixtures();
        let by_year = data
            .search(&SearchQuery::new("", Some("1975"), &[]), TODAY)
            .await
            .unwrap();
        assert_eq!(found(&by_year), ["unknown-ed-site1-2026-07-03-00ab"]);

        let invented_precision = data
            .search(&SearchQuery::new("", Some("1975-01-01"), &[]), TODAY)
            .await
            .unwrap();
        assert!(
            invented_precision.candidates.is_empty(),
            "a year must not silently become 1 January"
        );
    }

    #[tokio::test]
    async fn an_identifier_matches_only_within_its_own_system() {
        // Dropping the SYSTEM from pass 1's comparison left the suite green, because no two
        // fixtures share an identifier value. An MRN would then match a National id carrying
        // the same digits — putting a different patient's chart in front of the clerk AND
        // into the attested displayed list.
        let data = MockData::with_fixtures();
        let right_system = data
            .search(
                &SearchQuery::new("", None, &[("MRN".into(), "12345".into())]),
                TODAY,
            )
            .await
            .unwrap();
        assert_eq!(right_system.candidates.len(), 1, "her MRN must find her");

        let wrong_system = data
            .search(
                &SearchQuery::new("", None, &[("National".into(), "12345".into())]),
                TODAY,
            )
            .await
            .unwrap();
        assert!(
            wrong_system.candidates.is_empty(),
            "12345 is her MRN, not her National id — matching it as one is a wrong chart"
        );
    }

    #[tokio::test]
    async fn a_query_token_that_matches_is_enough_even_when_another_does_not() {
        // The name pass is a DISJUNCTION. Every other search here uses a single token, so a
        // conjunction would have passed the suite — and under it a clerk typing a middle name
        // the chart does not carry gets zero results and creates a duplicate. The module doc
        // calls a missed candidate "the dangerous direction"; this pins it.
        let data = MockData::with_fixtures();
        assert_eq!(
            found(&browse(&data, "Samantha Jane Michaelowski").await),
            ["Michaelowski, Samantha"]
        );
    }

    #[tokio::test]
    async fn a_confirmed_chart_is_not_displayed_as_identity_pending() {
        // The only trust assertion in this suite checked for `Unconfirmed`, so hard-coding
        // that in `as_candidate` would have passed — showing every confirmed chart as
        // identity-pending. That is alarm fatigue on the one flag that guards chart
        // selection.
        let data = MockData::with_fixtures();
        let list = browse(&data, "mich").await;
        assert_eq!(
            list.candidates[0].trust,
            cairn_patient_search::TrustState::Confirmed
        );
    }

    #[tokio::test]
    async fn a_displayed_age_says_it_came_from_a_date_of_birth() {
        // `basis` is the provenance label beside the number. Nothing pinned it, so "dob"
        // could have become "estimated" — a displayed age claiming a provenance it does not
        // have is the precise-untruth shape principle 4 forbids.
        let data = MockData::with_fixtures();
        let amina = browse(&data, "amina").await;
        let age = amina.candidates[0]
            .age
            .as_ref()
            .expect("a full date gives an age");
        assert_eq!(age.basis, "dob");
    }

    #[tokio::test]
    async fn a_fixture_registration_records_only_what_was_actually_supplied() {
        // Three fabrications were all unpinned: a default birth date, a `Confirmed` trust
        // state no act had earned, and a sex nobody supplied. Each would render as fact on
        // the wrong-chart-prevention surface.
        let data = MockData::with_fixtures();
        let mut store = TokenStore::new();
        let typed = "Ruslan Bakhtiyarov";
        let query = SearchQuery::new(typed, None, &[("MRN".into(), "55555".into())]);
        let displayed = data.search(&query, TODAY).await.unwrap();
        let token = store.record(query, bound_for_prompt(&displayed)).unwrap();
        let id = data
            .register(store.take(token).unwrap(), Some(typed))
            .await
            .unwrap();
        store.commit();

        let d = data.demographics(&id.to_string()).unwrap();
        assert_eq!(
            d.birth_date, "not recorded",
            "no date was supplied, and a BLANK field would read as not-yet-asked"
        );
        assert_eq!(d.sex, "not recorded", "the funnel asks for no sex at all");
        let candidate = browse(&data, "bakhtiyarov").await;
        assert_eq!(
            candidate.candidates[0].trust,
            cairn_patient_search::TrustState::Unconfirmed,
            "nobody has confirmed this identity yet"
        );
        assert!(
            candidate.candidates[0].age.is_none(),
            "no date of birth means no age, never an invented one"
        );
    }

    #[test]
    fn every_fixture_is_well_formed_and_distinctly_identified() {
        // The fixture uuids are parsed in `patient()`, so a typo panics here rather than
        // silently hiding a patient. This also pins that no two fixtures share an id, which
        // would make one of them unreachable through `demographics`.
        let population = crate::mock::fixtures::starting_population();
        let mut ids: Vec<_> = population.iter().map(|p| p.uuid).collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "two fixtures share a uuid");
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
            population
                .iter()
                .any(|p| p.uuid.to_string() == FIXTURE_UUID),
            "the long-standing fixture id must survive, or the demographics and note tabs \
             lose the patient their tests name"
        );
    }

    // --- #660: `--mock` must be able to fail, or 2c's two sentences ship untested ---

    /// A list with nothing on it — the "genuinely new patient" case, and all these tests need.
    fn nothing_displayed() -> cairn_gui_funnel::PromptList {
        bound_for_prompt(&CandidateList {
            candidates: vec![],
            incomplete: false,
            incomplete_reason: None,
        })
    }

    /// An armed failure is returned instead of the fixture answer, ONCE.
    ///
    /// One-shot rather than sticky: a sticky mock cannot express *"it failed, the clerk fixed
    /// it, it worked"*, which is the only walk that exercises the recovery path at all.
    #[tokio::test]
    async fn an_armed_browse_failure_fires_once_and_then_the_fixtures_come_back() {
        let data = MockData::with_fixtures();
        data.fail_next(DataError::Unavailable("the node was unreachable".to_string()));

        let first = data.search(&SearchQuery::new("mich", None, &[]), TODAY).await;
        assert!(
            matches!(&first, Err(DataError::Unavailable(t)) if t.contains("unreachable")),
            "the armed failure must reach the caller verbatim — a mock that rewrote it would \
             teach 2c's rendering the wrong sentence; got {first:?}"
        );

        assert_eq!(
            found(&browse(&data, "mich").await),
            ["Michaelowski, Samantha"],
            "the NEXT call must answer from fixtures again"
        );
    }

    /// A registration failure hands the attestation back, exactly as the live port does.
    ///
    /// This is the property the one-shot exists for: a mock that dropped the `AttestedSearch`
    /// on the failing path would let every `--mock` test of the recovery walk pass while the
    /// real walk latched the token store (#659).
    #[tokio::test]
    async fn an_armed_registration_failure_returns_the_attestation_it_was_given() {
        let data = MockData::with_fixtures();
        let mut store = TokenStore::new();
        let token = store
            .record(
                SearchQuery::new("Nobody Here", Some("1990-01-01"), &[]),
                nothing_displayed(),
            )
            .expect("a non-empty query");
        let attested = store.take(token).expect("the only token");

        data.fail_next(DataError::Refused("the floor said no".to_string()));
        let outcome = data.register(attested, Some("Nobody Here")).await;

        let Err((error, restored)) = store.settle(outcome) else {
            panic!("an armed failure must reach the caller as a failure");
        };
        assert!(matches!(&error, DataError::Refused(t) if t.contains("the floor said no")));
        assert_eq!(
            restored,
            cairn_gui_funnel::Restored::Kept,
            "settling must put the search back, or the clerk is made to re-search after a \
             failure that changed nothing"
        );
        assert!(
            store.take(token).is_ok(),
            "and the same token must still be redeemable for the retry"
        );
    }

    /// A future that is BUILT and never awaited must not spend the armed failure.
    ///
    /// The same await-time property `search_now`/`register_now` were split out to preserve. If
    /// the slot were consumed at call time, a dropped future would eat the failure and the
    /// NEXT call — the one the test meant to fail — would quietly succeed.
    #[tokio::test]
    async fn building_a_future_and_dropping_it_does_not_spend_the_armed_failure() {
        let data = MockData::with_fixtures();
        data.fail_next(DataError::Unavailable("still armed".to_string()));

        let q = SearchQuery::new("mich", None, &[]);
        let never_awaited = data.search(&q, TODAY);
        drop(never_awaited);

        let result = data.search(&SearchQuery::new("mich", None, &[]), TODAY).await;
        assert!(
            matches!(&result, Err(DataError::Unavailable(t)) if t.contains("still armed")),
            "the failure must still be armed for the first call that is actually AWAITED; got \
             {result:?}"
        );
    }

    /// THE WHOLE WALK, in `--mock`: register fails, the clerk edits, it succeeds.
    ///
    /// The design's *"Register fails. The form keeps its values."* — and the reason #660 asked
    /// for a one-shot. Nothing else in this crate exercises
    /// `record → take → settle(Err) → discard → record → take → settle(Ok)`.
    #[tokio::test]
    async fn a_failed_registration_is_recoverable_by_editing_and_registering_again() {
        let data = MockData::with_fixtures();
        let mut store = TokenStore::new();

        let first = store
            .record(
                SearchQuery::new("Jon Mistyped", Some("1974-05-06"), &[]),
                nothing_displayed(),
            )
            .expect("a non-empty query");
        let attested = store.take(first).expect("the only token");
        data.fail_next(DataError::Unavailable("a hiccup".to_string()));
        assert!(store
            .settle(data.register(attested, Some("Jon Mistyped")).await)
            .is_err());

        // The clerk corrects the spelling. The pre-edit search must not license the new
        // registration — `discard` is what makes that structural rather than merely unlikely.
        store.discard();
        let second = store
            .record(
                SearchQuery::new("John Corrected", Some("1974-05-06"), &[]),
                nothing_displayed(),
            )
            .expect("a non-empty query");
        let retry = store.take(second).expect("the corrected search");
        let id = store
            .settle(data.register(retry, Some("John Corrected")).await)
            .expect("the second attempt is not armed to fail");

        assert!(!id.is_nil());
        // Probed on the DISTINCTIVE token, not the whole typed name. The mock's name arm
        // matches per token, so browsing "John Corrected" also returns the starting fixture
        // "O'Brien-Smith, John" — wider recall than `db/046`, exactly as this module's header
        // warns. Asserting the whole name here would be asserting the fixture rule, not the
        // property under test.
        assert_eq!(
            found(&browse(&data, "Corrected").await),
            ["John Corrected"],
            "the chart the retry created must be findable, and under the CORRECTED name"
        );
        assert!(
            found(&browse(&data, "Mistyped").await).is_empty(),
            "and the failed attempt must have created NOTHING — a mock that minted a chart \
             for a call it reported as failed would hide a duplicate-chart bug"
        );
    }
}
