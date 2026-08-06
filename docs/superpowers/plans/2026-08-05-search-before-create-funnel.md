# Search-before-create funnel (node tier) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make patient registration a first-class, floored act that carries the search which preceded
it, and give the node a candidate search to precede it with.

**Architecture:** One new event type `identity.registration.asserted` with §5.3's three classes
(`standard` / `unidentified` / `pseudonymous`). Its *structural* floor lives in the database
(`db/045`); *finding* candidates is advisory SQL (`db/046`) with no scoring and no auto-decision.
A new pure crate `cairn-patient-search` holds the read model so the surface that displays candidates
and the act that attests to them cannot disagree. `cairn-node` gets a `patient` module (search +
register orchestrators) and two CLI verbs; `john_doe.rs` is re-expressed to emit the same act.

**Tech Stack:** Rust 1.96 (workspace-pinned), PostgreSQL 18 + `cairn_pgx`, `tokio-postgres`,
PL/pgSQL, `serde_json`, `uuid`.

**Spec:** [`docs/superpowers/specs/2026-08-04-search-before-create-funnel-design.md`](../specs/2026-08-04-search-before-create-funnel-design.md)
**Issue:** [#344](https://github.com/cairn-ehr/cairn-ehr/issues/344)
**Branch:** `feat/search-before-create-funnel-344` (already created, spec already committed)

## Global Constraints

- **Licence:** AGPL-3.0. Every dependency must be AGPL-3.0-compatible, checked *before* adding.
  This plan adds **no new third-party dependency**.
- **TDD, always:** failing test first, watch it fail, minimal code, watch it pass, commit.
- **Inline docs for a junior developer** on every non-trivial function/module: *why* it exists and
  how it fits, not what the next line does.
- **Files under 500 lines** wherever feasible.
- **NOT in this slice** ([#345](https://github.com/cairn-ehr/cairn-ehr/issues/345)): the precedence
  rule's enforcement, retiring `patient.created`, and the ~83-call-site fixture sweep. Do not add
  `cairn_patient_has_events` or touch `db/005`'s step ordering here.
- **NO authorship gate.** `--attester-key` is optional; a standard registration with no human author
  must SUCCEED (graded `Device`). See spec §2.6 — this is a decision, not an oversight.
  *(Corrected after implementation: the optional `--attester-key` itself was never built — every
  registration the shipped slice authors is graded `Device`; the attested path is
  [#359](https://github.com/cairn-ehr/cairn-ehr/issues/359). The no-gate decision stands.)*
- **`search.displayed` MAY be empty** — the normal case for a genuinely new patient.
- **UUIDs bind as text.** `cairn-node` does not enable `tokio-postgres`'s `with-uuid-1`, so a `Uuid`
  parameter has no `ToSql`. Bind `&uuid.to_string()` and cast in SQL: `$1::text::uuid`. Cast UUID
  columns back to text in the SELECT list and parse Rust-side.
- **Guard before connect.** DB-gated tests take `db::test_serial_guard(&base)` *before*
  `connect_and_load_schema`.
- **Test env:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`.
- **Run the FULL workspace suite** (`cargo test`), never `-p cairn-node` alone — a per-crate run
  hides cross-crate call-site arity breaks in `cairn-sync/tests/`. Do not pipe to `tail`; it masks
  cargo's exit code.
- **Never hard-code cryptographic material in tests.** Derive keys/seeds at runtime
  (`std::array::from_fn(|i| …)`); a literal trips CodeQL's `rust/hard-coded-cryptographic-value`.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/cairn-event/src/registration.rs` | The wire shape: registration body builder + twin renderer + the class enum | 1 |
| `crates/cairn-patient-search/Cargo.toml` | New pure crate manifest (uuid + serde only, no DB driver) | 2 |
| `crates/cairn-patient-search/src/lib.rs` | Re-exports + crate doc | 2 |
| `crates/cairn-patient-search/src/query.rs` | `SearchQuery` + tokenisation | 2 |
| `crates/cairn-patient-search/src/candidate.rs` | `Candidate`, `CandidateList`, `TrustState`, `Age`, `age_years` | 2 |
| `crates/cairn-patient-search/src/attestation.rs` | `SearchAttestation::from_displayed` | 2 |
| `db/045_patient_registration.sql` | Event type + structural floor + twin row + retained-set projection + earliest-wins VIEW | 3 |
| `db/tests/045_patient_registration_test.sql` | SQL mirror of the floor guards | 3 |
| `crates/cairn-node/tests/patient_registration.rs` | DB-gated floor tests | 3 |
| `db/046_patient_search.sql` | `cairn_search_candidates` — advisory three-pass blocking | 4 |
| `crates/cairn-node/src/patient/mod.rs` | Module doc + re-exports | 5 |
| `crates/cairn-node/src/patient/search.rs` | `search_patients` — the ONE candidate mapping | 5 |
| `crates/cairn-node/src/patient/register.rs` | `register_patient` — mint, build, sign, submit in one txn | 6 |
| `crates/cairn-node/tests/patient_search.rs` | DB-gated search tests | 5 |
| `crates/cairn-node/tests/patient_register.rs` | DB-gated registration + round-trip tests | 6 |
| `crates/cairn-node/src/john_doe.rs` | Re-expressed onto the registration act | 7 |
| `crates/cairn-node/src/main.rs` | `patient-search` / `patient-register` CLI verbs | 8 |
| `docs/spec/decisions/0061-*.md` | ADR-0061 | 9 |

**Guard files that MUST be updated in the same commit as `db/045` (Task 3):**
`crates/cairn-event/src/schema_generation.rs` (44 → 45) · `crates/cairn-node/src/db.rs` (SCHEMA
list) · `crates/cairn-node/tests/twin_registry.rs` (count 21 → 22 **and** the `expected` vec) ·
`db/tests/034_twin_registry_test.sql` (count 21 → 22).

**`db/045` and `db/046` go in `cairn-node`'s SCHEMA only, NOT `cairn-sync`'s subset.** Sync's subset
already omits `db/010`–`db/014`; a projection dispatches only if its `cairn_projection_apply` row
exists, and that row is inserted by the migration, so a sync database with no `db/045` simply never
dispatches registration — the same arrangement `demographic.identifier.asserted` already relies on.

---

### Task 1: The registration wire shape (`cairn-event`)

**Files:**
- Create: `crates/cairn-event/src/registration.rs`
- Modify: `crates/cairn-event/src/lib.rs` (add `pub mod registration;`)

**Interfaces:**
- Consumes: `serde_json::Value`, `uuid::Uuid` (both already dependencies of `cairn-event`).
- Produces:
  ```rust
  pub const REGISTRATION_EVENT_TYPE: &str = "identity.registration.asserted";
  pub const REGISTRATION_SCHEMA_VERSION: &str = "identity.registration.asserted/1";
  pub enum RegistrationClass { Standard, Unidentified, Pseudonymous }
  impl RegistrationClass { pub fn as_str(self) -> &'static str; }
  pub struct SearchTerms<'a> {
      pub name_tokens: &'a [String],
      pub birth_date: Option<&'a str>,
      pub identifiers: &'a [(String, String)],
  }
  pub struct SearchAttestationInput<'a> {
      pub terms: SearchTerms<'a>,
      pub displayed: &'a [Uuid],
      pub incomplete: bool,
  }
  pub struct RegistrationAssertion<'a> {
      pub class: RegistrationClass,
      pub basis: Option<&'a str>,
      pub search: Option<SearchAttestationInput<'a>>,
  }
  pub fn registration_assertion_body(a: &RegistrationAssertion) -> serde_json::Value;
  pub fn render_registration_twin(a: &RegistrationAssertion) -> String;
  ```

**Why the builder takes primitives instead of `cairn_patient_search::SearchAttestation`:**
`cairn-event` is the wire core and must not depend on a read-model crate — that would invert the
layering. Task 6 wires the two together and Task 6 carries the round-trip test that guards the seam.

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-event/src/registration.rs` containing ONLY the test module below (no
implementation yet), plus `pub mod registration;` added to `crates/cairn-event/src/lib.rs`
alphabetically (after `pub mod medication;`).

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn displayed() -> Vec<Uuid> {
        vec![
            Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111),
            Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222),
        ]
    }

    fn tokens() -> Vec<String> {
        vec!["smith".to_string(), "john".to_string()]
    }

    #[test]
    fn standard_registration_carries_its_search() {
        let ids = displayed();
        let toks = tokens();
        let sys: Vec<(String, String)> = vec![("MRN".into(), "12345".into())];
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms {
                    name_tokens: &toks,
                    birth_date: Some("1980-01-01"),
                    identifiers: &sys,
                },
                displayed: &ids,
                incomplete: false,
            }),
        };
        let b = registration_assertion_body(&a);
        assert_eq!(b["class"], "standard");
        assert_eq!(b["search"]["query"]["birth_date"], "1980-01-01");
        assert_eq!(b["search"]["query"]["name_tokens"][0], "smith");
        assert_eq!(b["search"]["query"]["identifiers"][0]["system"], "MRN");
        assert_eq!(b["search"]["query"]["identifiers"][0]["value"], "12345");
        assert_eq!(b["search"]["displayed"][0], ids[0].to_string());
        assert_eq!(b["search"]["incomplete"], false);
        // No count field: length(displayed) IS the count. Two representations of one
        // number is a lie waiting to happen (design §3).
        assert!(b["search"].get("displayed_count").is_none());
        // basis is omitted entirely for a standard registration (principle 4: a
        // mandatory free-text box here would be satisfiable only by fabrication).
        assert!(b.get("basis").is_none());
    }

    #[test]
    fn an_empty_candidate_list_is_a_valid_search() {
        // The NORMAL case for a genuinely new patient: the search ran and correctly
        // found nothing. `[]` must survive as an empty ARRAY, never become null or
        // vanish — a missing key would read as "no search ran".
        let toks = tokens();
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms { name_tokens: &toks, birth_date: None, identifiers: &[] },
                displayed: &[],
                incomplete: false,
            }),
        };
        let b = registration_assertion_body(&a);
        assert!(b["search"]["displayed"].is_array());
        assert_eq!(b["search"]["displayed"].as_array().unwrap().len(), 0);
        assert!(b["search"]["query"]["birth_date"].is_null());
    }

    #[test]
    fn non_standard_classes_carry_no_search_key_at_all() {
        // Structural absence, not an empty object: a search attestation on an
        // unconscious patient would be a precise untruth (principle 4).
        for class in [RegistrationClass::Unidentified, RegistrationClass::Pseudonymous] {
            let a = RegistrationAssertion {
                class,
                basis: Some("unidentified patient, no ID"),
                search: None,
            };
            let b = registration_assertion_body(&a);
            assert!(b.get("search").is_none(), "{} must carry no search key", class.as_str());
            assert_eq!(b["basis"], "unidentified patient, no ID");
        }
    }

    #[test]
    fn twin_is_non_empty_and_states_the_class_and_how_many_were_seen() {
        let ids = displayed();
        let toks = tokens();
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms { name_tokens: &toks, birth_date: None, identifiers: &[] },
                displayed: &ids,
                incomplete: false,
            }),
        };
        let twin = render_registration_twin(&a);
        assert!(!twin.trim().is_empty(), "the floor requires a non-empty twin");
        assert!(twin.contains("standard"));
        assert!(twin.contains('2'), "the twin states how many candidates were displayed");
    }

    #[test]
    fn twin_says_so_when_the_search_was_incomplete() {
        let toks = tokens();
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms { name_tokens: &toks, birth_date: None, identifiers: &[] },
                displayed: &[],
                incomplete: true,
            }),
        };
        // ADR-0060 decision 2: partial completion is REPORTED, never implied. A reader
        // of the twin alone must not believe the search was exhaustive.
        assert!(render_registration_twin(&a).contains("incomplete"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cairn-event registration`
Expected: FAIL — compile errors, `cannot find type RegistrationAssertion in this scope`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/cairn-event/src/registration.rs` (above the test module):

```rust
//! §5.3/§5.8 patient registration — the wire shape of the act that brings a chart into
//! being, and of the search that preceded it.
//!
//! # Why registration is an event at all
//!
//! Before this type a standard chart came into being as a *side effect* of whatever event
//! happened to carry its `patient_id` first. §5.8 requires the create act to record that N
//! near-matches were displayed, and a side effect has nowhere to record anything. So
//! registration becomes an act, with §5.3's three classes as one discriminant so the
//! floor's precedence rule never needs an exception (design §2.2).
//!
//! # Why the attestation NAMES candidates rather than counting them
//!
//! A duplicate found six months later poses one question: was it on the screen when the
//! clerk clicked create? "Yes" means human judgement failed (fix the UI); "no" means the
//! search failed (fix the comparator). Those have opposite fixes, and a bare `N = 3`
//! cannot tell them apart. So `displayed` carries the candidate ids themselves.
//!
//! The displayed-and-not-chosen set is WEAK evidence — the clerk may never have read it.
//! It is not an `unlink` and must never be projected as a judgement that the charts differ.
//!
//! # Layering
//!
//! The builders take plain primitives rather than `cairn_patient_search::SearchAttestation`
//! because `cairn-event` is the wire core: depending on a read-model crate would invert the
//! layering. `cairn-node`'s `patient::register` wires the two, and carries the round-trip
//! test that keeps the seam honest.
use serde_json::{json, Value};
use uuid::Uuid;

/// The event type registered in `event_type_class` and the twin-check registry (db/045).
pub const REGISTRATION_EVENT_TYPE: &str = "identity.registration.asserted";
/// Wire schema version. Bumping this is an ADDITIVE act (ADR-0012): add fields, never
/// remove or repurpose one.
pub const REGISTRATION_SCHEMA_VERSION: &str = "identity.registration.asserted/1";

/// §5.3's three registration classes. Closed set — the db/045 floor refuses anything else,
/// so adding a member here means adding it there in the same commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationClass {
    /// Normal registration. The only class that carries — and must carry — a search.
    Standard,
    /// §5.4 John Doe. Search-AFTER-create by necessity: there is nothing to search with.
    Unidentified,
    /// §5.6 legally sanctioned anonymous/protective care.
    Pseudonymous,
}

impl RegistrationClass {
    /// The wire token. These strings are the floor's closed set — do not reword them.
    pub fn as_str(self) -> &'static str {
        match self {
            RegistrationClass::Standard => "standard",
            RegistrationClass::Unidentified => "unidentified",
            RegistrationClass::Pseudonymous => "pseudonymous",
        }
    }
}

/// What the clerk actually typed. Borrowed so the caller keeps ownership of its buffers.
#[derive(Debug, Clone, Copy)]
pub struct SearchTerms<'a> {
    /// Lower-cased name tokens. May be empty if the clerk searched by identifier alone.
    pub name_tokens: &'a [String],
    /// ISO `YYYY-MM-DD`, or `None` when not asked/not known.
    pub birth_date: Option<&'a str>,
    /// `(system, value)` pairs, e.g. `("MRN", "12345")`.
    pub identifiers: &'a [(String, String)],
}

/// The search a standard registration attests to.
#[derive(Debug, Clone, Copy)]
pub struct SearchAttestationInput<'a> {
    pub terms: SearchTerms<'a>,
    /// The candidates ACTUALLY on screen. May be empty — that is the normal case for a
    /// genuinely new patient, and it must never be tightened into a non-empty requirement.
    pub displayed: &'a [Uuid],
    /// True when the node knows it could not show everything it found or could not read
    /// some candidate. ADR-0060 decision 2: partial completion is reported, never implied.
    pub incomplete: bool,
}

/// One registration act.
#[derive(Debug, Clone, Copy)]
pub struct RegistrationAssertion<'a> {
    pub class: RegistrationClass,
    /// Why this class. Carried for the non-standard classes, where it is genuinely
    /// informative ("unconscious ED arrival, no ID"). Omitted for `Standard`: there the
    /// class IS the explanation, and a mandatory free-text box would be a required field
    /// satisfiable only by fabrication (principle 4).
    pub basis: Option<&'a str>,
    /// Present iff `class == Standard`. The db/045 floor enforces both directions.
    pub search: Option<SearchAttestationInput<'a>>,
}

/// Build the event payload. Pure — every input is supplied by the caller, so the whole
/// wire shape is unit-testable with no clock, no database and no key.
pub fn registration_assertion_body(a: &RegistrationAssertion) -> Value {
    let mut body = json!({ "class": a.class.as_str() });
    if let Some(basis) = a.basis {
        body["basis"] = json!(basis);
    }
    if let Some(s) = &a.search {
        let identifiers: Vec<Value> = s
            .terms
            .identifiers
            .iter()
            .map(|(system, value)| json!({ "system": system, "value": value }))
            .collect();
        // `displayed` serialises as an array of canonical UUID strings even when empty —
        // an empty ARRAY means "the search ran and found nothing", which is entirely
        // different from an absent `search` key ("no search ran").
        let displayed: Vec<Value> = s.displayed.iter().map(|u| json!(u.to_string())).collect();
        body["search"] = json!({
            "query": {
                "name_tokens": s.terms.name_tokens,
                "birth_date": s.terms.birth_date,
                "identifiers": identifiers,
            },
            "displayed": displayed,
            "incomplete": s.incomplete,
        });
    }
    body
}

/// The mandatory §3.13 legibility twin: this act in plain language, for a reader with no
/// schema at all (principle 11). Mechanically derived from the same inputs as the payload.
pub fn render_registration_twin(a: &RegistrationAssertion) -> String {
    let mut out = format!("Patient registered ({} registration)", a.class.as_str());
    if let Some(basis) = a.basis {
        out.push_str(&format!("; basis: {basis}"));
    }
    if let Some(s) = &a.search {
        out.push_str(&format!(
            "; searched before creating, {} near-match(es) displayed",
            s.displayed.len()
        ));
        if s.incomplete {
            // Never let a reader of the twin alone believe the search was exhaustive.
            out.push_str(" (search incomplete — not everything found could be shown)");
        }
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cairn-event registration`
Expected: PASS, 5 tests.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
git add crates/cairn-event/src/registration.rs crates/cairn-event/src/lib.rs
git commit -m "feat(#344): the registration event wire shape

One act, §5.3's three classes. The search attestation NAMES the displayed
candidates rather than counting them: a duplicate found six months later poses
one question — was it on screen? — and 'search failed' vs 'judgement failed'
have opposite fixes that a bare N cannot distinguish.

Two shapes that read like oversights and are not: an EMPTY displayed array is
the normal case for a genuinely new patient (absent search key means no search
ran — different thing), and basis is omitted for standard registrations because
a mandatory free-text box there is satisfiable only by fabrication (principle 4).

Refs #344"
```

---

### Task 2: The pure read model (`cairn-patient-search`)

**Files:**
- Create: `crates/cairn-patient-search/Cargo.toml`, `src/lib.rs`, `src/query.rs`,
  `src/candidate.rs`, `src/attestation.rs`
- Modify: `Cargo.toml` (workspace `members`)

**Interfaces:**
- Consumes: nothing from Task 1 (deliberately — see Task 1's layering note).
- Produces:
  ```rust
  pub struct SearchQuery { pub name_tokens: Vec<String>, pub birth_date: Option<String>,
                           pub identifiers: Vec<(String, String)> }
  impl SearchQuery {
      pub fn new(raw_name: &str, birth_date: Option<&str>,
                 identifiers: &[(String, String)]) -> SearchQuery;
      pub fn is_empty(&self) -> bool;
  }
  pub enum TrustState { Confirmed, Unconfirmed, UnderReview }
  impl TrustState { pub fn as_str(self) -> &'static str; }
  pub struct Age { pub years: u32, pub basis: String }
  pub fn age_years(birth_date: &str, today: &str) -> Option<u32>;
  pub struct Candidate { pub patient_id: Uuid, pub display_name: String, pub age: Option<Age>,
                         pub trust: TrustState, pub last_activity: Option<String>,
                         pub locale: Option<String>, pub photo_ref: Option<String> }
  pub struct CandidateList { pub candidates: Vec<Candidate>, pub incomplete: bool,
                             pub incomplete_reason: Option<String> }
  pub struct SearchAttestation { pub query: SearchQuery, pub displayed: Vec<Uuid>,
                                 pub incomplete: bool }
  impl SearchAttestation { pub fn from_displayed(query: &SearchQuery,
                                                 list: &CandidateList) -> SearchAttestation; }
  ```

- [ ] **Step 1: Create the crate manifest and register it in the workspace**

Create `crates/cairn-patient-search/Cargo.toml`:

```toml
[package]
name = "cairn-patient-search"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

# Inherit the central workspace lint policy (#144).
[lints]
workspace = true

[dependencies]
# Deliberately minimal, exactly as cairn-medication-view is: this crate must stay
# linkable by a future picker WINDOW, which cannot depend on a crate carrying a
# Postgres driver. `v7` is omitted — this crate never MINTS a Uuid, it only carries
# ones that arrive from the database.
uuid = { version = "1", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
```

In the root `Cargo.toml`, add to `members` after `"crates/cairn-medication-view",`:

```toml
    "crates/cairn-patient-search",
```

- [ ] **Step 2: Write the failing tests**

Create `crates/cairn-patient-search/src/query.rs` with ONLY:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_lowercase_tokens_with_punctuation_dropped() {
        let q = SearchQuery::new("O'Brien-Smith,  John", None, &[]);
        assert_eq!(q.name_tokens, vec!["o", "brien", "smith", "john"]);
    }

    #[test]
    fn a_query_with_only_an_identifier_is_not_empty() {
        let q = SearchQuery::new("", None, &[("MRN".into(), "12345".into())]);
        assert!(q.name_tokens.is_empty());
        assert!(!q.is_empty(), "an identifier alone is a real search");
    }

    #[test]
    fn a_query_with_nothing_in_it_is_empty() {
        assert!(SearchQuery::new("   ", None, &[]).is_empty());
    }

    #[test]
    fn a_dob_alone_is_a_real_search() {
        assert!(!SearchQuery::new("", Some("1980-01-01"), &[]).is_empty());
    }
}
```

Create `crates/cairn-patient-search/src/candidate.rs` with ONLY:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn age_counts_whole_years_and_respects_the_birthday() {
        assert_eq!(age_years("1980-06-15", "2026-06-14"), Some(45));
        assert_eq!(age_years("1980-06-15", "2026-06-15"), Some(46));
        assert_eq!(age_years("1980-06-15", "2026-06-16"), Some(46));
    }

    #[test]
    fn a_partial_or_unparseable_dob_yields_no_age_rather_than_a_guess() {
        // Principle 4: an imprecise near-truth beats a precise untruth. A year-only DOB
        // must NOT silently become "assume 1 January".
        assert_eq!(age_years("1980", "2026-01-01"), None);
        assert_eq!(age_years("", "2026-01-01"), None);
        assert_eq!(age_years("not-a-date", "2026-01-01"), None);
    }

    #[test]
    fn a_future_birth_date_yields_no_age_rather_than_underflowing() {
        assert_eq!(age_years("2030-01-01", "2026-01-01"), None);
    }

    #[test]
    fn an_incomplete_list_says_so_and_says_why() {
        let list = CandidateList {
            candidates: vec![],
            incomplete: true,
            incomplete_reason: Some("2 candidates could not be read".into()),
        };
        assert!(list.incomplete);
        assert!(list.incomplete_reason.is_some());
    }

    #[test]
    fn trust_states_render_the_tokens_the_chart_contract_uses() {
        // §5.7's projection-side contract. A picker must be able to show a John Doe
        // chart AS identity-pending — that chart is exactly the one a clerk needs when
        // the family arrives with a name.
        assert_eq!(TrustState::Confirmed.as_str(), "confirmed");
        assert_eq!(TrustState::Unconfirmed.as_str(), "unconfirmed");
        assert_eq!(TrustState::UnderReview.as_str(), "under-review");
    }

    #[test]
    fn a_candidate_carries_a_photo_reference_never_bytes() {
        let c = Candidate {
            patient_id: Uuid::from_u128(1),
            display_name: "Smith, John".into(),
            age: Some(Age { years: 46, basis: "dob".into() }),
            trust: TrustState::Confirmed,
            last_activity: Some("2026-08-01".into()),
            locale: Some("Bamaga QLD".into()),
            photo_ref: Some("b3:deadbeef".into()),
        };
        assert_eq!(c.photo_ref.as_deref(), Some("b3:deadbeef"));
    }
}
```

Create `crates/cairn-patient-search/src/attestation.rs` with ONLY:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{Candidate, CandidateList, TrustState};
    use uuid::Uuid;

    fn candidate(n: u128) -> Candidate {
        Candidate {
            patient_id: Uuid::from_u128(n),
            display_name: format!("Patient {n}"),
            age: None,
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    #[test]
    fn the_attestation_names_exactly_what_the_list_held_in_order() {
        let list = CandidateList {
            candidates: vec![candidate(1), candidate(2)],
            incomplete: false,
            incomplete_reason: None,
        };
        let q = SearchQuery::new("smith", None, &[]);
        let a = SearchAttestation::from_displayed(&q, &list);
        assert_eq!(a.displayed, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
        assert!(!a.incomplete);
    }

    #[test]
    fn incompleteness_propagates_from_the_list_it_was_built_from() {
        // The whole reason this constructor exists: the surface that DISPLAYS and the act
        // that ATTESTS must not be able to disagree. A registration must never swear to a
        // complete search over a list the node knew was partial.
        let list = CandidateList {
            candidates: vec![candidate(7)],
            incomplete: true,
            incomplete_reason: Some("one chart unreadable".into()),
        };
        let q = SearchQuery::new("smith", None, &[]);
        assert!(SearchAttestation::from_displayed(&q, &list).incomplete);
    }

    #[test]
    fn an_empty_list_attests_to_an_empty_search_not_to_no_search() {
        let list = CandidateList { candidates: vec![], incomplete: false, incomplete_reason: None };
        let q = SearchQuery::new("nobody", None, &[]);
        let a = SearchAttestation::from_displayed(&q, &list);
        assert!(a.displayed.is_empty());
        assert_eq!(a.query.name_tokens, vec!["nobody"]);
    }
}
```

Create `crates/cairn-patient-search/src/lib.rs`:

```rust
//! The shared, pure patient-search read model: what a candidate IS, and the one definition
//! of what a registration attests to.
//!
//! # Why this is its own crate
//!
//! Same reason as `cairn-medication-view`: a future picker window cannot depend on a crate
//! carrying a Postgres driver, and the node and the window must not be able to answer
//! *"what was displayed?"* differently. A divergence there means a registration swearing to
//! candidates the clerk never saw — the exact forensic record the funnel exists to produce.
//!
//! Deliberately dependency-light (uuid + serde). No database, no clock: `age_years` takes
//! `today` as an argument so the whole crate is unit-testable and the edge owns the clock.
pub mod attestation;
pub mod candidate;
pub mod query;

pub use attestation::SearchAttestation;
pub use candidate::{age_years, Age, Candidate, CandidateList, TrustState};
pub use query::SearchQuery;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p cairn-patient-search`
Expected: FAIL — `cannot find type SearchQuery in this scope` and siblings.

- [ ] **Step 4: Write the implementations**

Prepend to `crates/cairn-patient-search/src/query.rs`:

```rust
//! What the clerk typed, normalised into the keys the blocking passes use.
use serde::{Deserialize, Serialize};

/// A normalised search query. Culture-neutral by construction: it tokenises on
/// non-alphanumerics and lower-cases, and does NOTHING else — no phonetics, no nickname
/// expansion, no name-order assumption. Locale-specific comparison is the advisory
/// matcher's job (ADR-0014), and baking one culture's name model in here would be exactly
/// the cultural capture that ADR forbids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Lower-cased alphanumeric tokens from the typed name. May be empty.
    pub name_tokens: Vec<String>,
    /// ISO `YYYY-MM-DD` as typed, or `None`.
    pub birth_date: Option<String>,
    /// `(system, value)` pairs.
    pub identifiers: Vec<(String, String)>,
}

impl SearchQuery {
    /// Normalise raw operator input. `raw_name` is split on any non-alphanumeric run, so
    /// "O'Brien-Smith, John" yields four tokens; each blocking pass then matches on tokens
    /// rather than on a whole string, which is what lets a name typed in a different order
    /// still find the chart.
    pub fn new(raw_name: &str, birth_date: Option<&str>, identifiers: &[(String, String)]) -> Self {
        let name_tokens = raw_name
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();
        Self {
            name_tokens,
            birth_date: birth_date.map(str::to_string).filter(|d| !d.trim().is_empty()),
            identifiers: identifiers.to_vec(),
        }
    }

    /// True when there is nothing to search on. The db/045 floor refuses a registration
    /// whose attested query is empty: "I searched for nothing and found nothing" is not a
    /// search, and must not be allowed to satisfy the funnel.
    pub fn is_empty(&self) -> bool {
        self.name_tokens.is_empty() && self.birth_date.is_none() && self.identifiers.is_empty()
    }
}
```

Prepend to `crates/cairn-patient-search/src/candidate.rs`:

```rust
//! One row of a candidate list — what §5.8 item 1 requires be shown before a chart may be
//! created: photo, age, locale, last visit, and (Cairn's addition) the chart's trust state.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// §5.7's chart trust states, projection-side contract.
///
/// Load-bearing for search, not decoration: a John Doe registered an hour ago is precisely
/// the chart a clerk must find when the family arrives with a name. A search that hid
/// identity-pending charts would manufacture a duplicate every time an unidentified patient
/// is later named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustState {
    Confirmed,
    Unconfirmed,
    UnderReview,
}

impl TrustState {
    pub fn as_str(self) -> &'static str {
        match self {
            TrustState::Confirmed => "confirmed",
            TrustState::Unconfirmed => "unconfirmed",
            TrustState::UnderReview => "under-review",
        }
    }
}

/// An age together with what it was derived from. The basis travels because an age derived
/// from a document-verified DOB and one derived from a clinician's estimate are different
/// claims, and a clerk comparing candidates needs to know which is which (principle 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Age {
    pub years: u32,
    pub basis: String,
}

/// Whole years between two ISO `YYYY-MM-DD` dates, or `None` when that cannot be said
/// honestly.
///
/// Returns `None` for a partial date (`"1980"`), an unparseable one, or a birth date after
/// `today`. It deliberately does NOT fill in a missing month/day: a year-only DOB silently
/// becoming "1 January" is a precise untruth, and principle 4 prefers showing no age at all.
/// `today` is a parameter so this stays pure and the edge owns the clock.
pub fn age_years(birth_date: &str, today: &str) -> Option<u32> {
    let ymd = |s: &str| -> Option<(i32, u32, u32)> {
        let mut it = s.split('-');
        let y = it.next()?.parse::<i32>().ok()?;
        let m = it.next()?.parse::<u32>().ok()?;
        let d = it.next()?.parse::<u32>().ok()?;
        if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        Some((y, m, d))
    };
    let (by, bm, bd) = ymd(birth_date)?;
    let (ty, tm, td) = ymd(today)?;
    let mut years = ty - by;
    // Not yet had this year's birthday → one fewer whole year.
    if (tm, td) < (bm, bd) {
        years -= 1;
    }
    u32::try_from(years).ok()
}

/// One chart offered to the clerk before they may create a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub patient_id: Uuid,
    /// The §4.2 display-winner name, or the John Doe callsign.
    pub display_name: String,
    pub age: Option<Age>,
    pub trust: TrustState,
    /// ISO date of the chart's last activity, for "have I seen this person recently?".
    pub last_activity: Option<String>,
    /// A one-line locale hint (suburb/town), never the full address: the point is to
    /// disambiguate two people with one name, not to display a dossier.
    pub locale: Option<String>,
    /// A content-addressed blob reference, NEVER bytes. Fetching the image is byte-tier
    /// work (ADR-0013) and must not sit on the search latency path.
    pub photo_ref: Option<String>,
}

/// The candidates plus what the node knows it could NOT show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateList {
    pub candidates: Vec<Candidate>,
    /// True when the node found more than it could show, or could not read something it
    /// found. ADR-0060 decision 2: partial completion is reported, never implied — a clerk
    /// must never believe an exhaustive search happened when it did not.
    pub incomplete: bool,
    /// Human-readable reason, shown beside the list. `Some` whenever `incomplete`.
    pub incomplete_reason: Option<String>,
}
```

Prepend to `crates/cairn-patient-search/src/attestation.rs`:

```rust
//! The ONE definition of what a registration attests to.
//!
//! This constructor is the whole reason the crate exists. If the surface that displays
//! candidates and the act that attests to them each built their own answer to "what was
//! shown?", a registration could swear to candidates the clerk never saw — destroying
//! exactly the forensic record the funnel is for. So the attestation is derived FROM the
//! displayed list and cannot be constructed independently of one.
use crate::candidate::CandidateList;
use crate::query::SearchQuery;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchAttestation {
    pub query: SearchQuery,
    /// The candidate ids that were on the screen, in display order.
    pub displayed: Vec<Uuid>,
    /// Carried straight through from the list — never re-decided here.
    pub incomplete: bool,
}

impl SearchAttestation {
    /// Derive the attestation from the query and the list that was actually displayed.
    pub fn from_displayed(query: &SearchQuery, list: &CandidateList) -> Self {
        Self {
            query: query.clone(),
            displayed: list.candidates.iter().map(|c| c.patient_id).collect(),
            incomplete: list.incomplete,
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cairn-patient-search`
Expected: PASS, 13 tests.

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
git add crates/cairn-patient-search Cargo.toml
git commit -m "feat(#344): the pure patient-search read model

A new dependency-light crate (uuid + serde), for the cairn-medication-view
reason: a future picker window cannot depend on a crate carrying a Postgres
driver, and the node and the window must not be able to answer 'what was
displayed?' differently.

SearchAttestation::from_displayed is the point of the crate — the attestation is
DERIVED from the displayed list and cannot be built independently of one, so a
registration can never swear to candidates the clerk never saw.

age_years returns None for a partial DOB rather than assuming 1 January: a
year-only date silently becoming a precise one is exactly the precise untruth
principle 4 rejects.

Refs #344"
```

---

### Task 3: The floor — event type, structural check, projection (`db/045`)

**Files:**
- Create: `db/045_patient_registration.sql`, `db/tests/045_patient_registration_test.sql`,
  `crates/cairn-node/tests/patient_registration.rs`
- Modify: `crates/cairn-event/src/schema_generation.rs` (44 → 45) ·
  `crates/cairn-node/src/db.rs` (SCHEMA list) ·
  `crates/cairn-node/tests/twin_registry.rs` (count + `expected` vec) ·
  `db/tests/034_twin_registry_test.sql` (count)

**Interfaces:**
- Consumes: `REGISTRATION_EVENT_TYPE` / the body shape from Task 1.
- Produces (SQL): `cairn_check_registration_assertion(p_type text, b jsonb) RETURNS void` ·
  table `patient_registration` · `patient_registration_apply(e event_log) RETURNS void` ·
  view `patient_registration_current`.

> **Do NOT add the precedence rule here.** `cairn_patient_has_events` and the `db/005` call site
> belong to [#345](https://github.com/cairn-ehr/cairn-ehr/issues/345). See spec §2.3.

- [ ] **Step 1: Write the failing DB-gated tests**

Create `crates/cairn-node/tests/patient_registration.rs`. Model the harness on
`crates/cairn-node/tests/demographics.rs` (same `common::setup` + `submit_signed` idiom, and
`db::test_serial_guard(&base)` BEFORE `connect_and_load_schema`). Test bodies:

```rust
//! db/045 floor tests: the structural contract of a registration act.
//!
//! What these must prove, beyond "the happy path works":
//!   * absence for the non-standard classes is STRUCTURAL, not merely optional;
//!   * an empty displayed list is ACCEPTED (the normal new-patient case);
//!   * an unattested standard registration is ACCEPTED (spec §2.6 — a grade, not a gate).
mod common;

// ... standard harness preamble, mirroring demographics.rs ...

#[tokio::test]
async fn standard_registration_with_a_well_formed_search_is_accepted() { /* submit, expect Ok */ }

#[tokio::test]
async fn standard_registration_without_a_search_is_refused() {
    // expect Err whose message contains "standard registration must carry its search"
}

#[tokio::test]
async fn unidentified_registration_carrying_a_search_is_refused() {
    // The trap this test exists for: absence must be STRUCTURAL. An implementation that
    // merely made `search` optional would pass every other test in this file and let a
    // John Doe claim a search nobody could have run.
}

#[tokio::test]
async fn an_unknown_class_is_refused() {
    // expect "unknown registration class"
}

#[tokio::test]
async fn a_non_uuid_in_displayed_is_refused() { }

#[tokio::test]
async fn a_missing_incomplete_flag_is_refused() {
    // ADR-0060 decision 2: completeness must be STATED, never assumed by its absence.
}

#[tokio::test]
async fn an_empty_query_object_is_refused() { }

#[tokio::test]
async fn an_empty_displayed_array_is_accepted() {
    // The NORMAL case for a genuinely new patient. This test exists so nobody later
    // "tightens" the array into a non-empty requirement and makes registering the first
    // patient on a fresh node impossible.
}

#[tokio::test]
async fn a_standard_registration_with_no_human_author_is_accepted() {
    // SPEC §2.6 — DO NOT "FIX" THIS INTO A REFUSAL.
    // Authorship confidence is a grade, not a gate (§5.11). Gating here would block care
    // documentation at 03:00 when a clerk's key is not unlocked, push named patients
    // through the John Doe path, and produce NO forensic record in the case it fires.
    // Assertion message must say exactly that.
}

#[tokio::test]
async fn a_registration_naming_a_registrar_who_did_not_sign_is_refused() {
    // Proves spec §2.6's "unforgeable for free" claim rather than assuming it: the refusal
    // comes from db/005's UNCONDITIONAL cairn_authorship_bound (step 4b), with no rule
    // added by db/045. Contributor role must be a BEARING one (e.g. "authored").
}

#[tokio::test]
async fn a_missing_twin_is_refused() { }

#[tokio::test]
async fn the_projection_keeps_every_registration_and_the_view_picks_the_earliest() {
    // Retained set + earliest-wins VIEW: registration is a BIRTH act, so the winner is the
    // earliest by (hlc_wall, hlc_counter, node_origin COLLATE "C", content_address), not
    // the latest as every standing-state overlay uses.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test patient_registration`
Expected: FAIL — `unknown event_type identity.registration.asserted (no classification — fail closed)`.

- [ ] **Step 3: Write `db/045_patient_registration.sql`**

Model the file structure on `db/010_demographics.sql` exactly: `BEGIN;` … `COMMIT;`, an
`event_type_class` row, the check fn, the twin-registry row *after* the fn, the projection table
with a paired idempotent `ALTER` for any added column, the apply fn, `REVOKE EXECUTE … FROM PUBLIC`,
`GRANT SELECT … TO cairn_agent`, and the `cairn_projection_apply` row with the #214
`DO UPDATE … WHERE IS DISTINCT FROM` arm.

Content requirements:

```sql
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.registration.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;
```

`cairn_check_registration_assertion(p_type text, b jsonb) RETURNS void` — one distinct, legible
exception per rule:

| Rule | Message must contain |
|---|---|
| `class` ∈ {standard, unidentified, pseudonymous} | `unknown registration class` |
| `class ≠ standard` ⇒ `basis` non-blank string | `non-standard registration states why` |
| `class = standard` ⇒ `search` present, object | `standard registration must carry its search` |
| `class ≠ standard` ⇒ `search` absent | `a search attestation the registrar could not have made` |
| `search.query` present, object, ≥1 non-empty term | `a search with no terms is not a search` |
| `search.displayed` present, array, every element a UUID | `candidate list malformed` |
| `search.incomplete` present, boolean | `completeness must be stated, not assumed` |

Validate the UUID elements with a cast inside an exception block, or by
`jsonb_array_elements_text(...) ~* '^[0-9a-f]{8}-...'`; either is fine, but an empty array MUST
pass. Carry a comment on that branch saying so.

Twin-registry row (placed AFTER the check fn so the fail-closed load-time trigger sees it declared):

```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.registration.asserted', 'cairn_check_registration_assertion',
     'registration requires a non-empty authored twin (§3.13)')
ON CONFLICT (event_type) DO NOTHING;
```

`patient_registration` — retained set, PK `(patient_id, content_address)` so every registration event
keeps its own row; columns `patient_id UUID NOT NULL`, `class TEXT NOT NULL`, `basis TEXT`,
`displayed_count INTEGER NOT NULL` (**derived at projection time from `jsonb_array_length`** — the
wire carries no such field, and this column is a read convenience, not a second source of truth: say
so in a comment), `search_incomplete BOOLEAN`, `registered_hlc_wall BIGINT NOT NULL`,
`registered_hlc_count INTEGER NOT NULL`, `registered_origin TEXT NOT NULL`,
`content_address BYTEA NOT NULL`, `first_seen TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()`.

`patient_registration_apply(e event_log)` — sealed rows return early (copy the ADR-0052 §2
seal-robustness comment and guard from `db/010`'s apply fn verbatim in intent);
`INSERT … ON CONFLICT (patient_id, content_address) DO NOTHING` is correct here **because the PK
includes the content address**, so the same event re-applied is genuinely the same row (note this
explicitly — it is NOT the #254 pattern being repeated).

`patient_registration_current` VIEW — earliest-wins:

```sql
CREATE OR REPLACE VIEW patient_registration_current AS
SELECT DISTINCT ON (patient_id)
    patient_id, class, basis, displayed_count, search_incomplete,
    registered_hlc_wall, registered_hlc_count, registered_origin, content_address
FROM patient_registration
ORDER BY patient_id,
         registered_hlc_wall ASC, registered_hlc_count ASC,
         registered_origin COLLATE "C" ASC, content_address ASC;
```

Carry a comment explaining the ASC: a registration is a **birth** act, so the winner is the
*earliest*, the mirror image of every standing-state overlay's latest-wins. `COLLATE "C"` per
ADR-0045 so a federation of mixed default collations converges.

Register the apply fn at `run_order 10` (alongside `patient_chart_apply`, since a registration is
also a chart-birth event).

- [ ] **Step 4: Wire the four guards**

```bash
# 1. schema generation
#    crates/cairn-event/src/schema_generation.rs: SCHEMA_GENERATION: i32 = 44  ->  45
# 2. the loader's SCHEMA list — append after the 044 entry in crates/cairn-node/src/db.rs:
#      (
#          "045_patient_registration",
#          include_str!("../../../db/045_patient_registration.sql"),
#      ),
# 3. crates/cairn-node/tests/twin_registry.rs: 21 -> 22 in the count assertion AND add
#      ("identity.registration.asserted", "cairn_check_registration_assertion",
#       Some("registration requires a non-empty authored twin (§3.13)")),
#    to the `expected` vec (it is asserted byte-for-byte, not just counted).
# 4. db/tests/034_twin_registry_test.sql: both the `n <> 21` guard and its message -> 22.
```

- [ ] **Step 5: Write the SQL mirror**

Create `db/tests/045_patient_registration_test.sql` mirroring the Rust floor tests that can be
expressed without a signature (the structural check called directly). Follow
`db/tests/043_deferred_readjudication_test.sql` for the `DO $$ … RAISE EXCEPTION 'FAIL: …' … $$;`
idiom. It is picked up automatically — `scripts/run-db-sql-tests.sh` globs `db/tests/[0-9]*.sql`.
At minimum mirror: unknown class refused · standard-without-search refused ·
unidentified-with-search refused · empty `displayed` accepted.

- [ ] **Step 6: Run the tests**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test patient_registration --test twin_registry
PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-sql-tests.sh
```
Expected: PASS.

> **If `patient_registration` fails with "invalid input syntax for type bigint"** on a dev database,
> the cause is a stale `cairn_test*` DB whose `event_log` column order predates a column add —
> recreate `cairn_test`/`2`/`3` on :5532. CI is immune (fresh databases).

- [ ] **Step 7: Full workspace suite, then commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test
git add db/045_patient_registration.sql db/tests/045_patient_registration_test.sql \
        db/tests/034_twin_registry_test.sql crates/cairn-node/tests/patient_registration.rs \
        crates/cairn-node/tests/twin_registry.rs crates/cairn-node/src/db.rs \
        crates/cairn-event/src/schema_generation.rs
git commit -m "feat(#344): the registration floor — db/045

Structural check, twin-registry row, and a RETAINED-SET projection whose
_current VIEW picks the EARLIEST registration: a registration is a birth act, so
the winner is the earliest, the mirror image of every standing-state overlay's
latest-wins.

Two floor rules that look like they could be relaxed and must not be: absence of
\`search\` for the non-standard classes is STRUCTURAL (merely-optional would let a
John Doe claim a search nobody could run), and an EMPTY displayed array is
ACCEPTED (the normal case for a genuinely new patient).

No authorship rule added — spec §2.6. A standard registration with no human
author is accepted and graded Device; a test asserts this so the grade cannot be
turned back into a gate silently.

Refs #344"
```

---

### Task 4: The advisory candidate search (`db/046`)

**Files:**
- Create: `db/046_patient_search.sql`
- Modify: `crates/cairn-node/src/db.rs` (SCHEMA list), `crates/cairn-event/src/schema_generation.rs`
  (45 → 46)

**Interfaces:**
- Consumes: `patient_identifier` (db/010), `patient_demographic` (db/011/013), `patient_name`
  (db/012).
- Produces:
  ```sql
  cairn_search_candidates(p_name_tokens text[], p_birth_date text, p_identifiers jsonb)
    RETURNS TABLE (patient_id uuid, matched_pass text)
  ```

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/patient_search.rs` (harness as Task 3). Tests:

```rust
#[tokio::test]
async fn the_identifier_pass_finds_a_chart_by_system_and_value() { }

#[tokio::test]
async fn the_dob_pass_finds_a_chart_by_exact_birth_date() { }

#[tokio::test]
async fn the_name_token_pass_finds_a_chart_by_one_shared_token() { }

#[tokio::test]
async fn a_chart_matching_two_passes_is_returned_once() {
    // Union + dedup. A duplicate row would double-count a candidate in the attestation.
}

#[tokio::test]
async fn a_john_doe_callsign_chart_is_returned_by_its_callsign_token() {
    // Load-bearing: the chart a clerk needs when the family arrives with a name.
    // Contrast with matcher/pipeline/db.py, which EXCLUDES callsigns from its feature
    // space — correct there (a callsign is not evidence of identity) and wrong here
    // (a clerk searching "Unknown-ED" must find the chart in front of them).
}

#[tokio::test]
async fn an_empty_query_returns_no_rows_rather_than_every_chart() {
    // The failure that would matter: a no-term query degenerating into a full scan that
    // "displays" the entire patient population into an attestation.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test patient_search`
Expected: FAIL — `function cairn_search_candidates(...) does not exist`.

- [ ] **Step 3: Write `db/046_patient_search.sql`**

```sql
-- Cairn — §5.8 search-before-create: advisory candidate generation.
--
-- ADVISORY, NOT A FLOOR (design §2.1, ADR-0014). A missed candidate produces a false
-- SPLIT — §5.2's explicitly safe direction — and ADR-0014 already names the standing
-- backstop: the hub-tier background duplicate sweep. So this function never blocks,
-- never vetoes and never decides; it offers rows to a human.
--
-- SQL rather than a call into the advisory Python tier because a registration path must
-- beat paper (§1.2) and §5.11's latency limb is explicit ("type a few chars and enter, no
-- spinner"). Coupling two services on that path buys no safety.
--
-- DRIFT NOTE: the three blocking keys below mirror matcher/pipeline/db.py's three-pass
-- disjunction. They are NOT the same query — the sweep blocks all-by-all, this maps
-- query -> set — so only the KEY EXTRACTION is shared. Convergence is tracked as an issue;
-- if you change a key here, check the matcher.
BEGIN;

CREATE OR REPLACE FUNCTION cairn_search_candidates(
    p_name_tokens text[],
    p_birth_date  text,
    p_identifiers jsonb          -- [{"system": "...", "value": "..."}]
) RETURNS TABLE (patient_id uuid, matched_pass text)
LANGUAGE sql STABLE
SET search_path = public
AS $$
    -- Pass 1: shared identifier. Highest precision — the same system and the same
    -- match_key is near-conclusive, which is why it is also a db/016 hard-veto axis.
    SELECT DISTINCT pi.patient_id, 'identifier'::text
      FROM patient_identifier pi
      JOIN jsonb_array_elements(COALESCE(p_identifiers, '[]'::jsonb)) q
        ON pi.system = (q ->> 'system')
       AND pi.match_key = (q ->> 'value')
    UNION
    -- Pass 2: exact DOB. No date parsing, no range logic — an exact string compare on the
    -- projected value, matching the deliberately parse-free db/016 discipline.
    SELECT DISTINCT pd.patient_id, 'dob'::text
      FROM patient_demographic pd
     WHERE p_birth_date IS NOT NULL
       AND pd.field = 'dob'
       AND pd.value = p_birth_date
    UNION
    -- Pass 3: shared name token. Culture-neutral: EXACT token equality in ANY position, so
    -- a name typed in a different order still finds the chart, with no name-order model.
    --
    -- The tokenising expression is COPIED VERBATIM from matcher/src/cairn_matcher/pipeline/
    -- db.py's _GROUPS_SQL: `regexp_split_to_table(lower(normalize(value, NFC)), '\s+')`.
    -- Same key extraction, so a chart the sweep would pair is a chart this search finds.
    -- NFC normalisation is load-bearing, not decoration: without it a composed and a
    -- decomposed "José" are different tokens and the chart is silently unfindable.
    --
    -- Exact equality, NOT `LIKE '%token%'`: a leading-wildcard match cannot use an index at
    -- all, and the §7 budget is 5 s to find an existing chart. Equality keeps the door open
    -- to an expression index on the same expression when a node grows large enough to need
    -- one.
    --
    -- Callsigns ARE included here, unlike in the matcher (which excludes them via
    -- `use_key <> ALL(...)`). Both are right: a callsign is not evidence of identity, so it
    -- must not feed the scorer — but a clerk must be able to find the John Doe in front of
    -- them.
    SELECT DISTINCT pn.patient_id, 'name'::text
      FROM patient_name pn
      CROSS JOIN LATERAL regexp_split_to_table(lower(normalize(pn.value, NFC)), '\s+') AS tok
      JOIN unnest(COALESCE(p_name_tokens, ARRAY[]::text[])) t
        ON tok = lower(normalize(t, NFC))
$$;

REVOKE EXECUTE ON FUNCTION cairn_search_candidates(text[], text, jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_search_candidates(text[], text, jsonb) TO cairn_agent;

COMMIT;
```

> **Note for the implementer:** pass 3 is a full scan of `patient_name` today, because no index
> exists on the tokenising expression. That is acceptable at current scale and is NOT to be
> "fixed" by adding an index speculatively — [#336](https://github.com/cairn-ehr/cairn-ehr/issues/336)
> already tracks the same shape for the med-list read, and the honest move is to measure first.
> If the DB-gated test shows a pathological result set, **report the skip, never silently cap**
> (the existing oversized-block discipline) and feed it into the caller's `incomplete` flag (Task 5).

- [ ] **Step 4: Wire the schema guards**

`SCHEMA_GENERATION` 45 → 46, and append the `046_patient_search` entry to `db.rs`'s SCHEMA list.

- [ ] **Step 5: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test patient_search`
Expected: PASS, 6 tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add db/046_patient_search.sql crates/cairn-node/tests/patient_search.rs \
        crates/cairn-node/src/db.rs crates/cairn-event/src/schema_generation.rs
git commit -m "feat(#344): advisory candidate search — db/046

Three-pass disjunction (shared identifier / exact DOB / shared name token),
union-deduped, no scoring and no auto-decision. Advisory by ADR-0014: a miss is
a false SPLIT, §5.2's safe direction, with the hub duplicate sweep as backstop.

Callsigns are INCLUDED here, unlike in the matcher's feature space — correct in
both places. A callsign is not evidence of identity (so the matcher excludes it),
but a clerk must be able to find the John Doe in front of them.

An empty query returns no rows rather than the whole population — tested,
because that degeneration would 'display' every chart into an attestation.

Refs #344"
```

---

### Task 5: The search orchestrator (`patient/search.rs`)

**Files:**
- Create: `crates/cairn-node/src/patient/mod.rs`, `crates/cairn-node/src/patient/search.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod patient;`),
  `crates/cairn-node/Cargo.toml` (add `cairn-patient-search` dependency)
- Test: extend `crates/cairn-node/tests/patient_search.rs`

**Interfaces:**
- Consumes: `cairn_search_candidates` (Task 4); `SearchQuery`, `Candidate`, `CandidateList`,
  `TrustState`, `Age`, `age_years` (Task 2).
- Produces:
  ```rust
  pub async fn search_patients<C: GenericClient>(
      client: &C, query: &SearchQuery, today: &str,
  ) -> anyhow::Result<CandidateList>;
  ```

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/patient_search.rs`:

```rust
#[tokio::test]
async fn a_candidate_carries_name_age_trust_and_last_activity() { }

#[tokio::test]
async fn an_identity_pending_chart_comes_back_marked_unconfirmed() {
    // NOT merely "is returned" — the trust state must be visible, or a clerk cannot tell
    // the John Doe from a confirmed chart and the §5.4 identification path breaks.
}

#[tokio::test]
async fn a_chart_with_no_readable_name_is_reported_incomplete_never_dropped() {
    // ADR-0060 decision 2 at the read layer: a candidate the node cannot render must
    // surface as `incomplete` with a reason, never vanish. A silently-dropped candidate is
    // the exact duplicate-creating failure the funnel exists to prevent.
}

#[tokio::test]
async fn an_empty_query_yields_an_empty_complete_list() {
    // Empty AND complete: "found nothing" is a true, exhaustive answer, not a partial one.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test patient_search`
Expected: FAIL — `cannot find function search_patients`.

- [ ] **Step 3: Write the implementation**

Add to `crates/cairn-node/Cargo.toml` `[dependencies]`:
`cairn-patient-search = { path = "../cairn-patient-search" }`

Create `crates/cairn-node/src/patient/mod.rs`:

```rust
//! §5.3/§5.8 patient registration and the search that precedes it.
//!
//! `search` is the ONE mapping from this node's projections to the shared candidate model —
//! the CLI reads through it, and the future picker window and native API (ADR-0023) are
//! expected to wrap this same function rather than re-derive the joins. `register` is the
//! act it feeds.
pub mod register;
pub mod search;
```

Create `crates/cairn-node/src/patient/search.rs`. Requirements:

- Generic over `GenericClient` so a caller can read through an open transaction (the
  `medication/read.rs` precedent).
- Call `cairn_search_candidates($1, $2, $3)` once; bind `p_name_tokens` as `&[String]`,
  `p_birth_date` as `Option<&str>`, `p_identifiers` as a `serde_json::Value`.
- **Short-circuit on an empty query** — return an empty, complete list without touching the
  database. Comment why: an empty query must never become a full scan.
- For each candidate id, read display fields with small, separately-checkable queries
  (`medication/read.rs`'s stated reason: reviewer-legibility over one clever join):
  `patient_name_current` → `display_name` · `patient_demographic` (`field='dob'`) → `Age` via
  `age_years(dob, today)` with `basis` from the row's `provenance` · `chart_trust`/
  `person_chart_trust` (mirror the helper `common::trust_of` uses) → `TrustState` ·
  `patient_chart.last_activity` → ISO date · `patient_address_current` → locale one-liner ·
  photo evidence → `photo_ref` (digest only).
- **Never drop a candidate.** If a display field cannot be read, keep the candidate with the
  honest placeholder and set `incomplete = true` with a reason naming how many.
- Cast every UUID parameter as text (`$1::text::uuid`) and every UUID column back to text.

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test patient_search`
Expected: PASS, 10 tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add crates/cairn-node/src/patient crates/cairn-node/src/lib.rs \
        crates/cairn-node/Cargo.toml crates/cairn-node/tests/patient_search.rs
git commit -m "feat(#344): search_patients — the one candidate mapping

Projections -> the shared candidate model, generic over GenericClient so a
caller can read through an open transaction (the medication/read.rs precedent).
Several small queries rather than one join, for the same reviewer-legibility
reason recorded there.

Two behaviours worth the tests they have: an unreadable candidate is reported
through \`incomplete\`, NEVER dropped (a silently-dropped candidate is precisely
the duplicate-creating failure the funnel exists to prevent), and an empty query
short-circuits before touching the database so it can never degenerate into a
full scan that 'displays' the whole population into an attestation.

Refs #344"
```

---

### Task 6: The register orchestrator (`patient/register.rs`)

**Files:**
- Create: `crates/cairn-node/src/patient/register.rs`, `crates/cairn-node/tests/patient_register.rs`

**Interfaces:**
- Consumes: `registration_assertion_body`, `render_registration_twin`, `RegistrationAssertion`,
  `RegistrationClass`, `SearchAttestationInput`, `SearchTerms` (Task 1); `SearchAttestation`,
  `CandidateList`, `SearchQuery` (Task 2); `crate::db::next_hlc`.
- Produces:
  ```rust
  pub fn build_registration_body(
      event_id: Uuid, patient_id: Uuid, class: RegistrationClass, basis: Option<&str>,
      attestation: Option<&SearchAttestation>, kid: &str, hlc: Hlc,
  ) -> EventBody;

  pub async fn register_patient(
      client: &mut Client, sk: &SigningKey, kid: &str, node_origin: &str,
      query: &SearchQuery, displayed: &CandidateList,
  ) -> anyhow::Result<Uuid>;
  ```

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/patient_register.rs`:

```rust
#[tokio::test]
async fn registering_mints_a_chart_and_records_what_was_displayed() { }

#[tokio::test]
async fn the_attestation_round_trips_from_the_displayed_list_to_the_stored_body() {
    // GUARDS THE CROSS-CRATE SEAM (design §6): cairn-event takes primitives and
    // cairn-patient-search owns the read model, so nothing but this test stops the two
    // drifting. Build a CandidateList -> SearchAttestation -> body -> submit -> read the
    // stored body back -> assert the displayed set is IDENTICAL, in order.
}

#[tokio::test]
async fn a_search_the_node_knew_was_partial_is_attested_as_incomplete() {
    // The list's incompleteness must survive all the way into the stored event. A
    // registration must never swear to an exhaustive search over a list known to be partial.
}

#[tokio::test]
async fn registering_with_no_attester_key_succeeds() {
    // SPEC §2.6 — a grade, not a gate. See the identical guard in patient_registration.rs.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test patient_register`
Expected: FAIL — `cannot find function register_patient`.

- [ ] **Step 3: Write the implementation**

Mirror `john_doe::register_john_doe`'s shape exactly: pure body assembly + an async orchestrator
that ticks the HLC (`crate::db::next_hlc`), signs, and submits **inside one transaction**. Contributor
set is `[{"actor_id": kid, "role": "recorded"}]` — the *node* recorded it; a human registrar, when
present, is added by the caller as a bearing role and is bound unforgeably by `db/005` step 4b with
no rule from us (spec §2.6).

Convert `SearchAttestation` → `SearchAttestationInput` inside `build_registration_body`, so the
conversion has exactly one home and the Step-1 round-trip test covers it.

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test patient_register`
Expected: PASS, 4 tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add crates/cairn-node/src/patient/register.rs crates/cairn-node/tests/patient_register.rs
git commit -m "feat(#344): register_patient — mint, attest, submit in one transaction

Pure body assembly plus an async orchestrator, mirroring register_john_doe: one
transaction, so a chart is never half-registered.

The round-trip test is the load-bearing one. cairn-event owns the wire shape and
takes primitives; cairn-patient-search owns the read model. Nothing but that test
stops the two drifting, and a drift there means a registration swearing to
candidates the clerk never saw.

Refs #344"
```

---

### Task 7: Re-express John Doe onto the registration act

**Files:**
- Modify: `crates/cairn-node/src/john_doe.rs`, `crates/cairn-node/tests/john_doe.rs`

**Interfaces:**
- Consumes: Task 1 + Task 6's `build_registration_body`.
- Produces: unchanged public signature — `register_john_doe(...) -> anyhow::Result<(Uuid, String, i64)>`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/john_doe.rs`:

```rust
#[tokio::test]
async fn a_john_doe_chart_begins_with_an_unidentified_registration() {
    // §5.3's three classes finally recorded. The registration is the chart's FIRST event
    // (lowest HLC of the three), which is what lets #345's precedence rule land later with
    // no carve-out for John Doe.
    // Assert: patient_registration_current.class == 'unidentified'
    //         AND the registration's HLC precedes the callsign name's.
}

#[tokio::test]
async fn the_unidentified_registration_carries_no_search_attestation() {
    // Structural absence, not empty: there is nothing to search an unconscious patient with.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test john_doe`
Expected: FAIL — no row in `patient_registration_current`.

- [ ] **Step 3: Modify `register_john_doe`**

Tick a THIRD HLC *first* (`h0 < h1 < h2`), build the registration body with
`RegistrationClass::Unidentified`, `basis: Some(basis)`, `search: None`, and submit it as the first
statement inside the existing transaction. Update the module doc: it now authors **three** events,
and the "no new event types" claim in the current header is no longer true — rewrite that paragraph
rather than leaving a stale comment.

- [ ] **Step 4: Run the John Doe suite**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test john_doe`
Expected: PASS — including every pre-existing test, unchanged. Those are the regression gate.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add crates/cairn-node/src/john_doe.rs crates/cairn-node/tests/john_doe.rs
git commit -m "feat(#344): John Doe registers through the same act

Three events now, registration first: class=unidentified, basis carried, and
structurally NO search attestation — there is nothing to search an unconscious
patient with, and claiming otherwise would be a precise untruth (principle 4).

This is what lets #345's precedence rule land with no carve-out: every chart
begins with a registration, so the floor rule needs no 'unless'.

The module header's 'no new event types' claim is now false and has been
rewritten rather than left to rot.

Refs #344"
```

---

### Task 8: CLI verbs

**Files:**
- Modify: `crates/cairn-node/src/main.rs`

**Interfaces:**
- Consumes: `patient::search::search_patients`, `patient::register::register_patient`.
- Produces: `patient-search` and `patient-register` subcommands.

- [ ] **Step 1: Add the `Cmd` variants**

```rust
    /// Search this node's charts before creating one (§5.8 item 1). Advisory: it ranks
    /// nothing and decides nothing — it shows a human what exists.
    PatientSearch {
        /// Name as typed; tokenised on non-alphanumerics, so order does not matter.
        #[arg(long, default_value = "")]
        name: String,
        /// ISO YYYY-MM-DD.
        #[arg(long)]
        birth_date: Option<String>,
        /// Repeatable `system=value`, e.g. --identifier MRN=12345
        #[arg(long = "identifier")]
        identifiers: Vec<String>,
    },

    /// Register a standard patient, recording the search that preceded it (§5.8).
    ///
    /// The search runs HERE, immediately before the write, and its result is what gets
    /// attested — so the attestation always describes a real search this command ran,
    /// never one an operator retyped.
    PatientRegister {
        #[arg(long, default_value = "")]
        name: String,
        #[arg(long)]
        birth_date: Option<String>,
        #[arg(long = "identifier")]
        identifiers: Vec<String>,
        /// Proceed even though candidates were displayed. Without it the command STOPS and
        /// prints them: a funnel that auto-proceeds past near-matches is not a funnel.
        #[arg(long)]
        confirm_new: bool,
    },
```

- [ ] **Step 2: Write the handlers**

`PatientSearch`: parse `system=value` pairs (reject a malformed one with a legible error naming the
expected form), build a `SearchQuery`, read `current_date::text` from the DB for `today` (the John
Doe precedent — the DB is the clock), call `search_patients`, print a table of
`patient_id · name · age · trust · last activity · locale`. When `incomplete`, print the reason
**prominently and last**, so it cannot scroll away unseen.

`PatientRegister`: run the search, print the candidates, then:
- if candidates exist and `--confirm-new` was not passed → print them, exit **non-zero** with a
  message naming `--confirm-new`;
- else → `register_patient(...)` and print the new `patient_id`.

**This is not a confirmation dialog** (principle 3 forbids those as safety mechanisms). It is the
paper affordance: the clerk sees the index entries before writing a new card. Say so in a comment on
the branch, or someone will "simplify" it into an auto-proceed.

- [ ] **Step 3: Verify the verbs run end-to-end against a real node**

```bash
cargo run -p cairn-node -- --conn "$CAIRN_TEST_PG" patient-search --name "smith"
cargo run -p cairn-node -- --conn "$CAIRN_TEST_PG" patient-register --name "Jane Doe" --birth-date 1980-01-01
```
Expected: the search prints a (possibly empty) table; the register prints a new UUID, or stops and
lists candidates when any exist.

> **`--key` and `--conn` are GLOBAL flags — they go BEFORE the subcommand.** Three commands in the
> med-list runbook were wrong on exactly this point.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add crates/cairn-node/src/main.rs
git commit -m "feat(#344): patient-search and patient-register verbs

The search runs inside patient-register, immediately before the write, so the
attestation always describes a real search this command ran rather than one an
operator retyped.

With candidates on screen the command STOPS and requires --confirm-new. That is
not a confirmation dialog (principle 3 forbids those as safety mechanisms) — it
is the paper affordance: the clerk sees the index entries before writing a new
card. Commented at the branch so it does not get 'simplified' into auto-proceed.

Refs #344"
```

---

### Task 9: ADR-0061, spec prose, and the deferred issues

**Files:**
- Create: `docs/spec/decisions/0061-registration-is-an-act-that-carries-its-search.md`
- Modify: `docs/spec/identity.md` (§5.3 + §5.8) · `docs/spec/index.md` (spec version) ·
  `docs/spec/decisions/README.md` (ADR index) · `CLAUDE.md` (ADR index line) ·
  `docs/HANDOVER.md` · `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0061**

Follow the format of `0060-partial-validity-*.md`. It must record, at minimum:

1. **Registration is an act**, one type with §5.3's three classes, so the precedence rule needs no
   carve-out.
2. **The attestation names candidates, it does not count them** — the six-months-later question and
   the two opposite fixes.
3. **Strict local submit, lenient remote apply** — and that enforcement is deliberately split to
   #345 with the measured blast radius (~83 call sites, and `patient.created` must be retired).
4. **REJECTED ALTERNATIVE: gating a standard registration on a bound human author.** Record all
   three failure scenarios (blocks care documentation, trains staff to use the John Doe path
   instead, produces no forensic record at all) and the §5.11 rule it violates. This is the section
   most likely to save a future reader from "fixing" the absence.
5. **The body is not born-sealed**, so it names third-party UUIDs in the clear, and an ADR-0005
   rung-2 erasure must therefore reach registration attestations naming them.

- [ ] **Step 2: Update the spec prose**

§5.3: the classes are now recorded by a registration event, naming the type.
§5.8 item 1: point at ADR-0061 and state plainly that enforcement of the funnel (the precedence
rule) is #345, so the spec never claims a guarantee the code does not yet make.
Bump the spec version in `docs/spec/index.md`; add the ADR-0061 row to both indexes.

- [ ] **Step 3: File the two deferred issues**

```bash
gh issue create --title "search-before-create: converge the blocking keys with matcher/pipeline/db.py" --body "..."
gh issue create --title "UI: flag a chart with no registration act on file" --body "..."
```
Reference both from the spec's §8 list, replacing "Issue to file".

- [ ] **Step 4: Build the docs and update HANDOVER/ROADMAP**

```bash
uv run --with-requirements docs/requirements.txt -- mkdocs build
```
Expected: no broken-link warnings for the new ADR.

Then update `docs/HANDOVER.md` (⇒ NEXT, the built-so-far list, a Slice 63 narrative) and
`docs/ROADMAP.md` (a Slice 63 entry), pruning both toward 500 lines as agreed.

- [ ] **Step 5: Full suite, then commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test
PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-sql-tests.sh
cd matcher && uv run pytest && cd ..
git add docs CLAUDE.md
git commit -m "docs(#344): ADR-0061 — registration is an act that carries its search

Records the rejected alternative at length, because it is the one a future
reader will try to 'fix': gating a standard registration on a bound human author
violates §5.11 ('a grade, not a gate'), blocks care documentation when a clerk's
key is not unlocked, trains staff to route named patients through the John Doe
path, and produces no forensic record at all in the case it fires.

Refs #344"
```

---

## Self-Review

**Spec coverage.** §2.1 → Tasks 4+5 · §2.2 → Tasks 1+7 · §2.3 → deferred to #345, recorded in Task 9's
ADR · §2.4 → Task 1 · §2.5 → Task 9 (ADR) · §2.6 → Tasks 3+6 (the "must succeed" tests) · §3 → Task 1 ·
§4.1 → Task 3 · §4.2 → #345 · §4.3 → Task 3 · §5 → Tasks 4+5 · §6 → Tasks 1,2,5,6 · §7 paper-parity →
Task 8 (the verbs are the surface the benchmark describes; the interactive measurement stays owed to
the UI slice, per the spec) · §8 → Task 9 (issues) · §9 → every task's tests · §10 → Tasks 3,6,7.

**Type consistency.** `SearchQuery` / `CandidateList` / `SearchAttestation` are defined in Task 2 and
used with the same names in Tasks 5 and 6. `RegistrationClass` / `SearchAttestationInput` /
`SearchTerms` are defined in Task 1 and used in Tasks 6 and 7. `cairn_search_candidates`'s signature
is fixed in Task 4 and consumed unchanged in Task 5.

**Known gap, stated rather than hidden.** Tasks 3, 5 and 6 give test *names, docstrings and required
assertions* but not every full body, because each needs the DB-gated harness preamble that must be
copied from the neighbouring suite (`demographics.rs`) rather than invented. An implementer must read
that file first. Tasks 1 and 2 — the pure, harness-free ones — carry complete test code.

## Paper-parity benchmark (§1.2)

Transcribed from the design doc's §7
(`docs/superpowers/specs/2026-08-04-search-before-create-funnel-design.md`), which is the
authoring home for these figures. It is restated here rather than referenced because house
rule 7's guard (`crates/cairn-node/tests/paper_parity_plan_section.rs`) reads
`docs/superpowers/plans/*.md` and nothing else — a plan that points elsewhere leaves the
benchmark invisible to the one check that enforces it, and the earlier
"carried in the spec (§7), not restated here" line failed that guard.

**Paper counterpart:** the registration desk — clerk, card index or day book, folder tabs.

| | Acts |
|---|---|
| **Paper N** | **3** — ask name + DOB · look it up in the index · write a new card and folder tab if absent |
| **Architecture-forced M** | **3** — the architecture forces a search *to have run* and its attestation to be *carried*; neither forces a discrete second gesture, because type-ahead fuses entry and search (§5.11: "type a few chars and enter, no spinner") |
| **UI bundling target K** | **2** — type-and-see → commit. Reviewing candidates is reading, not an act |

`M = N`, so no architecture defect under house rule 7.

**Steps:** paper 3 → architecture-forced 3 → UI bundling target 2, as tabulated above.

**Time + cognitive load:** budget ≤ **5 s** to find an existing chart (by far the commoner
path) and ≤ **20 s** to register a new one, first keystroke to committed chart.

**Measurement owed:** by the slice that first exposes a runnable surface. This slice is
CLI-only, so it measures the **node-tier write cost** as Slices 61/62 did (`cairn-node`'s
existing `ui_timing`/gesture-timing capture), and states the interactive half as owed. If a
measured figure falls outside the budget, **that is the finding** — file it; do not move the
budget to fit.
*(Corrected after implementation: the write-cost measurement was not wired either — `db/044`'s
`gesture_kind` CHECK would refuse a registration row until widened. Both halves are owed;
the node-tier half is [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360).)*
