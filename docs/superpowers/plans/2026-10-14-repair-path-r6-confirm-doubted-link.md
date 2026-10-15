# Repair path R6 — confirm a doubted link, and name the doubt at the front door (#716, #723) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician reading a linked record sees **which** link is in doubt and why. Each link has
one **"Review this link…"** door, whose panel offers every verdict that applies, so a doubted link
can be **confirmed** in the window, not only from the CLI. The bridge case is listed and can be
re-linked. At the front door, a doubted set no longer reads "One person".

**Architecture:**
- **In-DB (db/054, edited in place).** A per-link doubt read, `cairn_chart_set_doubts`, becomes the
  one spelling. `cairn_chart_set_has_doubted_link` is redefined as `EXISTS` over it, so the
  sign-off hold and the per-link marks cannot disagree.
- **`cairn-node`.**
  - `patient::edges::record_doubts` reads the doubts.
  - `chart_link::RecordsShown` gains `pair`, the standing assertion's content address as the panel
    read it, re-checked under CARNLK.
  - `chart_link::confirm_link_as_shown` is the confirm judgement.
  - The search marks a doubted person row (`PersonRow::is_doubted`) from the same statement that
    reads link components.
- **Window (`cairn-gui-tauri`).**
  - The links list gains doubt marks and `separated` lines, and its button becomes "Review this
    link…".
  - R2b-2's comparison panel becomes the review panel, with `same_label` / `can_confirm` beside
    `can_unlink`.
  - A new `link/confirm.rs` adds the `confirm_link` command.
  - `unlink.js` draws and sends the second verdict.
  - The front door's row label is reworded for a doubted row.

**Tech Stack:** PostgreSQL ≥ 18 + `cairn_pgx`; Rust (tokio-postgres, anyhow, serde, Tauri 2); plain JS
(no npm, no bundler).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`, section
**"R6 — confirm a doubted link, and name the doubt at the front door (#716, #723), designed
2026-10-14"**. Read also R1b's and R2b-2's sections above it, ADR-0076 (D5: attested first, then
HLC) and ADR-0077.

## Global Constraints

- **AGPL-3.0; no new dependency.** No new crate, so no lockfile changes in any of the three Cargo
  trees.
- **TDD.** Every behaviour starts with a test that fails for the right reason. Run the failing test
  and read its failure before writing the code.
- **No wire change and no new event type.** A confirm is an ordinary attested
  `identity.link.asserted`, written through the existing `chart_link` judgement path.
- **No new `db/*.sql` file. `SCHEMA_GENERATION` stays 58.** db/054 is edited in place; ADR-0080's
  stamp detects the changed text and replays it. Editing an `include_str!`'d SQL file relinks every
  `cairn-node` test binary: budget for it.
- **One spelling of "doubted".** After Task 1 every doubt read goes through
  `cairn_chart_set_doubts` (directly, or via `cairn_chart_set_has_doubted_link`). Never write the
  flag/veto/attested-unlink tests a second time, in SQL or in Rust.
- **The doubt never enters the signed list.** `CandidateList::displayed_charts()`,
  `SearchAttestation`, the prompt cap and the ranking are untouched. A row's JSON stays a bare array
  of candidates (`a_list_with_a_linked_row_survives_a_serde_round_trip`).
- **Byte-identical where nothing is doubted.** A linked row with no doubt keeps "One person — N
  linked charts". A link line with no doubt keeps today's text. The CLI's candidate text for an
  undoubted row is unchanged (its goldens).
- **No confirmation dialog** (principle 3). The verdict click IS the signature under the unlocked
  key (ADR-0053). A locked key is not a verdict: the button stays for after the unlock.
- **Every sentence lives in Rust** (`record_links.rs`, `unlink_view.rs`, `confirm_view.rs`,
  `funnel/rows.rs`, `candidate_text.rs`) with tests. `unlink.js` and `main.js` render and decide
  nothing.
- **Every panel message goes through `setMessage`**, never a bare `.textContent`. Both verdict
  buttons are disabled while either is in flight and hidden once a verdict lands (R5a's rule: one
  human must never sign two verdicts milliseconds apart).
- **`tokio::sync::Mutex` is not re-entrant.** Never hold `state.db`'s guard while calling
  `chart_set_of`, `read_record_links` or `read_chart_of` (each takes it itself).
- **Files under 500 lines where feasible.**
  - `link/mod.rs` (512) changes by one line only (Task 2's `pair: None`).
  - New code goes in the new `link/confirm.rs` and `link/confirm_view.rs`.
  - `chart_link.rs` (512) is not touched; Task 2's code goes in `chart_link/shown.rs` and
    `chart_link/judge.rs`.
  - New DB tests go in new files.
- **House rule 6:** no literal key material; never name a non-crypto value `salt`/`nonce`/`iv`. The
  content address carried by `pair`/`standing` is not key material.
- **Commit messages** say `Refs #716` / `Refs #723`, never a closing keyword, until the final PR
  body. Run `python3 scripts/check_closing_keywords.py <msgfile>` before every commit.
  `fix(#716):`-style prefixes are safe.
- **New test helpers stay file-local** (no new `pub fn` in `crates/cairn-node/tests/common/mod.rs`).
- **Subagents run FOREGROUND tests only.** The controller re-runs DB-gated suites with
  `--nocapture` and checks that no `skipped:` line appears (a self-skip still prints "ok").
- **DB env** (`scripts/pg-target.sh` prints the cluster; use its port):
  ```bash
  export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
         CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test2" \
         CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test3"
  ```
  - **Rust:** use `CARGO_TARGET_DIR=/tmp/cairn-r6-target` while an IDE is open (trap 18).
  - A narrow `cargo test --test X` can stall on one freshly linked binary (macOS Gatekeeper). If it
    does, exec the printed `target/debug/deps/X-<hash>` directly.
  - Never pipe a gate through `| tail` (it masks cargo's exit code).
  - The `cairn-gui` tree is its own workspace: `cd cairn-gui && cargo test -p cairn-gui-tauri`.
    Since 2026-09-22 a DB-free run there needs `CAIRN_ALLOW_DB_SKIP=1`.

## Review Focus

1. **A colleague's attested unlink of the same pair lands in the last milliseconds while a third
   chart still joins the record.** The set is unchanged, so `RecordsShown`'s set check passes.
   Expect: the confirm is refused as stale input ("review it again"), never signed over. Pinned in
   Task 2 (`a_confirm_never_overrules_a_colleagues_unlink_that_landed_mid_judgement`).
2. **A doubted link with one chart not held here.** Expect: the panel offers no Confirm and says
   which chart is not held; Unlink is still offered. A forced call is refused by the node
   (`NodeState`), never a bare error. Pinned in Task 6 (`same_verdict` tests) and by the node's
   existing `admit_judgement` tests.
3. **A confirm on an already-attested link** (a colleague confirmed it meanwhile). Expect: the panel
   re-read offers no Confirm. If the click raced it, the pair's content address changed, so the
   node refuses with "review it again". Pinned in Task 2
   (`a_confirm_is_refused_when_the_pair_changed_since_the_panel_read_it`).
4. **The hold's equivalence across every doubt case**, including the A–C–X bridge and an
   un-attested unlink inside the set (which is deliberately NOT a doubt). Expect:
   `has_doubted_link` ≡ `EXISTS doubts`, and an un-attested unlink yields no row. Pinned in Task 1's
   matrix test.
5. **A partially read panel** (one chart's facts or the findings failed). Expect: neither verdict is
   offered, because a judgement needs the whole picture. Pinned in Task 6
   (`a_partial_review_offers_no_verdict`).

---

## File structure

| File | Responsibility | Task |
|---|---|---|
| `db/054_person_charts.sql` | `cairn_chart_set_doubts` + `has_doubted_link` as `EXISTS` over it | 1 |
| `crates/cairn-node/src/patient/edges.rs` | `DoubtKind`, `LinkDoubt`, `record_doubts` | 1 |
| `crates/cairn-node/tests/link_doubts.rs` (new) | DB: each kind, the equivalence matrix | 1 |
| `crates/cairn-node/src/chart_link/shown.rs` | `RecordsShown::pair`, `stale_pair`, the under-CARNLK pair check | 2 |
| `crates/cairn-node/src/chart_link/judge.rs` | `confirm_link_as_shown` | 2 |
| 7 `RecordsShown { … }` sites (listed in Task 2) | `pair: None` | 2 |
| `crates/cairn-node/tests/confirm_link.rs` (new) | DB: confirm lifts the hold, the two races, the bridge re-link | 2 |
| `crates/cairn-medication-view/src/chart.rs` | `DOUBTED_LINK_INSTRUCTION` names "Review this link…" | 3 |
| `crates/cairn-patient-search/src/person.rs` | `PersonRow::with_doubt` / `is_doubted` | 4 |
| `crates/cairn-node/src/patient/search_person.rs` | `read_components` returns `Components` (members + doubted) | 4 |
| `crates/cairn-node/src/patient/search.rs` | marks doubted rows | 4 |
| `crates/cairn-node/src/patient/candidate_text.rs` | "↳ linked, in doubt: " | 4 |
| `crates/cairn-node/tests/search_doubted_row.rs` (new) | DB: a doubted row; attestation unchanged | 4 |
| `cairn-gui/cairn-gui-tauri/src/funnel/rows.rs` | the doubted row label | 5 |
| `cairn-gui/cairn-gui-tauri/src/link/record_links.rs` | `RecordLinks`, `PairKind`, doubt marks, `separated` lines, button labels | 6 |
| `cairn-gui/cairn-gui-tauri/src/link/unlink.rs` | `resolve_edge` by kind; `compare_linked` takes `kind` | 6 |
| `cairn-gui/cairn-gui-tauri/src/link/unlink_view.rs` | `LinkReviewView` (renamed) + verdict fields | 6 |
| `cairn-gui/cairn-gui-tauri/src/link/confirm_view.rs` (new) | `same_verdict`, `confirm_report` | 6 |
| `cairn-gui/cairn-gui-tauri/src/link/confirm.rs` (new) | `confirm_link_impl` + Tauri command | 7 |
| `cairn-gui/cairn-gui-tauri/src/{commands.rs,chart_set.rs,main.rs}` | wiring, drift guard | 6, 7 |
| `cairn-gui/cairn-gui-tauri/src-ui/{main.js,unlink.js,index.html}` | the door, the second verdict | 8 |
| `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` | §10's button wording; new §13 | 9 |

---

### Task 1: one spelling of "doubted", per link — db/054 + `record_doubts`

**Files:**
- Modify: `db/054_person_charts.sql` (section 3, lines ~89–161)
- Modify: `crates/cairn-node/src/patient/edges.rs`
- Create: `crates/cairn-node/tests/link_doubts.rs`

**Interfaces:**
- Produces (SQL): `cairn_chart_set_doubts(p_charts uuid[]) RETURNS TABLE (low uuid, high uuid, kind
  text, hlc_wall bigint)`, `kind ∈ {'clash','separated'}`, granted to `cairn_agent`.
- Produces (Rust, `cairn_node::patient::edges`):
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum DoubtKind { Clash, Separated }
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct LinkDoubt { pub low: Uuid, pub high: Uuid, pub kind: DoubtKind, pub recorded_on: String }
  pub async fn record_doubts<C: GenericClient + Sync>(client: &C, charts: &ChartSet) -> anyhow::Result<Vec<LinkDoubt>>
  ```

- [ ] **Step 1: Write the failing DB tests** — `crates/cairn-node/tests/link_doubts.rs`:

```rust
//! R6 (#716): the per-link doubt read, `cairn_chart_set_doubts` (db/054), and the rule that
//! `cairn_chart_set_has_doubted_link` is EXACTLY "some doubt row exists" — so the sign-off hold
//! (R1b) and the window's per-link marks can never disagree.
//!
//! The three ways a set holds a doubted link (db/054's comment): (a) a `link_veto_flag` row, (b)
//! an un-attested standing link that trips the hard veto now, (c) a clinician's ATTESTED unlink
//! between two charts the set still joins. (a)+(b) are kind `clash`; (c) is `separated`. An
//! UN-attested unlink inside the set is deliberately not a doubt.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via `db::test_serial_guard`. Keys minted at runtime.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{link_charts, unlink_charts, LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::patient::edges::{record_doubts, DoubtKind};
use common::{apply_remote_raw, cs, link_assertion_event, medication_setup as setup, register_pair, vetoed_pair};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "r6-doubts";

async fn clean(c: &Client) {
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
}

/// db/054's boolean, asked directly.
async fn has_doubt(c: &Client, charts: &[Uuid]) -> bool {
    let ids: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    c.query_one("SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])", &[&ids])
        .await
        .unwrap()
        .get(0)
}

/// The per-link rows, as `(low, high, kind)`.
async fn doubts(c: &Client, charts: &[Uuid]) -> Vec<(Uuid, Uuid, String)> {
    let ids: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    c.query(
        "SELECT low::text, high::text, kind FROM cairn_chart_set_doubts($1::text[]::uuid[]) \
         ORDER BY low, high",
        &[&ids],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get::<_, String>(0).parse().unwrap(),
            r.get::<_, String>(1).parse().unwrap(),
            r.get(2),
        )
    })
    .collect()
}

fn pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    (a.min(b), a.max(b))
}

/// (b): a peer machine's un-attested link of a vetoed pair is a `clash` on exactly that pair.
#[tokio::test]
async fn an_unattested_link_that_trips_the_veto_is_a_clash_on_that_pair() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk, &kid).await;
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 50, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    let (lo, hi) = pair(a, b);
    assert_eq!(doubts(&c, &[a, b]).await, vec![(lo, hi, "clash".to_string())]);
    assert!(has_doubt(&c, &[a, b]).await);
}

/// A human's attested link of the same vetoed pair is the decision the veto forces: no doubt.
#[tokio::test]
async fn a_confirmed_link_is_no_doubt() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk, &kid).await;
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 50, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    let who = Reviewer { human_sk: &hsk, human_kid: &hkid };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    assert!(doubts(&c, &[a, b]).await.is_empty());
    assert!(!has_doubt(&c, &[a, b]).await);
}

/// (c), the A–C–X bridge: a human says A and X are different people while A–C and C–X still
/// join them. One `separated` row on A–X; the bridging links themselves are not clashes.
#[tokio::test]
async fn an_attested_unlink_the_set_still_joins_is_separated() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let (a, x) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, x).await;
    let cc = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, cc, 1).await;
    for (p, q, wall) in [(a, cc, 40), (cc, x, 41)] {
        let ev = link_assertion_event(&kid, p, q, LinkVerb::Link, wall, 0, "peer-matcher", false);
        apply_remote_raw(&c, &sk, ev).await.unwrap();
    }
    let who = Reviewer { human_sk: &hsk, human_kid: &hkid };
    unlink_charts(&mut c, a, x, None, &who, ORIGIN).await.unwrap();
    let (lo, hi) = pair(a, x);
    assert_eq!(doubts(&c, &[a, cc, x]).await, vec![(lo, hi, "separated".to_string())]);
    assert!(has_doubt(&c, &[a, cc, x]).await);
    // And the Rust read carries the kind and a recorded day.
    let set = ChartSet::new([a, cc, x]).unwrap();
    let read = record_doubts(&c, &set).await.unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!((read[0].low, read[0].high, read[0].kind), (lo, hi, DoubtKind::Separated));
    assert_eq!(read[0].recorded_on.len(), 10, "YYYY-MM-DD: {}", read[0].recorded_on);
}

/// An UN-attested unlink inside the set is deliberately not a doubt (db/054's NOT COVERED note).
#[tokio::test]
async fn an_unattested_unlink_inside_the_set_is_no_doubt() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, x) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, x).await;
    let cc = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, cc, 1).await;
    for (p, q, wall) in [(a, cc, 40), (cc, x, 41)] {
        let ev = link_assertion_event(&kid, p, q, LinkVerb::Link, wall, 0, "peer-matcher", false);
        apply_remote_raw(&c, &sk, ev).await.unwrap();
    }
    let ev = link_assertion_event(&kid, a, x, LinkVerb::Unlink, 42, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    assert!(doubts(&c, &[a, cc, x]).await.is_empty());
    assert!(!has_doubt(&c, &[a, cc, x]).await);
}

/// The equivalence, over the case matrix: the boolean is EXACTLY "a doubt row exists".
/// (Review Focus 4.) Each fixture above leaves its state; this test rebuilds four states in one
/// database and asks both functions of each.
#[tokio::test]
async fn the_hold_is_exactly_some_doubt_row() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let who = Reviewer { human_sk: &hsk, human_kid: &hkid };
    // 1. a plain un-attested link, no veto: no doubt.
    let (p, q) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, p, q).await;
    apply_remote_raw(&c, &sk, link_assertion_event(&kid, p, q, LinkVerb::Link, 30, 0, "m", false))
        .await
        .unwrap();
    // 2. a vetoed un-attested link: clash.
    let (a, b) = vetoed_pair(&c, &sk, &kid).await;
    apply_remote_raw(&c, &sk, link_assertion_event(&kid, a, b, LinkVerb::Link, 31, 0, "m", false))
        .await
        .unwrap();
    // 3. a human-confirmed link: no doubt.
    let (d, e) = vetoed_pair(&c, &sk, &kid).await;
    link_charts(&mut c, d, e, &who, ORIGIN).await.unwrap();
    // 4. a single chart: no doubt.
    for set in [vec![p, q], vec![a, b], vec![d, e], vec![p]] {
        assert_eq!(
            has_doubt(&c, &set).await,
            !doubts(&c, &set).await.is_empty(),
            "the hold and the rows disagree over {set:?}"
        );
    }
    assert!(has_doubt(&c, &[a, b]).await, "case 2 must be doubted, or the matrix proves nothing");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `CARGO_TARGET_DIR=/tmp/cairn-r6-target cargo test -p cairn-node --test link_doubts -- --nocapture`
Expected: compile error, because `record_doubts` / `DoubtKind` do not exist. Once Step 3's Rust stubs
compile, expect a DB error: `function cairn_chart_set_doubts(uuid[]) does not exist`.

- [ ] **Step 3: Edit db/054 in place.** Replace section 3's `CREATE OR REPLACE FUNCTION
  cairn_chart_set_has_doubted_link … COMMIT;` tail (keep the existing comment block above it, and
  add the paragraph below to it) with:

```sql
--    R6 (#716, #723): the three cases are now ONE per-link read, cairn_chart_set_doubts, and
--    the boolean is EXISTS over it — so the sign-off hold (R1b) and the window's per-link marks
--    cannot disagree. kind 'clash' = (a)+(b) (the link a clinician may confirm or unlink);
--    'separated' = (c) (the attested unlink the bridge keeps standing). hlc_wall dates the line.
--    A link_veto_flag row always has its patient_link row: db/018 writes the flag FROM the
--    standing winner it has just read, so joining through patient_link loses no flag.
CREATE OR REPLACE FUNCTION cairn_chart_set_doubts(p_charts uuid[])
RETURNS TABLE (low uuid, high uuid, kind text, hlc_wall bigint)
LANGUAGE sql STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT pl.low, pl.high, 'clash'::text, pl.hlc_wall
    FROM patient_link pl
    WHERE pl.low = ANY(p_charts) AND pl.high = ANY(p_charts)
      AND (EXISTS (SELECT 1 FROM link_veto_flag f WHERE f.low = pl.low AND f.high = pl.high)
           OR (pl.state = 'link' AND NOT pl.attested AND cairn_has_hard_veto(pl.low, pl.high)))
    UNION ALL
    SELECT pl.low, pl.high, 'separated'::text, pl.hlc_wall
    FROM patient_link pl
    WHERE pl.state = 'unlink'
      AND pl.attested
      AND pl.low = ANY(p_charts) AND pl.high = ANY(p_charts)
$$;
REVOKE EXECUTE ON FUNCTION cairn_chart_set_doubts(uuid[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_chart_set_doubts(uuid[]) TO cairn_agent;

CREATE OR REPLACE FUNCTION cairn_chart_set_has_doubted_link(p_charts uuid[])
RETURNS boolean
LANGUAGE sql STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (SELECT 1 FROM cairn_chart_set_doubts(p_charts))
$$;
REVOKE EXECUTE ON FUNCTION cairn_chart_set_has_doubted_link(uuid[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_chart_set_has_doubted_link(uuid[]) TO cairn_agent;

COMMIT;
```

  `cairn_chart_set_doubts` must be created BEFORE `has_doubted_link` (a SQL function's body is
  checked at creation).

- [ ] **Step 4: Add the Rust read** to `crates/cairn-node/src/patient/edges.rs` (below
  `record_edges`; extend the module doc with one sentence: "`record_doubts` reads which of a
  record's pairs db/054 doubts — the marks beside the links, R6"):

```rust
/// Why db/054 doubts a pair of a record's charts (`cairn_chart_set_doubts`'s `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubtKind {
    /// A standing link joins the pair without a clinician's confirmation on record here, and
    /// the node's hard identity check finds a clash between them (db/054 cases (a) and (b)).
    /// A clinician may confirm it or unlink it.
    Clash,
    /// A clinician recorded that the pair are DIFFERENT people, and other links still join them
    /// (case (c), the A–C–X bridge). Not a standing link: it is the attested unlink row.
    Separated,
}

/// One doubted pair of a record's charts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkDoubt {
    /// The pair, canonical (`low < high`).
    pub low: Uuid,
    pub high: Uuid,
    pub kind: DoubtKind,
    /// The day the pair's standing assertion was recorded (UTC, `YYYY-MM-DD`).
    pub recorded_on: String,
}

const DOUBTS_SQL: &str = "SELECT low::text AS low, high::text AS high, kind, \
     to_char(to_timestamp(hlc_wall / 1000.0) AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS recorded_on \
     FROM cairn_chart_set_doubts($1::text[]::uuid[]) ORDER BY low, high";

/// Every pair of `charts` db/054 doubts, ordered by pair. The ONE Rust read of the doubt rule:
/// it asks db/054 and never re-derives the flag, veto or attestation tests.
pub async fn record_doubts<C: GenericClient + Sync>(
    client: &C,
    charts: &ChartSet,
) -> anyhow::Result<Vec<LinkDoubt>> {
    let ids: Vec<String> = charts.members().iter().map(Uuid::to_string).collect();
    let rows = client
        .query(DOUBTS_SQL, &[&ids])
        .await
        .map_err(|e| LocalDbFault::new("reading which of this record's links are in doubt", e))?;
    rows.iter()
        .map(|r| {
            let kind = match r.get::<_, String>("kind").as_str() {
                "clash" => DoubtKind::Clash,
                "separated" => DoubtKind::Separated,
                other => anyhow::bail!("db/054 returned an unknown doubt kind {other:?}"),
            };
            Ok(LinkDoubt {
                low: r.get::<_, String>("low").parse()?,
                high: r.get::<_, String>("high").parse()?,
                kind,
                recorded_on: r.get("recorded_on"),
            })
        })
        .collect()
}
```

- [ ] **Step 5: Run the new tests, then the existing doubted-link suites** (they pin that the
  boolean's behaviour did not change):

Run: `CARGO_TARGET_DIR=/tmp/cairn-r6-target cargo test -p cairn-node --test link_doubts --test doubted_link_withholds --test chart_link -- --nocapture`
Expected: all PASS, and no `skipped:` line.

- [ ] **Step 6: Commit**

```bash
git add db/054_person_charts.sql crates/cairn-node/src/patient/edges.rs crates/cairn-node/tests/link_doubts.rs
git commit -F <msgfile>   # "feat(#716): db/054 reads a record's doubts per link — one spelling of the hold"
```

---

### Task 2: the confirm judgement — `RecordsShown::pair` and `confirm_link_as_shown`

**Files:**
- Modify: `crates/cairn-node/src/chart_link/shown.rs`
- Modify: `crates/cairn-node/src/chart_link/judge.rs`
- Modify (add `pair: None`): `crates/cairn-node/src/duplicate_review/mod.rs:353`,
  `crates/cairn-node/tests/judgement_as_shown.rs` (4 sites: ~240, ~294, ~338, ~358),
  `cairn-gui/cairn-gui-tauri/src/link/mod.rs:210`, and the two in `shown.rs`'s own tests
- Create: `crates/cairn-node/tests/confirm_link.rs`

**Interfaces:**
- Consumes: `chart_link::{StandingLink, standing_link}` (parent module).
- Produces:
  ```rust
  pub struct RecordsShown<'a> { pub this: &'a ChartSet, pub other: &'a ChartSet, pub pair: Option<&'a [u8]> }
  pub fn stale_pair(shown: &[u8], now: Option<&StandingLink>, low: Uuid, high: Uuid) -> Option<String>
  pub async fn confirm_link_as_shown(client: &mut tokio_postgres::Client, a: Uuid, b: Uuid,
      record: &ChartSet, standing: &[u8], reviewer: &Reviewer<'_>, node_origin: &str)
      -> anyhow::Result<LinkOutcome>
  ```
  The refusal text from `stale_pair` contains **"review it again"**.

- [ ] **Step 1: Write the failing pure test** in `shown.rs`'s `mod tests`:

```rust
#[test]
fn a_pair_is_stale_exactly_when_another_assertion_now_stands() {
    let (lo, hi) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let shown: Vec<u8> = (0u8..34).collect(); // a content address, computed (house rule 6)
    let same = StandingLink { content_address: shown.clone(), state: "link".into(), attested: false };
    assert_eq!(stale_pair(&shown, Some(&same), lo, hi), None);
    let other = StandingLink { content_address: (1u8..35).collect(), state: "unlink".into(), attested: true };
    let text = stale_pair(&shown, Some(&other), lo, hi).expect("a different winner is stale");
    assert!(text.contains("review it again"), "{text}");
    assert!(stale_pair(&shown, None, lo, hi).is_some(), "no assertion at all is stale too");
}
```

Also add `pair: None` to the two `RecordsShown { … }` literals already in that test module.

- [ ] **Step 2: Write the failing DB tests** — `crates/cairn-node/tests/confirm_link.rs`.
  - Copy verbatim from `judgement_as_shown.rs` as file-local helpers (house convention): `setup`,
    `standing`, `judgements_signed`, `three_charts`, `PeerEvent`, `colleague_link` and
    `judged_while_the_record_changes`.
  - Write the module doc: R6's confirm, Review Focus 1's defect, and the deterministic harness
    (T1 holds CARNLK; the confirm is observed PARKED; only then does T1 apply the colleague's
    event and commit).
  - Then add:

```rust
use cairn_node::chart_link::{canonical_pair, confirm_link_as_shown, standing_link, unlink_charts};

const ORIGIN: &str = "r6-confirm";

/// A colleague's ATTESTED unlink of `x`–`y` (the mirror of `colleague_link`).
fn colleague_unlink(sk_h: &SigningKey, kid_h: &str, x: Uuid, y: Uuid, wall: i64) -> PeerEvent {
    let ev = link_assertion_event(kid_h, x, y, LinkVerb::Unlink, wall, 0, "colleague", true);
    let signed = sign(&ev, sk_h).unwrap().signed_bytes;
    let token = sign_attestation(&event_address(&signed), kid_h, "attested", sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    PeerEvent { signed, attestation: Some((token, vk_h)) }
}

/// Enrol a second human — the colleague whose judgement may land meanwhile.
async fn colleague(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"COLLEAGUE\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// A record A–C–B joined by three UN-attested (machine) links, A–B among them. Returns the
/// charts, the record, and A–B's standing content address as a panel would read it.
async fn bridged_record(
    c: &Client,
    sk_a: &SigningKey,
    kid_a: &str,
) -> (Uuid, Uuid, Uuid, ChartSet, Vec<u8>) {
    let (a, b, cc) = three_charts(c, sk_a, kid_a).await;
    for (p, q, wall) in [(a, cc, 40), (cc, b, 41), (a, b, 42)] {
        let ev = link_assertion_event(kid_a, p, q, LinkVerb::Link, wall, 0, "peer", false);
        common::apply_remote_raw(c, sk_a, ev).await.unwrap();
    }
    let (lo, hi) = canonical_pair(a, b);
    let ca = standing_link(c, lo, hi).await.unwrap().unwrap().content_address;
    (a, b, cc, ChartSet::new([a, b, cc]).unwrap(), ca)
}

/// Apply a peer's event through the sync door and commit it (outside any race).
async fn apply(c: &Client, ev: PeerEvent) {
    let (token, vk) = ev.attestation.unzip();
    c.execute("SELECT apply_remote_event($1, $2, $3)", &[&ev.signed, &token, &vk])
        .await
        .expect("the peer's event is admitted");
}

fn assert_review_again(err: &anyhow::Error) {
    assert_eq!(refusal_scope(err), Some(RefusalScope::Input), "stale input, not an outage: {err:#}");
    let text = format!("{err:#}");
    assert!(text.contains("review it again"), "the refusal must say what to do: {text}");
}

#[tokio::test]
async fn a_confirm_records_an_attested_link_over_the_machines_and_took_effect() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set, ca) = bridged_record(&c, &sk_a, &kid_a).await;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let out = confirm_link_as_shown(&mut c, a, b, &set, &ca, &who, ORIGIN).await.unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    assert_eq!(standing(&c, a, b).await, Some(("link".into(), true)));
    assert_eq!(out.charts, set, "a confirm changes no membership");
}

/// Review Focus 3: the panel read the machine link; a colleague confirmed it since. The pair's
/// winner changed, so ours is refused rather than recorded over a picture that no longer holds.
#[tokio::test]
async fn a_confirm_is_refused_when_the_pair_changed_since_the_panel_read_it() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set, ca) = bridged_record(&c, &sk_a, &kid_a).await;
    let (sk_col, kid_col) = colleague(&c).await;
    apply(&c, colleague_link(&sk_col, &kid_col, a, b, 43)).await;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let err = confirm_link_as_shown(&mut c, a, b, &set, &ca, &who, ORIGIN).await.unwrap_err();
    assert_review_again(&err);
    assert_eq!(judgements_signed(&c, "identity.link.asserted").await, 0, "nothing signed by us");
}

/// Review Focus 1, deterministic. The confirm parks on CARNLK; meanwhile a colleague's ATTESTED
/// UNLINK of A–B lands. C still joins A and B, so the record's SET is unchanged and the set check
/// alone would pass. The colleague's wall is LOW (43): without the pair check our confirm, stamped
/// later by this node's clock, would win the attested-vs-attested HLC tie-break and silently
/// overrule it. With the check it is refused.
#[tokio::test]
async fn a_confirm_never_overrules_a_colleagues_unlink_that_landed_mid_judgement() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set, ca) = bridged_record(&c, &sk_a, &kid_a).await;
    let (sk_col, kid_col) = colleague(&c).await;
    let (sk_j, kid_j) = (sk_h.clone(), kid_h.clone());
    let set_j = set.clone();
    let out = judged_while_the_record_changes(
        &base,
        move |mut jc| {
            tokio::spawn(async move {
                let who = Reviewer { human_sk: &sk_j, human_kid: &kid_j };
                confirm_link_as_shown(&mut jc, a, b, &set_j, &ca, &who, ORIGIN).await
            })
        },
        colleague_unlink(&sk_col, &kid_col, a, b, 43),
    )
    .await;
    let err = out.expect_err("the colleague's unlink landed after the panel read the pair");
    assert_review_again(&err);
    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)), "the colleague's stands");
    // Positive control: the set really was unchanged, so only the pair check could refuse.
    let now = cairn_node::patient::person::person_charts(&c, a).await.unwrap();
    assert_eq!(now, set, "C still joins A and B — the set check alone would have passed");
}

/// The bridge re-link (#716's comment, item 2): A–C–X joined, a colleague attested A and X are
/// different people; this clinician links them. Attested-first-then-HLC: ours is newer, it stands.
#[tokio::test]
async fn a_bridge_relink_overrules_the_earlier_attested_unlink_and_lifts_the_hold() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, x, cc) = three_charts(&c, &sk_a, &kid_a).await;
    for (p, q, wall) in [(a, cc, 40), (cc, x, 41)] {
        let ev = link_assertion_event(&kid_a, p, q, LinkVerb::Link, wall, 0, "peer", false);
        common::apply_remote_raw(&c, &sk_a, ev).await.unwrap();
    }
    let (sk_col, kid_col) = colleague(&c).await;
    let col = Reviewer { human_sk: &sk_col, human_kid: &kid_col };
    unlink_charts(&mut c, a, x, None, &col, ORIGIN).await.unwrap();
    let set = ChartSet::new([a, cc, x]).unwrap();
    let ids: Vec<String> = set.members().iter().map(Uuid::to_string).collect();
    let held: bool = c
        .query_one("SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])", &[&ids])
        .await
        .unwrap()
        .get(0);
    assert!(held, "precondition: the bridge is doubted");
    let (lo, hi) = canonical_pair(a, x);
    let ca = standing_link(&c, lo, hi).await.unwrap().unwrap().content_address;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let out = confirm_link_as_shown(&mut c, a, x, &set, &ca, &who, ORIGIN).await.unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    assert_eq!(standing(&c, a, x).await, Some(("link".into(), true)));
    let held: bool = c
        .query_one("SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])", &[&ids])
        .await
        .unwrap()
        .get(0);
    assert!(!held, "the hold lifts once no pair is doubted");
}

/// R1b's hold lifts for a vetoed machine link confirmed by a human.
#[tokio::test]
async fn a_confirm_lifts_the_sign_off_hold_on_a_vetoed_machine_link() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = common::vetoed_pair(&c, &sk_a, &kid_a).await;
    let ev = link_assertion_event(&kid_a, a, b, LinkVerb::Link, 50, 0, "peer-matcher", false);
    common::apply_remote_raw(&c, &sk_a, ev).await.unwrap();
    let set = ChartSet::new([a, b]).unwrap();
    let (lo, hi) = canonical_pair(a, b);
    let ca = standing_link(&c, lo, hi).await.unwrap().unwrap().content_address;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let out = confirm_link_as_shown(&mut c, a, b, &set, &ca, &who, ORIGIN).await.unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    let ids = vec![a.to_string(), b.to_string()];
    let held: bool = c
        .query_one("SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])", &[&ids])
        .await
        .unwrap()
        .get(0);
    assert!(!held);
}
```

  Match the imports to what the copied helpers use (`cairn_event::{event_address, generate_key,
  sign, sign_attestation, SigningKey}`, `cairn_medication_view::ChartSet`,
  `cairn_node::db_diagnosis::{refusal_scope, RefusalScope}`, `LinkEffect`, `LinkVerb`, `Reviewer`,
  `common::{cs, link_assertion_event}`). `three_charts`'s registrations use walls 1–3, below every
  link wall above. **Mutation check (RED for the race test):** comment out Step 4's `if let
  Some(pair)` block, run the race test, and see it FAIL (the confirm took effect over the
  colleague's unlink). Then restore it.

- [ ] **Step 3: Run to verify failure**

Run: `CARGO_TARGET_DIR=/tmp/cairn-r6-target cargo test -p cairn-node --lib chart_link::shown && CARGO_TARGET_DIR=/tmp/cairn-r6-target cargo test -p cairn-node --test confirm_link -- --nocapture`
Expected: compile errors (`stale_pair`, the `pair` field and `confirm_link_as_shown` do not exist).

- [ ] **Step 4: Implement in `shown.rs`.**
  - Add the field, documented: `/// The pair's STANDING assertion (its content address) as the
    panel the clinician judged from read it — set by a judgement about a link INSIDE one record
    (R6's confirm), where both shown records are that one record and only the pair can tell a
    colleague's judgement of it landing meanwhile. None when the judgement was made over two
    separate records.` → `pub pair: Option<&'a [u8]>`.
  - Add `stale_pair`:

```rust
/// Why the pair's standing assertion (`now`) is no longer the one the panel showed (`shown`),
/// or `None` if it is. **Pure.** No assertion at all is stale too: the panel showed one.
pub fn stale_pair(shown: &[u8], now: Option<&StandingLink>, low: Uuid, high: Uuid) -> Option<String> {
    match now {
        Some(winner) if winner.is(shown) => None,
        _ => Some(format!(
            "the link between charts {low} and {high} changed while you were judging it (another \
             judgement about it was recorded meanwhile) — nothing was done; review it again"
        )),
    }
}
```

  - In `refuse_unless_records_read_as_shown`, after the `for subject` loop:

```rust
    // R6: a confirm happens INSIDE one record, so a colleague's judgement of the same pair can
    // land without changing either record's set (C still joins A and B). Ask the pair itself,
    // under the same CARNLK, before anything is signed.
    if let Some(pair) = shown.pair {
        let now = super::standing_link(tx, low, high).await?;
        if let Some(stale) = stale_pair(pair, now.as_ref(), low, high) {
            return Err(deliberate_refusal(stale));
        }
    }
```

  - Update the module doc's first paragraph to mention the pair.

- [ ] **Step 5: Implement `confirm_link_as_shown` in `judge.rs`** (after `link_charts_as_shown`):

```rust
/// "Same person — confirm" a link INSIDE one record (R6, #716): a link that already stands
/// without a clinician's confirmation, or — the bridge — two charts a clinician once recorded as
/// different people while other links still join them. As [`link_charts`], but signed only while
/// the record still reads as `record` AND the pair's standing assertion is still `standing`, the
/// one the panel showed (see [`RecordsShown::pair`]). Both charts must be held here.
pub async fn confirm_link_as_shown(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    record: &ChartSet,
    standing: &[u8],
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    let shown = RecordsShown { this: record, other: record, pair: Some(standing) };
    judge(client, LinkVerb::Link, a, b, None, Some(shown), reviewer, node_origin).await
}
```

- [ ] **Step 6: Add `pair: None` at every other `RecordsShown { … }` site** listed under Files. Find
  them with `grep -rn "RecordsShown {" crates cairn-gui --include='*.rs'`.

- [ ] **Step 7: Run** the pure test, `confirm_link`, `judgement_as_shown` and `chart_link`, with
  `--nocapture`, all PASS. Then build the GUI tree (`cd cairn-gui && cargo build -p
  cairn-gui-tauri`): its `link/mod.rs` literal must compile.

- [ ] **Step 8: Commit** — `feat(#716): a confirm is signed only while the pair is still the one shown`.

---

### Task 3: the hold's remedy names the window

**Files:** Modify `crates/cairn-medication-view/src/chart.rs` (`DOUBTED_LINK_INSTRUCTION`, ~line 70,
and its test ~line 320).

- [ ] **Step 1: Change the test first.** In the test that asserts `DOUBTED_LINK_INSTRUCTION.contains("`unlink-charts ")`, add:

```rust
assert!(DOUBTED_LINK_INSTRUCTION.contains("\"Review this link…\""));
assert!(
    !DOUBTED_LINK_INSTRUCTION.contains("cannot confirm"),
    "the window confirms a standing link since R6 (#716)"
);
```

- [ ] **Step 2: Run** `cargo test -p cairn-medication-view` → FAIL.

- [ ] **Step 3: Reword.** Replace the window and CLI sentences (from `In the window,` through `(the window cannot confirm a link that already stands yet).`) with:

```text
In the window, "Review this link…" beside a link marked in doubt under "How these charts are \
linked" compares the two charts and either confirms the link (both charts must be held on this \
node) or unlinks it. In the CLI, `unlink-charts <chart_a> <chart_b> --attester-key <key-file>` \
unlinks (add `--from <open_chart>` when neither chart is held on this node), and `link-charts \
<chart_a> <chart_b> --attester-key <key-file>` confirms a link — both charts must be held on this \
node to link them.
```

  Keep the rest of the constant unchanged. Grep for any golden that quotes the old sentence
  (`grep -rn "cannot confirm a link that already stands" crates cairn-gui`) and update it.

- [ ] **Step 4: Run** `cargo test -p cairn-medication-view` and `cargo test -p cairn-node --lib` → PASS.
- [ ] **Step 5: Commit** — `fix(#716): the doubted-link remedy names the window's confirm`.

---

### Task 4: the search marks a doubted person row (node + shared crate + CLI)

**Files:**
- Modify: `crates/cairn-patient-search/src/person.rs`
- Modify: `crates/cairn-node/src/patient/search_person.rs` (`read_components`, `assemble_components`, its tests)
- Modify: `crates/cairn-node/src/patient/search.rs` (~lines 84–127)
- Modify: `crates/cairn-node/src/patient/candidate_text.rs` (~line 44, ~line 111, tests)
- Create: `crates/cairn-node/tests/search_doubted_row.rs`

**Interfaces:**
- Produces (`cairn_patient_search::PersonRow`): `pub fn with_doubt(self, doubted: bool) -> PersonRow`,
  `pub fn is_doubted(&self) -> bool`. Default false from every constructor and from `Deserialize`.
- Produces (`search_person`): `pub(super) struct Components { pub members: HashMap<Uuid, Vec<Uuid>>,
  pub doubted: HashSet<Uuid> }`. `doubted` holds the matched charts whose component is doubted.

- [ ] **Step 1: Failing pure tests.**
  - In `person.rs`'s tests:

```rust
#[test]
fn a_row_is_undoubted_unless_marked_and_the_mark_never_reaches_the_wire() {
    let row = PersonRow::new(vec![cand(1), cand(2)]).unwrap();
    assert!(!row.is_doubted());
    let doubted = row.clone().with_doubt(true);
    assert!(doubted.is_doubted());
    // The wire shape is the bare member array either way (the signed list is unchanged).
    assert_eq!(serde_json::to_value(&doubted).unwrap(), serde_json::to_value(&row).unwrap());
}
```

    Build `cand(n)` like the existing tests' candidates. If the module has no candidate builder,
    copy the one in `candidate.rs`'s tests.
  - In `search_person.rs`'s tests, change the two `assemble_components` tests to the new triple
    input and add:

```rust
#[test]
fn a_doubted_component_marks_its_anchor_chart() {
    let rows = vec![(id(1), id(1), true), (id(1), id(2), true), (id(3), id(3), false)];
    let out = assemble_components(rows).unwrap();
    assert!(out.doubted.contains(&id(1)) && !out.doubted.contains(&id(3)));
    assert_eq!(out.members[&id(1)], vec![id(1), id(2)]);
}
```

  - In `candidate_text.rs`'s tests:

```rust
#[test]
fn a_doubted_rows_further_members_say_in_doubt() {
    let row = PersonRow::new(vec![full(0xa, "Jane CITIZEN"), bare(0xb, "Mary SMYTHE")])
        .unwrap()
        .with_doubt(true);
    let lines = candidate_lines(&list(vec![row]));
    assert!(lines[2].contains("↳ linked, in doubt: Mary SMYTHE"), "{}", lines[2]);
}
```

    Use the module's existing `list` / `full` / `bare` helpers and its line-producing function
    (named as the existing `a_linked_member_is_printed_under_its_row` test calls it).

- [ ] **Step 2: Run** `cargo test -p cairn-patient-search` and `cargo test -p cairn-node --lib patient::` → FAIL (compile).

- [ ] **Step 3: Implement `PersonRow`'s mark.**
  - Add the field `doubted: bool`, documented: *"Whether db/054 doubts a link among this person's
    charts (R6, #723). A display property read at search time: it is NOT part of the row's wire
    shape (the serde conversion is the member array) and never part of what a registration
    signs."*
  - Set it to `false` in `new`, `alone` and `TryFrom`.
  - Add `with_doubt` and `is_doubted`.
  - `From<PersonRow> for Vec<Candidate>` is unchanged.

- [ ] **Step 4: Implement the one-statement read.** Replace `read_components`' SQL with:

```rust
"SELECT m::text AS chart, c::text AS member, d.doubted \
 FROM unnest($1::text[]::uuid[]) AS m \
 CROSS JOIN LATERAL (SELECT array_agg(x) AS charts FROM cairn_person_charts(m) AS x) AS s \
 CROSS JOIN LATERAL (SELECT CASE WHEN cardinality(s.charts) > 1 \
                                 THEN cairn_chart_set_has_doubted_link(s.charts) \
                                 ELSE false END AS doubted) AS d \
 CROSS JOIN LATERAL unnest(s.charts) AS c"
```

  - Map each row to `(chart, member, doubted)`.
  - `assemble_components(rows: Vec<(Uuid, Uuid, bool)>) -> anyhow::Result<Components>` keeps its
    refusal of a component missing its own chart, and inserts `chart` into `doubted` when
    `doubted` is true.
  - Update the doc comment: still ONE statement and one snapshot; a never-linked chart is not
    asked about doubt (`CASE`).

- [ ] **Step 5: Mark rows in `search.rs`.**
  - `let components = read_components(..)` now yields `Components`.
  - Use `components.members` wherever the map was used: the `ids` flattening and `group_by_person`.
  - Then:

```rust
let people: Vec<PersonRow> = groups
    .iter()
    .map(|group| {
        // R6 (#723): a row is doubted when any member the search anchored on reads a doubted
        // component — every member of a row shares one component, so one anchor suffices.
        let doubted = group.iter().any(|id| components.doubted.contains(id));
        PersonRow::new(group.iter().map(&mut candidate_for).collect())
            .map(|row| row.with_doubt(doubted))
            .ok_or_else(|| anyhow::anyhow!("a person row was grouped with no chart in it"))
    })
    .collect::<anyhow::Result<_>>()?;
```

- [ ] **Step 6: CLI prefix.** Add `const DOUBTED_PREFIX: &str = "\u{21b3} linked, in doubt: ";`.
  In the member loop, use it instead of `LINKED_PREFIX` when `row.is_doubted()`, with a comment
  that it sits inside the same ellipsized name cell, so no column moves.

- [ ] **Step 7: DB test** — `crates/cairn-node/tests/search_doubted_row.rs`:

```rust
//! R6 (#723): the search marks a person row whose links db/054 doubts, and the mark never
//! reaches what a registration signs. DB-gated on $CAIRN_TEST_PG; keys minted at runtime.
mod common;
use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_event::SigningKey;
use cairn_node::chart_link::LinkVerb;
use cairn_node::{db, patient::search::search_patients};
use cairn_patient_search::{CandidateList, SearchAttestation, SearchQuery};
use common::{apply_remote_raw, chart_named, cs, link_assertion_event, medication_setup as setup,
             submit_signed, EventSpec};
use tokio_postgres::Client;
use uuid::Uuid;

/// A document-verified DOB on `p` (two different ones on two charts trip the hard veto).
async fn verified_dob(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, value: &str, wall: i64) {
    submit_signed(c, sk, kid, EventSpec {
        patient: p,
        event_type: "demographic.field.asserted",
        schema_version: "demographic.field/1",
        payload: dob_assertion_body(value, "day", Some("document"), "document-verified"),
        plaintext_twin: Some(render_dob_twin(value, "day", "document-verified")),
        wall,
    })
    .await
    .expect("dob accepted");
}

async fn find(c: &Client, typed: &str) -> CandidateList {
    search_patients(c, &SearchQuery::new(typed, None, &[]), "2026-10-14").await.unwrap()
}

#[tokio::test]
async fn a_set_with_a_doubted_link_is_one_doubted_row_and_signs_the_same_charts() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag").await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    // A doubted pair: two DOUBTWELL charts with clashing verified DOBs, machine-linked.
    let a = chart_named(&c, &sk, &kid, 1, "Ann DOUBTWELL").await;
    let b = chart_named(&c, &sk, &kid, 3, "Ann DOUBTWELL").await;
    verified_dob(&c, &sk, &kid, a, "1980-07-15", 5).await;
    verified_dob(&c, &sk, &kid, b, "1975-01-02", 6).await;
    apply_remote_raw(&c, &sk, link_assertion_event(&kid, a, b, LinkVerb::Link, 50, 0, "m", false))
        .await
        .unwrap();
    // A plain linked pair, no clash: the control.
    let p = chart_named(&c, &sk, &kid, 7, "Bo QUIETLINK").await;
    let q = chart_named(&c, &sk, &kid, 9, "Bo QUIETLINK").await;
    apply_remote_raw(&c, &sk, link_assertion_event(&kid, p, q, LinkVerb::Link, 51, 0, "m", false))
        .await
        .unwrap();

    let doubted = find(&c, "DOUBTWELL").await;
    assert_eq!(doubted.people.len(), 1, "{doubted:?}");
    assert!(doubted.people[0].is_doubted());
    let mut shown = doubted.displayed_charts();
    shown.sort();
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(shown, both, "the signed list is every member, as before");

    let quiet = find(&c, "QUIETLINK").await;
    assert_eq!(quiet.people.len(), 1);
    assert!(!quiet.people[0].is_doubted(), "a linked pair with no doubt is not marked");

    // The attestation is built from the charts alone: the mark cannot reach it.
    let query = SearchQuery::new("DOUBTWELL", None, &[]);
    let signed = SearchAttestation::from_displayed(&query, &doubted);
    assert_eq!(signed.displayed, doubted.displayed_charts());
}
```

- [ ] **Step 8: Run** `cargo test -p cairn-patient-search`, `cargo test -p cairn-node --lib`,
  `cargo test -p cairn-node --test search_doubted_row --test search_by_person -- --nocapture` → PASS.
- [ ] **Step 9: Commit** — `feat(#723): the search marks a person row whose links are in doubt`.

---

### Task 5: the front door's doubted row label

**Files:** Modify `cairn-gui/cairn-gui-tauri/src/funnel/rows.rs`.

- [ ] **Step 1: Failing tests** (append to `rows.rs`'s tests):

```rust
#[test]
fn a_doubted_linked_row_never_says_one_person() {
    let row = PersonRow::new(vec![chart(2), chart(1)]).unwrap().with_doubt(true);
    let label = person_row_view(&row).label.unwrap();
    assert_eq!(label, "Linked charts — a link between them is in doubt (2 charts)");
    assert!(!label.contains("One person"));
}

/// Golden: an undoubted linked row is byte-identical to R3's.
#[test]
fn an_undoubted_linked_row_reads_exactly_as_before() {
    let row = PersonRow::new(vec![chart(2), chart(1)]).unwrap();
    assert_eq!(person_row_view(&row).label.as_deref(), Some("One person — 2 linked charts"));
}
```

- [ ] **Step 2: Run** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri funnel::rows` → FAIL.
- [ ] **Step 3: Implement.**
  - Add `pub fn doubted_row_label(charts: usize) -> String { format!("Linked charts — a link between them is in doubt ({charts} charts)") }`.
  - In `person_row_view`, use it when `row.is_doubted()`.
  - Extend the module doc's "Why the label is worded here" with one sentence: *"A row whose links db/054 doubts never says "One person" (#723, principle 4)."*
- [ ] **Step 4: Run** → PASS. **Step 5: Commit** — `feat(#723): the front door never calls a doubted set one person`.

---

### Task 6: the links list and the review panel (Rust, pure + reads)

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/link/record_links.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/link/unlink.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/link/unlink_view.rs` and `unlink_view_tests.rs`
- Create: `cairn-gui/cairn-gui-tauri/src/link/confirm_view.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/link/mod.rs` (one line: `pub mod confirm_view;`, plus
  `pub mod confirm;` in Task 7). `link/mod.rs` reaches 514 lines; record that in the commit.
- Modify: `cairn-gui/cairn-gui-tauri/src/commands.rs` (~line 56 and the drift guard ~line 504),
  `src/chart_set.rs` (~lines 201–241, 478)

**Interfaces:**
- Consumes: `cairn_node::patient::edges::{record_edges, record_doubts, RecordEdge, LinkDoubt, DoubtKind}`,
  `cairn_node::chart_link::standing_link`.
- Produces (`record_links.rs`):
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "lowercase")]
  pub enum PairKind { Link, Separated }
  impl PairKind { pub fn parse(s: &str) -> Option<PairKind> }   // "link" | "separated"
  pub struct RecordLinks { pub edges: Vec<RecordEdge>, pub doubts: Vec<LinkDoubt> }
  pub struct RecordLinkView { pub low: String, pub high: String, pub text: String,
                              pub kind: PairKind, pub button: String }
  pub async fn read_record_links(state: &AppState, charts: &ChartSet) -> Result<RecordLinks, String>
  pub fn links_section(links: Result<RecordLinks, String>, members: &[MemberLine], linked: bool)
      -> (Vec<RecordLinkView>, Option<String>)
  pub const REVIEW_LINK: &str = "Review this link…";
  pub const REVIEW_JUDGEMENT: &str = "Review this judgement…";
  ```
- Produces (`unlink.rs`): `ShownPair { kind: PairKind, attested: bool, recorded_on: String }`;
  `pub(super) fn shown_pair(links: Result<RecordLinks, String>, low, high, kind, act: Act) ->
  Result<ShownPair, ErrorView>` (replaces `standing_edge`); `pub(super) async fn resolve_edge(…,
  kind: PairKind) -> Result<ResolvedEdge, ErrorView>`. `ResolvedEdge` gains `shown: ShownPair`.
  `Act` gains `Confirm`. `compare_linked_impl` / the `compare_linked` command take `kind: String`.
- Produces (`unlink_view.rs`): `UnlinkComparisonView` is renamed **`LinkReviewView`** and gains
  `kind: PairKind`, `same_label: Option<String>`, `same_note: Option<String>`, `can_confirm: bool`,
  `standing: Vec<u8>`. `UnlinkParts` gains `standing: Result<Option<Vec<u8>>, String>`.
  `unlink_comparison_view` is renamed `link_review_view(parts, charts, low, high, shown:
  &ShownPair)`.
- Produces (`confirm_view.rs`):
  ```rust
  pub struct SameVerdict { pub label: Option<String>, pub note: Option<String> }
  pub fn same_verdict(shown: &ShownPair, held: &[(Uuid, bool)]) -> SameVerdict
  pub fn confirm_report(effect: LinkEffect, kind: PairKind, low: Uuid, high: Uuid) -> LinkReportView
  pub const CONFIRM_SAME: &str = "Same person — confirm";
  pub const LINK_THEM: &str = "Same person — link them";
  ```

- [ ] **Step 1: Failing pure tests — the list.** In `record_links.rs`'s tests (add a
  `doubt(kind)` builder beside `edge`):

```rust
#[test]
fn a_clash_line_says_it_is_in_doubt_and_why() {
    let links = RecordLinks { edges: vec![edge(false)], doubts: vec![doubt(DoubtKind::Clash)] };
    let (lines, err) = links_section(Ok(links), &[], true);
    assert!(err.is_none());
    assert_eq!(lines.len(), 1);
    assert!(lines[0].text.contains("in doubt: this node's identity check finds a clash"));
    assert_eq!(lines[0].button, REVIEW_LINK);
    assert_eq!(lines[0].kind, PairKind::Link);
}

#[test]
fn an_undoubted_link_line_is_byte_identical_to_before() {
    let links = RecordLinks { edges: vec![edge(true)], doubts: vec![] };
    let (lines, _) = links_section(Ok(links), &[], true);
    assert_eq!(lines[0].text, record_link_line(&edge(true), &[]).text);
    assert_eq!(lines[0].button, REVIEW_LINK);
}

#[test]
fn a_separated_pair_gets_its_own_line_after_the_links() {
    let sep = LinkDoubt { low: Uuid::from_u128(1), high: Uuid::from_u128(3),
                          kind: DoubtKind::Separated, recorded_on: "2026-10-01".into() };
    let links = RecordLinks { edges: vec![edge(false)], doubts: vec![sep] };
    let (lines, _) = links_section(Ok(links), &[], true);
    assert_eq!(lines.len(), 2);
    let l = &lines[1];
    assert_eq!(l.kind, PairKind::Separated);
    assert_eq!(l.button, REVIEW_JUDGEMENT);
    assert!(l.text.contains("a clinician recorded on 2026-10-01 (UTC) that these are different people, but other links still join them"));
}
```

  Keep the existing six tests. Change their `links_section(Ok(vec![..]))` calls to
  `Ok(RecordLinks { edges: vec![..], doubts: vec![] })`. A linked record with no EDGES (doubts
  ignored) still reads "reload the chart".

- [ ] **Step 2: Failing pure tests — the panel verdicts** (`confirm_view.rs`'s own tests):

```rust
fn shown(kind: PairKind, attested: bool) -> ShownPair {
    ShownPair { kind, attested, recorded_on: "2026-10-01".into() }
}
const BOTH: [(Uuid, bool); 2] = [(Uuid::from_u128(1), true), (Uuid::from_u128(2), true)];

#[test]
fn an_unattested_link_held_both_sides_offers_confirm() {
    let v = same_verdict(&shown(PairKind::Link, false), &BOTH);
    assert_eq!(v.label.as_deref(), Some(CONFIRM_SAME));
    assert_eq!(v.note, None);
}

#[test]
fn an_attested_link_offers_no_confirm_and_says_nothing() {
    let v = same_verdict(&shown(PairKind::Link, true), &BOTH);
    assert_eq!((v.label, v.note), (None, None));
}

#[test]
fn a_separated_pair_offers_link_them_and_names_the_earlier_judgement() {
    let v = same_verdict(&shown(PairKind::Separated, true), &BOTH);
    assert_eq!(v.label.as_deref(), Some(LINK_THEM));
    let note = v.note.unwrap();
    assert!(note.contains("2026-10-01"), "{note}");
    assert!(note.contains("records your judgement over theirs"), "{note}");
    assert!(note.contains("both judgements stay on record"), "{note}");
    assert!(note.contains("unlink one of the links that still joins them"), "{note}");
}

/// Review Focus 2.
#[test]
fn a_chart_not_held_here_offers_no_confirm_and_names_it() {
    let held = [(Uuid::from_u128(1), true), (Uuid::from_u128(2), false)];
    let v = same_verdict(&shown(PairKind::Link, false), &held);
    assert_eq!(v.label, None);
    let note = v.note.unwrap();
    assert!(note.contains(&Uuid::from_u128(2).to_string()) && note.contains("not held on this node"), "{note}");
}

#[test]
fn a_confirm_report_never_says_it_combined_charts() {
    let (lo, hi) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let r = confirm_report(LinkEffect::TookEffect, PairKind::Link, lo, hi);
    assert!(r.sentence.starts_with("Confirmed —") && r.reload, "{}", r.sentence);
    let r = confirm_report(LinkEffect::TookEffect, PairKind::Separated, lo, hi);
    assert!(r.sentence.contains("now stands over the earlier judgement") && r.reload);
    let r = confirm_report(LinkEffect::Outranked, PairKind::Link, lo, hi);
    assert!(!r.reload && r.sentence.contains("NOT in effect"));
}
```

  In `unlink_view_tests.rs`:
  - rename `UnlinkComparisonView` / `unlink_comparison_view` per Interfaces (a mechanical `sed`);
  - give the existing `UnlinkParts` builders `standing: Ok(Some(vec![7; 34]))` — computed, not
    key material;
  - pass `&ShownPair { kind: PairKind::Link, attested: false, recorded_on: "2026-10-01".into() }`;
  - add:

```rust
/// Review Focus 5: a partial read offers neither verdict.
#[test]
fn a_partial_review_offers_no_verdict() {
    let mut parts = whole_parts(); // the file's existing fully-read builder
    parts.findings = Err("boom".into());
    let v = link_review_view(parts, &set(), lo(), hi(), &unattested_link());
    assert!(!v.can_unlink && !v.can_confirm);
    assert_eq!(v.same_label, None);
}

#[test]
fn a_separated_review_offers_no_unlink() {
    let v = link_review_view(whole_parts(), &set(), lo(), hi(), &separated());
    assert!(!v.can_unlink, "a separated pair is already unlinked");
    assert!(v.can_confirm && v.same_label.as_deref() == Some(LINK_THEM));
}

#[test]
fn an_unread_standing_assertion_offers_no_confirm() {
    let mut parts = whole_parts();
    parts.standing = Ok(None);
    let v = link_review_view(parts, &set(), lo(), hi(), &unattested_link());
    assert!(!v.can_confirm);
    assert!(v.can_unlink, "unlink does not need the standing address");
}
```

  Use the file's existing builders, renamed as needed. If it builds parts inline, add a
  `whole_parts()` helper in the test file.

  Add the shown-pair membership tests next to the existing `standing_edge` tests in `unlink.rs`.
  Rename those tests to call `shown_pair`, and add:

```rust
#[test]
fn a_separated_pair_must_still_be_separated() {
    let sep = |a, b| LinkDoubt { low: Uuid::from_u128(a), high: Uuid::from_u128(b),
                                 kind: DoubtKind::Separated, recorded_on: "2026-10-01".into() };
    let links = || Ok(RecordLinks { edges: vec![], doubts: vec![sep(1, 3)] });
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(3));
    assert!(shown_pair(links(), l, h, PairKind::Separated, Act::Confirm).is_ok());
    let err = shown_pair(links(), l, h, PairKind::Link, Act::Confirm).unwrap_err();
    assert_eq!(err.text, LINK_GONE, "a separated pair is not a standing link");
}

#[test]
fn a_clash_link_carries_its_attestation_into_the_shown_pair() {
    let links = Ok(RecordLinks { edges: vec![RecordEdge { attested: false, ..edge(1, 2) }], doubts: vec![] });
    let s = shown_pair(links, Uuid::from_u128(1), Uuid::from_u128(2), PairKind::Link, Act::Compare).unwrap();
    assert!(!s.attested && s.kind == PairKind::Link);
}
```

- [ ] **Step 3: Run** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri link::` → FAIL (compile).

- [ ] **Step 4: Implement `record_links.rs`.**
  - Add `PairKind`, `RecordLinks`, the two button constants and the two `RecordLinkView` fields.
  - `read_record_links` takes `state.db`'s lock ONCE and runs `record_edges` then
    `record_doubts` under it. Single chart → `Ok(RecordLinks { edges: vec![], doubts: vec![] })`.
  - `links_section`:
    1. one line per edge (`record_link_line`), plus `CLASH_NOTE` when a `Clash` doubt names that
       pair;
    2. then one line per `Separated` doubt (`separated_line`);
    3. the empty-edges-but-linked rule is unchanged.
  - The wording constants, verbatim:

```rust
/// Appended to a standing link db/054 doubts (case (a)/(b)). Says what is held, and by what.
const CLASH_NOTE: &str = " — in doubt: this node's identity check finds a clash between these two \
     charts; lines not recorded only on the chart you opened cannot be signed off until a \
     clinician judges this link";

/// A bridged "different people" (case (c)) — not a standing link, so its own line.
fn separated_line(d: &LinkDoubt, members: &[MemberLine]) -> RecordLinkView {
    RecordLinkView {
        low: d.low.to_string(),
        high: d.high.to_string(),
        text: format!(
            "{} and {} — a clinician recorded on {} (UTC) that these are different people, but \
             other links still join them",
            chart_label(d.low, members),
            chart_label(d.high, members),
            d.recorded_on
        ),
        kind: PairKind::Separated,
        button: REVIEW_JUDGEMENT.into(),
    }
}
```

    `record_link_line` sets `kind: PairKind::Link, button: REVIEW_LINK.into()`. Update the module
    doc: one line per standing link plus one per bridged separation; ONE door per line ("Review
    this link…"), whose panel offers every verdict that applies (R6).

- [ ] **Step 5: Implement `confirm_view.rs`.**
  - Module doc: the confirm half of the review panel's wording, pure; on this panel the wording IS
    the safety content.
  - `same_verdict`:
    1. If any `held` entry is false → `label: None`, `note: Some(format!("Chart {id} is not held
       on this node, so {what} only on a node that holds both charts."))`. `what` is "this link
       can be confirmed" for `Link`, "these charts can be linked" for `Separated`.
    2. Else `Link` with `attested` → `(None, None)`.
    3. Else `Link` → `(Some(CONFIRM_SAME), None)`.
    4. Else `Separated` → `(Some(LINK_THEM), Some(format!("A clinician recorded on {d} (UTC) that
       these two charts are different people. Linking them records your judgement over theirs;
       both judgements stay on record. If they are different people, unlink one of the links that
       still joins them instead.")))`.
  - `confirm_report`:
    - `TookEffect` + `Link` → `"Confirmed — charts {low} and {high} are recorded as the same
      person by your judgement; this link is no longer in doubt."`, `reload: true`.
    - `TookEffect` + `Separated` → `"Linked — your judgement that charts {low} and {high} are the
      same person now stands over the earlier judgement that they were different people; both
      stay on record."`, `reload: true`.
    - `Outranked` → `super::view::LINK_OUTRANKED`, `reload: false`. In `view.rs`, extract the
      existing link `Outranked` sentence into `pub(super) const LINK_OUTRANKED: &str` and use it
      there too (its tests must stay green).
    - `StillJoined` → the same text as `view.rs`'s link `StillJoined` arm, `reload: true`.

- [ ] **Step 6: Implement the panel changes.**
  - `unlink_view.rs`: the rename, the new fields and `UnlinkParts::standing`. In
    `link_review_view`:
    - `held` comes from the two `ChartFacts::held` values (empty when a facts read failed, which
      already makes the view partial);
    - `let same = same_verdict(shown, &held)`;
    - `standing` is `Ok(Some(ca))` → `ca`, else empty;
    - `can_unlink = problems.is_empty() && shown.kind == PairKind::Link`;
    - `can_confirm = problems.is_empty() && same.label.is_some() && !standing.is_empty()`;
    - a `standing` read error is pushed to `problems` as "Which judgement stands on this link
      could not be read: {e}".
  - `unlink.rs`:
    - `Act::Confirm`;
    - `resolve_edge` takes `kind` and calls `read_record_links` + `shown_pair`;
    - `compare_linked_impl` takes `kind: &str` and parses it with `PairKind::parse`. Unknown →
      `refused(LINK_GONE)`;
    - it reads `standing_link` under the same db lock as the facts;
    - the `compare_linked` Tauri forwarder gains `kind: String`;
    - `unlink_impl` passes `PairKind::Link` (unchanged behaviour);
    - the existing `compare_linked_impl` tests in `unlink.rs` gain a `"link"` argument;
    - the module doc names the review panel.
  - `commands.rs` (~56) and `chart_set.rs` call `read_record_links` and the new `links_section`.
    Update the drift guard's `RecordLinkView` literal with `kind: PairKind::Link, button:
    String::new()`.

- [ ] **Step 7: Run** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri` → PASS.
- [ ] **Step 8: Commit** — `feat(#716): the links list marks doubts and the review panel offers both verdicts`.

---

### Task 7: the `confirm_link` command

**Files:** Create `cairn-gui/cairn-gui-tauri/src/link/confirm.rs`. Modify `link/mod.rs` (`pub mod
confirm;`) and `src/main.rs` (register `link::confirm::confirm_link` beside
`link::unlink::unlink_records`, ~line 105).

**Interfaces:**
- Consumes: `unlink::{resolve_edge, Act, ResolvedEdge}` (make them `pub(super)`),
  `confirm_view::confirm_report`, `cairn_node::chart_link::confirm_link_as_shown`.
- Produces: `pub async fn confirm_link_impl(state: &AppState, patient_id: &str, charts:
  Vec<String>, low: &str, high: &str, kind: &str, standing: Vec<u8>) -> Result<LinkReportView,
  ErrorView>` and the `#[tauri::command] confirm_link(…)` forwarder (camelCase JS keys: `patientId,
  charts, low, high, kind, standing`).

- [ ] **Step 1: Failing rule-order tests** (in `confirm.rs`; copy `unlink.rs`'s test helpers `fixture()` and `on_screen()` into its `mod tests`):

```rust
#[tokio::test]
async fn confirm_is_bound_to_the_chart_on_screen() {
    let state = AppState::mock(Some(fixture()));
    let err = confirm_link_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "1", "2", "link", vec![])
        .await.unwrap_err();
    assert!(err.text.contains("not the chart"));
}

#[tokio::test]
async fn confirm_refuses_a_changed_record_as_a_stale_comparison() {
    let state = AppState::mock(Some(fixture()));
    let (p, _) = on_screen();
    let err = confirm_link_impl(&state, &p, vec![p.clone(), Uuid::from_u128(7).to_string()],
        &Uuid::from_u128(1).to_string(), &Uuid::from_u128(2).to_string(), "link", vec![])
        .await.unwrap_err();
    assert_eq!(err.text, crate::link::view::THIS_CHANGED);
}

#[tokio::test]
async fn an_unknown_kind_is_refused_not_guessed() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = confirm_link_impl(&state, &p, charts, &p, &Uuid::from_u128(2).to_string(), "merge", vec![])
        .await.unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}

#[tokio::test]
async fn confirm_refuses_a_link_the_record_no_longer_has() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = confirm_link_impl(&state, &p, charts, &p, &Uuid::from_u128(2).to_string(), "link", vec![])
        .await.unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}
```

- [ ] **Step 2: Run** → FAIL (compile).
- [ ] **Step 3: Implement** (module doc: R6's confirm; the paper counterpart, laying the two
  folders side by side and initialling the clip; the rule order; "the click IS the signature";
  why `standing` travels):

```rust
pub async fn confirm_link_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    low: &str,
    high: &str,
    kind: &str,
    standing: Vec<u8>,
) -> Result<LinkReportView, ErrorView> {
    let kind = PairKind::parse(kind).ok_or_else(|| refused(LINK_GONE))?;
    // 1–3: the chart on screen, the displayed set, the pair still what the panel showed.
    let edge = resolve_edge(state, Act::Confirm, patient_id, &charts, low, high, kind).await?;
    // 4: fixture mode (unreachable in practice: fixture charts are never linked — as unlink).
    if state.is_mock() {
        return Err(refused("fixture mode: this window is showing mock data and cannot write"));
    }
    // An empty address means the panel never read which judgement stands: nothing to confirm.
    if standing.is_empty() {
        return Err(refused(LINK_GONE));
    }
    // 5: the key. A locked key is not a verdict: the button stays for after the unlock.
    let label = match kind { PairKind::Link => CONFIRM_SAME, PairKind::Separated => LINK_THEM };
    let (human_sk, human_kid) = state.live_key(Now::read()).await.ok_or_else(|| key_locked_for(label))?;
    let mut db = state.db.as_ref().ok_or_else(|| refused("no database connection"))?.lock().await;
    let reviewer = cairn_node::chart_link::Reviewer { human_sk: &human_sk, human_kid: &human_kid };
    // The node re-reads the record AND the pair under CARNLK before signing (R6, #708).
    let outcome = cairn_node::chart_link::confirm_link_as_shown(
        &mut db, edge.low, edge.high, &edge.set, &standing, &reviewer, &state.node_origin,
    )
    .await
    .map_err(|e| judgement_error_view("confirmation", &e))?;
    Ok(confirm_report(outcome.effect, kind, edge.low, edge.high))
}
```

- [ ] **Step 4: Run** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri` → PASS.
- [ ] **Step 5: Commit** — `feat(#716): the window can confirm a link that already stands`.

---

### Task 8: the webview — one door, two verdicts

**Files:** Modify `cairn-gui/cairn-gui-tauri/src-ui/main.js` (`renderLinks`, lines 93–115),
`src-ui/unlink.js`, `src-ui/index.html` (lines ~150–161).

- [ ] **Step 1: Failing drift guards.**
  - In `unlink_view_tests.rs`'s `unlink_js_reads_no_field_the_backend_does_not_send`, build the
    payload from `LinkReviewView` (renamed). Add the `confirm_link` report (a `LinkReportView`)
    to the bindings the guard checks, under whatever binding name the new JS uses (`report`).
  - In `commands.rs`'s guard, `recordLink` now has `kind` and `button`.
  - Then edit the JS (Step 3) so the guards pass. Run them first to see them fail on the missing
    fields once the JS reads them.

- [ ] **Step 2: index.html.** Inside `#unlink-panel`, before `#unlink-confirm`, add:

```html
<p id="unlink-same-note" hidden></p>
<button id="unlink-same" type="button" hidden></button>
```

  The heading "Are these two charts the same person?" already fits both verdicts. Leave it.

- [ ] **Step 3: main.js `renderLinks`.** The button's text is `recordLink.button`, its
  `aria-label` is `recordLink.button + " " + recordLink.text`, `b.dataset.kind = recordLink.kind`,
  and the click calls `compareLinked(recordLink.low, recordLink.high, recordLink.kind)`. Keep the
  `typeof compareLinked` guard (fixture-only builds).

- [ ] **Step 4: unlink.js.** Update the header comment: the review panel, two verdicts, the R5a
  rule. Then:
  - `compareLinked(low, high, kind)` sends `kind`. `unlinkOpener = { low, high, kind }`. Store
    `unlinkCompared_ = { patientId, charts: view.charts, low: view.low, high: view.high, kind:
    view.kind, standing: view.standing, canUnlink: view.can_unlink, canConfirm: view.can_confirm }`
    when `view.can_unlink || view.can_confirm`, else `null`.
  - `clearUnlinkComparison` also hides `unlink-same` and `unlink-same-note`.
  - `renderUnlinkComparison`:
    - `el("unlink-confirm").hidden = !view.can_unlink;`
    - `const same = el("unlink-same"); same.textContent = view.same_label || ""; same.hidden = !view.can_confirm;`
    - `setMessage(el("unlink-same-note"), view.same_note || "");`
  - A shared in-flight guard over BOTH buttons:

```js
/** Disable (or re-enable) BOTH verdicts: one human must never sign two verdicts ms apart. */
function setVerdictsDisabled(disabled) {
  el("unlink-confirm").disabled = disabled;
  el("unlink-same").disabled = disabled;
}
/** Hide BOTH verdicts once one has landed or been refused for good. */
function hideVerdicts() {
  el("unlink-confirm").hidden = true;
  el("unlink-same").hidden = true;
}
```

  - `unlinkCompared()` uses `setVerdictsDisabled(true/false)` in place of `button.disabled`. Where
    it hid its own button, `showUnlinkFailure` calls `hideVerdicts()`.
  - New `confirmCompared()`, a mirror of `unlinkCompared()`: it captures `sent`, calls
    `invoke("confirm_link", { patientId, charts, low, high, kind, standing })`, and catches ONLY
    the invoke. Success goes to `showUnlinkReport(sent, token, report)`, which is
    verdict-agnostic; failure goes to `showUnlinkFailure(sent, token, failure,
    el("unlink-same"))`. `finally` calls `setVerdictsDisabled(false)`.
  - `el("unlink-same").addEventListener("click", confirmCompared);`
  - `closeUnlinkPanel`'s focus return also matches `b.dataset.kind === opener.kind`.

- [ ] **Step 5: Run** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri` → PASS
  (both drift guards).
- [ ] **Step 6: Headless walk** (memory "Webview mock walk recipe"):
  1. Copy `src-ui` to the scratchpad and stub `window.__TAURI__.core.invoke`:
     - `med_list` returns a pane with one Link line and one Separated line;
     - `compare_linked` returns a `LinkReviewView` with `can_unlink`, `can_confirm` and
       `same_label` "Same person — confirm";
     - `confirm_link` resolves after 500 ms with `{sentence: "Confirmed — …", reload: true}`.
  2. Serve the copy and drive it with Playwright. Assert by **computed visibility** (`display !==
     "none"` and no hidden ancestor):
     - the line buttons read "Review this link…" / "Review this judgement…";
     - both verdicts are visible after compare;
     - both are `disabled` while `confirm_link` is pending;
     - the note shows for the separated stub;
     - the outcome sentence is visible after it lands.
  3. Record the walk in the PR body. No committed rig exists (#332).
- [ ] **Step 7: Commit** — `feat(#716): the review panel draws and sends the second verdict`.

---

### Task 9: runbook, docs, full gates, PR

**Files:** `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, the design page (R6 "as built"),
`docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1: RUNBOOK.**
  - §10: the line button now reads "Review this link…", and the panel's unlink button is
    unchanged.
  - Add **§13 "Confirm a doubted link (R6, #716 · #723)"**, laid out like §10:
    - setup: a record with a doubted machine link, plus a bridged separation;
    - steps: open the chart; read the doubt mark; Review this link…; read the clash; Same person —
      confirm (start the stopwatch at the click on Review, stop at the outcome line);
    - budget ≤ 15 s;
    - the front door's doubted row label is read aloud by VoiceOver;
    - keyboard-only: Tab reaches both verdicts and Escape closes the panel.
- [ ] **Step 2: Docs.** Add a short "R6 as built" block to the design page (any deviations). In
  HANDOVER ⇒ NEXT: R6 done on its PR, the next item is #742 + #743 part 2, and R6's durable rules:
  - one spelling of doubt;
  - `pair` re-checked under CARNLK;
  - the doubt never signed;
  - both verdicts disabled in flight.

  Add a ROADMAP entry with the §1.2 line. Keep both files concise, and never drop an open issue
  number.
- [ ] **Step 3: Gates, in CI's order, after the final edit** (memory "a gate is evidence only for
  the tree it ran against"):
  1. `cargo fmt --all --check` (root and `cairn-gui`);
  2. `cargo clippy --workspace --all-targets -- -D warnings` (root), then `cd cairn-gui && cargo
     clippy --locked --workspace --all-targets -- -D warnings`;
  3. `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --workspace` (both trees);
  4. `scripts/run-db-gated-tests.sh` (the one local DB gate; long — run it in the background and
     do the docs while it runs);
  5. `cd cairn-gui && cargo test --workspace` with the DB env set;
  6. `CAIRN_TEST_PG=… cargo test -p cairn-node --test paper_parity_plan_section`.
- [ ] **Step 4: PR.** Mark PR #777 ready. Its body closes #716 and #723 (`Closes #716`, `Closes
  #723`) and carries the walk record and the §1.2 benchmark. Run
  `scripts/check_closing_keywords.py` on the body.

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** two folders clipped together, laid side by side and the clip initialled
  ("checked — same person"). The bridge case: lay folders A and X side by side, strike a
  colleague's "not the same person" note with a signed note of your own, and re-clip.
- **Steps:**
  - confirm a doubted link: paper N = 2 (lay side by side, initial) → architecture-forced M = 2
    ("Review this link…", "Same person — confirm"; the click IS the signature, ADR-0053) → UI
    target K = 2;
  - bridge re-link: paper N = 3 → M = 2 → K = 2;
  - the front door adds 0 acts (a reworded label).

  `M ≤ N` throughout. An unlock, when the key is locked, is the existing session act, not one this
  slice adds.
- **Time + cognitive load:** confirming a doubted link from an open chart ≤ 15 s, the same budget
  as runbook §10's unlink. The load is the side-by-side read of the clash, which the panel puts
  first. The doubted row label adds no time to the find (≤ 5 s budget unchanged). The doubt rides
  the components statement, so it costs no extra round trip. Measured by RUNBOOK §13, a human act
  owed.
