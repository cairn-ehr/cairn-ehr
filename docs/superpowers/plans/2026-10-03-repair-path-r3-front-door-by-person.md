# Repair path R3 — the front door collapses by person (ADR-0076 decision 6) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A patient search returns one row per PERSON (link component), not per chart. A row lists
every member chart: the ones the search matched first, in rank order, then the linked charts it did
not match. The rows are ranked by their best member, and the prompt's five places are five people.
A registration signs every member chart of every row shown, and every member line on screen can be
opened.

**Architecture:**
- The shared crate `cairn-patient-search` gains `PersonRow` (non-empty, private fields) and a pure
  `group_by_person`. `CandidateList.candidates: Vec<Candidate>` becomes `people: Vec<PersonRow>`, with
  ONE flattening, `displayed_charts()`, which `SearchAttestation::from_displayed` uses.
- `cairn-node`'s `search_patients` reads each matched chart's link component in one query, ranks the
  matched charts exactly as before, groups, then reads the display fields over every member. A chart
  whose registration is not held here reads trust `Unknown` (R1's `person::trust_of`) and never sets
  `incomplete`.
- The funnel bounds by rows. The window renders a row as a nested list, one open button per member;
  the link panel does the same with Compare. The CLI prints a linked member indented under its row.

**Tech Stack:** Rust (tokio-postgres, anyhow, serde), PostgreSQL ≥ 18 + `cairn_pgx`, the `cairn-gui`
workspace (Tauri 2, plain JS, no npm).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` — section
**"R3 — the front door collapses by person"** and its sub-section **"R3 — designed 2026-10-03"**
(the maintainer's two decisions: every member line is its own open target; the browse list
collapses too). ADR-0076 decision 6; ADR-0075 (the ranking, the cap, `withheld` vs `incomplete`).

## Global Constraints

- **AGPL-3.0**; no new dependency.
- **TDD.** Every behaviour change starts with a test that fails for the right reason. A pure
  migration (Task 2) is guarded by the existing suites staying green.
- **No wire change, no SQL object, no migration.** db/045 is unchanged; `SCHEMA_GENERATION` stays
  **55**. `displayed` keeps its shape and now names every member of every row shown (ADR-0076 D6).
- **The ranking is unchanged** (`cairn_patient_search::rank_candidates`, the seven ADR-0075 keys). Never
  rank a member the search did not match; it has no keys.
- **`withheld` is never signed and `incomplete` is the SEARCH's partiality only** (ADR-0075). A
  member not held here is NOT a partial search. Never fold either into the other.
- **Every sentence lives in Rust** (`funnel/view.rs`, the new `funnel/rows.rs`, `link/search.rs`), never
  in JS. A summary over rows that are ALL single charts must stay byte-identical to today.
- **Every panel message goes through `setMessage`** (R2b-1's rule); a new JS read is covered by the
  webview-fields guard (`funnel_js_reads_no_field_the_backend_does_not_send`).
- **House rule 6:** no literal key material; no binding named `salt`/`nonce`/`iv`.
- **Files under 500 lines where feasible.** New code goes in new files: `cairn-patient-search/src/person.rs`,
  `cairn-node/src/patient/search_person.rs`, `cairn-node/src/patient/candidate_text.rs`,
  `cairn-gui-tauri/src/funnel/rows.rs`. `search.rs` (488) must not pass ~500 — move code out to make room;
  `view.rs` (635) and `commands.rs` (894, mostly tests) grow by ≤ 10 non-test lines.
- **Commit messages** say `Refs #679` and never a closing keyword; run
  `python3 scripts/check_closing_keywords.py` on the PR body.
- **Subagents run foreground tests only.** DB-gated suites are run by the controller, with
  `--nocapture`, checking that no `skipped:` line appears.
- **DB env for a targeted DB run** (`scripts/pg-target.sh` prints the cluster; use its port):
  ```bash
  export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
         CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test2" \
         CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test3"
  ```
  Use `CARGO_TARGET_DIR=/tmp/cairn-r3-target` when an IDE is open (trap 18). The `cairn-gui` tree
  needs `CAIRN_ALLOW_DB_SKIP=1` for a DB-free run.

## Review Focus

1. **Two members of one person BOTH matched** (one by name, one by DOB). One row, placed by the better
   of the two, both listed in rank order — never two rows for one person. Pinned in Task 1 (pure).
2. **A three-chart component where only the third chart matched** (A–B and B–X linked; X matched).
   The row is `[X, A, B]`: the matched member first, then the others oldest first (ascending UUIDv7).
   Pinned in Task 1 (pure).
3. **The same person reached from the other member.** A search for "Smythe" lists `[SMYTHE, SMITH]`;
   a search for "Smith" lists `[SMITH, SMYTHE]`. Pinned in Task 3 (DB).
4. **The cap's fifth row is a linked pair.** Five rows are shown, six charts are signed, and a sixth
   person is `withheld` as ONE. Pinned in Task 4 (pure).
5. **A member whose name arrived by sync but whose registration did not.** It shows that name, reads
   trust `Unknown`, and the list stays complete. Pinned in Task 3 (DB).
6. **The link panel's own record when it has a linked member the search did not match.** The whole
   row is left out, and the own-count counts both charts. Pinned in Task 5 (pure).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `crates/cairn-patient-search/src/person.rs` | **create** | `PersonRow`; `group_by_person`; `MissingComponent` |
| `crates/cairn-patient-search/src/candidate.rs` | modify | `TrustState::Unknown`; `CandidateList.people`, `charts()`, `displayed_charts()`, `empty()` |
| `crates/cairn-patient-search/src/attestation.rs` | modify | `from_displayed` uses `displayed_charts()` |
| `crates/cairn-patient-search/src/lib.rs` | modify | exports |
| `crates/cairn-node/src/patient/search_person.rs` | **create** | `read_components` (one query); `display_name_for` (pure); `trust_state_for` (pure) |
| `crates/cairn-node/src/patient/search.rs` | modify | the new flow: components → reads over all members → rank → group → rows |
| `crates/cairn-node/src/patient/candidate_text.rs` | **create** | `candidate_lines` (pure CLI rendering, moved out of `main.rs`) |
| `crates/cairn-node/src/main.rs` | modify | `print_candidates` prints `candidate_lines` |
| `cairn-gui/cairn-gui-funnel/src/prompt.rs` | modify | bound by rows; `PromptCounts::shown_charts` |
| `cairn-gui/cairn-gui-funnel/src/token.rs` | modify | Debug counts charts |
| `cairn-gui/cairn-gui-tauri/src/funnel/rows.rs` | **create** | `PersonRowView`, `person_row_view`, `linked_row_label`, `people_phrase` |
| `cairn-gui/cairn-gui-tauri/src/funnel/view.rs` | modify | `browse_summary`/`prompt_summary` count people and name charts |
| `cairn-gui/cairn-gui-tauri/src/funnel/commands.rs` | modify | `people` on `BrowseView`/`PromptView`; `remember_shown` records every member |
| `cairn-gui/cairn-gui-tauri/src/link/search.rs` | modify | filter rows, not candidates |
| `cairn-gui/cairn-gui-tauri/src-ui/funnel.js`, `link.js`, `style.css` | modify | `personItem`; nested member lists |
| `cairn-gui/cairn-gui-data/src/mock/funnel.rs` | modify | every mock row is a person of one |
| every test / example naming `.candidates` | modify | mechanical (Task 2) |
| design page, HANDOVER, ROADMAP | modify | as-built note; R3 durable rules (Task 7) |

---

### Task 1: `PersonRow`, `group_by_person`, `TrustState::Unknown` (shared crate, pure, additive)

**Files:**
- Create: `crates/cairn-patient-search/src/person.rs`
- Modify: `crates/cairn-patient-search/src/candidate.rs` (`TrustState`), `src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct PersonRow` (private `members: Vec<Candidate>`), `PersonRow::new(Vec<Candidate>) -> Option<PersonRow>`
    (`None` on empty), `PersonRow::alone(Candidate) -> PersonRow`, `PersonRow::each_alone(Vec<Candidate>) -> Vec<PersonRow>`,
    `members(&self) -> &[Candidate]`, `is_linked(&self) -> bool` (more than one member).
    Serde: `#[serde(try_from = "Vec<Candidate>", into = "Vec<Candidate>")]` — a row is a JSON array; an
    empty array fails to deserialize.
  - `pub fn group_by_person(ranked: &[Uuid], components: &HashMap<Uuid, Vec<Uuid>>) -> Result<Vec<Vec<Uuid>>, MissingComponent>`
  - `pub struct MissingComponent(pub Uuid)` with `Display` ("no link component was read for chart {id}") and `std::error::Error`.
  - `TrustState::Unknown`, `as_str() == "unknown"`, serialized `"unknown"`.

- [ ] **Step 1: Write the failing tests** in `person.rs`'s `mod tests`:

```rust
use super::*;
use std::collections::HashMap;
use uuid::Uuid;

fn id(n: u128) -> Uuid { Uuid::from_u128(n) }

/// Each chart's component, as `read_components` returns it: sorted ascending.
fn components(sets: &[&[u128]]) -> HashMap<Uuid, Vec<Uuid>> {
    let mut out = HashMap::new();
    for set in sets {
        let mut members: Vec<Uuid> = set.iter().map(|n| id(*n)).collect();
        members.sort();
        for m in &members { out.insert(*m, members.clone()); }
    }
    out
}

#[test]
fn never_linked_charts_are_one_row_each_in_rank_order() {
    let ranked = [id(3), id(1), id(2)];
    let got = group_by_person(&ranked, &components(&[&[1], &[2], &[3]])).unwrap();
    assert_eq!(got, vec![vec![id(3)], vec![id(1)], vec![id(2)]]);
}

#[test]
fn a_row_is_placed_by_its_best_ranked_member_and_lists_both_matched_members_in_rank_order() {
    // Review Focus 1: 2 and 5 are one person; 5 ranked first, 2 ranked third.
    let ranked = [id(5), id(9), id(2)];
    let got = group_by_person(&ranked, &components(&[&[2, 5], &[9]])).unwrap();
    assert_eq!(got, vec![vec![id(5), id(2)], vec![id(9)]]);
}

#[test]
fn members_the_search_did_not_match_follow_the_matched_ones_oldest_first() {
    // Review Focus 2: A(1)–B(2)–X(3) is one person; only X matched.
    let ranked = [id(3)];
    let got = group_by_person(&ranked, &components(&[&[1, 2, 3]])).unwrap();
    assert_eq!(got, vec![vec![id(3), id(1), id(2)]]);
}

#[test]
fn every_chart_appears_exactly_once() {
    let ranked = [id(4), id(1), id(7), id(2)];
    let got = group_by_person(&ranked, &components(&[&[1, 2, 6], &[4], &[7, 8]])).unwrap();
    let mut flat: Vec<Uuid> = got.concat();
    flat.sort();
    assert_eq!(flat, vec![id(1), id(2), id(4), id(6), id(7), id(8)]);
}

#[test]
fn a_chart_with_no_component_read_is_an_error_never_a_silent_row_of_one() {
    // Falling back to a row of one would put one person in two prompt places with nothing said.
    let err = group_by_person(&[id(1)], &HashMap::new()).unwrap_err();
    assert_eq!(err, MissingComponent(id(1)));
}

#[test]
fn a_row_is_never_empty() {
    assert!(PersonRow::new(vec![]).is_none());
    let empty: Result<PersonRow, _> = serde_json::from_str("[]");
    assert!(empty.is_err(), "an empty row must not deserialize");
}
```

Plus, in `candidate.rs`'s tests: `TrustState::Unknown.as_str() == "unknown"` and its serde round trip
(`"\"unknown\""` both ways), beside `trust_states_render_the_tokens_the_chart_contract_uses`.

- [ ] **Step 2: Run them to see them fail.**
  Run: `cargo test -p cairn-patient-search` — Expected: FAIL to compile (`group_by_person`, `PersonRow`,
  `TrustState::Unknown` not defined).

- [ ] **Step 3: Implement.** In `person.rs`, a module doc for a junior reader: what a person row is
(ADR-0076 decision 6; "two folders clipped together sit in one slot of the card index"), why the
grouping is pure and lives in the shared crate (the node and any future picker must agree on what a
row is, as they must on what was displayed), and why a missing component is an error. Then:

```rust
pub fn group_by_person(
    ranked: &[Uuid],
    components: &HashMap<Uuid, Vec<Uuid>>,
) -> Result<Vec<Vec<Uuid>>, MissingComponent> {
    // Where each matched chart sits in the ranking; a member absent here was not matched.
    let position: HashMap<Uuid, usize> = ranked.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut placed: HashSet<Uuid> = HashSet::new();
    let mut rows = Vec::new();
    for id in ranked {
        if placed.contains(id) {
            continue; // already in the row of a better-ranked member of the same person
        }
        let component = components.get(id).ok_or(MissingComponent(*id))?;
        let (mut matched, mut unmatched): (Vec<Uuid>, Vec<Uuid>) =
            component.iter().partition(|m| position.contains_key(*m));
        matched.sort_by_key(|m| position[m]);
        unmatched.sort(); // UUIDv7 ascending: oldest chart first
        let row: Vec<Uuid> = matched.into_iter().chain(unmatched).collect();
        placed.extend(row.iter().copied());
        rows.push(row);
    }
    Ok(rows)
}
```

The component of `id` is trusted to contain `id` (db/054 always unions the chart itself in); state it
in the doc. `PersonRow` with private `members`, the constructors, and `TryFrom<Vec<Candidate>>` (error
type: a `&'static str` or a small `EmptyRow` error — "a person row needs at least one chart") and
`From<PersonRow> for Vec<Candidate>`. Add `Unknown` to `TrustState` (doc: "this node does not hold the
chart's registration; it synced ahead, or lies outside this node's sync scope — R1's
`person::trust_of`"), and its `as_str` arm. Export `PersonRow`, `group_by_person`, `MissingComponent`
from `lib.rs`.

- [ ] **Step 4: Run** `cargo test -p cairn-patient-search` — Expected: PASS. Then `cargo build --workspace`
  and `cargo build --manifest-path cairn-gui/Cargo.toml`: any exhaustive `match` on `TrustState` must
  now name `Unknown`. Fix each by the same rule as its neighbours (a view string is `as_str()`).

- [ ] **Step 5: Commit** — `feat(R3): a person row, and the one grouping rule (Refs #679)`.

---

### Task 2: `CandidateList.people` — the shape change, every consumer migrated, behaviour unchanged

A pure migration. Every search still returns one row per chart (each built `PersonRow::alone`), so
every existing suite must stay green with only its spelling changed. This task makes NO grouping
decision; Task 3 does.

**Files:**
- Modify: `crates/cairn-patient-search/src/candidate.rs`, `src/attestation.rs`
- Modify: `crates/cairn-node/src/patient/search.rs` (wrap each candidate `PersonRow::alone`),
  `src/patient/register.rs` (doc only), `src/main.rs` (`print_candidates` iterates `list.charts()`),
  `examples/seed_measurement_corpus.rs`
- Modify: `cairn-gui/cairn-gui-funnel/src/{prompt,token,session}.rs`, `cairn-gui-data/src/mock/funnel.rs`,
  `cairn-gui-live/src/*.rs` + `tests/*.rs`, `cairn-gui-tauri/src/funnel/{commands,view,backend}.rs`,
  `cairn-gui-tauri/src/link/search.rs`
- Modify: `crates/cairn-node/tests/{patient_search,patient_search_ranking,patient_register,patient_register_demographics}.rs`

**Interfaces:**
- Consumes: Task 1's `PersonRow`.
- Produces:
  - `pub struct CandidateList { pub people: Vec<PersonRow>, pub incomplete: bool, pub incomplete_reason: Option<String> }`
  - `CandidateList::empty() -> CandidateList` (complete, no rows)
  - `CandidateList::charts(&self) -> impl Iterator<Item = &Candidate>` — every member of every row, in row order
  - `CandidateList::displayed_charts(&self) -> Vec<Uuid>` — `charts().map(|c| c.patient_id)`; THE flattening
    the attestation signs (doc says so, citing ADR-0076 D6)
  - `SearchAttestation::from_displayed` → `displayed: list.displayed_charts()`

- [ ] **Step 1: Write the failing test** in `attestation.rs`:

```rust
#[test]
fn a_linked_row_signs_every_member_in_row_order() {
    let list = CandidateList {
        people: vec![
            PersonRow::new(vec![candidate(5), candidate(2)]).unwrap(),
            PersonRow::alone(candidate(9)),
        ],
        incomplete: false,
        incomplete_reason: None,
    };
    let a = SearchAttestation::from_displayed(&SearchQuery::new("smith", None, &[]), &list);
    assert_eq!(a.displayed, vec![Uuid::from_u128(5), Uuid::from_u128(2), Uuid::from_u128(9)]);
}
```

  Rewrite the three existing tests there to `people: PersonRow::each_alone(vec![...])`.

- [ ] **Step 2: Run** `cargo test -p cairn-patient-search` — Expected: FAIL to compile (`people`).
- [ ] **Step 3: Change the struct and the attestation**, then the empty-list sites
  (`search.rs::empty_list`, the mock's empty query) to `CandidateList::empty()`.
- [ ] **Step 4: Migrate every consumer mechanically**, by these rules only:
  - building a list: `candidates: X` → `people: PersonRow::each_alone(X)`;
  - reading every chart: `list.candidates.iter()` → `list.charts()`; `list.candidates.len()` →
    `list.charts().count()` where the meaning is charts, `list.people.len()` where it is rows (in this
    task the two are equal — choose by what the sentence or assertion MEANS, and leave the bound in
    `prompt.rs` for Task 4);
  - indexing: `list.candidates[0]` → `list.people[0].members()[0]`.
  - `cairn-gui-tauri`'s `BrowseView`/`PromptView` keep their `candidates: Vec<CandidateView>` field in
    THIS task, built from `list.charts()`; Task 5 changes the view shape.
  Find every site with `grep -rn '\.candidates\b\|candidates:' --include='*.rs' crates cairn-gui`. JS is
  untouched in this task.
- [ ] **Step 5: Run every gate that compiles the change** (foreground):
  `cargo test -p cairn-patient-search`; `cargo build --workspace --all-targets`;
  `cargo build --manifest-path cairn-gui/Cargo.toml --all-targets`;
  `CAIRN_ALLOW_DB_SKIP=1 cargo test --manifest-path cairn-gui/Cargo.toml` — Expected: PASS.
  The controller then runs the DB suites `patient_search`, `patient_search_ranking`, `patient_register`,
  `patient_register_demographics` and `cairn-gui-live`'s `attestation_through_the_port` with `--nocapture`
  — Expected: PASS, no `skipped:`.
- [ ] **Step 6: Commit** — `refactor(R3): a candidate list is a list of person rows — one chart each, for now (Refs #679)`.

---

### Task 3: the node groups by person; a chart not held here reads `Unknown`

**Files:**
- Create: `crates/cairn-node/src/patient/search_person.rs` (+ `pub mod search_person;` in `patient/mod.rs`)
- Modify: `crates/cairn-node/src/patient/search.rs`
- Create: `crates/cairn-node/tests/search_by_person.rs`
- Modify: `crates/cairn-node/tests/patient_search.rs` (one test: the latent trust defect)

**Interfaces:**
- Consumes: `group_by_person`, `PersonRow`, `TrustState::Unknown` (Task 1); `person::read_held(client, &[String]) -> HashSet<Uuid>` and
  `person::trust_of(held: bool, row: Option<&str>) -> String` (R1, `crates/cairn-node/src/patient/person.rs`).
- Produces (in `search_person.rs`):
  - `pub(super) async fn read_components<C: GenericClient + Sync>(client: &C, ids: &[Uuid]) -> anyhow::Result<HashMap<Uuid, Vec<Uuid>>>`
  - `pub(super) fn display_name_for(id: Uuid, names: &HashMap<Uuid, String>, ever_named: &HashSet<Uuid>, held: &HashSet<Uuid>) -> DisplayName`
  - `pub(super) enum DisplayName { Known(String), Withheld, NotReceived, Unreadable }` with
    `fn text(&self) -> String` ("(name withheld)", "(registration not yet received here)", "(name unavailable)")
    and `fn is_unreadable(&self) -> bool` (only `Unreadable` counts toward `incomplete`)
  - `pub(super) fn trust_state_for(held: bool, row: Option<&str>) -> TrustState` — maps `person::trust_of`'s
    answer: `"confirmed"` → `Confirmed`, `"unknown"` → `Unknown`, `"unconfirmed"` → `Unconfirmed`, anything
    else → `UnderReview` (today's `trust_state_from_db` rule, moved here and deleted from `search.rs`).

- [ ] **Step 1: Write the failing pure tests** in `search_person.rs`:

```rust
#[test]
fn a_chart_not_held_here_reads_unknown_never_confirmed() {
    assert_eq!(trust_state_for(false, None), TrustState::Unknown);
    assert_eq!(trust_state_for(true, None), TrustState::Confirmed);
    // A positive claim wins whether or not the chart is held (person::trust_of's rule).
    assert_eq!(trust_state_for(false, Some("under-review")), TrustState::UnderReview);
    assert_eq!(trust_state_for(true, Some("unconfirmed")), TrustState::Unconfirmed);
    assert_eq!(trust_state_for(true, Some("a-future-state")), TrustState::UnderReview);
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
    assert_eq!(display_name_for(p, &HashMap::new(), &ever, &HashSet::new()), DisplayName::Withheld);
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
```

- [ ] **Step 2: Write the failing DB tests** in `tests/search_by_person.rs` (module doc: R3, ADR-0076
D6, what each test pins). Setup per test: `cs()` gate with the `skipped:` line, `test_serial_guard`,
`connect_and_load_schema`, `common::setup(&c, &EXTRA)` with
`EXTRA = ["patient_name", "patient_link", "person_member", "link_veto_flag", "chart_dispute", "patient_registration"]`.
Charts via `common::chart_named(&c, &sk, &kid, wall, name)`; links via
`common::submit_link_event(&c, &sk, &kid, a, b, wall, true)`. Search with
`search_patients(&c, &SearchQuery::new(typed, None, &[]), "2026-10-03")`. Tests:
  - `a_linked_pair_is_one_row` — "Mary Smith" and "Mary Smythe" linked; search "mary" → `people.len() == 1`,
    both ids in `people[0].members()`.
  - `the_member_the_search_matched_leads_its_row` (Review Focus 3) — search "smythe" → members
    `[smythe, smith]`; search "smith" → `[smith, smythe]`. Each list: one row, `!incomplete`.
  - `a_linked_chart_the_search_did_not_match_is_shown_and_named` — "Ann Lee" linked to "Bea Ngo"; search
    "lee" → the row has both, the second member's `display_name == "Bea Ngo"`, and
    `list.displayed_charts() == vec![lee, ngo]`.
  - `two_people_rank_by_their_best_member` — person P = {"Jo Kim"} ∪ {"Jo Kimura" (linked)}, person Q =
    {"Jo Kim Park"}; search "jo kim park": Q matches three tokens and leads; then P. Assert row order by
    the first member of each row.
  - `a_member_not_held_here_reads_unknown_and_the_search_stays_complete` (Review Focus 5) — chart `a`
    named "Rua Tane"; `elsewhere = Uuid::now_v7()` never registered; link `a`–`elsewhere`; apply a name
    event for `elsewhere` through the REMOTE door (`common::apply_remote_raw` with a `body_from_spec`
    body for `demographic.field.asserted`, name "Rua Taane") — the remote door admits a chart's event
    before its registration. Search "rua" → one row; the `elsewhere` member has `display_name == "Rua Taane"`,
    `trust == TrustState::Unknown`; `!list.incomplete`. Then a second chart linked to `a` with NO events
    at all (`ghost`): its member reads "(registration not yet received here)", `Unknown`, and the list is
    still complete.
  - `a_never_linked_search_is_one_row_per_chart` — three unlinked "Teo …" charts; search "teo" →
    `people.len() == 3`, every row `!is_linked()`, and the chart order equals the order
    `rank_candidates` gives (the same three ids the pre-R3 code returned; assert the explicit order of
    the fixture, ranked by tokens then id).
  Add to `patient_search.rs`: `a_matched_chart_this_node_does_not_hold_reads_unknown` — a name event
  through the remote door for an unregistered chart; search finds it; `trust == Unknown` (was
  `Confirmed`: the latent defect).

- [ ] **Step 3: Run** `cargo test -p cairn-node --lib patient::search_person` → FAIL (not defined).
  The controller runs the DB suite → FAIL (rows not grouped; `Unknown` never produced).

- [ ] **Step 4: Implement `read_components`** — ONE statement, so every component is read from one
snapshot (`cairn_person_charts` is `LANGUAGE sql STABLE`, db/054):

```rust
let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
let rows = client
    .query(
        "SELECT m::text AS chart, c::text AS member \
         FROM unnest($1::text[]::uuid[]) AS m, LATERAL cairn_person_charts(m) AS c",
        &[&id_strs],
    )
    .await?;
// Group by `chart`, parse each uuid with `?`, sort each member list ascending.
```

  Doc: why one query (one snapshot, one round trip per search, not one per candidate); that a chart
  never linked comes back as its own set of one; that the caller never falls back on a missing entry.

- [ ] **Step 5: Restructure `search_patients`** (keep it the readable orchestration; move helpers out
until `search.rs` stays under ~500 lines — `trust_state_from_db` goes, replaced by `trust_state_for`):

```text
passes   = read_candidate_passes(query)            -- matched charts, as today
matched  = ids of passes; empty → CandidateList::empty()
components = read_components(matched)               -- NEW
members  = every chart in any component, sorted, deduplicated
dobs     = read_dob(members)                        -- the ranking reads the matched entries
retained = read_retained_names(matched); query_tokens = normalise_query_tokens(...)
ranked   = rank_candidates(rank_keys(&passes, ...)) -- unchanged, matched only
groups   = group_by_person(&ranked, &components)?   -- MissingComponent → anyhow error (fails loudly)
names, ever_named (for members missing a name), held = person::read_held(members),
trust rows, last_activity, locales, photo_refs      -- all over `members`
people   = groups → PersonRow::new(members.map(candidate)).expect(non-empty by construction)
incomplete = count of members whose DisplayName::is_unreadable()
```

  `read_trust_states` returns the raw `chart_trust` rows (`HashMap<Uuid, String>`); the candidate's trust
  is `trust_state_for(held.contains(id), rows.get(id).map(String::as_str))`. Correct the stale comment
  "no `patient_chart` row is normal" (true before #345, not since: every registration now creates it).
  Keep the incomplete reason's wording ("N candidate(s) could not be read: no display name on file").

- [ ] **Step 6: Run** the pure tests, then (controller) `search_by_person`, `patient_search`,
  `patient_search_ranking`, `patient_register`, `patient_search_drift`, `person_charts` with `--nocapture`
  — Expected: PASS, no `skipped:`.
- [ ] **Step 7: Commit** — `feat(R3): the node returns one row per person; a chart not held here reads unknown (Refs #679)`.

---

### Task 4: the prompt bounds people, never splits one, and counts charts

**Files:**
- Modify: `cairn-gui/cairn-gui-funnel/src/prompt.rs`, `src/token.rs` (Debug)
- Modify: `cairn-gui/cairn-gui-tauri/src/funnel/view.rs` (`prompt_summary`, `browse_summary`)
- Modify: `cairn-gui/cairn-gui-tauri/src/funnel/commands.rs` (`browse_view` passes both counts)

**Interfaces:**
- Produces: `PromptCounts { shown: usize, shown_charts: usize, withheld: usize, incomplete: bool }` —
  `shown` and `withheld` count PEOPLE (rows); `shown_charts` counts the charts a registration signs.
  `browse_summary(people: usize, charts: usize, incomplete: bool) -> String`.

- [ ] **Step 1: Write the failing tests.** In `prompt.rs` (helper: `row(ids: &[u128]) -> PersonRow` of
plain candidates):

```rust
#[test]
fn the_cap_counts_people_and_never_splits_a_linked_row() {
    // Review Focus 4: the fifth row is a linked pair; a sixth person is cut.
    let list = CandidateList {
        people: vec![row(&[1]), row(&[2]), row(&[3]), row(&[4]), row(&[5, 6]), row(&[7])],
        incomplete: false,
        incomplete_reason: None,
    };
    let p = bound_for_prompt(&list);
    assert_eq!(p.as_list().people.len(), PROMPT_CAP);
    assert_eq!(p.as_list().displayed_charts().len(), 6, "both charts of the fifth row are signed");
    let c = p.counts();
    assert_eq!((c.shown, c.shown_charts, c.withheld), (5, 6, 1));
}
```

  In `view.rs` tests — **goldens first**: assert the CURRENT `prompt_summary`/`browse_summary` texts for
  single-chart rows verbatim (copy them from the functions as they stand: e.g. `browse_summary(3, 3,
  false) == "3 existing chart(s) found."`, and `prompt_summary` with `shown == shown_charts` for the
  withheld and not-withheld arms) — these must stay byte-identical. Then the new wording:

```rust
#[test]
fn a_linked_row_is_counted_as_one_person_and_its_charts_are_named() {
    assert_eq!(browse_summary(3, 4, false), "3 existing patient(s) found (4 charts).");
    assert_eq!(
        browse_summary(3, 4, true),
        "3 existing patient(s) found (4 charts) — the list is not complete."
    );
    let s = prompt_summary(&PromptCounts { shown: 5, shown_charts: 6, withheld: 98, incomplete: false });
    assert!(s.starts_with("5 existing patient(s) (6 charts) might be this person — the 5 closest of 103"), "{s}");
}
```

  Update the `counts(shown, withheld, incomplete)` test helper to set `shown_charts: shown`.

- [ ] **Step 2: Run** `CAIRN_ALLOW_DB_SKIP=1 cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-funnel -p cairn-gui-tauri`
  — Expected: FAIL (`shown_charts` missing; the bound takes charts).
- [ ] **Step 3: Implement.** `bound_to`: `people: list.people.iter().take(cap).cloned().collect()`,
  `withheld: list.people.len().saturating_sub(cap)`; update the module and `PromptList` docs (the cap is
  five PEOPLE — ADR-0076 D6 — and a row is never split, because the attestation must name what the
  screen shows). `counts()` sets `shown_charts: self.list.charts().count()`. `PromptCounts::total()` stays
  people. In `prompt_summary`, a `who` phrase: `"{n} existing patient(s)"`, plus `" ({c} charts)"` only
  when `shown_charts != shown`; same for `browse_summary`, whose people==charts arms are today's text
  verbatim. `browse_view` calls `browse_summary(list.people.len(), list.charts().count(), …)`.
  `token.rs`'s Debug says `"<{} chart(s), redacted>"` from `displayed_charts().len()`.
- [ ] **Step 4: Run** the same command — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(R3): the prompt's five places are five people (Refs #679)`.

---

### Task 5: the window and the link panel render person rows; any member opens

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/funnel/rows.rs` (+ `pub mod rows;` in `funnel/mod.rs`)
- Modify: `cairn-gui/cairn-gui-tauri/src/funnel/commands.rs`, `src/link/search.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/funnel.js`, `src-ui/link.js`, `src-ui/style.css`

**Interfaces:**
- Consumes: `candidate_view(&Candidate) -> CandidateView` (`funnel/view.rs`); Task 4's `browse_summary`.
- Produces:
  - `#[derive(Debug, Clone, Serialize)] pub struct PersonRowView { pub label: Option<String>, pub members: Vec<CandidateView> }`
  - `pub fn person_row_view(row: &PersonRow) -> PersonRowView` — `label` is `Some(linked_row_label(n))` only when `row.is_linked()`
  - `pub fn linked_row_label(charts: usize) -> String` → `"One person — {charts} linked charts"`
  - `BrowseView { revision, people: Vec<PersonRowView>, summary, incomplete_reason }` and
    `PromptView { …, people: Vec<PersonRowView>, … }` (the `candidates` field is gone)
  - `remember_shown(state, list: &CandidateList)` records `list.charts()`
  - `link_search_summary(other_people: usize, other_charts: usize, own_charts: usize, incomplete: bool) -> String`
  - JS: `personItem(row, memberItem)` in `funnel.js` (global; `link.js` loads after it — `index.html`)

- [ ] **Step 1: Write the failing Rust tests.**
  - `rows.rs`: a single-chart row has `label == None` and one member equal to `candidate_view(c)`;
    a two-chart row has `label == Some("One person — 2 linked charts")` and its members in row order;
    an `Unknown` member's view says `trust == "unknown"`.
  - `commands.rs`: `a_member_the_search_did_not_match_can_be_opened` — build an `AppState::mock`, call
    `remember_shown(&state, &list)` with one linked row `[a, b]`, then `open_chart_impl(&state, &b.to_string())`
    is `Ok` with `patient_id == b`, and `open_chart_impl` of an id on no row is still refused.
  - `commands.rs`: extend `funnel_js_reads_no_field_the_backend_does_not_send` with a `("row", person_row_view(..))`
    payload (a LINKED sample so `label` is present) and build `browseView` from a non-empty list;
    `funnel_js_reads_both_incompleteness_reports_and_the_retry_advice` adds `fields_read_in(js, "row")`
    containing `"members"` and `"label"`.
  - `link/search.rs` (Review Focus 6):

```rust
#[test]
fn this_records_whole_row_is_left_out_even_its_unmatched_member() {
    // The open record is {a, b}; the search matched only `a`, and returned the row [a, b]; one other
    // person [x] also matched.
    let v = link_search_view(browse_rows(&[&["a", "b"], &["x"]], false), &["a".into(), "b".into()]);
    assert_eq!(v.people.len(), 1);
    assert_eq!(v.people[0].members[0].patient_id, "x");
    assert_eq!(v.summary, "1 other chart(s) found. 2 chart(s) of this record also matched and are not listed.");
}
```

    plus `a_linked_other_person_is_one_row`: other rows `[x, y]` → summary
    `"1 other patient(s) found (2 charts)."`. Existing tests move to the row-shaped helper
    `browse_rows(rows: &[&[&str]], incomplete) -> BrowseView`, and their expected sentences stay verbatim.
- [ ] **Step 2: Run** `CAIRN_ALLOW_DB_SKIP=1 cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri`
  — Expected: FAIL.
- [ ] **Step 3: Implement Rust.** `rows.rs` with a module doc (a row is one person; why the label is
  worded in Rust; why every member gets its own open — the maintainer's decision: the opened chart
  decides what a doubted set lets you sign and where new content will go). `browse_view` and
  `prompt_search_impl` build `people` with `person_row_view`; `remember_shown` takes the list.
  `link_search_view` drops every ROW holding any `in_record` chart, `own` = the charts in the dropped
  rows, and summarises with `link_search_summary` — whose people==charts arms are today's sentences
  verbatim and whose linked arm reads `"{n} other patient(s) found ({c} charts)."`.
- [ ] **Step 4: Implement JS.** In `funnel.js`, after `candidateItem`:

```js
/**
 * One PERSON row (ADR-0076 decision 6). A chart never linked renders exactly as before — one
 * item, one button. Linked charts are one item holding the Rust-worded label ("One person — 2
 * linked charts") and a nested list with a button per member: the clerk opens the chart they
 * reached for, and the combined record reads either way.
 */
function personItem(row, memberItem) {
  if (row.members.length === 1) return memberItem(row.members[0]);
  const li = document.createElement("li");
  li.className = "person-row";
  const label = document.createElement("span");
  label.className = "person-row-label";
  label.textContent = row.label;
  const members = document.createElement("ul");
  members.append(...row.members.map(memberItem));
  li.append(label, members);
  return li;
}
```

  `runBrowse`: `...browseView.people.map((row) => personItem(row, (cand) => candidateItem(cand, "Open chart", "browse-status")))`.
  `renderPrompt`: the same over `prompt.people` with `"This is them — open chart"`, and
  `promptHadRows = prompt.people.length > 0`. In `link.js`, move the Compare button into a local
  `compareItem(c)` returning the `<li>`, and render `...view.people.map((row) => personItem(row, compareItem))`.
  `style.css`: `.person-row > ul { list-style: none; margin: 0.25rem 0 0 1.25rem; padding: 0; }` and
  `.person-row-label { font-weight: 600; }` (match the file's existing token/variable use).
- [ ] **Step 5: Run** the command from Step 2 — Expected: PASS. Then the headless walk
  (memory: webview mock walk recipe): stub `invoke` so `browse` returns one linked row and one single
  row; assert the single row's DOM is one `<li>` with one button (as before), the linked row shows its
  label and two buttons, and `getComputedStyle(...).display !== "none"` for the label and both buttons.
  Report the walk's result in the task report; nothing is committed for it (#332).
- [ ] **Step 6: Commit** — `feat(R3): the window lists a person once, with an open button per chart (Refs #679)`.

---

### Task 6: the CLI prints a linked member under its row

**Files:**
- Create: `crates/cairn-node/src/patient/candidate_text.rs` (+ `pub mod candidate_text;`)
- Modify: `crates/cairn-node/src/main.rs` (`print_candidates` → prints `candidate_lines`; `ellipsize` and
  `NAME_COLUMN_WIDTH` move with it)

**Interfaces:**
- Produces: `pub fn candidate_lines(list: &CandidateList) -> Vec<String>` — every line `print_candidates`
  prints, in order, the incomplete reason LAST (ADR-0060 decision 2, as today).

- [ ] **Step 1: Write the golden test FIRST, against today's output.** In `candidate_text.rs`'s tests,
  build a two-row, never-linked list (fixed ids, one candidate with age/locale/last activity, one with
  none) and assert `candidate_lines` equals the exact lines `print_candidates` prints today — derive
  each expected line by reading its `println!` format strings (header, two rows, no reason), and add
  the empty-list line `"no candidates found"` and an incomplete-list case.
- [ ] **Step 2: Move** the rendering into `candidate_lines` unchanged (`print_candidates` becomes a loop
  of `println!`); run `cargo test -p cairn-node --lib patient::candidate_text` — Expected: PASS (a pure
  move under a golden).
- [ ] **Step 3: Write the failing test** for a linked row:

```rust
#[test]
fn a_linked_member_is_printed_under_its_row() {
    // Row [a, b]: a is printed as today; b on the next line, its name cell "↳ linked: …".
    let lines = candidate_lines(&linked_list());
    assert!(lines[1].starts_with(&a.to_string()));
    assert!(lines[2].starts_with(&b.to_string()));
    assert!(lines[2].contains("↳ linked: Mary SMYTHE"), "{}", lines[2]);
}
```

- [ ] **Step 4: Implement**: the first member of a row prints as today; each further member prints the
  same columns with the name cell `format!("↳ linked: {}", name)` (ellipsized to the same width). Doc:
  `patient-register` attests `displayed_charts()`, which is this print order.
- [ ] **Step 5: Run** the module tests — Expected: PASS. Commit —
  `feat(R3): the CLI prints a linked chart under its person (Refs #679)`.

---

### Task 7: docs, the as-built note, HANDOVER/ROADMAP, the issue for the mock, full gates, PR

**Files:**
- Modify: the design page (an as-built note under "R3 — designed 2026-10-03": every deviation, or
  "none" — one is already known: `group_by_person` lives in the shared crate, not in `search_person.rs`,
  because it is pure and any picker must agree with the node on what a row is).
- Modify: `docs/HANDOVER.md` (⇒ NEXT; an **R3 durable rules** block, each rule with its pinning test:
  rows are built ONLY by `group_by_person`; the signed list is ONLY `displayed_charts()`; the cap counts
  rows and never splits one; a chart not held here is `Unknown` and never `incomplete`; a missing
  component fails the search), `docs/ROADMAP.md` (an R3 entry). Prune both toward 500 lines (HANDOVER
  626, ROADMAP 756 at the start) without dropping an open issue number — check by diffing the sets of
  `#NNN` before and after.
- File an issue: "the `--mock` window has no linked pair, so a person row cannot be walked in `--mock`"
  (label as the repo labels UI follow-ons).
- Verify: `crates/cairn-node/tests/paper_parity_plan_section.rs` passes for this plan file.

- [ ] **Step 1:** Write the as-built note and the docs; file the issue.
- [ ] **Step 2:** Gates, in CI's order, AFTER the last edit (trap 18):
  - `cargo fmt --all --check`, and the same with `--manifest-path cairn-gui/Cargo.toml`;
  - clippy on both trees (`-D warnings`);
  - `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`, and on the `cairn-gui` tree;
  - `cargo deny check`;
  - `scripts/run-db-gated-tests.sh` (background; about two hours — do the docs pass while it runs);
  - the `cairn-gui` tree's tests with the DB env set (`cairn-gui-live`);
  - `python3 scripts/check_closing_keywords.py` on every commit message and on the PR body.
  Every gate must pass; report any failure with its output.
- [ ] **Step 3:** Push; update PR #721's body (what each task built); take it out of draft. It
  references #679 without closing it (R4 is #679's build).

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk's card index, where two folders found to be one patient
  are clipped together and filed in ONE slot. A clerk looking up the patient sees one card listing
  both folder numbers, and pulls whichever folder they reached for — the clip brings the other.
- **Steps:** finding a patient at the desk is paper 1 (look at the card) → architecture-forced 1
  (look at the row) → UI target 1. Opening is paper 1 (pull a folder) → forced 1 (press a member's
  open button) → target 1. Registering after the prompt is unchanged (1 → 1 → 1). `M ≤ N` throughout;
  R3 adds no act. It removes reading effort: one person takes one of the prompt's five places
  instead of two, so the fifth-closest PERSON is no longer cut to make room for a second folder of the
  first.
- **Time + cognitive load:** budget unchanged from the funnel's — find ≤ 5 s, register ≤ 20 s
  (runbook section 8; a human act, still owed). One added query per search (the component read, one
  statement over every matched chart); its cost is reported by `scripts/measure_patient_search.py`
  at the next Pi re-run, and a regression past the funnel's "no spinner" read is filed, never
  absorbed. The load falls: a linked person is read once, with each folder's own name and age side by
  side, and the label says why a name that was not typed appears.
