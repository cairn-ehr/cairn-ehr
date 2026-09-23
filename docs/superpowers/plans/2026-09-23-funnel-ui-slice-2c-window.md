# Funnel UI slice 2c — the window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put the §5.3/§5.8 search-before-create funnel in front of a person: browse → pick → chart,
and browse → register → step-3 prompt → chart, in the reference Tauri window, in `--mock` and
against a live node.

**Architecture:** The pure rules stay in `cairn-gui-funnel` (a new `session` module pairs the raw
typed name with the token and drops stale out-of-order searches). `cairn-gui-live` gains three
small reads (`require_provisioned`, `standing`, `today`) and shares its connection with the
chart commands. `cairn-gui-tauri` gains a `funnel/` module (pure view-model + sentences, a
mock/live backend enum, the Tauri commands), `--patient` becomes optional, and the frontend
gains a second plain-JS file. One change below the GUI: `search_patients` ranks candidates by
how many `db/046` passes each matched, so the bounded prompt shows the strongest five rather
than the five oldest charts.

**Tech Stack:** Rust (Tauri 2, tokio, tokio-postgres), plain JS (no npm, `withGlobalTauri`),
PostgreSQL 18 + `cairn_pgx`, Python (uv) for the measurement rig.

**Spec:** `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md` (read
*Slicing*, *Architecture*, *Error handling*, and every dated note). Also
`docs/superpowers/plans/2026-09-23-funnel-ui-slice-2c-prerequisites.md` for what #661 settled.

## Global Constraints

- AGPL-3.0; **no new dependency** in any tree (none is needed — check before adding one).
- No npm, no bundler, no `package.json` (`src-ui/` is plain JS served as-is).
- Files under 500 lines where feasible: `commands.rs` (456) and `state.rs` (393) must NOT grow
  past it — new code goes in the new `funnel/` module.
- `today` for a live search comes from the DATABASE (`SELECT current_date::text`), never the
  wall clock; `--mock` has no database and uses the process's UTC date, and says so.
- The step-3 prompt shows at most `PROMPT_CAP` (5) candidates; the attested list is ONLY ever a
  `PromptList` (`bound_for_prompt`).
- Every `take` ends in `settle` (never `map_err(|(e, _)| e)`); `settle_is_the_only_end.rs`
  scans for the bad shape.
- Launch probe matches all four `ActorStanding` arms; never a boolean.
- Nothing provisions an actor: never call `enroll_device_actor` from `cairn-gui` (the
  `enrolment_is_never_a_write_side_effect.rs` scan covers every shipped `.rs`).
- A refusal and an outage render differently: `Unavailable` → retry now; `Refused` → change the
  form; `NotProvisioned` → operator remedy, retry after; a failed search is NEVER an empty list.
- Registration form: **one** free name field + date of birth. No sex field (decision 4's
  negative limb). No identifier field (maintainer decision 2026-09-23 — filed as its own issue).
- No confirmation dialogs (§1.2); outcomes render inline in `role="status"` regions.
- No ADR, no migration, no `SCHEMA_GENERATION` change, no wire change.

## Review Focus

1. **A stale step-3 search landing after an edit.** Clerk types "Jon Smith" → search A in
   flight → corrects to "John Smith" → search B → A completes last. Expected: A is dropped,
   nothing it found is redeemable, the Register button acts on B. (Task 2 test
   `a_search_for_an_older_form_revision_is_dropped`.)
2. **Register clicked with no step-3 search yet** (mononymous patient, unknown DOB). Expected:
   the search runs anyway on whatever is typed (`force`), the prompt shows, and the next click
   registers — never a registration without a search. (Task 5 test
   `a_forced_search_runs_even_when_the_trigger_is_waiting`.)
3. **Opening a chart id the window never displayed.** Expected: refused with a sentence;
   only candidates from a list the backend itself returned can be opened. (Task 5 test
   `only_a_displayed_candidate_can_be_opened`.)
4. **An unprovisioned node.** Expected: the window opens, the chrome carries the
   `enroll-device-actor` sentence (or the new-key sentence for `Retired`), and Register returns
   `NotProvisioned` WITHOUT taking the attestation. (Tasks 3, 4, 5.)
5. **A chart command with no chart open** (front door showing). Expected: `med_list`,
   `sign_off`, `cease` refuse with "no chart is open", never act on a stale patient. (Task 5
   test `chart_commands_refuse_when_no_chart_is_open`.)

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `crates/cairn-node/src/patient/search.rs` | modify | rank candidate ids by passes matched |
| `crates/cairn-node/tests/patient_search_ranking.rs` | create | DB-gated proof of the ranking |
| `cairn-gui/cairn-gui-funnel/src/session.rs` | create | `FormSnapshot`, `FunnelSession` (pure) |
| `cairn-gui/cairn-gui-funnel/src/lib.rs` | modify | export `session` |
| `cairn-gui/cairn-gui-live/src/lib.rs` | modify | shared `Arc<Mutex<Client>>`; `sharing`, `connection` |
| `cairn-gui/cairn-gui-live/src/node.rs` | create | `require_provisioned`, `standing`, `today` |
| `cairn-gui/cairn-gui-live/tests/node_reads.rs` | create | DB-gated tests for `node.rs` |
| `cairn-gui/cairn-gui-tauri/src/funnel/mod.rs` | create | module doc + re-exports |
| `cairn-gui/cairn-gui-tauri/src/funnel/view.rs` | create | pure view models + every sentence |
| `cairn-gui/cairn-gui-tauri/src/funnel/backend.rs` | create | `FunnelBackend` (mock/live dispatch), `utc_today` |
| `cairn-gui/cairn-gui-tauri/src/funnel/commands.rs` | create | the Tauri commands + drift guard |
| `cairn-gui/cairn-gui-tauri/src/state.rs` | modify | `AppState` gains chart/funnel/backend fields |
| `cairn-gui/cairn-gui-tauri/src/commands.rs` | modify | chart commands read the OPEN chart |
| `cairn-gui/cairn-gui-tauri/src/main.rs` | modify | `--patient` optional, launch probe, handlers |
| `cairn-gui/cairn-gui-tauri/src-ui/{index.html,funnel.js,main.js,style.css}` | modify/create | the front door |
| `scripts/measure_prompt_truncation.py` | create | truncation-frequency evidence |
| `cairn-gui/cairn-gui-tauri/results/` | modify/create | runbook section + measured result |

---

### Task 1: `search_patients` ranks by passes matched

**Files:**
- Modify: `crates/cairn-node/src/patient/search.rs` (`read_candidate_ids`, ~line 180–213)
- Test: `crates/cairn-node/src/patient/search.rs` (`#[cfg(test)]` unit tests for the pure fn)
- Create: `crates/cairn-node/tests/patient_search_ranking.rs`

**Interfaces:**
- Produces: `fn rank_by_passes_matched(rows: Vec<(Uuid, i64)>) -> Vec<Uuid>` (private, pure).
  `search_patients`' candidate order becomes: passes matched DESC, then `patient_id` ASC.
  The candidate SET is unchanged.

- [ ] **Step 1: Write the failing pure test** (append to `search.rs`'s test module, creating
  `#[cfg(test)] mod tests { use super::*; … }` if none exists):

```rust
/// The step-3 prompt shows the FIRST five candidates (`bound_for_prompt`), and `db/046` is a
/// disjunction — a registration search for "John Smith 1980-01-01" returns every John, every
/// Smith and everyone born that day. Ordered by id alone (chart-creation order) the five shown
/// are the five OLDEST charts, and the duplicate that matters is withheld. Ranking by how many
/// passes a chart matched puts the chart that shares name AND birth date first.
#[test]
fn a_chart_matching_more_passes_ranks_first_whatever_its_age() {
    let old = Uuid::from_u128(1);
    let newer = Uuid::from_u128(2);
    let newest = Uuid::from_u128(3);
    let ranked = rank_by_passes_matched(vec![(old, 1), (newest, 1), (newer, 2)]);
    assert_eq!(ranked, vec![newer, old, newest]);
}

/// Ties keep the old, stable order — id ascending — so the ranking adds a key and changes
/// nothing a single-pass browse ever showed.
#[test]
fn equal_pass_counts_keep_id_order() {
    let a = Uuid::from_u128(10);
    let b = Uuid::from_u128(20);
    assert_eq!(rank_by_passes_matched(vec![(b, 1), (a, 1)]), vec![a, b]);
}
```

- [ ] **Step 2: Run to verify it fails** —
  `cargo test -p cairn-node --lib patient::search::tests` → FAIL, `rank_by_passes_matched`
  not found.

- [ ] **Step 3: Implement.** Replace the query and sort in `read_candidate_ids`:

```rust
    let rows = client
        .query(
            // One row per (patient, pass) comes back from db/046 — its UNION dedups on
            // exactly that pair — so counting rows per patient counts the DISTINCT passes it
            // matched (1..=3). `DISTINCT` inside the count states that rather than relying on it.
            "SELECT patient_id::text AS patient_id, count(DISTINCT matched_pass) AS passes \
             FROM cairn_search_candidates($1, $2, $3::text::jsonb) \
             GROUP BY patient_id",
            &[&query.name_tokens, &birth_date, &identifiers_json],
        )
        .await?;

    let rows: Vec<(Uuid, i64)> = rows
        .iter()
        .map(|row| {
            let id = row.get::<_, String>("patient_id").parse::<Uuid>()?;
            Ok((id, row.get::<_, i64>("passes")))
        })
        .collect::<Result<_, uuid::Error>>()?;
    Ok(rank_by_passes_matched(rows))
}

/// Order candidates strongest-first: more `db/046` passes matched, then id (UUIDv7, so chart
/// age) for a stable tie-break.
///
/// # Why this exists (slice 2c, 2026-09-23)
///
/// `cairn_search_candidates` is a DISJUNCTION of three passes, so a full-name-plus-DOB search
/// returns everyone sharing ANY name token or the birth date. The funnel's step-3 prompt
/// signs only the first `PROMPT_CAP` of them, and in id order those were the oldest charts —
/// the recently-registered duplicate it exists to catch sat at position hundreds. Pass count
/// is the cheapest honest strength signal: it is already in db/046's output, it reorders
/// without adding or removing a candidate (so the drift invariant "sweep-paired ⊆
/// search-found" is untouched), and it needs no new `Candidate` field.
///
/// Known limit, stated rather than hidden: the name pass counts ONCE however many name tokens
/// matched, so "John Brown" and "John Smith" tie for a "John Smith" query at equal passes.
fn rank_by_passes_matched(mut rows: Vec<(Uuid, i64)>) -> Vec<Uuid> {
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows.into_iter().map(|(id, _)| id).collect()
}
```

  Also rewrite `read_candidate_ids`' doc: "sorted for determinism" becomes "ranked
  strongest-first, see `rank_by_passes_matched`". Check the error-type conversion compiles
  (`anyhow` absorbs `uuid::Error`; adjust the `collect` turbofish if needed).

- [ ] **Step 4: Run unit tests** → PASS.

- [ ] **Step 5: Write the DB-gated test** `crates/cairn-node/tests/patient_search_ranking.rs`:

```rust
//! `search_patients` ranks a chart that matched MORE `db/046` passes above older charts that
//! matched fewer — the order the funnel's bounded step-3 prompt signs (slice 2c).
mod common;

use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_node::db;
use cairn_patient_search::SearchQuery;
use common::{chart_named, cs, setup, submit_signed, EventSpec};

#[tokio::test]
async fn the_chart_sharing_name_and_birth_date_outranks_older_name_only_charts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_name", "patient_registration"]).await;

    // Six OLDER charts sharing only the token "smith" — more than PROMPT_CAP (5), so under
    // id order the real duplicate would be cut from the prompt.
    let mut older = Vec::new();
    for i in 0..6 {
        older.push(chart_named(&c, &sk, &kid, 10 * i, &format!("Smith Other{i}")).await);
    }
    // The duplicate: registered LAST (newest id), sharing the name AND the birth date.
    let dup = chart_named(&c, &sk, &kid, 100, "John Smith").await;
    let dob = "1980-01-01";
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: dup,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body(dob, "day", None, "patient-stated"),
            plaintext_twin: Some(render_dob_twin(dob, "day", "patient-stated")),
            wall: 102,
        },
    )
    .await
    .expect("dob accepted");

    let query = SearchQuery::new("John Smith", Some(dob), &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-09-23")
        .await
        .expect("search succeeds");

    assert_eq!(list.candidates.len(), 7, "the SET is unchanged: {list:?}");
    assert_eq!(
        list.candidates[0].patient_id, dup,
        "the chart matching name AND dob must come first, not the oldest chart: {list:?}"
    );
    let rest: Vec<_> = list.candidates[1..].iter().map(|c| c.patient_id).collect();
    assert_eq!(rest, older, "single-pass ties keep id (chart-age) order");
}
```

  Check `setup`'s truncation list: it must clear `patient_name` (pass 3's source) — pass the
  same extra tables `patient_search.rs` uses if the run shows residue.

- [ ] **Step 6: Run** — `CAIRN_TEST_PG=… cargo test -p cairn-node --test patient_search_ranking`,
  then the three existing search suites (`patient_search`, `patient_search_drift`,
  `patient_search_equivalence`) and `patient_register`. Expected: PASS. If a test pinned id
  order across mixed pass counts, it pinned the defect — update its expectation and say so in
  its comment.

- [ ] **Step 7: Commit** — `feat(search): rank candidates by passes matched, so the prompt shows the strongest five`.

---

### Task 2: `FunnelSession` — the raw name travels with its token, and stale searches drop

**Files:**
- Create: `cairn-gui/cairn-gui-funnel/src/session.rs`
- Modify: `cairn-gui/cairn-gui-funnel/src/lib.rs` (`pub mod session;` + re-exports)

**Interfaces:**
- Consumes: `TokenStore`, `SearchToken`, `AttestedSearch`, `Restored`, `TokenError`,
  `PromptList` (all `cairn_gui_funnel`); `SearchQuery::new`.
- Produces:
  - `pub struct FormSnapshot { pub revision: u64, pub raw_name: String, pub birth_date: String }`
    with `pub fn query(&self) -> SearchQuery`.
  - `pub enum Recorded { Current(SearchToken), Stale }`
  - `pub struct FunnelSession` with `new()`, `edited(&mut self, revision: u64)`,
    `record(&mut self, form: &FormSnapshot, prompt: PromptList) -> Result<Recorded, TokenError>`,
    `take_for_register(&mut self, token: SearchToken) -> Result<(AttestedSearch, String), TokenError>`,
    `settle<T, E>(&mut self, outcome: Result<T, (E, AttestedSearch)>) -> Result<T, (E, Restored)>`.

- [ ] **Step 1: Write the failing tests** (in `session.rs`'s test module; `candidate(n)` built
  exactly as `token.rs`'s test helper builds one):

```rust
fn form(revision: u64, name: &str, dob: &str) -> FormSnapshot {
    FormSnapshot { revision, raw_name: name.to_string(), birth_date: dob.to_string() }
}
fn prompt(n: u128) -> PromptList {
    bound_for_prompt(&CandidateList {
        candidates: vec![candidate(n)],
        incomplete: false,
        incomplete_reason: None,
    })
}

#[test]
fn the_registered_name_is_the_one_the_search_ran_on() {
    let mut s = FunnelSession::new();
    let f = form(1, "  John   Smith ", "1980-01-01");
    let Recorded::Current(t) = s.record(&f, prompt(1)).unwrap() else { panic!("current") };
    let (attested, name) = s.take_for_register(t).unwrap();
    assert_eq!(name, "  John   Smith ", "the RAW typed string, never reassembled");
    assert_eq!(attested.query(), &f.query());
}

#[test]
fn a_search_for_an_older_form_revision_is_dropped() {
    let mut s = FunnelSession::new();
    let Recorded::Current(newer) = s.record(&form(2, "John Smith", "1980-01-01"), prompt(2)).unwrap()
    else { panic!() };
    // The "Jon" search was sent first and lands last.
    assert_eq!(s.record(&form(1, "Jon Smith", "1980-01-01"), prompt(1)).unwrap(), Recorded::Stale);
    let (attested, name) = s.take_for_register(newer).unwrap();
    assert_eq!(name, "John Smith");
    assert_eq!(attested.displayed().candidates[0].patient_id, Uuid::from_u128(2));
}

#[test]
fn a_search_for_the_form_before_an_edit_is_dropped() {
    let mut s = FunnelSession::new();
    s.edited(5);
    assert_eq!(s.record(&form(4, "Jon", "1980"), prompt(1)).unwrap(), Recorded::Stale);
    // The search for the edited form itself (same revision) is current.
    assert!(matches!(s.record(&form(5, "John", "1980"), prompt(1)).unwrap(), Recorded::Current(_)));
}

#[test]
fn an_edit_makes_the_held_search_unredeemable() {
    let mut s = FunnelSession::new();
    let Recorded::Current(t) = s.record(&form(1, "John Smith", "1980"), prompt(1)).unwrap() else { panic!() };
    s.edited(2);
    assert_eq!(s.take_for_register(t).unwrap_err(), TokenError::Absent);
}

#[test]
fn a_failed_registration_keeps_the_name_with_the_search() {
    let mut s = FunnelSession::new();
    let Recorded::Current(t) = s.record(&form(1, "John Smith", "1980"), prompt(1)).unwrap() else { panic!() };
    let (attested, _) = s.take_for_register(t).unwrap();
    let out: Result<(), (&str, Restored)> = s.settle(Err(("db down", attested)));
    assert_eq!(out.unwrap_err().1, Restored::Kept);
    let (_, name) = s.take_for_register(t).expect("a Kept search is redeemable again");
    assert_eq!(name, "John Smith");
}

#[test]
fn a_successful_registration_consumes_the_form() {
    let mut s = FunnelSession::new();
    let Recorded::Current(t) = s.record(&form(1, "John Smith", "1980"), prompt(1)).unwrap() else { panic!() };
    let (_attested, _) = s.take_for_register(t).unwrap();
    // The port consumed the attestation on success, so `Ok` carries no value back.
    let ok: Result<u8, (&str, Restored)> = s.settle(Ok(7));
    assert_eq!(ok.unwrap(), 7);
    assert_eq!(s.take_for_register(t).unwrap_err(), TokenError::Absent);
}

#[test]
fn an_unsearchable_form_records_nothing_and_clears_the_name() {
    let mut s = FunnelSession::new();
    let Recorded::Current(t) = s.record(&form(1, "John Smith", "1980"), prompt(1)).unwrap() else { panic!() };
    assert_eq!(s.record(&form(2, "  ", ""), prompt(1)).unwrap_err(), TokenError::EmptyQuery);
    assert_eq!(s.take_for_register(t).unwrap_err(), TokenError::Absent);
}

#[test]
fn the_query_treats_a_blank_birth_date_as_not_supplied() {
    assert_eq!(form(1, "John", "   ").query().birth_date, None);
    assert_eq!(form(1, "John", " 1980 ").query().birth_date.as_deref(), Some("1980"));
}
```

  (`Recorded` derives `Debug, Clone, Copy, PartialEq, Eq`.)

- [ ] **Step 2: Run to verify it fails** —
  `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-funnel session` → does not compile.

- [ ] **Step 3: Implement** `session.rs` (module doc: WHY — the design's "one typed string feeds
  `SearchQuery::new` and `register` alike" made structural on the Rust side; why revisions come
  from the webview and why trusting them is safe: a wrong revision can only DROP a search or
  accept one that genuinely ran on the stored name, never pair a name with a foreign query):

```rust
pub struct FormSnapshot {
    /// The webview's edit counter at the moment the form was read. Monotonic per window.
    pub revision: u64,
    /// The name field exactly as typed — never trimmed here, never reassembled.
    pub raw_name: String,
    /// The date-of-birth field exactly as typed; blank means "not supplied".
    pub birth_date: String,
}

impl FormSnapshot {
    /// The ONE place a funnel query is built from a form. The search and the record both call
    /// this on the same snapshot, so what was searched and what is attested cannot differ.
    pub fn query(&self) -> SearchQuery {
        let dob = Some(self.birth_date.trim()).filter(|d| !d.is_empty());
        SearchQuery::new(&self.raw_name, dob, &[])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    Current(SearchToken),
    /// A newer form revision was already seen; this search described a form that no longer
    /// exists and was NOT recorded.
    Stale,
}

#[derive(Debug, Default)]
pub struct FunnelSession {
    store: TokenStore,
    /// The raw name the held search ran on, beside the token that names it.
    name_for: Option<(SearchToken, String)>,
    /// Highest revision seen by `edited` or `record`.
    revision: u64,
}

impl FunnelSession {
    pub fn new() -> Self { Self::default() }

    pub fn edited(&mut self, revision: u64) {
        self.revision = self.revision.max(revision);
        // Discard unconditionally, even for an out-of-order (older) edit: discarding is the
        // fail-safe direction — the worst it costs is one re-search.
        self.store.discard();
        self.name_for = None;
    }

    pub fn record(&mut self, form: &FormSnapshot, prompt: PromptList) -> Result<Recorded, TokenError> {
        if form.revision < self.revision {
            return Ok(Recorded::Stale);
        }
        self.revision = form.revision;
        self.name_for = None; // before `?`: a refused record has invalidated the store too
        let token = self.store.record(form.query(), prompt)?;
        self.name_for = Some((token, form.raw_name.clone()));
        Ok(Recorded::Current(token))
    }

    pub fn take_for_register(&mut self, token: SearchToken) -> Result<(AttestedSearch, String), TokenError> {
        let attested = self.store.take(token)?;
        match &self.name_for {
            Some((t, name)) if *t == token => Ok((attested, name.clone())),
            // Unreachable while `record` is the only writer of both halves; if it is ever
            // reached, put the search back and refuse rather than register with no name.
            _ => {
                let _ = self.store.restore(attested);
                Err(TokenError::Absent)
            }
        }
    }

    pub fn settle<T, E>(&mut self, outcome: Result<T, (E, AttestedSearch)>) -> Result<T, (E, Restored)> {
        let settled = self.store.settle(outcome);
        if settled.is_ok() {
            self.name_for = None; // `commit` consumed the form
        }
        settled
    }
}
```

  Export: `pub use session::{FormSnapshot, FunnelSession, Recorded};` and add a `session` line
  to the lib's "The three rules" list (make it four).

- [ ] **Step 4: Run** the crate's tests including `tests/settle_is_the_only_end.rs` (it scans
  source for `map_err(|(`; `session.rs` must not trip it) → PASS.

- [ ] **Step 5: Commit** — `feat(funnel): a session that pairs the typed name with its token and drops stale searches`.

---

### Task 3: `cairn-gui-live` — a shared connection, and three node reads

**Files:**
- Modify: `cairn-gui/cairn-gui-live/src/lib.rs`
- Create: `cairn-gui/cairn-gui-live/src/node.rs`
- Create: `cairn-gui/cairn-gui-live/tests/node_reads.rs`

**Interfaces:**
- Produces:
  - `LiveData::sharing(db: Arc<Mutex<Client>>, node_sk: SigningKey, identity: &Identity) -> Self`
    (`new` delegates to it); `LiveData::connection(&self) -> Arc<Mutex<Client>>`;
    `LiveData::node_kid(&self) -> &str`.
  - `impl LiveData { pub async fn require_provisioned(&self) -> Result<(), DataError>;
    pub async fn standing(&self) -> Result<ActorStanding, DataError>;
    pub async fn today(&self) -> Result<String, DataError>; }`

- [ ] **Step 1: Write the failing DB-gated tests** `tests/node_reads.rs` (copy the file header
  shape and skip discipline from `tests/attestation_through_the_port.rs`; `db_gate_ran.rs`
  already fails closed when the gate did not run):

```rust
mod common;
use cairn_gui_data::port::DataError;
use cairn_gui_live::LiveData;
use cairn_node::actor_enrolment::ActorStanding;

/// A node whose signing key nothing enrolled: the window must be able to SAY so at launch
/// (#654 option 2) and must refuse a registration with the remedy, not db/005's key id (#665).
#[tokio::test]
async fn an_unenrolled_node_key_is_not_provisioned_and_names_the_remedy() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (_enrolled, _kid) = common::setup(&reader).await; // enrols a DIFFERENT key
    let unenrolled = cairn_event::generate_key().unwrap().0;
    let live = LiveData::new(common::connect_for_live(&cs).await, unenrolled, &common::identity("ab"));

    assert_eq!(live.standing().await.unwrap(), ActorStanding::NeverEnrolled);
    let err = live.require_provisioned().await.unwrap_err();
    assert!(
        matches!(&err, DataError::NotProvisioned(t) if t.contains("enroll-device-actor")),
        "got {err:?}"
    );
}

#[tokio::test]
async fn an_enrolled_node_key_is_provisioned() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(common::connect_for_live(&cs).await, sk, &common::identity("ab"));
    assert_eq!(live.standing().await.unwrap(), ActorStanding::Enrolled);
    live.require_provisioned().await.expect("an enrolled key may write");
}

/// `today` is the DATABASE's date — the one `cairn-node`'s CLI uses — never this machine's.
#[tokio::test]
async fn today_is_the_databases_current_date() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let expected: String = reader
        .query_one("SELECT current_date::text", &[]).await.unwrap().get(0);
    let live = LiveData::new(common::connect_for_live(&cs).await, sk, &common::identity("ab"));
    assert_eq!(live.today().await.unwrap(), expected);
}

/// The chart commands and the funnel share ONE connection, so the window answers "which node
/// am I" once (LiveData::new's doc).
#[tokio::test]
async fn the_connection_is_shared_not_copied() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let shared = std::sync::Arc::new(tokio::sync::Mutex::new(common::connect_for_live(&cs).await));
    let live = LiveData::sharing(shared.clone(), sk, &common::identity("ab"));
    assert!(std::sync::Arc::ptr_eq(&shared, &live.connection()));
}
```

  Check `common::setup`/`connect` signatures against `tests/common/mod.rs` (`connect` returns
  `(Client, Client)`: adapt the destructuring — the second value may be the guard-holding
  client; read the helper before writing the call). If `ActorStanding` lacks `Debug`/`PartialEq`
  it has both (checked: `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`).

- [ ] **Step 2: Run** — `CAIRN_TEST_PG=… cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live --test node_reads` → compile FAIL.

- [ ] **Step 3: Implement.** In `lib.rs`: `db: Arc<Mutex<Client>>`; add `sharing`,
  `connection`, `node_kid`; `new(db, sk, id)` = `Self::sharing(Arc::new(Mutex::new(db)), sk, id)`;
  `funnel.rs` keeps `self.db.lock().await` (unchanged via `Arc` deref). Add `pub mod node;`.
  `node.rs`:

```rust
//! Three small reads the window needs from its node, beside the two ports. No clinical logic;
//! each maps its failure through `data_error_from` like the ports do.
use crate::error::data_error_from;
use crate::LiveData;
use cairn_gui_data::port::DataError;
use cairn_node::actor_enrolment::{device_actor_standing, require_device_actor, ActorStanding};

impl LiveData {
    /// Refuse unless this node's key may author — BEFORE a registration takes its attestation
    /// out of the token store. The same `require_device_actor` all fifteen CLI write commands
    /// ask, so the clerk gets the remedy-naming refusal (#665) as `NotProvisioned`
    /// (`RefusalScope::NodeState`), not db/005's key id.
    ///
    /// Called by the window's register command, not inside `PatientRegistration::register`:
    /// the port suites deliberately use an unenrolled signer to reach db/005 INSIDE the
    /// registration transaction, and a pre-check in the port would stop them reaching it.
    pub async fn require_provisioned(&self) -> Result<(), DataError> {
        let db = self.db.lock().await;
        require_device_actor(&db, &self.node_kid).await.map_err(|e| data_error_from(&e))
    }

    /// The launch probe (#654 option 2): where this node's key stands, all four answers.
    pub async fn standing(&self) -> Result<ActorStanding, DataError> {
        let db = self.db.lock().await;
        device_actor_standing(&db, &self.node_kid).await.map_err(|e| data_error_from(&e))
    }

    /// The DATABASE's date, for `PatientSearch::search`'s `today`. Asked per search, not
    /// cached at launch: a window left open past midnight must not age every patient wrong.
    pub async fn today(&self) -> Result<String, DataError> {
        let db = self.db.lock().await;
        let row = db
            .query_one("SELECT current_date::text", &[])
            .await
            .map_err(|e| data_error_from(&anyhow::Error::from(e)))?;
        Ok(row.get(0))
    }
}
```

  (Adjust `data_error_from`'s argument type to what `error.rs` actually takes — read its
  signature first.) Update `LiveData::new`'s ⚠️ doc paragraph: the window now probes at launch
  and pre-checks registration via `require_provisioned`.

- [ ] **Step 4: Run** the new suite and the whole `cairn-gui-live` crate → PASS. Confirm
  `refusal_is_not_an_outage.rs` is unchanged and green.

- [ ] **Step 5: Commit** — `feat(live): a shared connection, the launch probe, and the database's date (#665)`.

---

### Task 4: the window's view models and every sentence, as pure functions

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/funnel/mod.rs`, `src/funnel/view.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (`mod funnel;`)
- Modify: `cairn-gui/cairn-gui-tauri/Cargo.toml` — add path deps `cairn-gui-funnel`,
  `cairn-gui-live`, `cairn-patient-search` (all in-tree; no new external crate)

**Interfaces:**
- Produces (all `#[derive(serde::Serialize)]` view types, in `funnel::view`):
  - `pub enum Retry { Now, AfterOperator, Never }` serialized `snake_case`.
  - `pub struct ErrorView { pub text: String, pub retry: Retry }`
  - `pub fn search_error_view(e: &DataError) -> ErrorView`
  - `pub fn register_error_view(e: &DataError, restored: Restored) -> ErrorView`
  - `pub fn token_error_view(e: TokenError) -> ErrorView`
  - `pub fn standing_sentence(standing: ActorStanding, kid: &str) -> Option<String>`
  - `pub fn waiting_sentence(state: &TriggerState) -> Option<String>`
  - `pub struct CandidateView { pub patient_id: String, pub name: String, pub age: String, pub trust: String }`
    and `pub fn candidate_view(c: &Candidate) -> CandidateView`
  - `pub struct ChartHeaderView { pub patient_id: String, pub name: String, pub born: String, pub trust: String }`
    with `pub fn header_from_candidate(c: &Candidate) -> ChartHeaderView`,
    `pub fn header_from_registration(id: Uuid, form: &FormSnapshot) -> ChartHeaderView`,
    `pub fn header_opened_by_id(id: Uuid) -> ChartHeaderView`.

- [ ] **Step 1: Write the failing tests** in `view.rs`:

```rust
#[test]
fn every_actor_standing_but_enrolled_has_its_own_sentence() {
    assert_eq!(standing_sentence(ActorStanding::Enrolled, "k"), None);
    let never = standing_sentence(ActorStanding::NeverEnrolled, "k").unwrap();
    let retired = standing_sentence(ActorStanding::Retired, "k").unwrap();
    let ambiguous = standing_sentence(ActorStanding::Ambiguous, "k").unwrap();
    assert!(never.contains("enroll-device-actor"), "{never}");
    // Retired must NOT send the operator to enroll-device-actor as the fix (#152): the
    // sentence may mention it only to say it cannot help.
    assert!(retired.contains("new signing key") || retired.contains("new key"), "{retired}");
    assert!(ambiguous.contains("more than one"), "{ambiguous}");
    assert_ne!(never, retired);
    assert_ne!(retired, ambiguous);
}

#[test]
fn a_failed_search_never_reads_as_nothing_found() {
    let v = search_error_view(&DataError::Unavailable("connection closed".into()));
    assert!(v.text.contains("NOT"), "must say this is not a no-match: {}", v.text);
    assert!(v.text.contains("connection closed"));
    assert_eq!(v.retry, Retry::Now);
}

#[test]
fn a_refusal_withholds_the_retry_and_an_outage_offers_it() {
    let r = register_error_view(&DataError::Refused("bad dob".into()), Restored::Kept);
    assert_eq!(r.retry, Retry::Never);
    assert!(r.text.contains("bad dob"));
    let u = register_error_view(&DataError::Unavailable("timeout".into()), Restored::Kept);
    assert_eq!(u.retry, Retry::Now);
    let p = register_error_view(&DataError::NotProvisioned("run x".into()), Restored::Kept);
    assert_eq!(p.retry, Retry::AfterOperator);
    assert!(p.text.contains("run x"));
}

#[test]
fn a_dropped_search_says_to_wait_for_the_new_one_not_to_press_register() {
    let v = register_error_view(&DataError::Unavailable("timeout".into()), Restored::SupersededAndDropped);
    assert_eq!(v.retry, Retry::Never, "there is nothing to retry WITH");
    assert!(v.text.contains("new search"), "{}", v.text);
}

#[test]
fn the_waiting_sentence_names_every_missing_part() {
    let s = waiting_sentence(&trigger_state("John", "")).unwrap();
    assert!(s.contains("1 of 2"), "{s}");
    assert!(s.contains("date of birth"), "{s}");
    assert_eq!(waiting_sentence(&trigger_state("John Smith", "1980")), None);
}

#[test]
fn an_unknown_age_renders_as_absence_not_a_blank() {
    let mut c = sample_candidate();
    c.age = None;
    assert_eq!(candidate_view(&c).age, "age not recorded");
}

#[test]
fn a_registration_header_carries_what_was_typed_and_names_absence() {
    let id = Uuid::from_u128(9);
    let h = header_from_registration(id, &FormSnapshot { revision: 1, raw_name: " ".into(), birth_date: "".into() });
    assert_eq!(h.name, "(no name recorded)");
    assert_eq!(h.born, "date of birth not recorded");
    let h = header_from_registration(id, &FormSnapshot { revision: 1, raw_name: "Mary Poppins".into(), birth_date: "1910".into() });
    assert_eq!(h.name, "Mary Poppins");
    assert_eq!(h.born, "born 1910");
    assert_eq!(h.trust, "unconfirmed");
}

#[test]
fn a_chart_opened_by_id_says_its_name_was_not_read() {
    let h = header_opened_by_id(Uuid::from_u128(9));
    assert!(h.name.contains("not read"), "{}", h.name);
}
```

  (`sample_candidate()` — a helper building a `Candidate` with every field set, age `Some(Age
  { years: 46, basis: "dob".into() })`, trust `Confirmed`.)

- [ ] **Step 2: Run** — `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri funnel::view` → compile FAIL.

- [ ] **Step 3: Implement `view.rs`.** Rules for the wording (each a doc comment on its fn):
  - `standing_sentence`: `Enrolled → None`; the other three reuse the node's OWN sentences —
    `format!("{:#}", not_enrolled_refusal(kid))`, `retired_actor_refusal`,
    `ambiguous_actor_refusal` — so the window and the CLI cannot word one fact two ways. The
    `kid` passed MUST be the one the standing was computed for (#670). Then verify the three
    Step-1 assertions hold against those texts; if the retired text lacks "new signing key",
    assert on the phrase it actually uses for its remedy (read `retired_actor_refusal`).
  - `search_error_view`: `Unavailable(t)` → `"The search FAILED — this is NOT a 'no match'. Do
    not register on the strength of it. ({t}) Try again."`, `Retry::Now`. `Refused(t)` →
    `"The record refused this search as typed: {t}"`, `Never`. `NotProvisioned(t)` →
    `"{t}"`, `AfterOperator`. `NotFound` → `"No such chart."`, `Never`.
  - `register_error_view(e, restored)`: base sentence per variant — `Unavailable`: `"Nothing
    was saved; the node could not be reached ({t})."`; `Refused`: `"The record refused this
    registration as typed, and will refuse it again: {t}. Change what was typed."`;
    `NotProvisioned`: `"This workstation's node may not write yet: {t}. Nothing was saved;
    an operator must act before registering."`; `NotFound`: `"Nothing was saved."`. Then by
    `restored`: `Kept` + `Unavailable` → append `" Press Register again."`, `Retry::Now`;
    `Kept` + `NotProvisioned` → `AfterOperator`; `Kept` + other → `Never`;
    `SupersededAndDropped` (any variant) → append `" The form changed while this was saving —
    wait for the new search before registering."`, `Retry::Never`.
  - `token_error_view(e)` → `ErrorView { text: e.to_string(), retry: Retry::Never }`.
  - `waiting_sentence`: `Ready → None`; `Waiting(parts)` → `"The record will be searched for
    this person automatically once it has: "` + parts joined with `", "`, where `NameTokens {
    have }` → `format!("at least {MIN_NAME_TOKENS} words of the name ({have} of {MIN_NAME_TOKENS} typed)")`
    and `BirthDate` → `"a date of birth"`, then `". Register still searches on whatever is
    typed."` (the trigger is advisory, never a gate).
  - `candidate_view`: id = full uuid string; age = `"{years} y"` or `"age not recorded"`;
    trust = `TrustState::as_str()`.
  - headers: `header_from_candidate` → name, `born` = the age label, trust;
    `header_from_registration` → name = trimmed raw name or `"(no name recorded)"`, `born` =
    `"born {dob}"` or `"date of birth not recorded"`, trust `"unconfirmed"` (a chart nobody has
    confirmed — matching the mock's `TrustState::Unconfirmed`); `header_opened_by_id` → name
    `"(opened by chart id at launch — name not read)"`, born `"not read"`, trust `"not read"`.
  `funnel/mod.rs`: module doc (the front door is a shell state, not a tab; what lives in each
  submodule) + `pub mod view; pub mod backend; pub mod commands;` (add the latter two in Task 5).

- [ ] **Step 4: Run** → PASS. Also `cargo clippy --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri -- -D warnings`
  (dead-code warnings are expected until Task 5 wires callers; add `#[allow(dead_code)]` on the
  module ONLY if clippy blocks, and remove it in Task 5).

- [ ] **Step 5: Commit** — `feat(window): the funnel's sentences and view models, as pure functions`.

---

### Task 5: window state, the mock/live backend, the commands, `--patient` optional

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/funnel/backend.rs`, `src/funnel/commands.rs`
- Modify: `src/state.rs`, `src/commands.rs`, `src/main.rs`

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - `pub enum FunnelBackend { Mock(MockData), Live(LiveData) }` with
    `async fn search(&self, q: &SearchQuery) -> Result<CandidateList, DataError>` (gets `today`
    itself: live → `LiveData::today`, mock → `utc_today(SystemTime::now())`),
    `async fn register(&self, a: AttestedSearch, name: &str) -> Result<Uuid, (DataError, AttestedSearch)>`,
    `async fn require_provisioned(&self) -> Result<(), DataError>` (mock → `Ok(())`).
  - `pub fn utc_today(now: SystemTime) -> String` (pure civil-from-days; ISO `YYYY-MM-DD`).
  - `AppState` new fields: `funnel_backend: FunnelBackend`, `funnel: tokio::sync::Mutex<FunnelSession>`,
    `shown: tokio::sync::Mutex<HashMap<Uuid, Candidate>>`,
    `chart: tokio::sync::Mutex<Option<OpenChart>>` (`OpenChart { patient: Uuid, header: ChartHeaderView }`),
    `provisioning: Option<String>` (the launch probe's sentence). `db` becomes
    `Option<Arc<tokio::sync::Mutex<Client>>>`. `patient: Uuid` is REMOVED.
  - `AppState::open_patient(&self) -> Result<Uuid, String>` (`"no chart is open — find or
    register a patient first"`).
  - Commands: `funnel_status`, `form_edited(revision)`, `browse(form)`,
    `prompt_search(form, force)`, `register(token)`, `open_chart(patient_id)`, `close_chart`.

- [ ] **Step 1: Write failing tests.**
  In `backend.rs`:

```rust
#[test]
fn utc_today_is_the_civil_date() {
    use std::time::{Duration, UNIX_EPOCH};
    assert_eq!(utc_today(UNIX_EPOCH), "1970-01-01");
    // 2000-02-29 is day 11016; a leap day is where civil-from-days goes wrong.
    assert_eq!(utc_today(UNIX_EPOCH + Duration::from_secs(11_016 * 86_400 + 3_600)), "2000-02-29");
    assert_eq!(utc_today(UNIX_EPOCH + Duration::from_secs(20_719 * 86_400)), "2026-09-23");
}

#[tokio::test]
async fn the_mock_backend_is_always_provisioned_and_searches_fixtures() {
    let b = FunnelBackend::Mock(MockData::with_fixtures());
    b.require_provisioned().await.unwrap();
    let list = b.search(&SearchQuery::new("mich", None, &[])).await.unwrap();
    assert!(!list.candidates.is_empty());
}
```

  In `funnel/commands.rs`, the command BODIES are written as plain `async fn …_impl(state:
  &AppState, …)` functions and the `#[tauri::command]` wrappers only forward, so the tests
  drive the bodies against `AppState::mock()` with no Tauri runtime:

```rust
fn f(rev: u64, name: &str, dob: &str) -> FormSnapshot {
    FormSnapshot { revision: rev, raw_name: name.into(), birth_date: dob.into() }
}

#[tokio::test]
async fn browse_then_open_a_displayed_candidate_opens_that_chart() {
    let state = AppState::mock(None);
    let list = browse_impl(&state, f(1, "mich", "")).await.unwrap();
    let id = list.candidates[0].patient_id.clone();
    let header = open_chart_impl(&state, &id).await.unwrap();
    assert_eq!(header.patient_id, id);
    assert_eq!(state.open_patient().await.unwrap().to_string(), id);
}

#[tokio::test]
async fn only_a_displayed_candidate_can_be_opened() {
    let state = AppState::mock(None);
    let never_shown = uuid::Uuid::from_u128(424242).to_string();
    assert!(open_chart_impl(&state, &never_shown).await.is_err());
    assert!(state.open_patient().await.is_err());
}

#[tokio::test]
async fn the_whole_register_walk_opens_the_new_chart_and_the_next_browse_finds_it() {
    let state = AppState::mock(None);
    let p = prompt_search_impl(&state, f(1, "Zebedee Quixote", "1990-05-05"), false).await.unwrap();
    assert!(p.waiting.is_none());
    let token = p.token.expect("a searchable form mints a token");
    let header = register_impl(&state, token).await.unwrap();
    assert_eq!(header.name, "Zebedee Quixote");
    let found = browse_impl(&state, f(2, "quixote", "")).await.unwrap();
    assert!(found.candidates.iter().any(|c| c.patient_id == header.patient_id));
}

#[tokio::test]
async fn a_forced_search_runs_even_when_the_trigger_is_waiting() {
    let state = AppState::mock(None);
    let waiting = prompt_search_impl(&state, f(1, "Cher", ""), false).await.unwrap();
    assert!(waiting.waiting.is_some() && waiting.token.is_none());
    let forced = prompt_search_impl(&state, f(1, "Cher", ""), true).await.unwrap();
    assert!(forced.token.is_some(), "Register must still be reachable for a mononymous patient");
}

#[tokio::test]
async fn a_second_register_with_the_same_token_is_refused() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815-12-10"), false).await.unwrap().token.unwrap();
    register_impl(&state, t).await.unwrap();
    assert!(register_impl(&state, t).await.is_err(), "one search, one chart");
}

#[tokio::test]
async fn an_edit_after_the_prompt_makes_its_token_unredeemable() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Jon Smith", "1980"), false).await.unwrap().token.unwrap();
    form_edited_impl(&state, 2).await;
    assert!(register_impl(&state, t).await.is_err());
}

#[tokio::test]
async fn a_failed_registration_keeps_the_search_and_the_retry_works() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false).await.unwrap().token.unwrap();
    state.mock_data().unwrap().fail_next(DataError::Unavailable("disk full".into()));
    let err = register_impl(&state, t).await.unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    register_impl(&state, t).await.expect("the Kept search registers on retry");
}

#[tokio::test]
async fn chart_commands_refuse_when_no_chart_is_open() {
    let state = AppState::mock(None);
    assert!(state.open_patient().await.is_err());
}

#[tokio::test]
async fn a_patient_given_at_launch_opens_straight_on_the_chart() {
    let id = uuid::Uuid::from_u128(7);
    let state = AppState::mock(Some(id));
    assert_eq!(state.open_patient().await.unwrap(), id);
}
```

  `AppState::mock(patient: Option<Uuid>)` is the ONE mock constructor (both `main.rs` and
  tests use it, so fixture mode can never hold a connection); `AppState::mock_data(&self) ->
  Option<&MockData>` is `#[cfg(test)]` so the shipped binary still has no arming path (#668's
  gating note). Move `state.rs`'s own test helper `state_holding` onto `AppState::mock(None)`.

- [ ] **Step 2: Run** — compile FAIL.

- [ ] **Step 3: Implement.**
  - `backend.rs`: the enum; `search` gets `today` per call (doc: why per call, why the mock's
    date is UTC and says so); `register` passes `Some(name)`; `utc_today` via Howard Hinnant's
    `civil_from_days` (days = secs / 86_400; `z = days + 719_468`; era; doe; yoe; doy; mp; d; m;
    y) — pre-1970 clocks are not a case (a `SystemTime` before the epoch returns `"1970-01-01"`
    rather than panicking, stated in the doc).
  - `state.rs`: new fields; `OpenChart`; `open_patient`; `mock(patient)`; keep `is_mock()`
    as `db.is_none()`; add a unit test that `AppState::mock(None)` has `db.is_none()` AND a
    `FunnelBackend::Mock` (the two can never disagree). If `state.rs` would exceed 500 lines,
    put `OpenChart` + constructors in `src/funnel/window_state.rs` instead.
  - `commands.rs`: replace every `state.patient` with `state.open_patient().await?`; `db` is
    now `Arc<Mutex<_>>` (`.lock().await` unchanged). No other change.
  - `funnel/commands.rs` bodies:
    - `funnel_status_impl` → `FunnelStatus { mock: bool, provisioning: Option<String>, chart: Option<ChartHeaderView> }`.
    - `form_edited_impl(state, revision)` → `state.funnel.lock().await.edited(revision)`.
    - `browse_impl(state, form)` → `BrowseView { revision, candidates: Vec<CandidateView>, incomplete_reason: Option<String> }`;
      on Ok, REPLACE `shown` with this list's candidates (keyed by id); error →
      `search_error_view`. The browse list is NOT bounded (it scrolls; it attests nothing).
    - `prompt_search_impl(state, form, force)` → `PromptView { revision, waiting: Option<String>, stale: bool, token: Option<SearchToken>, candidates, incomplete_reason }`.
      If `!force` and `trigger_state(&form.raw_name, &form.birth_date)` is `Waiting` → return
      the waiting sentence, no search, no token. Else: `query = form.query()`; `list =
      backend.search(&query)`; `prompt = bound_for_prompt(&list)`; ADD the prompt's candidates
      to `shown` (so "yes, it's this one" can open them); `session.record(&form, prompt)` —
      `Stale` → `stale: true, token: None`; `EmptyQuery` → `waiting` =
      `token_error_view(EmptyQuery).text`. Render candidates from `prompt.as_list()` (what is
      shown IS what is attested) — capture the view before `record` consumes the prompt.
    - `register_impl(state, token)` → `Result<ChartHeaderView, ErrorView>`: FIRST
      `backend.require_provisioned()` (error → `register_error_view(e, Restored::Kept)` —
      nothing was taken); then `take_for_register` (error → `token_error_view`); keep the form
      snapshot needed for the header: rebuild from the attestation (`attested.query().birth_date`)
      plus the returned raw name; await `backend.register(attested, &name)`; `session.settle(…)`;
      Ok(id) → `header_from_registration`, set `chart`, clear `shown`; Err((e, restored)) →
      `register_error_view(&e, restored)`. Hold the session mutex only around `take` and
      `settle`, NOT across the await (a background prompt search must still be able to land —
      that is exactly the case `commit`'s invalidate covers).
      ⚠️ Do NOT race this future against a timeout/`select!` (#649/#669): say so in the doc.
    - `open_chart_impl(state, id)` → parse, look up `shown`, else `Err("that chart was not in
      a list on screen — search again")`; set `chart` with `header_from_candidate`.
    - `close_chart_impl` → `chart = None`.
    Each `#[tauri::command]` wrapper takes `tauri::State<'_, AppState>` and forwards.
  - `main.rs`: `patient: Option<uuid::Uuid>` with doc "open straight on this chart (the timing
    runbook and the accessibility pass use it); without it the window opens on the patient
    search"; `--mock` doc → "Run against fixtures with no database. Clinical writes are refused
    in this mode; registering a patient succeeds into an in-memory population that vanishes with
    the window."; module doc: replace the "There is no patient picker" paragraph with the
    funnel's arrival (possession, not a dropdown: the identity header). `build_live_state`:
    one `Arc<Mutex<Client>>` shared by `AppState.db` and `LiveData::sharing`; probe
    `live.standing()` → `provisioning = standing_sentence(s, live.node_kid())`, a probe
    ERROR → `Some(format!("Could not check whether this node may write: {}", …))` — the window
    still opens (reading needs no actor). With `--patient`, `chart =
    Some(OpenChart { patient, header: header_opened_by_id(patient) })`. Register the seven new
    handlers in `generate_handler!`.

- [ ] **Step 4: Run** — `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri`
  (FOREGROUND), then clippy `-D warnings` on the tree → PASS.

- [ ] **Step 5: Commit** — `feat(window): the funnel's commands, a persistent mock, and --patient optional (#668)`.

---

### Task 6: the front door in the webview, and the drift guard

**Files:**
- Modify: `src-ui/index.html`, `src-ui/main.js`, `src-ui/style.css`
- Create: `src-ui/funnel.js`
- Test: `src/funnel/commands.rs` (drift guard), `src/commands.rs` (make the JS scanner reusable)

**Interfaces:**
- Consumes: the seven commands and their payload fields.
- JS bindings the guard scans in `funnel.js`: `status` (FunnelStatus), `header`
  (ChartHeaderView), `cand` (CandidateView), `browseView` (BrowseView), `prompt` (PromptView),
  `failure` (ErrorView).

- [ ] **Step 1: Write the failing drift-guard test** in `funnel/commands.rs`. First refactor
  `commands.rs`'s `fields_read_by_the_webview(binding)` into
  `pub(crate) fn fields_read_in(js: &str, binding: &str) -> BTreeSet<String>` (the old fn calls
  it with `include_str!("../src-ui/main.js")`), then:

```rust
#[test]
fn funnel_js_reads_no_field_the_backend_does_not_send() {
    let js = include_str!("../../src-ui/funnel.js");
    let header = header_opened_by_id(uuid::Uuid::nil());
    let cand = candidate_view(&sample_candidate());
    let payloads: [(&str, serde_json::Value); 6] = [
        ("status", serde_json::to_value(FunnelStatus { mock: true, provisioning: None, chart: Some(header.clone()) }).unwrap()),
        ("header", serde_json::to_value(&header).unwrap()),
        ("cand", serde_json::to_value(&cand).unwrap()),
        ("browseView", serde_json::to_value(BrowseView { revision: 0, candidates: vec![], incomplete_reason: None }).unwrap()),
        ("prompt", serde_json::to_value(PromptView { revision: 0, waiting: None, stale: false, token: None, candidates: vec![], incomplete_reason: None }).unwrap()),
        ("failure", serde_json::to_value(ErrorView { text: String::new(), retry: Retry::Now }).unwrap()),
    ];
    for (binding, value) in payloads {
        let available: BTreeSet<String> = value.as_object().unwrap().keys().cloned().collect();
        for field in crate::commands::fields_read_in(js, binding) {
            assert!(available.contains(&field), "funnel.js reads `{binding}.{field}`, not sent. Available: {available:?}");
        }
    }
}

/// The two fields whose silence is the dangerous failure: a partial list rendering as whole.
#[test]
fn funnel_js_reads_both_incompleteness_reports_and_the_retry_advice() {
    let js = include_str!("../../src-ui/funnel.js");
    assert!(crate::commands::fields_read_in(js, "browseView").contains("incomplete_reason"));
    assert!(crate::commands::fields_read_in(js, "prompt").contains("incomplete_reason"));
    assert!(crate::commands::fields_read_in(js, "failure").contains("retry"));
}
```

  (`sample_candidate` from Task 4's tests: make it `pub(crate)` under `#[cfg(test)]`.)

- [ ] **Step 2: Run** → FAIL (`funnel.js` missing).

- [ ] **Step 3: Implement the markup and JS.**
  - `index.html`: wrap the existing chart content in `<section id="chart-view" hidden>` whose
    first child is the identity header — `<header id="identity" aria-label="Patient on this
    chart">` with `<h1 id="patient-heading">`, `<p id="identity-born">`, `<p id="identity-id">`,
    `<p id="identity-trust">` and a `<button id="close-chart">Find another patient</button>`.
    Add `<section id="front-door" aria-labelledby="find-heading">` BEFORE it:
    `<p id="provisioning" role="status" hidden>`; an `<h1 id="find-heading">Find a patient</h1>`;
    a `<form id="browse-form">` with labelled `browse-name` and `browse-dob` inputs; `<p
    id="browse-status" role="status" aria-live="polite">`; `<ul id="browse-list">` (scrolls);
    a `<h2>Not on file? Register a new patient</h2>` and `<form id="register-form">` with
    labelled `reg-name` (one free name field) and `reg-dob` (hint: "YYYY, YYYY-MM or
    YYYY-MM-DD, or leave blank if unknown") inputs; `<p id="prompt-status" role="status"
    aria-live="polite">`; `<section id="prompt" hidden>` with `<h3>Could this be one of these
    existing patients?</h3>`, `<ol id="prompt-list">` (styled `overflow: visible` — the prompt
    never scrolls), `<p id="prompt-incomplete" hidden>`; `<button id="register" type="submit">`;
    `<p id="register-outcome" role="status" aria-live="polite">`. Load `main.js` then
    `funnel.js`. Keep the DOM-order rule: warnings/partiality ABOVE the list they qualify.
  - `main.js`: remove the unconditional `void refresh();` at the bottom (funnel.js decides);
    keep `pollLock` start. Add `function showIdentity(header)` rendering the header fields.
  - `funnel.js` (plain JS, `"use strict"`, header comment: renders and decides nothing; the
    revision counter; debounce ~250 ms; supersession — a response whose `revision` is not the
    latest sent is ignored; every `input` event on the register form increments `revision`,
    clears the held token SYNCHRONOUSLY, disables `#register`'s "use this search" state and
    calls `form_edited`; the browse form re-browses debounced):
    - `boot()`: `status = await invoke("funnel_status")`; show `status.provisioning`; if
      `status.chart` → `enterChart(status.chart)` else show front door.
    - browse: render each `cand` as an `<li>` with a `<button>` whose accessible name is
      `cand.name + ", " + cand.age + ", " + cand.trust`; click → `invoke("open_chart",
      {patientId: cand.patient_id})` → `enterChart(header)`. Empty list with no
      `incomplete_reason` → "No existing chart matched." A `failure` → `failure.text` (never
      "no match").
    - register form input (debounced) → `invoke("prompt_search", {form, force: false})`;
      render `prompt.waiting` in `#prompt-status`; when `prompt.token` present, hold it and
      render the prompt: each candidate → "This is them — open this chart" button (open_chart);
      `#register` label becomes `prompt.candidates.length ? "None of these — register a new
      patient" : "Register new patient"`; `prompt.incomplete_reason` → `#prompt-incomplete`.
    - `#register` submit: no held token → `invoke("prompt_search", {form, force: true})`, render
      the prompt (the clerk answers it with the next click — never auto-register); held token
      → `invoke("register", {token})` → `enterChart(header)`; `failure` → `#register-outcome`
      = `failure.text`, and `failure.retry === "now"` keeps `#register` enabled, otherwise it
      is disabled until the form changes (`"after_operator"`: also show the text in
      `#provisioning`).
    - `enterChart(header)`: hide front door, show chart view, `showIdentity(header)`,
      `refresh()`. `#close-chart` → `invoke("close_chart")`, reset both forms (increment
      revision, `form_edited`), show front door, focus `#browse-name`.
    The form object sent is `{revision, raw_name, birth_date}` (serde field names of
    `FormSnapshot` — add `#[derive(serde::Deserialize)]` to `FormSnapshot` in Task 2's crate if
    not already, it already depends on serde).
  - `style.css`: `#browse-list { max-height: 40vh; overflow-y: auto }`,
    `#prompt-list { overflow: visible }`, a visible focus ring, `.visually-hidden` reuse.

- [ ] **Step 4: Run** the tauri crate's tests (both drift guards) → PASS. Then launch:
  `cargo run --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri -- --mock` and walk:
  browse `mich` → open → header shows Michaelowski → Find another patient → register
  "Zebedee Quixote" 1990-05-05 → prompt → register → chart opens → browse `quix` finds it.
  Also `-- --mock --patient 00000000-0000-0000-0000-000000000001` opens on the chart. Record
  what was seen in the commit message (a screenshot via the `run` skill if available).

- [ ] **Step 5: Commit** — `feat(window): the front door — browse, register, the bounded prompt`.

---

### Task 7: the evidence — truncation frequency, and the §1.2 measurement

**Files:**
- Create: `scripts/measure_prompt_truncation.py`
- Create: `cairn-gui/cairn-gui-tauri/results/2026-09-23-funnel-prompt-truncation.md`
- Modify: `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (a funnel section), `TEMPLATE.md`

**Interfaces:**
- Consumes: `cairn_search_candidates` (db/046) directly over `psql`, as
  `scripts/measure_patient_search.py` does (read its connection and projection-insert helpers
  and REUSE them by import if they are importable; otherwise copy the minimum and say so).

- [ ] **Step 1: Write the pure functions first, with `--self-test`** (the same pattern
  `measure_patient_search.py` uses): `rank(rows) -> list` (passes desc, id asc — the Python
  twin of Task 1's rule, asserted equal on a fixed example) and `summarise(results) -> dict`
  giving: count of step-3 searches; share with > 5 candidates (truncated); self-match rank
  percentiles under id order vs pass-count order; share where the self-match falls outside
  the top 5 under each order. Run `uv run scripts/measure_prompt_truncation.py --self-test` →
  FAIL then PASS.

- [ ] **Step 2: Implement the rig.** Populate N patients (default 50 000, the §8.1 population)
  straight into `patient_name` AND a dob row in `patient_demographic` (read db/010/db/011 for
  that projection's columns first; the insert must match what db/046's pass 2 reads: `field =
  'dob'`, `value`), using the deterministic synthetic pool (or `--name-pool`). Birth dates:
  uniform over 1930–2025. Then for a sample of 500 patients run the step-3 query exactly as the
  window builds it — their full stored name + dob — via `SELECT patient_id, count(DISTINCT
  matched_pass) … GROUP BY 1`, and record where the patient itself lands under each order.
  Clean up (TRUNCATE the two projections) at the end, as the other rig does.

- [ ] **Step 3: Run it** against a local DB (`scripts/pg-target.sh` for discovery), with and
  without `--name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3` (note memory:
  that copy is not the current generator — say so in the result). Write the result file with the
  exact command, machine, and the table. **The finding to state plainly:** how often the
  prompt truncates (expected: nearly always — the disjunction), and whether ranking puts the
  self-match in the top 5 (expected: nearly always). If truncation is routine even with
  ranking, that is the design's "cap is wrong" condition: file an issue for the design
  revision (the attestation's `incomplete` flag carries no signal when it is always set) and
  record it — never change `PROMPT_CAP` in this slice.

- [ ] **Step 4: Runbook section** in `RUNBOOK.md`: "Funnel (§5.3/§5.8)": seed a live node
  (`init`, `enroll-human` not needed for registration; `patient-register` a handful of
  charts); launch WITHOUT `--patient`; time with a stopwatch (a) find an existing chart:
  from first keystroke to the chart header visible — budget **≤ 5 s**; (b) register a new
  patient: from first keystroke to the new chart open — budget **≤ 20 s**; count human acts
  for each against the plan's N/M/K table; run once in `--mock` and once live. Note that the
  machine half is in the truncation result + #639's search figures, and the stopwatch half is
  a HUMAN act owed (HANDOVER's "human acts" list). Add matching rows to `TEMPLATE.md`.

- [ ] **Step 5: Commit** — `measure(funnel): how often the prompt truncates, before and after ranking`.

---

### Task 8: documents

**Files:** the design page (a dated 2026-09-23 note under *Risks* and *Architecture*: the
disjunction finding, the ranking decision, name+DOB only, `require_provisioned` placement,
header shows age not DOB because `Candidate` carries none), `cairn-gui/cairn-gui-tauri/README.md`
(launch modes), `docs/HANDOVER.md`, `docs/ROADMAP.md`, issues.

- [ ] **Step 1:** File issues: (a) identifier entry in the registration form; (b) the identity
  header cannot show DOB — `Candidate` carries none (sibling of #645); (c) whatever Task 7
  found if the cap is still wrong. Comment on #668 (persistent `MockData` landed; the arming
  affordance + typed slots remain), #665 (the window now pre-checks; the orchestrator question
  stays open), #654 (option 2 built). Run `scripts/check_closing_keywords.py` on every commit
  message and the PR body before pushing.
- [ ] **Step 2:** Update HANDOVER/ROADMAP (condense; HANDOVER is ~1200 lines — prune finished
  narrative into ROADMAP pointers, keep traps).
- [ ] **Step 3:** Commit, push, open the PR.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the registration desk and the alphabetical patient index drawer (the
design page's own counterpart; this slice is the first runnable surface for it).

**Steps:**

| | Register a new patient | Find an existing chart |
|---|---|---|
| Paper acts (N) | 5 — ask details, flip drawer, take blank card, write it, file it | 3 — ask details, flip drawer, pull card |
| Architecture-forced (M) | 4 — type fragment, read list, complete the form, answer the prompt | 2 — type fragment, pick |
| UI bundling target (K) | 4 (3 when the prompt is empty: "Register new patient" is itself the answer) | 2 |

`M ≤ N` for both; no architecture defect. One path costs one more: a mononymous patient or an
unknown DOB never trips the trigger, so the first Register click runs the search and the second
registers (5 acts, still ≤ paper's 5). Stated, not hidden.

**Time + cognitive load:** find ≤ 5 s and register ≤ 20 s end-to-end (the seeded Slice 63
figures), measured by the operator with a stopwatch per the runbook section Task 7 adds — this
slice exposes the surface, so it owes the runbook and the machine-side evidence; the stopwatch
figure is a human act. Cognitive load: the step-3 prompt fires only when it has something to
say and now shows the STRONGEST matches first (Task 1), and the identity header removes the need
to hold the patient's identity in your head.
