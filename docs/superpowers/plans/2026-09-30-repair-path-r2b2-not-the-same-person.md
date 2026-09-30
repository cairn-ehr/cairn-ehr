# Repair path R2b-2 — "Not the same person": unlink one link from the window, and #699 (a) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** From a combined (linked) record, a clinician sees every LINK that joins its charts, picks
the one that is wrong, reads the two charts side by side, and unlinks it with the one signature their
unlocked key already covers — even when neither of that link's charts is held on this node (#699 (a))
— and the window reports what the unlink actually did.

**Architecture:** The node gains (1) a corrected "still joined?" question and a typed filing rule
(`FiledUnder`) so an unlink may be filed under the chart it was judged from, (2) `unlink_charts`'s
`opened` parameter and `admit_judgement`'s third-chart arm, and (3) a read of the standing links
inside a chart set (`patient::edges::record_edges`). The window (`cairn-gui-tauri/src/link/`) adds
the pane's "How these charts are linked" list, two commands (`compare_linked`, `unlink_records`)
behind the same screen/set checks every chart command applies, pure wording in
`link/unlink_view.rs` and `link/record_links.rs`, and a separate `src-ui/unlink.js` panel.

**Tech Stack:** Rust (tokio-postgres, anyhow, serde, clap), PostgreSQL ≥ 18 + `cairn_pgx`, Tauri 2
with plain JavaScript (no npm, no bundler).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` — section
**"R2b — the window's gesture (designed 2026-09-28)"**, its **"R2b-2 — 'Not the same person' and
#699 (a)"** bullets, and the R2b-1 as-built note. ADR-0076 decisions 3–5. Issue #699 (decided (a),
2026-09-28). R2b-1's plan (`docs/superpowers/plans/2026-09-29-repair-path-r2b1-same-person-as.md`)
is the template this one follows.

## Decisions made while planning (each becomes an as-built line in Task 7)

1. **One list of links, not links under each member line** (maintainer, 2026-09-30). The design said
   "each member line lists the links that actually join it"; that puts every link on screen twice
   (under both of its charts), so every control exists twice. Instead: under the member lines, a
   list **"How these charts are linked"**, one entry per standing link, each with its own
   **"Not the same person…"**. A `StillJoined` outcome points at that one list.
2. **The un-attested wording is "without a clinician's confirmation on record here"**, not the
   design's "by the matcher (not reviewed)". Locally only the matcher writes un-attested links, but a
   PEER's human link whose attester is not enrolled here also stores `attested = false` — so
   "matcher" can be untrue (principle 4; the R2b-1 precedent: principle 4 outranks a plan's text).
3. **"Still joined?" is asked of the two SUBJECTS, not of the filed-under chart** (audit finding,
   2026-09-30). Today `judge` asks `other ∈ person_charts(about)`; with a third-chart filing that
   answers `StillJoined` for every successful A–B–C split. The question is `high ∈
   person_charts(low)` — identical to today's answer whenever `about` is a subject.
4. **The unlink panel is a SEPARATE `<section>` from the link panel**, not a mode of it: one panel
   with two verbs can show the wrong verb's button over the other's comparison.
5. **#699 (a) gets a short ADR (ADR-0077, spec v0.79).** It changes an event-core convention — an
   identity event's envelope `patient_id` may now name a chart that is neither subject — and the ADR
   log is the home of *why*. It records the maintainer's settled decision; it re-opens nothing.

## The audit (#699 (a)'s "filed-under ∈ subjects" readers — done 2026-09-30, before this plan)

Every projection, flag, trust view, heal/re-fold pass (db/018, db/019, db/023–025, db/039, db/043,
db/054, db/055), the sync doors and page selection (db/020, db/051, `cairn-sync`, `cairn-node::sync`),
the medium, and the plaintext twin (`render_unlink_twin`) read the pair from the PAYLOAD subjects;
none reads the envelope for a link/unlink event, and replication has no patient scope. db/005 step
8b (keyed on the envelope, deliberately) admits an event filed under a held chart. Observable,
accepted changes: the event sits in the opened chart's `event_log` stream; it takes that chart's
effective sensitivity grade (`cairn_effective_sensitivity`, db/048 — today the other subject's
grade is already ignored, so no new gap); a receiver that lacks the opened chart counts it as "has
events" (an existing pattern for any replicated event). The only blockers are in `chart_link.rs`:
the `about ∈ {low, high}` guard, and decision 3 above. Task 2's tests pin the audit's findings.

## Global Constraints

- **AGPL-3.0**; **no new dependency** in any tree (check `Cargo.toml` before adding a `use`; if one is
  missing, stop and ask).
- **No SQL object, no migration: `SCHEMA_GENERATION` stays 55.** If a task seems to need a
  `db/*.sql` change, stop.
- **No confirmation dialog anywhere** (principle 3). The panel's safety is what it SHOWS.
- **No node-key fallback**: a judgement is signed and attested by the unlocked human key (ADR-0053).
- **The third-chart filing (`FiledUnder::RecordOf`) is for UNLINK only.** A link filed under a
  chart that is neither subject is refused before anything is signed (the design: "so the relaxation
  cannot reach `link`").
- **Per EDGE, never per member.** The machine never guesses which link is wrong (principle 2).
- **Every chart command names the chart on screen AND the displayed set** (`displayed_patient`
  first, then `chart_set::check_displayed_set`), and the unlink commands also require the link to
  still be one of the record's standing links.
- **Show `LinkEffect`, never assume it**: `StillJoined` and `Outranked` must never read as done.
- **Absence is worded, never blank** (principle 4).
- **Files under 500 lines** (house rule 4). `chart_link.rs` is 738 and `chart_set.rs` 555 already —
  Task 1 moves the pure admission rule out of `chart_link.rs`; nothing new goes into `chart_set.rs`
  beyond the fields named in Task 5; `link/mod.rs` (489) gains only a visibility change.
- **CodeQL**: a test assertion message must never echo a name, DOB or identity text built from
  fixture data (`rust/cleartext-logging`) — use static messages. No binding named `salt`/`nonce`/`iv`.
- **Doc comments for a junior reader** on every non-trivial function (house rule 3).
- **Test helpers stay FILE-LOCAL** (a `pub fn` added to `crates/cairn-node/tests/common/mod.rs` must
  also go into `identity_scaffolding_shared.rs`'s expected list).
- **Every panel message goes through `setMessage`, never a bare `.textContent`** (R2b-1 rule: an
  empty status line is `hidden`, and `[hidden]` wins).
- **Gates.** Root DB-gated sweep: `scripts/run-db-gated-tests.sh` (no args). Targeted run of one
  suite: `CAIRN_TEST_PG="host=127.0.0.1 port=$(scripts/pg-target.sh | cut -d' ' -f2) user=$USER dbname=cairn_test" cargo test -p cairn-node --test <suite>`
  — a targeted `ok` is NOT proof (trap 18); the sweep is. DB-free runs need `CAIRN_ALLOW_DB_SKIP=1`.
  GUI tree: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace && cargo clippy --workspace --all-targets --locked -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.
  Root: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`. With an IDE open, prefix cargo with
  `CARGO_TARGET_DIR=/tmp/cairn-r2b2-target`. **Never pipe a cargo run into `tail`** (it masks the
  exit code). **Subagents: run tests in the FOREGROUND only** — a subagent waiting on a background
  job never wakes.
- **Commit messages**: run `python3 scripts/check_closing_keywords.py <file-with-message>` before
  committing anything that names an issue; `Refs #699` is safe, `closes`/`fixes` adjacent to a
  number is not.

## Review Focus

1. **The wrong link is the far one (A open, A–B–C, B–C wrong, neither B nor C held here)** →
   the unlink is admitted, filed under A, and reports `TookEffect` with C leaving the record — never
   `StillJoined` (Task 2 DB test `a_chain_split_from_the_opened_chart_took_effect`).
2. **The picked link sits on a cycle (A–B, B–C, A–C; unlink B–C)** → recorded, reported as
   `StillJoined`, the pane reloads and the other links stay listed (Task 2 DB test; Task 4 wording).
3. **The link was already undone (a peer's unlink, or this clinician's own in another window)
   between the pane read and the click** → refused "that link is no longer part of this record —
   nothing was done; reload the chart", nothing signed (Task 5 test `unlink_refuses_a_link_the_record_no_longer_has`).
4. **A `link` is offered a third-chart filing** (a refactor passes `RecordOf` to `assert_link_in_tx`)
   → refused before signing (Task 1 pure test `a_link_is_never_filed_under_a_third_chart`).
5. **The pane's link read fails while the member read succeeds** → the medication list AND the member
   lines still show, the links section says it could not be read and that unlinking is unavailable
   until reload — never an empty list that reads as "nothing is linked" (Task 4 pure test
   `an_unread_link_list_says_so_and_offers_no_unlink`).

---

## File structure

| File | Responsibility |
|---|---|
| `crates/cairn-node/src/chart_link.rs` (modify) | `FiledUnder` through `assert_link_in_tx`; `judge` asks the subjects; `unlink_charts(…, opened, …)`; `LinkOutcome::record_of`. |
| `crates/cairn-node/src/chart_link/admit.rs` (create) | PURE: `FiledUnder`, `filing_for`, `OpenedChart`, `admit_judgement` (moved) + their unit tests. |
| `crates/cairn-node/src/apply_proposal.rs` (modify) | Pass `FiledUnder::Subject(low)`. |
| `crates/cairn-node/src/main.rs` (modify) | `unlink-charts --from <chart>`; report names `record_of`. |
| `crates/cairn-node/src/patient/edges.rs` (create) | `RecordEdge`, `record_edges`. |
| `crates/cairn-node/src/patient/mod.rs` (modify) | `pub mod edges;` |
| `crates/cairn-node/tests/unlink_from_record.rs` (create) | DB-gated: #699 (a) and the audit's pins. |
| `crates/cairn-node/tests/record_edges.rs` (create) | DB-gated: the link read. |
| `crates/cairn-node/tests/chart_link.rs` (modify) | `unlink_charts` call sites gain `None`. |
| `crates/cairn-node/tests/db_errors_stay_legible.rs` (modify) | `patient/edges.rs` joins `GUARDED` with its count pin. |
| `cairn-gui/cairn-gui-tauri/src/link/record_links.rs` (create) | Pure `record_link_line`, `links_section`; async `read_record_edges`. |
| `cairn-gui/cairn-gui-tauri/src/link/unlink_view.rs` (+ `unlink_view_tests.rs`) (create) | Pure: `UnlinkComparisonView`, `unlink_comparison_view`, `unlink_report`, `unlink_error_view`, `LINK_GONE`. |
| `cairn-gui/cairn-gui-tauri/src/link/unlink.rs` (create) | `compare_linked_impl`, `unlink_impl`, `resolve_edge`, Tauri forwarders. |
| `cairn-gui/cairn-gui-tauri/src/link/view.rs` (modify) | `fact_rows`, `heading` → `pub(crate)`; `judgement_error_from`. |
| `cairn-gui/cairn-gui-tauri/src/link/mod.rs` (modify) | `pub mod record_links; pub mod unlink; mod unlink_view;` + `chart_set_of` → `pub(crate)`. |
| `cairn-gui/cairn-gui-tauri/src/chart_set.rs` (modify) | `MemberLine::name`; `ChartPane::{links, links_error}`; `chart_pane` takes the edges. |
| `cairn-gui/cairn-gui-tauri/src/commands.rs` (modify) | `med_list_impl` reads the edges; the webview-fields guard covers `recordLink`. |
| `cairn-gui/cairn-gui-tauri/src/main.rs` (modify) | Two handlers. |
| `cairn-gui/cairn-gui-tauri/src-ui/index.html`, `main.js`, `unlink.js` (create), `style.css` | The links list and the unlink panel. |
| `docs/spec/decisions/0077-…md`, `docs/spec/identity.md`, `docs/spec/index.md`, `docs/spec/decisions/README.md`, `mkdocs.yml` | ADR-0077, prose, v0.79. |
| `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `TEMPLATE.md` | Section 10: unlink. |

---

### Task 1: `FiledUnder`, the pure admission module, and the corrected "still joined?" question

No behaviour changes for any two-chart judgement; this task makes Task 2 possible and fixes the
question Task 2 would otherwise get wrong.

**Files:**
- Create: `crates/cairn-node/src/chart_link/admit.rs`
- Modify: `crates/cairn-node/src/chart_link.rs` (add `pub mod admit; pub use admit::*;`; delete the
  moved `admit_judgement` and its two unit tests; `assert_link_in_tx` signature; `judge`'s effect)
- Modify: `crates/cairn-node/src/apply_proposal.rs` (its `assert_link_in_tx` call)

**Interfaces:**
- Produces:
  ```rust
  // chart_link::admit (re-exported from chart_link)
  pub enum FiledUnder { Subject(Uuid), RecordOf(Uuid) }
  impl FiledUnder { pub fn chart(self) -> Uuid }
  pub fn filing_for(verb: LinkVerb, low: Uuid, high: Uuid, filed: FiledUnder) -> Result<Uuid, String>
  pub fn admit_judgement(verb, (a, a_held), (b, b_held), shared_record: bool) -> Result<Uuid, String> // moved, unchanged in Task 1
  // chart_link
  pub async fn assert_link_in_tx(tx, verb, low, high, filed: FiledUnder, provenance, confidence, reviewer, hlc) -> anyhow::Result<Asserted>
  ```
  `FiledUnder` derives `Debug, Clone, Copy, PartialEq, Eq`.

- [ ] **Step 1: Create `chart_link/admit.rs` with the failing tests.** Rust 2018 module layout:
  `src/chart_link.rs` with `pub mod admit;` resolves to `src/chart_link/admit.rs` (no `mod.rs`).
  Move `admit_judgement` (and its doc comment) verbatim from `chart_link.rs` into `admit.rs`,
  together with its two unit tests `a_link_needs_both_charts_held` and
  `an_unlink_may_name_a_displayed_member_not_held_here_but_not_a_stranger` (and the `pair()` helper
  they use). Keep `is_held` in `chart_link.rs` — it is a DB call, and `chart_link.rs` is in
  `db_errors_stay_legible.rs`'s `GUARDED` list; `admit.rs` must stay pure (no postgres). Then add:

  ```rust
  //! Who may make a link/unlink judgement from this node, and which chart its event is FILED
  //! under. **Pure** — no database — so every rule here is unit-tested on its own; `judge` in the
  //! parent module reads the facts (is each chart held? does a record contain the pair?) and asks.
  //!
  //! "Filed under" is the event ENVELOPE's `patient_id`: the chart whose `event_log` stream the
  //! event sits in. It is not what the event is ABOUT — db/018 reads the pair from the payload's
  //! `subject_a`/`subject_b`, never from the envelope (audit, R2b-2 plan). db/005 step 8b refuses a
  //! local event filed under a chart with no history here, so the filed-under chart must be HELD.
  use super::{canonical_pair, LinkVerb};
  use uuid::Uuid;

  /// Which chart a judgement's event is filed under, and WHY that chart may carry it.
  ///
  /// Typed rather than a bare `Uuid` so the one relaxation #699 (a) makes cannot leak: only an
  /// UNLINK may be filed under a chart that is neither subject ([`filing_for`] refuses the rest).
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum FiledUnder {
      /// One of the two charts being judged (the C1 convention: `low`, or the held one).
      Subject(Uuid),
      /// The chart the clinician has OPEN, held here, whose record reads both subjects as part of
      /// it (#699 (a)): the far link B–C of an A–B–C record, when neither B nor C is held here.
      RecordOf(Uuid),
  }

  impl FiledUnder {
      /// The chart the envelope names, whichever reason admitted it.
      pub fn chart(self) -> Uuid {
          match self {
              FiledUnder::Subject(c) | FiledUnder::RecordOf(c) => c,
          }
      }
  }

  /// Check a filing against the pair before anything is signed, and return the envelope chart.
  /// `Subject` must name `low` or `high` (a wrong one would misfile the event in an unrelated
  /// patient's stream, invisibly to the database floor); `RecordOf` is refused for a LINK and for
  /// a chart that IS a subject (that is `Subject`, and saying otherwise hides which rule admitted
  /// it). `Err(text)` names what is wrong.
  pub fn filing_for(
      verb: LinkVerb,
      low: Uuid,
      high: Uuid,
      filed: FiledUnder,
  ) -> Result<Uuid, String> {
      match filed {
          FiledUnder::Subject(c) if c == low || c == high => Ok(c),
          FiledUnder::Subject(c) => Err(format!(
              "a judgement about ({low}, {high}) cannot be filed under chart {c} as one of its subjects"
          )),
          FiledUnder::RecordOf(_) if verb == LinkVerb::Link => Err(format!(
              "a link between {low} and {high} must be filed under one of them, never under a third chart"
          )),
          FiledUnder::RecordOf(c) if c == low || c == high => Err(format!(
              "chart {c} is a subject of the judgement, not a third chart"
          )),
          FiledUnder::RecordOf(c) => Ok(c),
      }
  }
  ```

  And in `admit.rs`'s test module (alongside the two moved tests):

  ```rust
      #[test]
      fn a_subject_filing_must_name_one_of_the_pair() {
          let (lo, hi) = pair();
          let third = Uuid::from_u128(9);
          for verb in [LinkVerb::Link, LinkVerb::Unlink] {
              assert_eq!(filing_for(verb, lo, hi, FiledUnder::Subject(lo)), Ok(lo));
              assert_eq!(filing_for(verb, lo, hi, FiledUnder::Subject(hi)), Ok(hi));
              assert!(filing_for(verb, lo, hi, FiledUnder::Subject(third)).is_err());
          }
      }

      /// Review Focus 4: the #699 (a) relaxation must never reach a link.
      #[test]
      fn a_link_is_never_filed_under_a_third_chart() {
          let (lo, hi) = pair();
          let third = Uuid::from_u128(9);
          let refusal = filing_for(LinkVerb::Link, lo, hi, FiledUnder::RecordOf(third)).unwrap_err();
          assert!(refusal.contains("never under a third chart"), "{refusal}");
          assert_eq!(
              filing_for(LinkVerb::Unlink, lo, hi, FiledUnder::RecordOf(third)),
              Ok(third)
          );
      }

      #[test]
      fn a_subject_is_not_a_third_chart() {
          let (lo, hi) = pair();
          assert!(filing_for(LinkVerb::Unlink, lo, hi, FiledUnder::RecordOf(lo)).is_err());
      }
  ```

  The test module header: `#[cfg(test)] mod tests { use super::*; fn pair() -> (Uuid, Uuid) { … } … }`
  (copy `pair()` from `chart_link.rs`'s tests; keep a copy there too if its remaining tests use it).

- [ ] **Step 2: Run to verify they fail.** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib chart_link`
  Expected: compile error (`pub mod admit` not declared) — then, once declared, the three new tests
  pass only after Step 3 (they fail to compile against the missing `filing_for`).

- [ ] **Step 3: Wire it.** In `chart_link.rs`: add `pub mod admit;` and `pub use admit::*;` after the
  `use` block (the re-export keeps `chart_link::admit_judgement` and every existing import working).
  Change `assert_link_in_tx`'s `about: Uuid` parameter to `filed: FiledUnder` and replace its second
  `ensure!` with:

  ```rust
      // Checked before anything is locked or signed: a wrong envelope chart is invisible to the
      // database floor (db/018 reads the pair from the payload). See `admit::filing_for`.
      let about = filing_for(verb, low, high, filed).map_err(anyhow::Error::msg)?;
  ```

  (keep the `low < high` `ensure!` above it). Update its doc comment: "`filed` must name `low` or
  `high` — or, for an UNLINK only, the held chart whose record contains both
  ([`FiledUnder::RecordOf`], #699 (a))". Update `build_attested_assertion_body`'s doc the same way
  ("`about` is the chart the envelope is filed under — a subject, or for an unlink the chart the
  judgement was made from; see [`FiledUnder`]"). In `apply_proposal.rs`, its call passes
  `FiledUnder::Subject(low)` where it passed `low`. In `judge`, pass `FiledUnder::Subject(about)`.

- [ ] **Step 4: Fix the "still joined?" question in `judge`.** Replace

  ```rust
      let other = if about == a { b } else { a };
      let effect = link_effect(verb, asserted.agrees, charts.contains(&other));
  ```

  with

  ```rust
      // "Still joined?" is a question about the two SUBJECTS — do they still read as one
      // record? — asked of the subjects themselves, never of the filed-under chart: once an
      // unlink may be filed under a THIRD chart (#699 (a)), "is the other chart in the filed-under
      // chart's record" answers StillJoined for every successful split (the far link of A–B–C,
      // filed under A, leaves B in A's record). Read in this transaction, like `charts`.
      let joined = crate::patient::person::person_charts(&tx, low)
          .await
          .context("reading whether the two charts still read as one record")?
          .contains(&high);
      let effect = link_effect(verb, asserted.agrees, joined);
  ```

  and update `link_effect`'s doc: "`other_in_record`" → "`still_joined`: the two subjects still
  read as one record (`high ∈ person_charts(low)`), read inside the judgement's transaction". Rename
  the parameter to `still_joined` too.

- [ ] **Step 5: Run.** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib` (PASS), then the DB suites
  that exercise every path: `… --test chart_link`, `… --test apply_proposal`, `… --test auto_apply`,
  `… --test link_precedence` (all PASS — no two-chart behaviour changed). `wc -l
  crates/cairn-node/src/chart_link.rs` should now be well under the 738 it started at.

- [ ] **Step 6: Commit.**
  ```bash
  git add crates/cairn-node/src/chart_link.rs crates/cairn-node/src/chart_link/admit.rs crates/cairn-node/src/apply_proposal.rs
  git commit -m "refactor(R2b-2): FiledUnder, a pure admission module, and 'still joined?' asked of the subjects (Refs #699)"
  ```

---

### Task 2: #699 (a) — an unlink judged from a record filed under the opened chart; `--from`

**Files:**
- Modify: `crates/cairn-node/src/chart_link/admit.rs` (`OpenedChart`, the new arm)
- Modify: `crates/cairn-node/src/chart_link.rs` (`unlink_charts`, `judge`, `LinkOutcome::record_of`)
- Modify: `crates/cairn-node/src/main.rs` (`UnlinkArgs`, `chart_judgement`, `chart_judgement_report`)
- Modify: `crates/cairn-node/tests/chart_link.rs` (every `unlink_charts(` call gains `None` before `&who`)
- Create: `crates/cairn-node/tests/unlink_from_record.rs`

**Interfaces:**
- Consumes: Task 1's `FiledUnder`, `filing_for`.
- Produces:
  ```rust
  pub struct OpenedChart { pub chart: Uuid, pub held: bool, pub holds_both: bool }
  pub fn admit_judgement(verb, (a, a_held), (b, b_held), shared_record: bool, opened: Option<OpenedChart>) -> Result<FiledUnder, String>
  pub async fn unlink_charts(client: &mut Client, a: Uuid, b: Uuid, opened: Option<Uuid>, reviewer: &Reviewer<'_>, node_origin: &str) -> anyhow::Result<LinkOutcome>
  // LinkOutcome gains:
  pub record_of: Uuid   // `opened` when given, else `filed_under`; `charts` is ITS chart set
  ```
  `link_charts` is unchanged.

- [ ] **Step 1: Failing unit tests in `admit.rs`.** Change the two moved tests to pass `None` as the
  new last argument and compare against `Ok(FiledUnder::Subject(x))`. Add:

  ```rust
      /// #699 (a): the far link of A–B–C, neither B nor C held, judged from A.
      #[test]
      fn an_unlink_neither_held_is_filed_under_the_opened_record_that_holds_both() {
          let (lo, hi) = pair();
          let a = Uuid::from_u128(9);
          let opened = OpenedChart { chart: a, held: true, holds_both: true };
          assert_eq!(
              admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), true, Some(opened)),
              Ok(FiledUnder::RecordOf(a))
          );
          // Never for a link, whatever the opened record holds.
          assert!(admit_judgement(LinkVerb::Link, (lo, false), (hi, false), true, Some(opened)).is_err());
      }

      #[test]
      fn the_opened_chart_must_be_held_and_its_record_must_hold_both() {
          let (lo, hi) = pair();
          let a = Uuid::from_u128(9);
          for (held, holds_both) in [(false, true), (true, false), (false, false)] {
              let opened = OpenedChart { chart: a, held, holds_both };
              let refusal =
                  admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), false, Some(opened))
                      .unwrap_err();
              assert!(refusal.contains(&a.to_string()), "names the opened chart");
          }
      }

      /// A held subject is still preferred: the third-chart arm is only for neither-held.
      #[test]
      fn a_held_subject_is_filed_under_itself_even_when_a_chart_is_open() {
          let (lo, hi) = pair();
          let opened = OpenedChart { chart: Uuid::from_u128(9), held: true, holds_both: true };
          assert_eq!(
              admit_judgement(LinkVerb::Unlink, (lo, false), (hi, true), true, Some(opened)),
              Ok(FiledUnder::Subject(hi))
          );
      }
  ```

- [ ] **Step 2: Run to verify they fail** (compile: wrong arity / `OpenedChart` missing).

- [ ] **Step 3: Implement the arm.** In `admit.rs`:

  ```rust
  /// The chart the clinician has OPEN when they judge — the record the judgement is made from.
  /// Only consulted for an UNLINK where neither subject is held here (#699 (a)).
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct OpenedChart {
      pub chart: Uuid,
      /// A `patient_chart` row: db/005 step 8b will admit an event filed under it.
      pub held: bool,
      /// Its record (`person_charts`) reads BOTH subjects as part of it, here.
      pub holds_both: bool,
  }
  ```

  `admit_judgement` gains `opened: Option<OpenedChart>` and returns `Result<FiledUnder, String>`:
  every existing `Ok(x)` becomes `Ok(FiledUnder::Subject(x))`, and the `(_, false, false)` arm
  becomes two arms, the first before it:

  ```rust
          (LinkVerb::Unlink, false, false) => match opened {
              Some(o) if o.held && o.holds_both => Ok(FiledUnder::RecordOf(o.chart)),
              Some(o) => Err(format!(
                  "neither chart {a} nor chart {b} is held on this node, and the chart you have \
                   open ({}) {} — {rule}",
                  o.chart,
                  if o.held { "does not read both as part of its record" } else { "is not held here either" }
              )),
              None => Err(format!("neither chart {a} nor chart {b} is held on this node — {rule}")),
          },
          (_, false, false) => Err(format!(
              "neither chart {a} nor chart {b} is held on this node — {rule}"
          )),
  ```

  and extend `rule` with "; or, for an unlink, the chart you have open held here with both in its
  record". Update the doc comment's bullet list with the third-chart case.

- [ ] **Step 4: Thread `opened` through `judge`.** `unlink_charts` gains `opened: Option<Uuid>`
  (after `b`) and passes it; `link_charts` passes `None`. `judge` gains `opened: Option<Uuid>`.
  After computing `(a_held, b_held)`:

  ```rust
      // Only read when it can change the answer: an unlink with neither chart held here, judged
      // from an open chart (#699 (a)). Both are pre-checks for a LEGIBLE refusal; db/005 step 8b
      // is the enforcement (it refuses an event filed under a chart with no history here).
      let opened_chart = match (verb, a_held || b_held, opened) {
          (LinkVerb::Unlink, false, Some(o)) => {
              let record = crate::patient::person::person_charts(&*client, o)
                  .await
                  .context("reading the open chart's record")?;
              Some(OpenedChart {
                  chart: o,
                  held: is_held(client, o).await?,
                  holds_both: record.contains(&a) && record.contains(&b),
              })
          }
          _ => None,
      };
      let filed = admit_judgement(verb, (a, a_held), (b, b_held), shared_record, opened_chart)
          .map_err(node_state_refusal)?;
  ```

  Pass `filed` to `assert_link_in_tx`; let `about = filed.chart()`. Replace the `charts` read with
  the chart set of `record_of`:

  ```rust
      // The record the clinician judged FROM (the open chart), else the filed-under chart —
      // what the caller shows next. Read in this transaction.
      let record_of = opened.unwrap_or(about);
      let charts = crate::patient::person::person_charts(&tx, record_of)
          .await
          .context("reading the chart set the judgement leaves")?;
  ```

  and add `record_of` to `LinkOutcome` (doc: "The chart whose record [`LinkOutcome::charts`] is:
  the chart the judgement was made from when the caller named one (`unlink_charts`'s `opened`), else
  [`LinkOutcome::filed_under`]"). Update `filed_under`'s doc: "always one this node HOLDS — a subject,
  or for an unlink judged from an open record, that chart (#699 (a))".

- [ ] **Step 5: Write the DB-gated suite `crates/cairn-node/tests/unlink_from_record.rs`.** Header doc:
  "#699 (a): an unlink where neither chart is held here, judged from an open chart whose record holds
  both, is filed under that chart. And the audit's pins (R2b-2 plan): the pair comes from the payload
  wherever the envelope files it." Use the same scaffolding as `tests/chart_link.rs` —
  `mod common; use common::{apply_remote_raw, link_assertion_event};`, a file-local `cs()`, a
  file-local `setup(&c)` copied from `chart_link.rs:26-58` (it enrols an agent key and a human key),
  `const ORIGIN`, `db::test_serial_guard`, `db::connect_and_load_schema`. A file-local helper builds
  the A–B–C fixture:

  ```rust
  /// A held; B and C never registered here; a peer's machine links A–B and B–C (both filed under
  /// A, the only chart with history here — as a peer holding A would file them).
  async fn chain(c: &Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid, Uuid) {
      let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
      common::submit_registration(c, sk, kid, a, 1).await;
      for (x, y, wall) in [(a, b, 50), (b, cc, 51)] {
          let mut ev = link_assertion_event(kid, x, y, LinkVerb::Link, wall, 0, "peer-matcher", false);
          ev.patient_id = a.to_string();
          apply_remote_raw(c, sk, ev).await.expect("a peer's link is admitted");
      }
      (a, b, cc)
  }
  ```

  (If `link_assertion_event` requires `a < b` for its subjects, canonicalise with
  `chart_link::canonical_pair` before calling it.) Tests — each asserts with STATIC messages:

  1. `a_chain_split_from_the_opened_chart_took_effect` — `unlink_charts(&mut c, b, cc, Some(a), &who, ORIGIN)`
     is `Ok`; `out.filed_under == a`; `out.record_of == a`; `out.effect == LinkEffect::TookEffect`;
     `out.charts.members() == [a, b]` sorted as `ChartSet` sorts; the `event_log` row for
     `out.event_id` has `patient_id = a` (`SELECT patient_id::text FROM event_log WHERE event_id = $1::text::uuid`);
     `patient_link` for `canonical_pair(b, cc)` is `('unlink', attested = true)`; the stored body's
     `subject_a`/`subject_b` are the canonical pair (`SELECT body->>'subject_a', body->>'subject_b' FROM event_log …`
     — `event_log.body` IS the payload, no wrapper); the twin (`plaintext_twin`) contains both `b`
     and `cc` and does not contain `a`.
  2. `a_link_on_a_cycle_is_recorded_and_says_still_joined` — as `chain`, plus a peer link A–C (wall
     52); unlink B–C from A → `effect == StillJoined`, `charts` has all three.
  3. `the_open_chart_must_hold_both_in_its_record` — a fourth chart D registered here, not linked;
     `unlink_charts(&mut c, b, cc, Some(d), …)` is `Err`; `refusal_scope(&err) == Some(RefusalScope::NodeState)`;
     `patient_link` for (b, cc) still `('link', false)` — nothing written.
  4. `without_an_open_chart_a_neither_held_unlink_is_still_refused` — `unlink_charts(…, b, cc, None, …)`
     is `Err`, NodeState, nothing written (the pre-#699 rule, kept for the CLI without `--from`).
  5. `a_receiver_without_the_opened_chart_applies_the_unlink` — the audit's sync pin, in one
     database: build an un-attested unlink of (b, cc) with `ev.patient_id` set to a chart `z` this
     node has NEVER seen (`Uuid::now_v7()`), wall 60, and `apply_remote_raw` it after `chain`; assert
     `Ok` and `patient_link` for (b, cc) is `'unlink'` — the sync door files it anywhere and projects
     from the payload.
  6. `a_reprojection_reproduces_the_third_chart_unlink` — after test 1's unlink, run
     `SELECT count(*) FROM cairn_reproject('identity.', true, 'test')` (rebuild) and assert
     `patient_link` for (b, cc) is still `('unlink', true)` and `cairn_person_charts(a)` is `{a, b}`.

- [ ] **Step 6: Run to verify they fail, then pass.** Before Step 4 the suite does not compile; after,
  run `CAIRN_TEST_PG=… cargo test -p cairn-node --test unlink_from_record` → PASS, and
  `--test chart_link` (after adding `None` to its `unlink_charts` calls) → PASS.

- [ ] **Step 7: The CLI.** In `main.rs`: `Cmd::UnlinkCharts` takes a new
  ```rust
  /// `unlink-charts`: the pair, plus the chart the judgement is made from.
  #[derive(clap::Args, Clone, Debug)]
  struct UnlinkArgs {
      #[command(flatten)]
      pair: ChartPairArgs,
      /// The chart you are judging FROM — held here, its record reading both charts as part of
      /// it. Needed only when neither chart is held on this node (#699 (a)); the judgement is then
      /// filed under this chart.
      #[arg(long)]
      from: Option<Uuid>,
  }
  ```
  `chart_judgement` returns `Option<(LinkVerb, &ChartPairArgs, Option<Uuid>)>` (`None` for link,
  `args.from` for unlink); `run_chart_judgement` takes and forwards `opened`. `chart_judgement_report`'s
  last line becomes `"chart {} now reads as: {}"` over `out.record_of`, and the `StillJoined` line names
  both subjects: `"recorded that {a} and {b} are different people — but they still read as one record
  through another link; unlink that link too"`. Update the existing pure test
  `link_and_unlink_report_says_what_the_judgement_did` (it builds a `LinkOutcome` — add `record_of`) and
  the command→verb mapping test for the new tuple. Run `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --bin cairn-node chart_judgement link_and_unlink` → PASS.

- [ ] **Step 8: Cross-crate call sites.** `grep -rn "unlink_charts(\|LinkOutcome {" crates cairn-gui --include='*.rs'`
  (quote the glob in zsh) and update each; then `CAIRN_ALLOW_DB_SKIP=1 cargo check --workspace --all-targets`
  in the root AND `cd cairn-gui && cargo check --workspace --all-targets` — PASS.

- [ ] **Step 9: Commit.**
  ```bash
  git add crates/cairn-node cairn-gui
  git commit -m "feat(R2b-2): an unlink judged from an open record is filed under it — #699 (a); unlink-charts --from"
  ```

---

### Task 3: `record_edges` — the standing links inside a chart set

**Files:**
- Create: `crates/cairn-node/src/patient/edges.rs`; modify `patient/mod.rs` (`pub mod edges;`)
- Create: `crates/cairn-node/tests/record_edges.rs`
- Modify: `crates/cairn-node/tests/db_errors_stay_legible.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct RecordEdge { pub low: Uuid, pub high: Uuid, pub attested: bool, pub recorded_on: String }
  pub async fn record_edges<C: GenericClient + Sync>(client: &C, charts: &ChartSet) -> anyhow::Result<Vec<RecordEdge>>
  ```

- [ ] **Step 1: Failing DB tests** (`tests/record_edges.rs`, same scaffolding as Task 2, file-local
  helpers): (a) `a_chain_has_two_links_each_with_its_standing` — register A, B, C here; a HUMAN link
  A–B via `cairn_node::chart_link::link_charts` and a peer's un-attested link B–C via
  `apply_remote_raw`; `record_edges(&c, &person_charts(a))` returns exactly two edges, canonical
  `(low, high)`, ordered by `(low, high)`, the A–B one `attested == true`, the B–C one `false`, each
  `recorded_on` matching `^\d{4}-\d{2}-\d{2}$` (check with `chars().filter(char::is_ascii_digit).count() == 8`
  and `len() == 10`); (b) `an_unlinked_pair_is_not_a_link` — after `unlink_charts(… a, b, None …)`,
  `record_edges` over `{a}` and over `person_charts(b)` returns no A–B edge; (c)
  `a_single_chart_has_no_links` — returns empty. Run → FAIL (module missing).

- [ ] **Step 2: Implement.**

  ```rust
  //! The links that join a record's charts (repair path R2b-2, ADR-0076 decision 4).
  //!
  //! Paper counterpart: the paper clips holding two folders together — one per pair the clerk
  //! clipped. A combined record is joined by LINKS, not by members: in A–C–B the wrong clip may
  //! be A–C or C–B, and only a human can say which (principle 2). So the window lists every
  //! standing link, each with its own "Not the same person…", and this is the read behind it.
  //!
  //! Reads `patient_link` (db/018): one row per pair ever asserted, holding the STANDING
  //! assertion. Only `state = 'link'` rows join anything; an `unlink` row is a pair a judgement
  //! keeps apart. Both ends are required to be in the set — in a link component that is true of
  //! every standing link touching it, and the double test keeps a stale set from listing a link
  //! half outside it.
  use crate::db_diagnosis::LocalDbFault;
  use cairn_medication_view::ChartSet;
  use tokio_postgres::GenericClient;
  use uuid::Uuid;

  /// One standing link between two charts of a record.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct RecordEdge {
      /// The pair, canonical (`low < high`) as db/018 stores it.
      pub low: Uuid,
      pub high: Uuid,
      /// Whether the standing assertion is a human's vouched judgement (db/018's ONE definition,
      /// stored as `patient_link.attested`) — never re-derived here.
      pub attested: bool,
      /// The day the standing assertion was recorded (its HLC wall clock, UTC, `YYYY-MM-DD`).
      pub recorded_on: String,
  }

  const EDGES_SQL: &str = "SELECT low::text AS low, high::text AS high, attested, \
       to_char(to_timestamp(hlc_wall / 1000.0) AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS recorded_on \
       FROM patient_link \
       WHERE state = 'link' AND low = ANY($1::text[]::uuid[]) AND high = ANY($1::text[]::uuid[]) \
       ORDER BY low, high";

  /// Every standing link whose two charts are both in `charts`, ordered by pair.
  pub async fn record_edges<C: GenericClient + Sync>(
      client: &C,
      charts: &ChartSet,
  ) -> anyhow::Result<Vec<RecordEdge>> {
      let ids: Vec<String> = charts.members().iter().map(Uuid::to_string).collect();
      let rows = client
          .query(EDGES_SQL, &[&ids])
          .await
          .map_err(|e| LocalDbFault::new("reading the links that join this record's charts", e))?;
      rows.iter()
          .map(|r| {
              Ok(RecordEdge {
                  low: r.get::<_, String>("low").parse()?,
                  high: r.get::<_, String>("high").parse()?,
                  attested: r.get("attested"),
                  recorded_on: r.get("recorded_on"),
              })
          })
          .collect()
  }
  ```

  Confirm `hlc_wall` is milliseconds before relying on `/ 1000.0` (`crates/cairn-node/tests/chart_link.rs`'s
  `now_ms()` feeds it; `cairn_event::Hlc::wall` doc). In `db_errors_stay_legible.rs` add
  `"crates/cairn-node/src/patient/edges.rs"` to `GUARDED` (alphabetical) and, mirroring
  `COMPARE_LOCAL_DB_FAULT_SITES` and its test, an `EDGES_LOCAL_DB_FAULT_SITES: usize = 1` pin with its
  own test `every_postgres_call_in_the_link_read_names_what_it_was_doing`.

- [ ] **Step 3: Run** `--test record_edges` and `--test db_errors_stay_legible` → PASS.

- [ ] **Step 4: Commit.** `git commit -m "feat(R2b-2): record_edges — the standing links inside a record (Refs #681)"`

---

### Task 4: the wording — pure view builders for the links list and the unlink panel

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/link/record_links.rs` (pure half; the async read is Task 5)
- Create: `cairn-gui/cairn-gui-tauri/src/link/unlink_view.rs`, `unlink_view_tests.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/link/view.rs` (`fact_rows`, `heading` → `pub(crate)`; `judgement_error_from`)
- Modify: `cairn-gui/cairn-gui-tauri/src/link/mod.rs` (declare the modules)
- Modify: `cairn-gui/cairn-gui-tauri/src/chart_set.rs` (`MemberLine::name`)

**Interfaces:**
- Consumes: `cairn_node::patient::edges::RecordEdge`, `cairn_node::chart_link::LinkEffect`,
  `cairn_node::patient::compare::{ChartFacts, VetoFinding}`, `view::{finding_line, ColumnView, FactRowView, LinkReportView, refused}`.
- Produces:
  ```rust
  // chart_set.rs
  pub struct MemberLine { pub patient_id: String, pub name: String, pub text: String }
  // link/record_links.rs
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct RecordLinkView { pub low: String, pub high: String, pub text: String }
  pub fn record_link_line(edge: &RecordEdge, members: &[MemberLine]) -> RecordLinkView
  pub fn links_section(edges: Result<Vec<RecordEdge>, String>, members: &[MemberLine]) -> (Vec<RecordLinkView>, Option<String>)
  // link/view.rs
  pub fn judgement_error_from(act: &str, error: DataError) -> ErrorView   // link_error_from = judgement_error_from("link", e)
  // link/unlink_view.rs
  pub const LINK_GONE: &str;
  pub struct UnlinkParts { pub low: Result<Vec<ChartFacts>, String>, pub high: Result<Vec<ChartFacts>, String>, pub findings: Result<Vec<VetoFinding>, String> }
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct UnlinkComparisonView { pub findings: Vec<String>, pub columns: Vec<ColumnView>, pub rows: Vec<FactRowView>, pub charts: Vec<String>, pub low: String, pub high: String, pub problems: Vec<String>, pub can_unlink: bool }
  pub fn unlink_comparison_view(parts: UnlinkParts, charts: &ChartSet, low: Uuid, high: Uuid) -> UnlinkComparisonView
  pub fn unlink_report(effect: LinkEffect, low: Uuid, high: Uuid, before: &ChartSet, after: &ChartSet) -> LinkReportView
  pub fn unlink_error_view(e: &anyhow::Error) -> ErrorView
  ```

- [ ] **Step 1: `MemberLine::name`.** Add `pub name: String` (doc: "The display name as the line
  shows it — the name, or its worded absence — so a link line can name both charts without a second
  read"). In `member_line`, set `name: name.to_string()`. Update the one struct literal in
  `commands.rs`'s `the_webview_reads_no_field_the_backend_does_not_send` (`name: String::new()`).

- [ ] **Step 2: Failing tests.** In `record_links.rs`'s `#[cfg(test)] mod tests`:

  ```rust
      fn edge(attested: bool) -> RecordEdge {
          RecordEdge { low: Uuid::from_u128(1), high: Uuid::from_u128(2), attested, recorded_on: "2026-09-28".into() }
      }
      fn member(n: u128, name: &str) -> MemberLine {
          MemberLine { patient_id: Uuid::from_u128(n).to_string(), name: name.into(), text: String::new() }
      }

      #[test]
      fn a_link_line_names_both_charts_how_it_was_made_and_when() {
          let members = [member(1, "SMITH John"), member(2, "SMYTHE John")];
          let human = record_link_line(&edge(true), &members);
          assert!(human.text.contains("SMITH John") && human.text.contains("SMYTHE John"));
          assert!(human.text.contains("by a clinician's judgement"));
          assert!(human.text.contains("2026-09-28"));
          assert_eq!(human.low, Uuid::from_u128(1).to_string());
          let machine = record_link_line(&edge(false), &members);
          assert!(machine.text.contains("without a clinician's confirmation on record here"));
          assert!(!machine.text.contains("matcher"), "un-attested is not proof of the matcher");
      }

      #[test]
      fn a_chart_missing_from_the_members_is_named_by_its_id_alone() {
          let line = record_link_line(&edge(true), &[]);
          assert!(line.text.contains(&Uuid::from_u128(1).to_string()));
      }

      /// Review Focus 5.
      #[test]
      fn an_unread_link_list_says_so_and_offers_no_unlink() {
          let (links, error) = links_section(Err("connection reset".into()), &[]);
          assert!(links.is_empty());
          let error = error.expect("an unread list is said, never shown empty");
          assert!(error.contains("could not be read"));
          assert!(error.contains("reload"));
      }

      #[test]
      fn a_read_list_carries_no_error() {
          let (links, error) = links_section(Ok(vec![edge(true)]), &[]);
          assert_eq!(links.len(), 1);
          assert!(error.is_none());
      }
  ```

  In `unlink_view_tests.rs` (sibling file, `#[path]`-included from `unlink_view.rs` exactly like
  `view.rs` includes `view_tests.rs`): reuse a `held(n)` fixture copied from `view_tests.rs:10-29`, and:

  ```rust
  fn set(v: &[u128]) -> ChartSet { ChartSet::new(v.iter().map(|n| Uuid::from_u128(*n))).unwrap() }
  fn parts() -> UnlinkParts { UnlinkParts { low: Ok(vec![held(1)]), high: Ok(vec![held(2)]), findings: Ok(vec![]) } }

  #[test]
  fn an_unlink_comparison_has_the_two_charts_and_sends_back_the_record() {
      let v = unlink_comparison_view(parts(), &set(&[1, 2, 3]), id(1), id(2));
      assert_eq!(v.columns.len(), 2);
      assert_eq!(v.charts.len(), 3, "the record compared FROM, sent back with the unlink");
      assert_eq!((v.low.clone(), v.high.clone()), (id(1).to_string(), id(2).to_string()));
      assert!(v.can_unlink);
  }

  #[test]
  fn a_partial_unlink_comparison_names_what_is_missing_and_cannot_unlink() {
      let mut p = parts();
      p.findings = Err("timeout".into());
      let v = unlink_comparison_view(p, &set(&[1, 2]), id(1), id(2));
      assert!(!v.can_unlink);
      assert!(v.problems.iter().any(|m| m.contains("could not be run")));
  }

  #[test]
  fn a_split_names_the_charts_that_left_and_reloads() {
      let r = unlink_report(LinkEffect::TookEffect, id(2), id(3), &set(&[1, 2, 3]), &set(&[1, 2]));
      assert!(r.sentence.starts_with("Unlinked"));
      assert!(r.sentence.contains(&id(3).to_string()));
      assert!(r.reload);
  }

  /// Review Focus 2.
  #[test]
  fn still_joined_never_reads_as_done_and_points_at_the_links_list() {
      let r = unlink_report(LinkEffect::StillJoined, id(2), id(3), &set(&[1, 2, 3]), &set(&[1, 2, 3]));
      assert!(!r.sentence.starts_with("Unlinked"));
      assert!(r.sentence.contains("still"));
      assert!(r.sentence.contains("How these charts are linked"));
      assert!(r.reload, "the list must re-read: this link is gone from it, the others remain");
  }

  #[test]
  fn outranked_is_a_disagreement_to_settle_not_retry() {
      let r = unlink_report(LinkEffect::Outranked, id(2), id(3), &set(&[1, 2, 3]), &set(&[1, 2, 3]));
      assert!(r.sentence.contains("NOT in effect"));
      assert!(r.sentence.contains("the same person"));
      assert!(!r.reload);
  }

  #[test]
  fn an_unlink_error_is_worded_as_an_unlink() {
      let v = crate::link::view::judgement_error_from(
          "unlink",
          cairn_gui_data::port::DataError::Refused("x".into()),
      );
      assert!(v.text.starts_with("The unlink was refused"));
  }
  ```

  (`id(n)` = `Uuid::from_u128(n)`.) Run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri link::` → FAIL (missing items).

- [ ] **Step 3: Implement `record_links.rs` (pure half).**

  ```rust
  //! The pane's "How these charts are linked" list (repair path R2b-2): one line per standing
  //! link, each with its own "Not the same person…" in the webview. Per LINK, not per member
  //! chart (maintainer, 2026-09-30): each link appears once, so each control does too, and an
  //! unlink that leaves two charts joined through another link can point at one list.
  use crate::chart_set::MemberLine;
  use cairn_node::patient::edges::RecordEdge;
  use serde::Serialize;
  use uuid::Uuid;

  /// One link as the pane lists it. `low`/`high` travel back with "Not the same person…".
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct RecordLinkView {
      pub low: String,
      pub high: String,
      pub text: String,
  }

  /// A chart as a link line names it: its member line's name and its id, or the id alone when
  /// the member lines could not be read (the ids still tie the line to the rows' source labels).
  fn chart_label(chart: Uuid, members: &[MemberLine]) -> String {
      let id = chart.to_string();
      match members.iter().find(|m| m.patient_id == id) {
          Some(m) => format!("{} (chart {id})", m.name),
          None => format!("chart {id}"),
      }
  }

  /// One link's line. "Attested" is db/018's stored definition; its absence is worded as what
  /// is known — no clinician's confirmation is on record HERE — never as "the matcher", which a
  /// peer's human link with an attester this node has not enrolled would make untrue (principle 4).
  pub fn record_link_line(edge: &RecordEdge, members: &[MemberLine]) -> RecordLinkView {
      let how = if edge.attested {
          "linked by a clinician's judgement"
      } else {
          "linked without a clinician's confirmation on record here"
      };
      RecordLinkView {
          low: edge.low.to_string(),
          high: edge.high.to_string(),
          text: format!(
              "{} and {} — {how}, recorded {}",
              chart_label(edge.low, members),
              chart_label(edge.high, members),
              edge.recorded_on
          ),
      }
  }

  /// The list, or — when it could not be read — no lines and a sentence saying so. An unread
  /// list must never render as an empty one: that reads as "nothing joins these charts".
  pub fn links_section(
      edges: Result<Vec<RecordEdge>, String>,
      members: &[MemberLine],
  ) -> (Vec<RecordLinkView>, Option<String>) {
      match edges {
          Ok(edges) => (edges.iter().map(|e| record_link_line(e, members)).collect(), None),
          Err(e) => (
              vec![],
              Some(format!(
                  "The links joining these charts could not be read, so none can be undone from \
                   here until the chart is reloaded: {e}"
              )),
          ),
      }
  }
  ```

- [ ] **Step 4: Implement `unlink_view.rs`.** Make `view.rs`'s `fact_rows` and `heading`
  `pub(crate)`; add to `view.rs`:

  ```rust
  /// A failed judgement worded for its act ("link" / "unlink"), by the classification the funnel
  /// uses — see [`link_error_from`], which is this with `"link"`.
  pub fn judgement_error_from(act: &str, error: DataError) -> ErrorView { … }
  ```

  moving `link_error_from`'s body into it with `act` interpolated (`"The {act} was refused: {t}"`,
  `"This node cannot record the {act} yet: {t}"`, `"The {act} was not confirmed: {t}"`,
  `"The {act} was not recorded."`), and `link_error_from` becomes `judgement_error_from("link", error)`
  (its existing tests stay green unchanged). Then `unlink_view.rs`:

  ```rust
  //! Every sentence the "Not the same person" panel shows, as pure functions (R2b-2). The same
  //! rule as `view.rs`: on this panel the wording IS the safety content.
  use super::view::{fact_rows, finding_line, heading, judgement_error_from, ColumnView, FactRowView, LinkReportView};
  use crate::funnel::view::ErrorView;
  use cairn_medication_view::ChartSet;
  use cairn_node::chart_link::LinkEffect;
  use cairn_node::patient::compare::{ChartFacts, VetoFinding};
  use serde::Serialize;
  use uuid::Uuid;

  /// Refused because the link is no longer one of the record's standing links — a peer's unlink
  /// landed, or this clinician already undid it — so there is nothing left to judge.
  pub const LINK_GONE: &str =
      "that link is no longer part of this record — nothing was done; reload the chart";
  ```

  `UnlinkParts` / `UnlinkComparisonView` as in Interfaces (doc each field; `charts` = "the record's
  chart set this comparison was made FROM — sent back with the unlink, which refuses a changed one").
  `unlink_comparison_view` follows `comparison_view`'s availability rule: each `Err` pushes a problem
  (`"Chart {low}'s identity facts could not be read: {e}"`, same for high, `"The check for disagreeing
  facts could not be run: {e}"`), `can_unlink = problems.is_empty()`, columns = low's then high's facts
  with `heading`, rows = `fact_rows(&both)`, findings = `finding_line` each. `unlink_report`:

  ```rust
  /// What the unlink did, as the outcome line says it. `before` is the record compared from,
  /// `after` the record now (read in the judgement's own transaction). Never "Unlinked" for an
  /// unlink that did not split the record (R2a: recorded is not took effect).
  pub fn unlink_report(effect: LinkEffect, low: Uuid, high: Uuid, before: &ChartSet, after: &ChartSet) -> LinkReportView {
      match effect {
          LinkEffect::TookEffect => {
              let left: Vec<String> = before.members().iter().filter(|c| !after.contains(c)).map(Uuid::to_string).collect();
              let sentence = if left.is_empty() {
                  "Unlinked — the two charts are recorded as different people.".to_string()
              } else {
                  format!("Unlinked — chart(s) {} no longer part of this record.", left.join(", "))
              };
              LinkReportView { sentence, reload: true }
          }
          LinkEffect::StillJoined => LinkReportView {
              sentence: format!(
                  "Recorded that charts {low} and {high} are different people — but they still read \
                   as one record through another link, so this record did not change. The links \
                   still joining them are listed under \"How these charts are linked\"."
              ),
              reload: true,
          },
          LinkEffect::Outranked => LinkReportView {
              sentence: "Recorded, but NOT in effect: a later judgement on this pair says these \
                         are the same person. The two judgements disagree — settle it with the \
                         person who made the other one; unlinking again records another judgement \
                         but changes nothing."
                  .into(),
              reload: false,
          },
      }
  }

  /// A failed unlink, classified exactly as a failed link is.
  pub fn unlink_error_view(e: &anyhow::Error) -> ErrorView {
      judgement_error_from("unlink", cairn_gui_live::error::data_error_from(e))
  }
  ```

  In `link/mod.rs`: `pub mod record_links; pub mod unlink_view;` (Task 5 adds `pub mod unlink;`).

- [ ] **Step 5: Run** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri` → PASS;
  `cargo clippy -p cairn-gui-tauri --all-targets --locked -- -D warnings` → clean.

- [ ] **Step 6: Commit.** `git commit -m "feat(R2b-2): the links list and the unlink panel's wording as pure view builders"`

---

### Task 5: the pane's links, and the window commands `compare_linked` / `unlink_records`

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/link/record_links.rs` (async `read_record_edges`)
- Create: `cairn-gui/cairn-gui-tauri/src/link/unlink.rs`
- Modify: `link/mod.rs` (`pub mod unlink;`, `chart_set_of` → `pub(crate)`), `chart_set.rs`
  (`ChartPane::{links, links_error}`, `chart_pane`'s third argument), `commands.rs` (`med_list_impl`,
  the webview-fields guard), `main.rs` (handlers)

**Interfaces:**
- Consumes: Task 2's `unlink_charts(…, Some(opened), …)` and `LinkOutcome::{effect, charts}`;
  Task 3's `record_edges`; Task 4's views.
- Produces: Tauri commands `link::unlink::compare_linked(patient_id, charts, low, high) -> UnlinkComparisonView`,
  `link::unlink::unlink_records(patient_id, charts, low, high) -> LinkReportView`; `ChartPane { list, members, members_error, links, links_error }`.

- [ ] **Step 1: Failing tests** in `unlink.rs`'s test module, fixture-mode like `link/mod.rs`'s
  tests (copy `fixture()`, `on_screen()`; fixture charts are never linked, so the fixture record has
  no links — which is exactly what the "gone" test needs):

  ```rust
      #[tokio::test]
      async fn compare_linked_is_bound_to_the_chart_on_screen() {
          let state = AppState::mock(Some(fixture()));
          let err = compare_linked_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "1", "2")
              .await
              .unwrap_err();
          assert!(err.text.contains("not the chart"), "must refuse a chart that is not on screen");
      }

      #[tokio::test]
      async fn unlink_is_bound_to_the_chart_on_screen() { /* same shape, unlink_impl */ }

      #[tokio::test]
      async fn unlink_refuses_a_changed_record() {
          let state = AppState::mock(Some(fixture()));
          let (p, _) = on_screen();
          let err = unlink_impl(&state, &p, vec![p.clone(), Uuid::from_u128(7).to_string()],
              &Uuid::from_u128(1).to_string(), &Uuid::from_u128(2).to_string())
              .await
              .unwrap_err();
          assert_eq!(err.text, super::view::THIS_CHANGED);
      }

      /// Review Focus 3.
      #[tokio::test]
      async fn unlink_refuses_a_link_the_record_no_longer_has() {
          let state = AppState::mock(Some(fixture()));
          let (p, charts) = on_screen();
          let err = unlink_impl(&state, &p, charts, &p, &Uuid::from_u128(2).to_string())
              .await
              .unwrap_err();
          assert_eq!(err.text, super::unlink_view::LINK_GONE);
      }

      #[tokio::test]
      async fn compare_linked_refuses_a_link_the_record_no_longer_has() { /* same, compare_linked_impl, LINK_GONE */ }

      #[tokio::test]
      async fn an_unparseable_link_is_refused_not_guessed() {
          let state = AppState::mock(Some(fixture()));
          let (p, charts) = on_screen();
          let err = unlink_impl(&state, &p, charts, "not-a-uuid", "2").await.unwrap_err();
          assert_eq!(err.text, super::unlink_view::LINK_GONE);
      }
  ```

  And in `chart_set.rs`'s tests, extend the existing pane-availability test so `chart_pane(&list,
  Ok(vec![]), Err("x".into()))` still yields the list and `links_error.is_some()`. In `commands.rs`'s
  `the_webview_reads_no_field_the_backend_does_not_send`: build the pane with
  `links: vec![RecordLinkView { low: String::new(), high: String::new(), text: String::new() }], links_error: None`,
  and add `("recordLink", serialized_keys(&serde_json::to_value(&that_link).unwrap()))` to the table.

- [ ] **Step 2: Run to verify they fail.**

- [ ] **Step 3: The pane.** `ChartPane` gains

  ```rust
      /// The standing links joining the record's charts, one line each (R2b-2) — empty for a
      /// chart linked to nothing.
      pub links: Vec<RecordLinkView>,
      /// Set when those links could not be read; the list and the member lines still show.
      pub links_error: Option<String>,
  ```

  `chart_pane(list, members, edges: Result<Vec<RecordEdge>, String>)` builds member lines first, then
  `let (links, links_error) = crate::link::record_links::links_section(edges, &members);`. In
  `record_links.rs` add:

  ```rust
  /// Read the standing links of a record. None for a single chart (nothing joins it); an error,
  /// never an empty list, when a linked set cannot be read — see [`links_section`].
  pub async fn read_record_edges(state: &AppState, charts: &ChartSet) -> Result<Vec<RecordEdge>, String> {
      if !charts.is_linked() {
          return Ok(vec![]);
      }
      let Some(db) = state.db.as_ref() else {
          return Err("there is no database to read the links from".into());
      };
      let db = db.lock().await;
      cairn_node::patient::edges::record_edges(&*db, charts)
          .await
          .map_err(|e| cairn_node::db_diagnosis::operator_chain(&e))
  }
  ```

  `med_list_impl` reads `let edges = read_record_edges(state, &list.charts).await;` after the members
  (NOT `?` — availability) and calls `chart_pane(&list, members, edges)`. Update every other
  `chart_pane(` call (tests) with `Ok(vec![])`.

- [ ] **Step 4: The commands.** `unlink.rs`:

  ```rust
  //! "Not the same person" — undo ONE link of a combined record from the window (repair path
  //! R2b-2, ADR-0076 decision 4; #699 (a)).
  //!
  //! Paper counterpart: unclip the two folders and annotate "not the same person". Two acts:
  //! "Not the same person…" on the link's line (reads the two charts side by side), then
  //! "Unlink — not the same person". The Unlink click IS the signature under the unlocked key
  //! (ADR-0053); there is no confirmation dialog (principle 3).
  //!
  //! Both commands apply the chart-command rules IN THIS ORDER, each pinned by a test: the chart
  //! on screen (`displayed_patient`), the displayed set (`check_displayed_set`), the link is still
  //! one of that set's standing links (`record_edges`). Only then fixture mode, then the key.
  //! The link is named by its two charts; the node is told which chart it is judged FROM, so an
  //! unlink of a link whose charts are not held here is filed under the open chart (#699 (a)).
  ```

  `resolve_edge(state, act: Act, patient_id, charts, low, high) -> Result<(Uuid, ChartSet, Uuid, Uuid), ErrorView>`
  — `Act` is a local `enum { Compare, Unlink }` (a changed set reads `THIS_CHANGED` for Unlink, the
  list's own wording for Compare — the R2b-1 rule): `displayed_patient` → `check_displayed_set(&chart_set_of(state, patient).await?, charts)` →
  parse `low`/`high` (either failing → `refused(LINK_GONE)`), canonicalise with
  `cairn_node::chart_link::canonical_pair` → read `read_record_edges(state, &set)` (an `Err` →
  `ErrorView { text: format!("Could not read the links joining this record's charts — nothing was done: {e}"), retry: Retry::Now }`)
  → `edges.iter().any(|e| (e.low, e.high) == (low, high))` else `refused(LINK_GONE)`.

  `compare_linked_impl(state, patient_id, charts: Vec<String>, low: &str, high: &str) -> Result<UnlinkComparisonView, ErrorView>`:
  after `resolve_edge(Act::Compare, …)`, in live mode build `UnlinkParts` from
  `chart_facts(&*db, &ChartSet::single(low))`, the same for `high`, and
  `cross_vetoes(&*db, &ChartSet::single(low), &ChartSet::single(high))`, each through `as_text`
  (make `link/mod.rs`'s `as_text` `pub(crate)`); fixture mode is unreachable past `resolve_edge`
  (no links) — return `refused(LINK_GONE)` defensively if `state.db` is `None`. Return
  `unlink_comparison_view(parts, &set, low, high)`.

  `unlink_impl(state, patient_id, charts, low, high) -> Result<LinkReportView, ErrorView>`:
  `resolve_edge(Act::Unlink, …)` → `if state.is_mock() { refused("fixture mode: …cannot write") }` →
  `state.live_key(Now::read()).await.ok_or_else(key_locked)?` → lock the db →
  `cairn_node::chart_link::unlink_charts(&mut db, low, high, Some(patient), &reviewer, &state.node_origin)`
  mapped with `unlink_error_view` → `Ok(unlink_report(outcome.effect, low, high, &set, &outcome.charts))`.
  No gesture-timing row (db/044's CHECK; the runbook measures it). Forwarders `compare_linked` and
  `unlink_records` (camelCase JS keys → snake_case, as `link/mod.rs`'s). Register both in `main.rs`'s
  `generate_handler!` after `link::link_records`.

- [ ] **Step 5: Run** the GUI gate (`CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace`, clippy `--locked`, `cargo doc` with `-D warnings`) → PASS.

- [ ] **Step 6: Commit.** `git commit -m "feat(R2b-2): the pane lists its links; compare_linked and unlink_records (Refs #699)"`

---

### Task 6: the webview — the links list and the unlink panel

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/index.html`, `main.js`, `style.css`
- Create: `cairn-gui/cairn-gui-tauri/src-ui/unlink.js` (loaded after `link.js`)

- [ ] **Step 1: Markup.** After `#linked-charts-error` in the header:

  ```html
          <!-- The links that join this record's charts (R2b-2): one line per link, each with its
               own "Not the same person…". Per LINK, never per chart — in A–C–B only a human can say
               which clip is wrong (principle 2). Hidden for a chart linked to nothing. -->
          <p id="record-links-label" hidden>How these charts are linked:</p>
          <ul id="record-links" aria-labelledby="record-links-label" hidden></ul>
          <p id="record-links-error" role="alert" hidden></p>
  ```

  After `#link-panel`, a SEPARATE section (Decision 4):

  ```html
        <!-- "Not the same person" (R2b-2). Its own section, never a mode of the link panel: one
             panel with two verbs could show one verb's button over the other's comparison. -->
        <section id="unlink-panel" aria-labelledby="unlink-heading" hidden>
          <h2 id="unlink-heading" tabindex="-1">Are these two charts the same person?</h2>
          <p id="unlink-problems" role="alert" hidden></p>
          <ul id="unlink-findings" role="alert" aria-label="Facts that disagree between the two charts" hidden></ul>
          <table id="unlink-table" hidden>
            <caption>The two linked charts, side by side</caption>
            <thead></thead>
            <tbody></tbody>
          </table>
          <button id="unlink-confirm" type="button" hidden>Unlink — not the same person</button>
          <p id="unlink-status" role="status" aria-live="polite" hidden></p>
          <button id="unlink-close" type="button">Close comparison</button>
        </section>
  ```

  and `<script src="unlink.js"></script>` after `link.js`'s script tag.

- [ ] **Step 2: `main.js`.** In `render(pane, patient)`, after `renderMembers`, call
  `renderLinks(pane.links)` and `setMessage(el("record-links-error"), pane.links_error);`. Define in
  `main.js` (so the webview-fields guard scans it — it reads only `main.js`):

  ```js
  /**
   * The links joining this record's charts, one line each, each with its own "Not the same
   * person…" (R2b-2). The button's accessible name carries the link's own text, so a screen
   * reader hears WHICH link it undoes, not ten identical buttons.
   */
  function renderLinks(links) {
    const list = el("record-links");
    list.replaceChildren(
      ...links.map((recordLink) => {
        const li = cell("li", recordLink.text + " ");
        const b = document.createElement("button");
        b.type = "button";
        b.textContent = "Not the same person…";
        b.setAttribute("aria-label", "Not the same person: " + recordLink.text);
        b.addEventListener("click", () => compareLinked(recordLink.low, recordLink.high));
        li.append(b);
        return li;
      }),
    );
    const any = links.length > 0;
    list.hidden = !any;
    el("record-links-label").hidden = !any;
  }
  ```

  In `funnel.js`'s `enterChart`/`closeChart` (wherever `closeLinkPanel(false)` is called), also call
  `closeUnlinkPanel(false)`.

- [ ] **Step 3: `unlink.js`.** Mirror `link.js`'s structure and its rules (read its header first):
  a `compareToken`-style `unlinkToken`; `compared = null` until a WHOLE comparison lands
  (`view.can_unlink`); `compareLinked(low, high)` opens the panel, focuses `#unlink-heading`, invokes
  `compare_linked` with `{ patientId: renderedPatient, charts: renderedCharts, low, high }` and renders
  problems → findings → table (two columns, one `<th scope="col">` per column heading, one row per
  fact row; no colgroup headers); `unlinkCompared()` sends `unlink_records` with the SENT
  `{ patientId, charts: view.charts, low: view.low, high: view.high }`, disables the button for the
  round trip, and places the answer by the same rule as `linkAnswerPlace` (panel / chart / elsewhere):
  `report.reload` → `say(report.sentence)`, close the panel, `await refresh(report.sentence)`;
  otherwise (Outranked) `setMessage(el("unlink-status"), report.sentence)`; a failure with
  `retry === "never" || retry === "after_operator"` takes the button away. Every message through
  `setMessage`. `closeUnlinkPanel(returnFocus)` hides and forgets, returning focus to the list only
  when `returnFocus`. Escape and "Close comparison" close it. Key-lock labelling: extend
  `updateLinkLock` in `link.js` to also relabel `#unlink-confirm` ("Unlink — not the same person
  (unlock your signing key first)").

- [ ] **Step 4: Headless walk** (the webview-mock-walk recipe: copy `src-ui`, stub
  `window.__TAURI__.core.invoke` with Rust-shaped payloads before `main.js`, serve, drive with
  Playwright MCP). Stub `med_list` to return a pane with two members and two `links`;
  `compare_linked` to return a view with one finding; `unlink_records` first with `StillJoined`'s
  sentence and `reload: true`, then `Outranked`'s with `reload: false`. Assert, by
  `getComputedStyle(el).display !== "none"` and no hidden ancestor — NEVER by `textContent` alone:
  the list and its label are visible with two buttons whose accessible names differ; the panel opens
  with focus on its heading; findings precede the table in DOM order; the Outranked sentence is
  VISIBLE in `#unlink-status`; after the StillJoined answer `med_list` was invoked again. Record the
  walk's result in the PR body. No committed rig exists (#332) — the walk is evidence, not a gate.

- [ ] **Step 5: GUI gate** (as Task 5) → PASS. **Commit.** `git commit -m "feat(R2b-2): the links list and the Not-the-same-person panel in the webview"`

---

### Task 7: ADR-0077, spec prose, runbook section 10, docs, full gates

**Files:**
- Create: `docs/spec/decisions/0077-an-unlink-may-be-filed-under-the-record-it-was-judged-from.md`
- Modify: `docs/spec/identity.md`, `docs/spec/index.md` (0.78 → 0.79), `docs/spec/decisions/README.md`, `mkdocs.yml` (nav line)
- Modify: `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `TEMPLATE.md`
- Modify: the design page (as-built note under R2b-2), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: ADR-0077** in the log's own format (read ADR-0076's head for it): *Context* — #699's
  scenario (A held; B, C not; the far link B–C wrong; refused, and the tool's "unlink that link too"
  untrue on this node); *Decision* — (a), maintainer 2026-09-28: an unlink where neither subject is held
  here may be filed under the chart it was judged from when that chart is held and its record reads
  both; a LINK never is; the pair is always the payload's; *Consequences* — the audit's findings (no
  reader keys a link event on its envelope; the event sits in the opened chart's stream and takes its
  sensitivity grade; a receiver lacking that chart counts it as having events); "still joined?" is
  asked of the subjects. Link it from identity.md §5.7's `link`/`unlink` row / §5.2 prose with one
  sentence; bump index.md to **0.79**; add the README index line and the `mkdocs.yml` nav line.

- [ ] **Step 2: Runbook section 10** in section 9's format: *Unlink one link* — live only (fixture
  charts are never linked; say so); start on a linked chart; stopwatch from pressing **Not the same
  person…** on a link to the outcome line; budget **review-and-unlink ≤ 15 s**; VoiceOver: each link's
  button is announced with its own text; the findings are read before the table. `TEMPLATE.md` rows.
  A figure outside budget is a finding to file, never a budget to adjust.

- [ ] **Step 3: As-built note** under the design's R2b-2 bullets: Decisions 1–5 above, and anything
  else the build changed.

- [ ] **Step 4: Full gates, in CI's order, AFTER the last edit** (the Global Constraints list), plus
  `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test paper_parity_plan_section`, the docs build
  (`uv run --with-requirements docs/requirements.txt -- mkdocs build`), and
  `python3 scripts/check_closing_keywords.py` on every commit message. Paste the summary lines into the PR.

- [ ] **Step 5: HANDOVER and ROADMAP**: ⇒ NEXT moves to #697 (b) + #701; R2b-2's durable rules
  (FiledUnder is unlink-only; "still joined?" asks the subjects; per-link list; separate panel);
  prune both toward ~500 lines, never dropping an open issue number.

- [ ] **Step 6: Commit, push, open the PR** (draft if any gate is red), `Refs #681`, `Refs #699`
  (closes #699 only if the maintainer agrees — write "Resolves the decision in #699 (a)" without an
  adjacent closing keyword, and let the maintainer close it).

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk unclips two folders that were wrongly clipped together
  and annotates the front sheet "not the same person" (the annotation is the paper audit trail).
- **Steps:** paper 2 (unclip, annotate) → architecture-forced 1 (the signed, attested unlink; the
  Unlink click IS the signature under the unlocked key, ADR-0053, so authorship adds no act) → UI
  bundling target 2 ("Not the same person…" on the link's line, which lays the two charts side by
  side; then "Unlink — not the same person"). `M ≤ N`. The side-by-side read is the paper clerk's own
  look at the two front sheets before unclipping, not an added confirmation. An unlock, when the key
  is locked, is the existing session act.
- **Time + cognitive load:** review-and-unlink ≤ 15 s. The load is choosing WHICH link is wrong: the
  links list names both charts of each link by name and id, how it was made (a clinician's judgement,
  or without one on record here) and when — so the clinician never has to reconstruct the chain from
  member lines. Findings come first in the panel. A `StillJoined` outcome names the remaining links'
  location instead of leaving the clinician to wonder why the record did not change. Measured by
  runbook section 10 (Task 7) — a human act, owed by this slice's runnable surface.
