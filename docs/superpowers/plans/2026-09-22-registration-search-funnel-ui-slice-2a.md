# The registration/search funnel — slice 2a, the pure core — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every *decision* on the funnel design page becomes an executable rule, testable with no
window and no database — so that slice 2b is wiring rather than judgement.

**Architecture:** A new pure crate `cairn-gui-funnel` in the `cairn-gui` workspace holding the three
rules the design states (the step-3 trigger, the bounded prompt's cap and `incomplete` derivation,
and the attested-search token), plus the two narrow ports in `cairn-gui-data` and one mock
implementing them. No Tauri, no Postgres, no frontend.

**Tech Stack:** Rust (the `cairn-gui` workspace — a *separate* cargo tree from the root), `serde`,
`cairn-patient-search` by path into `/crates`. No new third-party dependency.

**Spec:** `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md` (amended
2026-09-22 — see its *Slicing*, *Revised*, and *Split* notes)

## Global Constraints

- **AGPL-3.0**; **no new third-party dependencies in this plan.** Every crate added is a path
  dependency on code we already own.
- **TDD**: the failing test is written and *seen to fail* before implementation.
- **The dependency direction is one-way** (ADR-0021 / §9.5): `cairn-gui/*` may depend on `crates/*`,
  never the reverse. `cairn-patient-search` is safe to pull in — it is `uuid` + `serde` only, no
  database driver and no clock.
- **`cairn-gui` is its own cargo workspace and its own lockfile.** A new member crate means
  `cairn-gui/Cargo.lock` must be refreshed and **committed**, because CI runs
  `cargo clippy --locked --manifest-path cairn-gui/Cargo.toml`. No root-workspace gate sees that
  staleness — only CI does.
- **The GUI gate is `cargo fmt`, `cargo clippy --tests -- -D warnings`, `cargo test`, `cargo doc`,
  and `cargo deny`, all `--manifest-path cairn-gui/Cargo.toml`.** `cargo doc` on this tree runs with
  `RUSTDOCFLAGS=-D warnings` in CI, so an intra-doc link to a private item fails the build.
- **Never `cmd | tail`** when judging a gate — it reports the last stage's status, not cargo's.
- **This slice touches no `db/*.sql`, no `SCHEMA_GENERATION`, no event body, and no wire format.**
  If a task appears to need one, stop: it belongs in 2b or in a different slice.

## File structure

| File | Responsibility | Change |
|---|---|---|
| `cairn-gui/Cargo.toml` | workspace members | add `cairn-gui-funnel` |
| `cairn-gui/Cargo.lock` | the `--locked` CI gate | refresh + commit |
| `cairn-gui/cairn-gui-funnel/Cargo.toml` | the new pure crate | create |
| `cairn-gui/cairn-gui-funnel/src/lib.rs` | crate doc + re-exports | create |
| `cairn-gui/cairn-gui-funnel/src/trigger.rs` | when the machine searches unasked | create |
| `cairn-gui/cairn-gui-funnel/src/prompt.rs` | the bounded attested list | create |
| `cairn-gui/cairn-gui-funnel/src/token.rs` | `AttestedSearch` + `TokenStore` | create |
| `cairn-gui/cairn-gui-data/Cargo.toml` | + `cairn-patient-search`, + `cairn-gui-funnel` | modify |
| `cairn-gui/cairn-gui-data/src/port.rs` | `PatientSearch`, `PatientRegistration` | modify |
| `cairn-gui/cairn-gui-data/src/mock.rs` | a fixture patient set that can be browsed | modify |

---

### Task 1: The step-3 trigger rule

**Files:**
- Create: `cairn-gui/cairn-gui-funnel/{Cargo.toml,src/lib.rs,src/trigger.rs}`
- Modify: `cairn-gui/Cargo.toml` (members), `cairn-gui/Cargo.lock`

**Interfaces:**

```rust
pub const MIN_NAME_TOKENS: usize = 2;

/// A part of the form the early search is still waiting on — so the screen can SAY what it
/// is waiting for, rather than leaving the clerk to guess (principle 4: *not-yet-asked* is a
/// distinct state, not silence).
pub enum MissingPart { NameTokens { have: usize, need: usize }, BirthDate }

pub enum TriggerState { Ready, Waiting(Vec<MissingPart>) }

/// Should the machine search NOW, unasked? Pure; takes the raw field text.
pub fn trigger_state(raw_name: &str, birth_date: &str) -> TriggerState;
```

**Why two tokens and not given+surname.** The spec's *Revised 2026-09-22* note has the argument:
separate given/surname boxes are one culture's name model (ADR-0014), and they force the raw name to
be reassembled, which is precisely the drift `register_patient`'s doc warns about and cannot enforce
against. Counting whitespace-separated tokens carries the same information with no name model, and
the one typed string feeds `SearchQuery::new` and `register_patient` alike.

- [ ] **Step 1: Write the failing tests** in `src/trigger.rs`'s `mod tests`.

```rust
#[test]
fn two_name_tokens_and_a_birth_date_are_enough_to_search_unasked() { /* Ready */ }

#[test]
fn one_name_token_is_not_yet_enough_and_the_form_can_say_why() {
    // Waiting([NameTokens { have: 1, need: 2 }]) — the `have` is load-bearing: a form that
    // says "type more of the name" without saying how much is a guessing game.
}

#[test]
fn a_name_with_no_birth_date_is_waiting_on_the_birth_date_alone() { /* Waiting([BirthDate]) */ }

#[test]
fn an_empty_form_is_waiting_on_both_parts_not_on_one() {
    // Both, in field order. Reporting only the first missing part makes the clerk discover
    // the second one only after satisfying the first.
}

#[test]
fn whitespace_is_not_a_name_token() {
    // "   " has zero tokens, not one. `split_whitespace` already yields nothing; this pins it
    // so a future switch to `split(' ')` (which yields empty strings) cannot pass silently.
}

#[test]
fn punctuation_does_not_split_a_name_into_more_tokens_than_the_clerk_typed() {
    // "O'Brien-Smith" is ONE token here, exactly as `SearchQuery::new` treats one
    // whitespace-delimited word as one whole token. If this counted parts, a single
    // hyphenated surname would trip the trigger on its own and the early search would fire
    // on half a person.
}

#[test]
fn a_mononymous_patient_with_no_birth_date_never_becomes_ready() {
    // Principle 4. This is NOT a refusal to register — see `token.rs`, where the commit path
    // mints a token for exactly this form. The trigger only decides whether the search ALSO
    // runs early.
}
```

- [ ] **Step 2: Run them and watch them fail** (they will not compile — the crate does not exist).

- [ ] **Step 3: Create the crate and implement.**

`cairn-gui/cairn-gui-funnel/Cargo.toml` mirrors `cairn-gui-data`'s shape (workspace edition /
rust-version / license, `publish = false`). Dependencies: `cairn-patient-search` by path,
`serde` with `derive`. Add `"cairn-gui-funnel"` to `cairn-gui/Cargo.toml`'s `members`.

The crate doc must state what the crate is *for*: the funnel's rules live here so they are testable
with no window and no database, and so the surface that displays candidates and the act that attests
to them cannot each grow their own answer.

- [ ] **Step 4: Verify.**
  `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-funnel` — green.
  `cargo clippy --manifest-path cairn-gui/Cargo.toml --workspace --tests -- -D warnings` — clean.

---

### Task 2: The bounded prompt, and its `incomplete` derivation

**Files:**
- Create: `cairn-gui/cairn-gui-funnel/src/prompt.rs`
- Modify: `cairn-gui/cairn-gui-funnel/src/lib.rs` (re-export)

**Interfaces:**

```rust
/// The most candidates the step-3 prompt shows. A NAMED CONSTANT with a test pinning it,
/// because it is a clinical decision — how many existing charts a clerk is shown before
/// being allowed to create another — not a layout detail.
pub const PROMPT_CAP: usize = 5;

/// Bound a node-returned list to what the prompt can truthfully claim was displayed.
pub fn bound_for_prompt(list: &CandidateList, cap: usize) -> CandidateList;
```

**Why a constant and not a measurement.** Design decision 3 rejects viewport tracking outright: a
signed clinical record must not assert *"this row was on screen"*, and no test can pin it. So the cap
is a number a reviewer can argue with, and the prompt is laid out to fit it.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test]
fn a_list_that_fits_comes_back_untouched() {
    // Same candidates, same order, same `incomplete`, same reason. Bounding must be a no-op
    // when there is nothing to bound, or every honest prompt starts claiming partiality.
}

#[test]
fn a_longer_list_keeps_the_first_cap_candidates_in_display_order() {
    // Order is the attestation's order (`SearchAttestation::from_displayed` reads the vec in
    // order), so a reorder here silently changes what gets signed.
}

#[test]
fn truncating_marks_the_list_incomplete_and_says_how_many_were_withheld() {
    // ADR-0060 decision 2: reported, never implied. A bare `incomplete: true` is a flag a
    // clerk cannot act on.
}

#[test]
fn the_nodes_own_reason_is_never_overwritten_by_the_truncation_reason() {
    // THE ONE THAT MATTERS. If the node already said "2 charts could not be read" and we
    // replace it with "3 further candidates are not shown", the clerk loses the warning that
    // the search itself was partial — a different and worse partiality than a long list.
    // Both survive.
}

#[test]
fn a_cap_of_zero_yields_an_empty_list_that_admits_it_is_empty_by_truncation() {
    // Kept total rather than panicking, but it must never look like "found nothing": the
    // reason distinguishes a search that matched nobody from a prompt that showed nobody.
}

#[test]
fn the_prompt_cap_is_five() {
    // Pinned so a change is visible in a diff and has to be argued for.
}
```

- [ ] **Step 2: Run and watch fail.**

- [ ] **Step 3: Implement.** Keep the reason composition as its own small pure helper
  (`fn withheld_reason(withheld: usize) -> String`) so the sentence is testable on its own and is not
  built inline inside a `match`.

- [ ] **Step 4: Verify** — `cargo test -p cairn-gui-funnel`, clippy clean.

---

### Task 3: The attested-search token

**Files:**
- Create: `cairn-gui/cairn-gui-funnel/src/token.rs`
- Modify: `cairn-gui/cairn-gui-funnel/src/lib.rs` (re-export)

**Interfaces:**

```rust
/// An opaque handle to one search. Crosses to the webview and comes back; the webview can
/// only ever echo it.
pub struct SearchToken(u64);

/// A (query, displayed-list) pair a registration may attest to.
///
/// **There is no public constructor.** The ONLY way to obtain one is to put a search into a
/// `TokenStore` and take it back out by its token — which is what makes it impossible for any
/// caller, frontend or Rust, to mint an attestation for a search that never ran.
pub struct AttestedSearch { /* private */ }

impl AttestedSearch {
    pub fn token(&self) -> SearchToken;
    pub fn query(&self) -> &SearchQuery;
    pub fn displayed(&self) -> &CandidateList;
}

pub enum TokenError { EmptyQuery, Absent, Mismatched }

pub struct TokenStore { /* at most one outstanding search */ }

impl TokenStore {
    pub fn new() -> Self;
    /// Record the search that just ran, replacing any earlier one, and mint its token.
    pub fn record(&mut self, query: SearchQuery, displayed: CandidateList)
        -> Result<SearchToken, TokenError>;
    /// Take the attested search a token names, REMOVING it.
    pub fn take(&mut self, token: SearchToken) -> Result<AttestedSearch, TokenError>;
    /// Put one back after a registration that failed, keeping its token.
    pub fn restore(&mut self, attested: AttestedSearch);
    /// The form was edited: whatever is held no longer describes it.
    pub fn discard(&mut self);
}
```

**Why the port takes an `AttestedSearch` and not a token.** The design says `register` takes only the
token *"so the frontend cannot mint an attestation"*. Handing the port an `AttestedSearch` with no
public constructor is the same guarantee moved into the type system: it holds against a Rust caller
too, not merely against JavaScript, and it needs no lookup to be correct.

**Why a `u64` counter and not a UUID.** The token never crosses a trust boundary — it goes to the
window's own webview and back. A monotonic counter is unforgeable enough for that, adds no dependency
and no randomness, and a window session will not approach JavaScript's 2^53 integer limit. If JS ever
did round one, `take` returns `Mismatched` and the registration is refused: the failure is closed.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test]
fn a_recorded_search_can_be_taken_back_by_its_token() { }

#[test]
fn taking_a_token_twice_fails_the_second_time() {
    // Double-submit protection. Two clicks on Register must not produce two charts off one
    // attested search — the second finds nothing to attest and refuses.
}

#[test]
fn a_stale_token_is_refused_after_the_form_was_edited() {
    // `discard()` then `take(old)` -> Absent. Editing the form invalidates the search that
    // described the OLD form; registering on it would attest a search for a different person.
}

#[test]
fn a_token_from_a_superseded_search_is_refused_after_a_newer_one_is_recorded() {
    // `record(a)`, `record(b)`, `take(a)` -> Mismatched, NOT the b pair. The dangerous bug is
    // returning whatever is held regardless of which token was presented.
}

#[test]
fn a_restored_search_keeps_its_token_so_a_retry_after_a_failure_works() {
    // Design's "Register fails. The form keeps its values." — the clerk must not be made to
    // re-search because the database hiccuped.
}

#[test]
fn an_empty_query_is_refused_a_token() {
    // db/045 refuses a term-less attested search — "I searched for nothing and found nothing"
    // is not a search. Refusing it HERE is the same cheap pre-check main.rs makes before
    // unsealing a key and ticking an HLC.
}

#[test]
fn a_mononymous_form_with_no_birth_date_still_gets_a_token() {
    // THE TRIGGER IS NOT A GATE. `trigger_state` says Waiting for this form; `record` mints a
    // token for it anyway. Principle 4 — a patient with one name and an unknown DOB must
    // still be registrable, attesting the search that was actually possible.
}

#[test]
fn the_attested_pair_is_exactly_what_was_recorded() {
    // Feeds `SearchAttestation::from_displayed`, which is the ONE definition of what a
    // registration swears to. If this pair can drift from what was recorded, that crate's
    // whole guarantee is defeated one layer up.
}
```

- [ ] **Step 2: Run and watch fail.**

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Verify** — `cargo test -p cairn-gui-funnel`, clippy clean.

---

### Task 4: The two ports

**Files:**
- Modify: `cairn-gui/cairn-gui-data/src/port.rs`, `cairn-gui/cairn-gui-data/Cargo.toml`

**Interfaces:**

```rust
pub trait PatientSearch {
    /// Run the §5.8 candidate search. `today` is the caller's clock (ISO `YYYY-MM-DD`),
    /// exactly as `cairn_node::patient::search::search_patients` takes it — the edge owns the
    /// clock, so this stays testable against a fixed date.
    fn search(&self, query: &SearchQuery, today: &str)
        -> impl Future<Output = Result<CandidateList, DataError>> + Send;
}

pub trait PatientRegistration {
    /// Create the chart, attesting the search that licensed it. `name` is the RAW typed
    /// string the query was built from — never a reassembled one.
    fn register(&self, attested: &AttestedSearch, name: Option<&str>)
        -> impl Future<Output = Result<Uuid, DataError>> + Send;
}
```

**Why two traits and not one.** The design's reason, unchanged: the write surface is then one method
wide, and `--mock` can exercise browsing with no signing in it at all.

**Why RPITIT with an explicit `+ Send` and not `async fn`.** A bare `async fn` in a public trait
raises the `async_fn_in_trait` lint, which CI turns into an error, and the future needs `Send` to be
awaited inside a Tauri command. Spelling the bound is the fix that adds no `async-trait` dependency.
Note the consequence in the trait docs: these traits are **not dyn-compatible**, so 2b dispatches
mock-vs-live with the same `is_mock()` branch `commands.rs` already uses rather than a trait object.

- [ ] **Step 1: Write the failing test.** A compile-level test is the honest one here — the traits
  have no behaviour of their own:

```rust
/// The ports must be usable from an async context that requires `Send`, which is the only
/// property of these signatures that can actually be wrong. A Tauri command is such a
/// context, and discovering the bound is missing in 2b would mean redesigning the port.
#[test]
fn the_ports_produce_send_futures() {
    fn assert_send<T: Send>(_: T) {}
    // ... over the mock, once Task 5 lands; until then over a trivial local impl.
}
```

- [ ] **Step 2: Run and watch fail.**

- [ ] **Step 3: Implement.** Add `cairn-patient-search` and `cairn-gui-funnel` to
  `cairn-gui-data/Cargo.toml`, with the same one-way-dependency comment the existing
  `cairn-medication-view` entry carries. Keep `ClinicalData` untouched — it is read-only and stays so.

- [ ] **Step 4: Verify** — workspace test + clippy + `cargo doc`.

---

### Task 5: The mock funnel

**Files:**
- Modify: `cairn-gui/cairn-gui-data/src/mock.rs`

**What changes.** `MockData` today holds ONE patient. Browsing needs several, and picking one must
open a chart — so `demographics`/`note_refs`/`medications` have to answer for every fixture patient,
not just `FIXTURE_UUID` (which keeps its id, so nothing existing moves).

**The fixture set** is chosen to exercise slice 1's real matching shapes, not to look tidy:

| Fixture | What it is there to exercise |
|---|---|
| `Amina أمينة अमीना 阿明娜` (the existing `FIXTURE_UUID`) | multi-script shaping, the Spike 0004 pass |
| `Michaelowski, Samantha` | #636's headline case — `mich` must find her |
| `Fyodorowksi-Eschenbacher, Katarzyna` | a hyphenated compound found by either half |
| `O'Brien-Smith, John` | apostrophe + hyphen, `SearchQuery::new`'s own case |
| `Wu, Mei` | a SHORT name — found by exact match, never gated away by a prefix minimum |
| `unknown-ed-site1-2026-07-03-00ab` | a §5.4 John Doe, `TrustState::Unconfirmed` — the chart a clerk most needs when the family arrives with a name |

> **The mock's matching rule is NOT db/046's**, and its doc must say so loudly. It is a
> case-insensitive substring over the display name plus an exact birth-date compare — enough to walk
> the funnel, deliberately not a second implementation of the real semantics, which are pinned by the
> DB-gated tests in `crates/cairn-node/tests/patient_search.rs`. A fixture that quietly claimed to be
> the real matcher would teach an accessibility pass the wrong expectations.

**Mock registration.** It **succeeds**, minting a patient into the in-memory set so the next browse
finds it — which is the only thing that makes the design's *"browse, no fit, register, prompt,
commit"* mock walk mean anything. It signs nothing and touches no record, because in `--mock` there
is no record. This makes one sentence in `cairn-gui-tauri/src/main.rs` false —
*"Writes are refused in this mode"* — and 2b must narrow it to *clinical* writes; note it there, do
not edit `main.rs` in this slice.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn browsing_a_fragment_finds_the_chart_it_is_a_fragment_of() { }
#[test] fn a_short_name_is_found_by_typing_it_whole() { }
#[test] fn a_john_doe_chart_is_browsable_and_shows_as_identity_pending() { }
#[test] fn every_fixture_candidate_can_have_its_chart_opened() {
    // Browse -> pick -> `demographics` answers. A candidate a clerk can see and cannot open
    // is a dead end, and the old single-patient mock would produce exactly that.
}
#[test] fn a_fixture_registration_is_findable_by_the_next_browse() { }
#[test] fn the_browse_query_never_carries_a_sex_and_the_port_offers_nowhere_to_put_one() {
    // Design decision 4's NEGATIVE limb, which is the safety half: a candidate whose sex is
    // unrecorded or recorded wrongly can never be made invisible, because nothing narrows on
    // it. Structural, not behavioural — `SearchQuery` has no such field and the port takes
    // no such argument. The display/rank half is #645.
}
```

- [ ] **Step 2: Run and watch fail.**

- [ ] **Step 3: Implement.** Keep `mock.rs` under the 500-line guideline; if the fixture data pushes
  it over, split the data into `mock/fixtures.rs` and leave the trait impls in `mock/mod.rs`.

- [ ] **Step 4: Verify** — workspace test + clippy.

---

### Task 6: The gate, the lockfile, and the tracking documents

- [ ] **Step 1: The full GUI gate**, each judged on its own exit code (never `| tail`):

```
cargo fmt --all --check --manifest-path cairn-gui/Cargo.toml
cargo clippy --locked --manifest-path cairn-gui/Cargo.toml --workspace --tests -- -D warnings
cargo test  --locked --manifest-path cairn-gui/Cargo.toml --workspace
RUSTDOCFLAGS=-D warnings cargo doc --locked --no-deps --manifest-path cairn-gui/Cargo.toml --workspace
cargo deny --manifest-path cairn-gui/Cargo.toml check
```

The `--locked` runs are the ones that prove `cairn-gui/Cargo.lock` was refreshed and committed. A
`--locked` failure here is the exact staleness no root-workspace gate can see.

- [ ] **Step 2: The root workspace is unaffected but must be proven so.** This slice adds no root
  crate, so `cairn-gui/Cargo.lock` is the only lockfile that moves. Run
  `cargo test --workspace` at the root (DB-free is fine with `CAIRN_ALLOW_DB_SKIP=1`) purely to
  confirm nothing there moved — in particular `paper_parity_plan_section.rs`, which reads *this
  plan*.

- [ ] **Step 3: HANDOVER and ROADMAP.** Record 2a as built and 2b as next, prune the ⇒ NEXT block
  (#621/#619/#636/#639 are all merged and can be condensed to their durable rules), and keep both
  files under 500 lines without dropping an issue number.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the registration desk and the alphabetical patient index drawer.

**Steps:** unchanged from the design page, because this slice adds no surface and removes no act:

| | Register a new patient | Find an existing chart |
|---|---|---|
| Paper acts (N) | 5 — ask details, flip drawer, take blank card, write it, file it | 3 — ask details, flip drawer, pull card |
| Architecture-forced (M) | 4 — type name, read list, complete the form, answer the prompt | 2 — type fragment, pick |
| UI bundling target (K) | 4 | 2 |

`M ≤ N` on both, so there is no architecture defect to file. One thing in this slice could have
raised `M` and deliberately does not: the step-3 trigger is **advisory, never a gate**. Had
`TokenStore::record` consulted it, a mononymous patient or one with an unknown date of birth would
have been forced to fabricate a second name or a date before a chart could exist — an
architecture-forced act with no paper counterpart, and a direct principle-4 violation.
`a_mononymous_form_with_no_birth_date_still_gets_a_token` is the test that holds it open.

**Time + cognitive load:** not measurable in this slice and not claimed — 2a exposes no runnable
surface. The end-to-end measurement is owed by **2b**, which is the funnel's first runnable surface,
taken in `--mock` and against a database with the existing timing instrumentation. What 2a fixes is
the *budget* the measurement will be taken against: browse results within §5.11's *no spinner*
(now plausible — #639 brought the Pi-class floor to ~883 ms), and the 5 s ceiling for *find an
existing chart* inherited from `db/046`.

## Out of scope

Everything in the design's own *Out of scope*, plus: the Tauri commands, the shell state and the
optional `--patient`, the frontend, the JS/Rust drift-guard extension, the DB-gated attestation
test, and the §1.2 measurement. All of those are 2b.

## Risks

- **The mock's recall is not the node's.** Stated loudly in its doc, but an accessibility or timing
  pass run only in `--mock` will still form expectations the real search does not meet. 2b's
  measurement must be taken against a database, not against fixtures, and the design already says so.
- **`PROMPT_CAP = 5` is a guess at "what fits without scrolling".** The design's own risk paragraph
  names the failure: if the prompt is *routinely* incomplete, the cap is wrong and the design needs
  revisiting rather than quietly signing partial lists. 2b should report how often it truncates.
- **A `u64` token crossing into JavaScript.** Argued above as fail-closed, but it is the one place in
  this slice where a value leaves Rust's type system and comes back. If 2b finds any rounding at all,
  the fix is to serialize it as a string, not to widen the acceptance.
