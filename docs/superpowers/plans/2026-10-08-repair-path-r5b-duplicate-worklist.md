# Repair path R5b — the possible-duplicate worklist (#680) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A records clerk at the front door sees **"Possible duplicates (N)"**. Opening the tray lists
every open possible duplicate, newest first, as two person rows (the newer record and the one
already on file). **Review** opens the newer record, and R5a's banner then does the judging. Before
that list can be trusted, the matcher must propose every pair no human has judged (#741). The matcher
must also withdraw `review` rows it no longer proposes (#743 part 1).

**Architecture:**
- **Matcher (Python, advisory).**
  - `judged.py` counts a pair as judged only when it is in one record or has an **attested**
    `patient_link` row, which is db/057's rule exactly. A drift test pins the two together.
  - The reconciliation queries widen from `pending` to `pending` + `review`.
- **`cairn-node` (Rust).**
  - `auto_apply.rs` sends a pair with a standing **un-attested unlink** to `review` and writes nothing.
  - `duplicate_review.rs` becomes a directory module and gains `worklist.rs`: one count statement,
    one row read, and a pure grouping by record pair.
  - The banner's read gains `accepted` and `disputed` flags. "Different people" refuses an
    `accepted` pair.
  - `patient/candidate_read.rs` is extracted from `search.rs`, so the worklist builds the same
    `Candidate` the search does.
- **Window.**
  - A new `worklist/` module (pure `view.rs` with goldens, and `mod.rs` with two Tauri commands)
    and `src-ui/worklist.js` draw a native `<details>` tray below the registration form.
  - Review calls the existing `open_chart`. The worklist puts its charts into `AppState::shown` as
    any list on screen does.
  - R5a's banner gains the #736 wording and the dispute note.

**Tech Stack:** PostgreSQL ≥ 18 + `cairn_pgx`; Python 3 (psycopg, pytest, uv); Rust (tokio-postgres,
anyhow, serde, Tauri 2); plain JS (no npm, no bundler).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`, section
**"R5b — the worklist, designed 2026-10-08"** (it wins over the older "R5 — the banner and the
worklist" bullets). Read also the R5a sections above it, ADR-0076 and ADR-0077.

## Global Constraints

- **AGPL-3.0; no new dependency.** No new crate or Python package, so no lockfile changes in any of
  the three Cargo trees or in `matcher/uv.lock`.
- **TDD.** Every behaviour starts with a test that fails for the right reason.
- **Nothing here links.** The worklist reads; Review opens a chart. The only identity write in this
  slice is R5a's existing "Different people" (`chart_link::unlink_charts`). `auto_apply.rs` gains a
  path that writes **no event**, only a status move to `review`.
- **No schema change. `SCHEMA_GENERATION` stays 57.** No new `db/*.sql` file.
- **One "still needs a human" predicate.** Every worklist and banner read selects from db/057's
  `match_proposal_open`, never from `match_proposal` with its own status filter.
- **"Disputed" has ONE SQL spelling**: `duplicate_review::DISPUTED_SQL` (Task 5). Never write the
  `state = 'unlink' AND NOT attested` test a second time.
- **An empty tray means "checked, none open" only on a Current node.** Every failed read is a worded
  line. The tray is never `role="alert"` and never takes focus. No confirmation dialog anywhere
  (principle 3).
- **Every sentence lives in Rust** (`worklist/view.rs`, `duplicates/view.rs`) with goldens. The one
  exception is the existing `failureText` for a backend that cannot be reached at all. The JS
  renders and decides nothing.
- **`tokio::sync::Mutex` is not re-entrant.** `state.db`'s guard must never be held while calling
  `read_chart_of` or `chart_set_of` (each takes it itself). The worklist calls neither.
- **Files under 500 lines.**
  - `funnel/commands.rs` (957) and `link/mod.rs` (492) are **not touched**.
  - `funnel.js` (460) grows by ≤ 4 lines.
  - `search.rs` (477) shrinks.
  - `duplicate_review/mod.rs` stays < 450.
  - `tests/duplicate_review.rs` (497) is **not touched**; new DB tests go in new files.
- **House rule 6:** no literal key material; never name a non-crypto value `salt`/`nonce`/`iv`.
- **Commit messages** say `Refs #680` (and `Refs #741` / `Refs #743` / `Refs #736` where they
  apply), never a closing keyword. Run `python3 scripts/check_closing_keywords.py <msgfile>` before
  every commit. `fix(#741):`-style prefixes are safe.
- **New test helpers stay file-local.** A new `pub fn` in `crates/cairn-node/tests/common/mod.rs`
  must also be added to `identity_scaffolding_shared.rs`'s expected list; this plan adds none.
- **Subagents run FOREGROUND tests only.** The controller re-runs DB-gated suites with
  `--nocapture` and checks that no `skipped:` line appears (a self-skip still prints "ok").
- **DB env** (`scripts/pg-target.sh` prints the cluster; use its port):
  ```bash
  export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
         CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test2" \
         CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test3"
  ```
  - **Rust:** `CARGO_TARGET_DIR=/tmp/cairn-r5b-target` while an IDE is open (trap 18). A narrow
    `cargo test --test X` can stall on one freshly linked binary (macOS Gatekeeper); exec the printed
    `target/debug/deps/X-<hash>` directly.
  - **Matcher:** `cd matcher && uv run --extra pipeline pytest tests/<file> -q`. Reproduce CI's pure
    job with `CAIRN_ALLOW_DB_SKIP=1 uv run --isolated pytest`.
  - **GUI tree:** needs `CAIRN_ALLOW_DB_SKIP=1` for a DB-free run.
- **`cargo doc` with `RUSTDOCFLAGS=-D warnings`** fails on an intra-doc link to a private item.
  Write private names in plain backticks, never `[`…`]`.

## Review Focus

1. **A pair whose only `patient_link` row is an un-attested unlink.** It must be proposed by both the
   per-chart worker and the sweep, appear on the banner and the worklist with the dispute note, and
   never be auto-linked. Pinned in Task 1 (Python), Task 2 (DB), and Tasks 5 and 6 (DB).
2. **Two charts of one record each proposed against one other record.** This makes one worklist
   entry, counted once, and the count statement agrees with the list's `total`. Pinned in Task 5
   (pure and DB).
3. **An `accepted` row.** It reads "Accepted as the same person — not yet linked" on the banner and
   the worklist, offers no "Different people", and the backend refuses "Different people" on it even
   if the webview sends the command. Pinned in Task 6 (DB) and Task 7 (window).
4. **The tray on a node whose check is not Current, with zero entries.** It is shown with R4's status
   line, never hidden. On a Current node with zero entries it is hidden. Pinned in Task 8 (goldens).
5. **Returning to the front door after a chart.** `close_chart` cleared `shown`, so an open tray must
   re-read its list, or Review is refused ("not in a list on screen"). Pinned in Task 8 (fixture
   test: the worklist read re-admits) and Task 9 (the headless walk).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `matcher/src/cairn_matcher/pipeline/judged.py` | modify | the skip rule = db/057's rule |
| `matcher/tests/test_judged.py` | modify | attested arm, drift test vs db/057 |
| `matcher/tests/test_check_chart.py` | modify | un-attested-unlink pair is proposed; `review` row retracted |
| `matcher/src/cairn_matcher/pipeline/queue_db.py` | modify | `pending_pairs_involving` covers `review` |
| `matcher/src/cairn_matcher/pipeline/db.py` | modify | `AWAITING_HUMAN`; `pending_proposal_pairs` / `retract_pending_proposal` cover `review`; upsert docstring |
| `matcher/tests/test_proposal_retraction.py` | modify | sweep and propose retract a `review` row |
| `docs/spec/decisions/0078-a-pair-is-judged-only-by-a-human.md` | **create** | ADR-0078 |
| `docs/spec/decisions/README.md`, `mkdocs.yml`, `docs/spec/index.md`, `docs/spec/identity.md` | modify | index row, nav, v0.80, §5.2 sentence |
| `crates/cairn-node/src/auto_apply.rs` | modify | `DisputedToReview`; summary bucket |
| `crates/cairn-node/src/main.rs` | modify | summary line names the new bucket |
| `crates/cairn-node/tests/auto_apply.rs` | modify | the disputed test |
| `crates/cairn-node/src/patient/candidate_read.rs` | **create** | `DisplayFacts`, `read_display_facts`, `candidates_by_id` |
| `crates/cairn-node/src/patient/search.rs` | modify | calls `read_display_facts`; read helpers moved out |
| `crates/cairn-node/src/patient/mod.rs` | modify | `pub mod candidate_read;` |
| `crates/cairn-node/tests/candidates_by_id.rs` | **create** | equals the search's candidates |
| `crates/cairn-node/src/duplicate_review.rs` → `duplicate_review/mod.rs` | **move** + modify | `DISPUTED_SQL`; banner flags; `AcceptedAsSame` |
| `crates/cairn-node/src/duplicate_review/worklist.rs` | **create** | `ProposalRow`, `WorklistEntry`, `group_by_record_pair`, `registered_ms`, `worklist_count`, `worklist` |
| `crates/cairn-node/tests/duplicate_worklist.rs` | **create** | DB tests of the worklist reads |
| `crates/cairn-node/tests/duplicate_review_flags.rs` | **create** | DB tests of the banner flags and `AcceptedAsSame` |
| `cairn-gui/cairn-gui-tauri/src/duplicates/view.rs`, `view_tests.rs`, `mod.rs` | modify | `EntryFlags`, accepted heading, dispute note, `offers_different_people`, `ACCEPTED_NOT_OVERRULED` |
| `cairn-gui/cairn-gui-tauri/src/worklist/view.rs` | **create** | every tray sentence (pure) |
| `cairn-gui/cairn-gui-tauri/src/worklist/view_tests.rs` | **create** | goldens |
| `cairn-gui/cairn-gui-tauri/src/worklist/mod.rs` | **create** | `tray_count_impl`, `worklist_impl`, fixture, commands, JS field guard |
| `cairn-gui/cairn-gui-tauri/src/main.rs` | modify | `mod worklist;` + two commands |
| `cairn-gui/cairn-gui-tauri/src-ui/index.html` | modify | the `<details>` tray; the script tag |
| `cairn-gui/cairn-gui-tauri/src-ui/worklist.js` | **create** | draw the tray; Review |
| `cairn-gui/cairn-gui-tauri/src-ui/funnel.js` | modify | `refreshTray()` on boot and on return |
| `cairn-gui/cairn-gui-tauri/src-ui/duplicates.js` | modify | "Different people" follows `entry.offers_different_people` |
| `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `TEMPLATE.md` | modify | §12 the tray |
| design page, HANDOVER, ROADMAP | modify | as-built note; state |

---

### Task 1: the matcher's skip rule agrees with db/057 (#741) — and ADR-0078

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/judged.py`
- Modify: `matcher/tests/test_judged.py`, `matcher/tests/test_check_chart.py`
- Create: `docs/spec/decisions/0078-a-pair-is-judged-only-by-a-human.md`
- Modify: `docs/spec/decisions/README.md`, `mkdocs.yml`, `docs/spec/index.md`, `docs/spec/identity.md`

**Interfaces:**
- Consumes: db/057's view `match_proposal_open`; `patient_link.attested` (db/018); `person_member`.
- Produces: `judged_partners(conn, patient)` and `judged_pairs(conn)` (unchanged signatures), now
  excluding a pair whose only row is an un-attested unlink.

- [ ] **Step 1: Write the failing tests** — in `matcher/tests/test_judged.py`.

  Replace `_link` so a test can say whether the row is attested:

```python
def _link(conn, x, y, state, attested=False):
    low, high = canonical_pair(x, y)
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO patient_link (low, high, state, hlc_wall, hlc_counter, origin, "
            "provenance, content_address, attested) VALUES (%s,%s,%s,1,0,'seed','test:link',%s,%s)",
            (low, high, state, b"\x12\x20" + hashlib.sha256(f"{low}{high}".encode()).digest(),
             attested))
    conn.commit()


def _propose(conn, x, y):
    low, high = canonical_pair(x, y)
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) VALUES (%s,%s,1,'review','[]','[]','v')",
            (low, high))
    conn.commit()
```

  Replace the two DB tests at the bottom. The human unlink is now attested, and an un-attested
  unlink sits beside it:

```python
E = str(uuid.UUID(int=5))


def test_partners_are_the_component_and_every_attested_link_row(pg_conn):
    # A–B–C one record; A–D a human's (attested) unlink; A–E an agent's (un-attested) unlink.
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link", attested=True)
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink", attested=True)
    _link(pg_conn, A, E, "unlink", attested=False)
    assert judged_partners(pg_conn, A) == frozenset({A, B, C, D})
    assert judged_partners(pg_conn, D) == frozenset({D, A})
    assert judged_partners(pg_conn, E) == frozenset({E}), "nobody judged A–E"


def test_judged_pairs_cover_members_and_attested_unlinks_only(pg_conn):
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link", attested=True)
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink", attested=True)
    _link(pg_conn, A, E, "unlink", attested=False)
    want = {canonical_pair(x, y) for x, y in [(A, B), (B, C), (A, C), (A, D)]}
    assert judged_pairs(pg_conn) == frozenset(want)


def test_judged_is_exactly_what_db057_does_not_hold_open(pg_conn):
    """#741's drift guard: the matcher's skip rule and db/057's openness are ONE rule.

    Five pairs, one per case. A `pending` proposal on each. A pair is judged (never proposed)
    exactly when db/057 does NOT hold its proposal open — no case may be judged by one and open
    by the other, or a pair is silently never shown (#741) or proposed forever.
    """
    p = [str(uuid.UUID(int=i)) for i in range(21, 31)]
    cases = {
        "one record": (p[0], p[1]),
        "attested link": (p[2], p[3]),
        "attested unlink": (p[4], p[5]),
        "un-attested unlink": (p[6], p[7]),
        "no row": (p[8], p[9]),
    }
    _member(pg_conn, p[0], p[0])
    _member(pg_conn, p[1], p[0])
    _member(pg_conn, p[2], p[2])
    _member(pg_conn, p[3], p[2])
    _link(pg_conn, p[2], p[3], "link", attested=True)
    _link(pg_conn, p[4], p[5], "unlink", attested=True)
    _link(pg_conn, p[6], p[7], "unlink", attested=False)
    for x, y in cases.values():
        _propose(pg_conn, x, y)
    with pg_conn.cursor() as cur:
        cur.execute("SELECT patient_low::text, patient_high::text FROM match_proposal_open")
        still_open = {(lo, hi) for lo, hi in cur.fetchall()}
    pg_conn.rollback()
    judged = judged_pairs(pg_conn)
    for name, (x, y) in cases.items():
        pair = canonical_pair(x, y)
        assert (pair in judged) == (pair not in still_open), name
        assert (y in judged_partners(pg_conn, x)) == (pair in judged), name
    # And the expected split, so the guard cannot pass by both sides being wrong together.
    assert still_open == {canonical_pair(*cases["un-attested unlink"]),
                          canonical_pair(*cases["no row"])}
```

  In `matcher/tests/test_check_chart.py`, after `test_a_linked_pair_is_never_proposed`, add:

```python
def test_a_pair_only_an_unattested_unlink_stands_on_is_proposed(pg_conn):
    # #741: an agent's unconfirmed "different people" is not a human judgement. The pair must
    # still reach a human (banner + worklist), with the dispute shown there.
    import hashlib
    _near_duplicates(pg_conn)
    lo, hi = sorted([A, B])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO patient_link (low, high, state, hlc_wall, hlc_counter, origin, "
            "provenance, content_address, attested) "
            "VALUES (%s,%s,'unlink',1,0,'seed','test:agent',%s,false)",
            (lo, hi, b"\x12\x20" + hashlib.sha256(f"{lo}{hi}".encode()).digest()))
    pg_conn.commit()
    assert check_chart(pg_conn, B, Settings()).proposed == 1
```

- [ ] **Step 2: Run them to see them fail**

  Run: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest tests/test_judged.py tests/test_check_chart.py -q`

  Expected: FAIL. `E` is in `judged_partners(A)`; the drift test fails on "un-attested unlink";
  the check_chart test reports `proposed == 0`.

- [ ] **Step 3: Implement** — `judged.py`.

  Replace the module docstring's first paragraph and both queries:

```python
"""Pairs a human (or the identity algebra) has already judged — never proposed again.

The commit-time check's skip rule (ADR-0076 decision 7, as ADR-0078 refines decision 4): a pair
already in ONE record (same person_member.person_id) is the same person already; a pair with an
ATTESTED patient_link row — a human's link or "not the same person" unlink — has been judged.
That is db/057's `match_proposal_open` rule exactly, pinned by
tests/test_judged.py::test_judged_is_exactly_what_db057_does_not_hold_open.

An UN-attested unlink is NOT a judgement (#741): unlinks are not veto-gated and the ADR-0030
agent writer can author one, so skipping on it would let any unreviewed writer keep a pair off
the banner and the worklist for ever. Such a pair is proposed; the window shows the dispute, and
auto_apply.rs sends it to human review rather than linking over it.

Requires the optional `pipeline` extra (psycopg) at call time, except drop_judged (pure).
"""
```

```python
def judged_partners(conn, patient) -> frozenset[str]:
    """Every chart already judged against `patient`: its record, and any ATTESTED link-row partner.

    Includes `patient` itself (cairn_person_charts always returns the chart), which is harmless:
    a self-pair is never generated.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT c::text FROM cairn_person_charts(%s::uuid) AS c "
            "UNION SELECT (CASE WHEN low = %s::uuid THEN high ELSE low END)::text "
            "FROM patient_link WHERE (low = %s::uuid OR high = %s::uuid) AND attested",
            (patient, patient, patient, patient),
        )
        return frozenset(r[0] for r in cur.fetchall())


def judged_pairs(conn) -> frozenset[tuple[str, str]]:
    """Every judged pair node-wide, canonical — for the bulk sweep's skip filter.

    Two members of one record are judged even with no direct link row between them (A–B and
    B–C linked make A–C the same person), so the record self-join is needed as well as the
    ATTESTED link rows.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT a.patient_id::text, b.patient_id::text FROM person_member a "
            "JOIN person_member b ON a.person_id = b.person_id AND a.patient_id < b.patient_id "
            "UNION SELECT low::text, high::text FROM patient_link WHERE attested"
        )
        return frozenset(canonical_pair(x, y) for x, y in cur.fetchall())
```

  Also update the module's first line of `matcher/tests/test_judged.py` to: `"""R4 Task 3, refined
  by #741: a pair in one record, or with an ATTESTED patient_link row, is never proposed."""`

- [ ] **Step 4: Run them to see them pass**

  Run the same command, then the whole matcher suite: `uv run --extra pipeline pytest -q`.

  Expected: all PASS, with no `skipped` for the DB tests (check `-rs`).

- [ ] **Step 5: Write ADR-0078** — `docs/spec/decisions/0078-a-pair-is-judged-only-by-a-human.md`.

```markdown
# ADR-0078 — A pair is judged only by a human

- **Status:** Accepted
- **Date:** 2026-10-08
- **Spec version at acceptance:** 0.80
- **Issues:** [#741](https://github.com/cairn-ehr/cairn-ehr/issues/741) (maintainer, 2026-10-07) ·
  [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680)
- **Supersedes:** [ADR-0076](0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)'s
  skip rule wording **only** (its Consequences: "it must skip pairs already linked or unlinked"; and
  the R4 reading of decision 4, "a pair with ANY `patient_link` row has been judged").
- **Design:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (R5b)

## Context

R4's commit-time check skipped any pair with a `patient_link` row, so that a settled question is never
put back on the worklist. R5a's db/057 then ruled that only an **attested** unlink closes a possible
duplicate: unlinks are not veto-gated, and the ADR-0030 agent writer can author one, so counting an
un-attested unlink would let any unreviewed writer silently clear a banner. The two rules disagreed
before a proposal existed. A pair whose only row was an agent's un-attested unlink was never
proposed, so it reached neither the banner nor the worklist. No human had judged it, and a drug on
the other chart stayed invisible to the prescriber.

## Decision

1. **A pair is judged — and the matcher never proposes it — only when** its two charts read as one
   record (`person_member`), **or** `patient_link` holds an **attested** row for it (a human's link
   or unlink). This is db/057's openness rule, and a drift test pins the two together.
2. **A pair whose standing row is an un-attested unlink is proposed, and never auto-linked.**
   `auto_apply` moves it to `review` and writes nothing. Otherwise a matcher link would overrule the
   agent's unlink by HLC, which is one machine overruling another where only a human may decide.
3. **The dispute is shown, not hidden.** The banner and the worklist say the pair is "recorded as not
   the same person, without a clinician's confirmation on record here" (principle 4).

## Consequences

- Every pair no human has judged reaches a human.
- An agent's unlink now produces work for a human instead of suppressing it. That is the cost, and it
  is accepted.
- No wire change, no schema change, no new event type.

## Rejected

- **Count an un-attested unlink as judged in db/057 too.** That makes the two rules agree by letting
  any unreviewed writer clear a banner, which is the hazard db/057 exists to close.
- **Auto-apply over an un-attested unlink.** The overlay would then decide between two machine
  assertions by HLC.
```

  Then:
  - `docs/spec/decisions/README.md`: add a row after 0077, in the table's format:
    `| [0078](0078-a-pair-is-judged-only-by-a-human.md) | **A pair is judged only by a human**: the matcher skips a pair only when it is in one record or has an ATTESTED patient_link row (db/057's rule; #741); an un-attested unlink is proposed, sent to review by auto-apply, and shown as a dispute. | Accepted (supersedes ADR-0076's skip-rule wording only) | 2026-10-08 |`
  - `mkdocs.yml`: after the ADR-0077 nav line, add
    `      - ADR-0078 · A pair is judged only by a human: spec/decisions/0078-a-pair-is-judged-only-by-a-human.md`
  - `docs/spec/index.md` line 9: `**Spec version:** 0.79` → `0.80`.
  - `docs/spec/identity.md`: in the §5.2 "Probabilistic tier" bullet, after "…a machine's link or
    unlink never overrides a human's ([ADR-0076]… decision 5).", insert: ` A pair is "judged" — never
    proposed again — only when it reads as one record or a human's attested link or unlink stands
    for it; an un-attested unlink is a dispute for a human, never a judgement
    ([ADR-0078](decisions/0078-a-pair-is-judged-only-by-a-human.md)).`

- [ ] **Step 6: Build the docs**

  Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict 2>&1 | tail -5`

  Expected: builds with no warning naming 0078.

- [ ] **Step 7: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/judged.py matcher/tests/test_judged.py \
  matcher/tests/test_check_chart.py docs/spec mkdocs.yml
# message: "fix(#741): the matcher skips only a pair a human judged — db/057's rule; ADR-0078 (Refs #680)"
```

---

### Task 2: auto-apply never links over a disputed pair

**Files:**
- Modify: `crates/cairn-node/src/auto_apply.rs`, `crates/cairn-node/src/main.rs`
- Test: `crates/cairn-node/tests/auto_apply.rs`

**Interfaces:**
- Produces: `AutoOutcome::DisputedToReview`; `AutoSummary::disputed_to_review: usize`.

- [ ] **Step 1: Write the failing test** — append to `crates/cairn-node/tests/auto_apply.rs`.

  Also add `use common::{apply_remote_raw, link_assertion_event};` if not already imported (the
  file uses `common::` paths; follow its style).

```rust
/// ADR-0078 decision 2 (#741): another writer's UN-attested "different people" stands for the
/// pair. A matcher link would overrule it by HLC — one machine overruling another — so the pair
/// goes to a human (`review`) and nothing is written.
#[tokio::test]
async fn a_pair_another_writer_unlinked_unconfirmed_goes_to_review_and_nothing_is_written() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, low, high).await;
    seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;
    // The agent signer (`recorded`, no responsibility) unlinks — through the remote door, as a
    // peer's agent would.
    let unlink = common::link_assertion_event(
        &seed_kid, low, high, LinkVerb::Unlink, 50, 0, "peer", false,
    );
    common::apply_remote_raw(&c, &seed_sk, unlink).await.expect("an agent's unlink lands");
    let events_before: i64 = c.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa").await.unwrap();
    let out = apply_auto_candidate(
        &mut c, low, high, &sk, &kid,
        Hlc { wall: 100, counter: 0, node_origin: "testnode".into() },
    )
    .await
    .unwrap();
    assert!(matches!(out, AutoOutcome::DisputedToReview), "must go to a human");
    let events_after: i64 = c.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);
    assert_eq!(events_after, events_before, "no matcher link was written");
    let status: String = c
        .query_one(
            "SELECT status FROM match_proposal \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "review");
    let (state, attested): (String, bool) = {
        let r = c
            .query_one(
                "SELECT state, attested FROM patient_link \
                 WHERE low=$1::text::uuid AND high=$2::text::uuid",
                &[&low.to_string(), &high.to_string()],
            )
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };
    assert_eq!((state.as_str(), attested), ("unlink", false), "the agent's unlink still stands");

    // The batch driver counts it in its own bucket.
    c.execute(
        "UPDATE match_proposal SET status='pending' \
         WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
        &[&low.to_string(), &high.to_string()],
    )
    .await
    .unwrap();
    let s: AutoSummary = apply_auto_candidates(&mut c, dir.path(), None, "testnode").await.unwrap();
    assert_eq!(
        (s.applied, s.disputed_to_review, s.human_judged, s.skipped, s.errored),
        (0, 1, 0, 0, 0)
    );
}
```

- [ ] **Step 2: Run it to see it fail**

  Run: `cargo test -p cairn-node --test auto_apply a_pair_another_writer -- --nocapture`

  Expected: compile error (`DisputedToReview` and `disputed_to_review` do not exist).

- [ ] **Step 3: Implement** — `auto_apply.rs`.

  Add the variant after `AlreadyJudged`:

```rust
    /// Another writer's UN-attested `unlink` stands for the pair (ADR-0078 decision 2, #741):
    /// "different people", unconfirmed. A matcher link would overrule it by HLC order — one
    /// machine overruling another — so the proposal went to human `review` and nothing was
    /// written. The worklist and banner show the dispute.
    DisputedToReview,
```

  In `apply_auto_candidate`, insert after the `if let Some(state) = judged { … }` block and before
  step 3:

```rust
    // 2b. ANOTHER WRITER says "different people", without a clinician's confirmation (ADR-0078
    //     decision 2). Step 2 returned for any ATTESTED row, so a row left here is un-attested.
    //     Linking over it would decide between two machine assertions by HLC; a human decides
    //     instead. Same move as a veto: to `review`, no event.
    let disputed: bool = tx
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM patient_link \
              WHERE low=$1::text::uuid AND high=$2::text::uuid \
                AND state='unlink' AND NOT attested)",
            &[&low_s, &high_s],
        )
        .await
        .map_err(|e| LocalDbFault::new("reading whether another writer unlinked the pair", e))?
        .get(0);
    if disputed {
        tx.execute(
            "UPDATE match_proposal SET status='review', updated_at=clock_timestamp() \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low_s, &high_s],
        )
        .await
        .map_err(|e| LocalDbFault::new("sending a disputed pair to human review", e))?;
        tx.commit()
            .await
            .map_err(|e| LocalDbFault::new("committing the dispute-to-review update", e))?;
        return Ok(AutoOutcome::DisputedToReview);
    }
```

  Extend the function's doc comment's first sentence to say: "…skip a pair a human has already
  judged …, send a pair another writer unlinked without a clinician's confirmation to `review`
  (ADR-0078), RE-CHECK the db/016 veto …".

  In `AutoSummary`, after `vetoed_to_review`, add:

```rust
    /// Pairs sent to human review because another writer's UN-attested unlink stands
    /// ([`AutoOutcome::DisputedToReview`], ADR-0078).
    pub disputed_to_review: usize,
```

  Initialise it to `0` in `apply_auto_candidates`'s `AutoSummary { … }`, and add the match arm:
  `Ok(AutoOutcome::DisputedToReview) => summary.disputed_to_review += 1,`.

  In `crates/cairn-node/src/main.rs`, the auto-apply `println!`:

```rust
            println!(
                "auto-apply: applied {}  vetoed->review {}  disputed->review {}  \
                 human-judged (left pending) {}  skipped {}  errored {}",
                s.applied, s.vetoed_to_review, s.disputed_to_review, s.human_judged, s.skipped,
                s.errored
            );
```

  Any other `AutoSummary { … }` literal in tests must gain the field (`grep -rn "AutoSummary {"
  crates`).

- [ ] **Step 4: Run it to see it pass**

  Run: `cargo test -p cairn-node --test auto_apply -- --nocapture`

  Expected: every test PASSES, including the existing `a_pair_a_human_already_judged…` (the attested
  arm still runs first). No `skipped:` lines.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/auto_apply.rs crates/cairn-node/src/main.rs crates/cairn-node/tests/auto_apply.rs
# message: "fix(#741): auto-apply sends a pair another writer unlinked, unconfirmed, to review (Refs #680)"
```

---

### Task 3: the matcher re-assesses `review` rows (#743 part 1)

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py`, `matcher/src/cairn_matcher/pipeline/queue_db.py`
- Test: `matcher/tests/test_check_chart.py`, `matcher/tests/test_proposal_retraction.py`

**Interfaces:**
- Produces: `queue_db.AWAITING_HUMAN = ("pending", "review")`, defined in the PURE `queue_db.py`
  (which imports nothing); `db.py` imports it from there. `retract_pending_proposal`,
  `pending_proposal_pairs` and `queue_db.pending_pairs_involving` keep their names and signatures and
  now cover both statuses.

- [ ] **Step 1: Write the failing tests**

  In `matcher/tests/test_check_chart.py`, after `test_a_stale_pending_proposal_no_longer_blocked_is_reassessed`:

```python
def test_a_stale_review_row_no_longer_blocked_is_retracted(pg_conn):
    # #743 part 1: auto_apply moved this pair to status 'review' (a veto appeared), then the
    # facts changed and the matcher no longer proposes it. It must be withdrawn, not left asking
    # a human to judge a pair the matcher has dropped.
    c = str(uuid.UUID(int=13))
    seed_patient(pg_conn, A, names=[("Mary Smith", 20)])
    seed_patient(pg_conn, c, names=[("Zed Quux", 20)])
    lo, hi = sorted([A, c])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version, status) "
            "VALUES (%s,%s,1,'auto_candidate','[]','[]','v','review')",
            (lo, hi))
    pg_conn.commit()
    result = check_chart(pg_conn, A, Settings())
    assert result.retracted == 1
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "retracted"
```

  In `matcher/tests/test_proposal_retraction.py`, after the #210 sweep test:

```python
def test_sweep_reconciles_a_review_row_that_left_the_blocking_universe(pg_conn):
    """#743 part 1: the #210 reconciliation covers a row auto-apply moved to 'review' too."""
    from cairn_matcher.pipeline import db
    from cairn_matcher.pipeline.runner import canonical_pair
    from cairn_matcher.pipeline.sweep import sweep

    doe, prior = _seed_forced_review_pair(pg_conn)
    seed_identity_pending(pg_conn, doe)
    assert sweep(pg_conn).errors == []
    low, high = canonical_pair(doe, prior)
    with pg_conn.cursor() as cur:  # what auto_apply.rs's veto kick leaves behind
        cur.execute("UPDATE match_proposal SET status='review' "
                    "WHERE patient_low=%s AND patient_high=%s", (low, high))
    pg_conn.commit()

    _fully_identify(pg_conn, doe)
    generated, _ = db.generate_candidate_pairs(pg_conn)
    pg_conn.rollback()
    assert (low, high) not in set(generated), "setup: the pair must have left blocking"

    result = sweep(pg_conn)
    assert result.errors == []
    assert result.reconciled_retracted == 1
    assert _proposal_status(pg_conn, low, high) == "retracted"


def test_a_review_row_is_retracted_once_the_doe_is_identified(pg_conn):
    """#743 part 1, the main-loop path: propose() withdraws a 'review' row that bands None."""
    from cairn_matcher.pipeline.runner import canonical_pair, propose

    doe, prior = _seed_forced_review_pair(pg_conn)
    seed_identity_pending(pg_conn, doe)
    propose(pg_conn, doe, prior)
    low, high = canonical_pair(doe, prior)
    with pg_conn.cursor() as cur:
        cur.execute("UPDATE match_proposal SET status='review' "
                    "WHERE patient_low=%s AND patient_high=%s", (low, high))
    pg_conn.commit()
    _identify(pg_conn, doe)
    assert propose(pg_conn, doe, prior) is None
    assert _proposal_status(pg_conn, low, high) == "retracted"
```

  Keep `test_retraction_preserves_a_human_disposition` unchanged; `accepted` must still be preserved.

- [ ] **Step 2: Run them to see them fail**

  Run: `cd matcher && uv run --extra pipeline pytest tests/test_check_chart.py tests/test_proposal_retraction.py -q`

  Expected: the three new tests FAIL (status stays `review`).

- [ ] **Step 3: Implement**

  In `queue_db.py`, after its docstring. `queue_db.py` is a PURE module: it imports nothing, and must
  never import `db.py`, the one module that imports psycopg at import time
  (`test_pure_modules_import_without_psycopg.py`).

```python
# The statuses the MATCHER may still revise: no human has decided them. 'pending' is the
# matcher's own proposal; 'review' is auto_apply.rs's machine kick (a veto appeared, or another
# writer's un-attested unlink stands — ADR-0078). A human's 'accepted'/'rejected'/'applied' and
# the matcher's 'auto_applied' are never revised here (#743 part 1).
AWAITING_HUMAN = ("pending", "review")
```

  In `db.py`, add `from cairn_matcher.pipeline.queue_db import AWAITING_HUMAN` with its other
  `cairn_matcher` imports. Then:

  - `retract_pending_proposal`: change the SQL `AND status='pending'` to `AND status = ANY(%s)` and
    pass `list(AWAITING_HUMAN)` as the third parameter. In its docstring, replace "Only 'pending'
    rows transition" with "Only rows in AWAITING_HUMAN transition (pending, or auto_apply's
    'review' kick — #743 part 1)".
  - `pending_proposal_pairs`: `WHERE status = ANY(%s)` with `(list(AWAITING_HUMAN),)`. Docstring:
    "(status in AWAITING_HUMAN)".
  - `upsert_proposal`'s docstring: replace "a human's decision (accepted / rejected / applied /
    auto_applied / the C2b veto-driven 'review') is PRESERVED" with "a human's decision (accepted /
    rejected / applied), the matcher's auto_applied, and auto_apply's 'review' kick (a machine
    verdict — a re-run must not send the pair back to the auto band) are PRESERVED".

  In `queue_db.py`, `pending_pairs_involving`:

```python
def pending_pairs_involving(conn, patient) -> list[tuple[str, str]]:
    """Proposals involving `patient` that the matcher may still revise (AWAITING_HUMAN)."""
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_low::text, patient_high::text FROM match_proposal "
            "WHERE status = ANY(%s) AND (patient_low = %s::uuid OR patient_high = %s::uuid)",
            (list(AWAITING_HUMAN), patient, patient),
        )
        return [(lo, hi) for lo, hi in cur.fetchall()]
```

  ⚠️ Never the other way round: a top-level `from cairn_matcher.pipeline.db import …` in
  `queue_db.py` would import psycopg at import time and break CI's pure job, which no local
  `--extra pipeline` run can see.

- [ ] **Step 4: Run them to see them pass**

  Run: the whole matcher suite, `uv run --extra pipeline pytest -q`, **and**
  `CAIRN_ALLOW_DB_SKIP=1 uv run --isolated pytest -q` (CI's pure job).

  Expected: PASS in both.

- [ ] **Step 5: Commit**

```bash
git add matcher/
# message: "fix(#743): the matcher re-assesses and retracts 'review' rows too (Refs #680)"
```

---

### Task 4: `candidates_by_id` — the search's candidate read, extracted

**Files:**
- Create: `crates/cairn-node/src/patient/candidate_read.rs`
- Modify: `crates/cairn-node/src/patient/search.rs`, `crates/cairn-node/src/patient/mod.rs`
- Test: `crates/cairn-node/tests/candidates_by_id.rs`

**Interfaces:**
- Produces:
  - `pub struct DisplayFacts` (private fields).
  - `pub async fn read_display_facts<C: GenericClient + Sync>(client: &C, ids: &[Uuid]) -> anyhow::Result<DisplayFacts>`.
  - `impl DisplayFacts { pub fn dobs(&self) -> &HashMap<Uuid, (String, String)>; pub fn candidate(&self, id: Uuid, today: &str) -> (Candidate, bool /* name unreadable */) }`.
  - `pub async fn candidates_by_id<C: GenericClient + Sync>(client: &C, ids: &[Uuid], today: &str) -> anyhow::Result<Vec<Candidate>>`, in input order, one per id.

- [ ] **Step 1: Write the failing test** — `crates/cairn-node/tests/candidates_by_id.rs`.

```rust
//! R5b Task 4: the worklist builds the SAME candidate the search does, through the read the
//! search now calls. DB-gated on $CAIRN_TEST_PG; serialized via `db::test_serial_guard`.
mod common;
use cairn_node::db;
use cairn_node::patient::candidate_read::candidates_by_id;
use cairn_node::patient::search::search_patients;
use cairn_patient_search::SearchQuery;
use common::{chart_named, cs, setup};
use uuid::Uuid;

#[tokio::test]
async fn candidates_by_id_equal_what_the_search_returns() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_link", "person_member"]).await;
    let a = chart_named(&c, &sk, &kid, 10, "Wilhelmina Quarrington").await;
    let b = chart_named(&c, &sk, &kid, 20, "Wilhelmina Quarringdon").await;
    let today = "2026-10-08";
    let list = search_patients(&c, &SearchQuery::new("Wilhelmina", None, &[]), today)
        .await
        .unwrap();
    let from_search: Vec<_> = list.charts().cloned().collect();
    let ids: Vec<Uuid> = from_search.iter().map(|c| c.patient_id).collect();
    assert!(ids.contains(&a) && ids.contains(&b), "setup: the search finds both");
    let by_id = candidates_by_id(&c, &ids, today).await.unwrap();
    assert_eq!(by_id, from_search, "one read, one candidate");
    // A chart this node has never heard of is still a candidate (never dropped) — named as
    // unknown, as the search names it.
    let ghost = Uuid::now_v7();
    let one = candidates_by_id(&c, &[ghost], today).await.unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].patient_id, ghost);
}
```

  (`list.charts()` returns an iterator of `&Candidate`; check `CandidateList` in
  `crates/cairn-patient-search/src/candidate.rs` and adapt if it returns a `Vec`.)

- [ ] **Step 2: Run it to see it fail**

  Run: `cargo test -p cairn-node --test candidates_by_id`

  Expected: compile error (no module `candidate_read`).

- [ ] **Step 3: Implement — move, do not rewrite.**
  - Create `candidate_read.rs` with a module doc: *"The display half of a candidate: name, age,
    trust, last activity, locale, photo. Shared by the search (`search.rs`) and the
    possible-duplicate worklist (R5b), so the two show one person the same way. Several small
    reads, joined in Rust — see `search.rs`'s module doc for why."*
  - **Move** (cut, not copy) from `search.rs` into it: `read_display_names`,
    `read_names_ever_asserted`, `read_dob`, `read_trust_states`, `read_last_activity`,
    `read_locale`, `read_photo_refs`, together with their doc comments.
  - Add:

```rust
/// Every display read for `ids`, done once. `dobs` is exposed because the search's ranking reads it
/// too (one query serves both).
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

pub async fn read_display_facts<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<DisplayFacts> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let names = read_display_names(client, ids).await?;
    // Only pay for the repudiated-vs-never-named distinction when a name is missing (#344 N4).
    let missing: Vec<Uuid> = ids.iter().filter(|id| !names.contains_key(id)).copied().collect();
    let ever_named = if missing.is_empty() {
        HashSet::new()
    } else {
        read_names_ever_asserted(client, &missing).await?
    };
    Ok(DisplayFacts {
        names,
        ever_named,
        held: person::read_held(client, &id_strs).await?,
        dobs: read_dob(client, ids).await?,
        trust_states: read_trust_states(client, ids).await?,
        last_activity: read_last_activity(client, ids).await?,
        locales: read_locale(client, ids).await?,
        photo_refs: read_photo_refs(client, ids).await?,
    })
}

impl DisplayFacts {
    pub fn dobs(&self) -> &HashMap<Uuid, (String, String)> {
        &self.dobs
    }

    /// One candidate, never dropped; `true` when its name could not be read (the search counts
    /// these into `incomplete`).
    pub fn candidate(&self, id: Uuid, today: &str) -> (Candidate, bool) {
        let name = display_name_for(id, &self.names, &self.ever_named, &self.held);
        let age = self.dobs.get(&id).and_then(|(dob, basis)| {
            age_years(dob, today).map(|years| Age { years, basis: basis.clone() })
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

/// One candidate per id, in `ids`' order — what the search would show for each.
pub async fn candidates_by_id<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
    today: &str,
) -> anyhow::Result<Vec<Candidate>> {
    let facts = read_display_facts(client, ids).await?;
    Ok(ids.iter().map(|id| facts.candidate(*id, today).0).collect())
}
```

  Match the moved functions' actual parameter types (`&[Uuid]` vs `&[String]`). Where a function
  took `&Vec<Uuid>` or `&[String]`, pass what it took; do not change its body.

  - In `search.rs`, replace the display reads and the `candidate_for` closure:
    - after `ids`/`id_strs` are built: `let facts = read_display_facts(client, &ids).await?;`
    - ranking uses `facts.dobs()` where it used `&dobs`;
    - `let mut candidate_for = |id: &Uuid| { let (c, unreadable) = facts.candidate(*id, today); if unreadable { unreadable_names += 1; } c };`
    - delete the now-unused reads and imports.
  - `patient/mod.rs`: add `pub mod candidate_read;` (alphabetically, before `candidate_text`).
    `display_name_for` and `trust_state_for` live in `search_person` (`pub(super)`), which is
    visible from a sibling module of `patient`. If not, widen to `pub(crate)`.

- [ ] **Step 4: Run it and every search suite**

  Run: `cargo test -p cairn-node --test candidates_by_id --test patient_search --test patient_search_ranking --test patient_search_equivalence --test patient_search_drift --test search_by_person --test search_path_pg_temp -- --nocapture`

  Expected: all PASS **unchanged** (no search test edited). This is the pin that the refactor moved
  code and changed nothing. Also run `cargo test -p cairn-node --lib` (unit tests in `search.rs`).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/patient crates/cairn-node/tests/candidates_by_id.rs
# message: "refactor(R5b): the search's candidate read is candidate_read.rs, shared with the worklist (Refs #680)"
```

---

### Task 5: the worklist reads — `duplicate_review/worklist.rs`

**Files:**
- Move: `crates/cairn-node/src/duplicate_review.rs` → `crates/cairn-node/src/duplicate_review/mod.rs` (`git mv`)
- Create: `crates/cairn-node/src/duplicate_review/worklist.rs`
- Test: `crates/cairn-node/tests/duplicate_worklist.rs`

**Interfaces:**
- Consumes: `person_charts` (`crate::patient::person`), `is_another_record` (sibling `mod.rs`).
- Produces (in `cairn_node::duplicate_review`):
  - `pub const DISPUTED_SQL: &str` — the one spelling of "an un-attested unlink stands for this
    open proposal", over `match_proposal_open`'s unqualified `patient_low` / `patient_high`.
- Produces (in `cairn_node::duplicate_review::worklist`):

```rust
pub struct ProposalRow { pub low: Uuid, pub high: Uuid, pub low_record: Uuid, pub high_record: Uuid,
                         pub band: String, pub status: String, pub vetoed: bool, pub disputed: bool,
                         pub created_ms: i64 }
pub struct WorklistEntry { pub newer_record: Uuid, pub older_record: Uuid, pub open_chart: Uuid,
                           pub older_chart: Uuid, pub pairs: Vec<(Uuid, Uuid)>, pub band: String,
                           pub vetoed: bool, pub disputed: bool, pub accepted: bool, pub newest_ms: i64 }
pub struct WorklistItem { pub entry: WorklistEntry, pub newer: ChartSet, pub older: ChartSet }
pub struct Worklist { pub items: Vec<WorklistItem>, pub total: usize }
pub fn registered_ms(id: Uuid) -> u64
pub fn group_by_record_pair(rows: &[ProposalRow]) -> Vec<WorklistEntry>
pub async fn worklist_count(client: &(impl GenericClient + Sync)) -> anyhow::Result<usize>
pub async fn worklist(client: &(impl GenericClient + Sync), limit: usize) -> anyhow::Result<Worklist>
```

  A record key is `COALESCE(person_member.person_id, chart)`: a never-linked chart is a record of
  one.

- [ ] **Step 1: Move the module**

  `git mv crates/cairn-node/src/duplicate_review.rs crates/cairn-node/src/duplicate_review/mod.rs`,
  then add `pub mod worklist;` after the `use` block and this constant:

```rust
/// "Another writer recorded these two as different people, without a clinician's confirmation"
/// (ADR-0078): an UN-attested unlink stands for the open proposal's pair. The ONE spelling — the
/// banner (`open_proposals_touching`) and the worklist both select it — written over
/// `match_proposal_open`'s own column names, so it reads the row the query is on.
pub const DISPUTED_SQL: &str = "EXISTS (SELECT 1 FROM patient_link pl \
     WHERE pl.low = patient_low AND pl.high = patient_high \
       AND pl.state = 'unlink' AND NOT pl.attested)";
```

  Run `cargo build -p cairn-node`. Expected: builds (the path is unchanged for callers).

- [ ] **Step 2: Write the failing pure tests** — at the bottom of the new `worklist.rs`.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A chart id whose UUIDv7 time is `ms`, with `n` telling ids of one millisecond apart.
    fn at(ms: u64, n: u128) -> Uuid {
        Uuid::from_u128(((ms as u128) << 80) | n)
    }
    fn row(low: Uuid, high: Uuid, lr: Uuid, hr: Uuid, created_ms: i64) -> ProposalRow {
        ProposalRow {
            low: low.min(high), high: low.max(high),
            low_record: if low < high { lr } else { hr },
            high_record: if low < high { hr } else { lr },
            band: "review".into(), status: "pending".into(),
            vetoed: false, disputed: false, created_ms,
        }
    }

    #[test]
    fn registered_ms_reads_the_uuidv7_time() {
        assert_eq!(registered_ms(at(1_700_000_000_000, 7)), 1_700_000_000_000);
    }

    #[test]
    fn two_members_against_one_record_are_one_entry_with_both_pairs() {
        let (a1, a2, b) = (at(10, 1), at(11, 2), at(50, 3));
        let rec_a = a1; // a1 and a2 are one record keyed a1
        let rows = vec![row(a1, b, rec_a, b, 100), row(a2, b, rec_a, b, 200)];
        let got = group_by_record_pair(&rows);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pairs.len(), 2);
        assert_eq!(got[0].newest_ms, 200);
    }

    #[test]
    fn the_newer_record_holds_the_most_recently_registered_chart_and_review_opens_it() {
        let (old, new) = (at(10, 1), at(99, 2));
        let got = group_by_record_pair(&[row(old, new, old, new, 5)]);
        assert_eq!(got[0].newer_record, new);
        assert_eq!(got[0].older_record, old);
        assert_eq!((got[0].open_chart, got[0].older_chart), (new, old));
        // Either orientation of the input row gives the same answer.
        assert_eq!(group_by_record_pair(&[row(new, old, new, old, 5)]), got);
    }

    #[test]
    fn a_registration_time_tie_goes_to_the_smaller_id() {
        let (x, y) = (at(10, 1), at(10, 2));
        let got = group_by_record_pair(&[row(x, y, x, y, 5)]);
        assert_eq!(got[0].newer_record, x);
    }

    #[test]
    fn a_pair_inside_one_record_is_not_an_entry() {
        let (x, y) = (at(10, 1), at(11, 2));
        assert!(group_by_record_pair(&[row(x, y, x, x, 5)]).is_empty());
    }

    #[test]
    fn flags_and_band_are_the_strongest_over_the_pairs() {
        let (a1, a2, b) = (at(10, 1), at(11, 2), at(50, 3));
        let mut r1 = row(a1, b, a1, b, 1);
        r1.band = "auto_candidate".into();
        r1.disputed = true;
        let mut r2 = row(a2, b, a1, b, 2);
        r2.status = "accepted".into();
        r2.vetoed = true;
        let got = &group_by_record_pair(&[r1, r2])[0];
        assert_eq!(got.band, "auto_candidate");
        assert!(got.vetoed && got.disputed && got.accepted);
    }

    #[test]
    fn entries_are_newest_first_and_input_order_does_not_matter() {
        let (p, q, r, s) = (at(1, 1), at(2, 2), at(3, 3), at(4, 4));
        let rows = vec![row(p, q, p, q, 10), row(r, s, r, s, 20)];
        let got = group_by_record_pair(&rows);
        assert_eq!(got[0].newest_ms, 20);
        let mut rev = rows.clone();
        rev.reverse();
        assert_eq!(group_by_record_pair(&rev), got);
    }
}
```

  (`ProposalRow` and `WorklistEntry` derive `Debug, Clone, PartialEq, Eq`.)

- [ ] **Step 3: Run them to see them fail**

  Run: `cargo test -p cairn-node --lib duplicate_review::worklist`

  Expected: compile errors (nothing defined yet).

- [ ] **Step 4: Implement the pure half**

```rust
//! The possible-duplicate worklist's node reads (repair path R5b, #680; design page "R5b — the
//! worklist, designed 2026-10-08"): the front door's tray, "Possible duplicates (N)".
//!
//! Every row comes from db/057's `match_proposal_open` — the ONE "still needs a human" predicate
//! the banner shares — and an entry is a PAIR OF RECORDS, not a proposal row: two charts of one
//! person proposed against one other person are one entry, counted once. A record's key is
//! `COALESCE(person_member.person_id, chart)` (a never-linked chart is a record of one), the same
//! `person_member` notion db/057 uses for "same record", so the count, the list and the view can
//! never disagree about who is one person.
//!
//! Two reads, split by cost: [`worklist_count`] is ONE statement (the front door pays for it on
//! every show); [`worklist`] reads the rows, groups them here ([`group_by_record_pair`], pure),
//! and reads the two records of only the entries it will show.
use super::{is_another_record, DISPUTED_SQL};
use crate::patient::person::person_charts;
use anyhow::Context;
use cairn_medication_view::ChartSet;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// The millisecond a UUIDv7 chart id was minted — its registration time, as the registering
/// node's clock claimed it. Used only to decide which record Review opens; never shown.
pub fn registered_ms(id: Uuid) -> u64 {
    (id.as_u128() >> 80) as u64
}

/// The band's strength, for "the strongest of the pairs".
fn band_rank(band: &str) -> u8 {
    match band {
        "auto_candidate" => 2,
        "review" => 1,
        _ => 0,
    }
}

/// Group open proposals into entries, one per pair of RECORDS; newest proposal first. **Pure.**
///
/// - A row whose two record keys are equal is no entry (they read as one record — db/057 already
///   drops it; kept here so a direct caller cannot be wrong).
/// - **Newer record**: the record holding the chart with the latest UUIDv7 time among the charts
///   the entry's pairs name; a tie goes to the SMALLER chart id.
/// - **`open_chart`**: the newer record's side of the entry's NEWEST proposal (what Review opens);
///   `older_chart` its other side. A `created_ms` tie picks the smaller `(low, high)`.
/// - `band` the strongest; `vetoed` / `disputed` / `accepted` if ANY pair is.
/// - Ordering: `newest_ms` descending, then `(newer_record, older_record)` ascending — so the same
///   rows in any order give the same entries.
pub fn group_by_record_pair(rows: &[ProposalRow]) -> Vec<WorklistEntry> {
    let mut groups: BTreeMap<(Uuid, Uuid), Vec<&ProposalRow>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.low_record != r.high_record) {
        let key = (r.low_record.min(r.high_record), r.low_record.max(r.high_record));
        groups.entry(key).or_default().push(r);
    }
    let mut entries: Vec<WorklistEntry> = groups.into_values().map(entry_of).collect();
    entries.sort_by(|a, b| {
        b.newest_ms
            .cmp(&a.newest_ms)
            .then_with(|| (a.newer_record, a.older_record).cmp(&(b.newer_record, b.older_record)))
    });
    entries
}

/// One entry from its (non-empty) group of rows. **Pure.**
fn entry_of(group: Vec<&ProposalRow>) -> WorklistEntry {
    // Every chart the pairs name, with its record.
    let charts = group
        .iter()
        .flat_map(|r| [(r.low, r.low_record), (r.high, r.high_record)]);
    let (_, newer_record) = charts
        .max_by_key(|(chart, _)| (registered_ms(*chart), Reverse(*chart)))
        .expect("a group has at least one row");
    let newest = group
        .iter()
        .max_by_key(|r| (r.created_ms, Reverse((r.low, r.high))))
        .expect("a group has at least one row");
    let (open_chart, older_chart, older_record) = if newest.low_record == newer_record {
        (newest.low, newest.high, newest.high_record)
    } else {
        (newest.high, newest.low, newest.low_record)
    };
    let mut pairs: Vec<(Uuid, Uuid)> = group.iter().map(|r| (r.low, r.high)).collect();
    pairs.sort();
    pairs.dedup();
    WorklistEntry {
        newer_record,
        older_record,
        open_chart,
        older_chart,
        pairs,
        band: group
            .iter()
            .max_by_key(|r| band_rank(&r.band))
            .map(|r| r.band.clone())
            .unwrap_or_default(),
        vetoed: group.iter().any(|r| r.vetoed),
        disputed: group.iter().any(|r| r.disputed),
        accepted: group.iter().any(|r| r.status == "accepted"),
        newest_ms: newest.created_ms,
    }
}
```

  Add the doc-commented struct definitions from *Interfaces* above `group_by_record_pair`.

- [ ] **Step 5: Run the pure tests to see them pass**

  Run: `cargo test -p cairn-node --lib duplicate_review::worklist`

  Expected: PASS.

- [ ] **Step 6: Write the failing DB tests** — `crates/cairn-node/tests/duplicate_worklist.rs`.

```rust
//! Repair path R5b (#680): the worklist's node reads over db/057's `match_proposal_open`.
//! DB-gated on $CAIRN_TEST_PG; serialized via `db::test_serial_guard`; keys minted at runtime.
mod common;
use cairn_node::chart_link::LinkVerb;
use cairn_node::db;
use cairn_node::duplicate_review::worklist::{worklist, worklist_count};
use common::{
    apply_remote_attested, apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair,
    seed_proposal, setup, submit_link_event, vetoed_pair,
};
use uuid::Uuid;

const TABLES: [&str; 5] = [
    "patient_link", "person_member", "identity_projection_flag", "link_veto_flag", "match_proposal",
];

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// Review Focus 2: two of one record's charts against one other record are ONE entry, counted
/// once — and the count statement agrees with the list.
#[tokio::test]
async fn two_members_against_one_record_count_once() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, m, x) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, m).await;
    register_pair(&c, &sk, &kid, x, Uuid::now_v7()).await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await; // one record: a + m
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, m, x, "review").await;
    assert_eq!(worklist_count(&c).await.unwrap(), 1);
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(w.total, 1);
    assert_eq!(w.items.len(), 1);
    assert_eq!(w.items[0].entry.pairs.len(), 2);
}

/// The newer record is the one holding the latest-minted chart; Review opens it.
#[tokio::test]
async fn review_opens_the_newer_record() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let old = Uuid::now_v7();
    std::thread::sleep(std::time::Duration::from_millis(3)); // a later UUIDv7 millisecond
    let new = Uuid::now_v7();
    register_pair(&c, &sk, &kid, old, new).await;
    seed_proposal(&c, old, new, "pending").await;
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(w.items[0].entry.open_chart, new);
    assert!(w.items[0].newer.contains(&new) && w.items[0].older.contains(&old));
}

/// A peer's ATTESTED unlink, arriving by sync, clears the entry — at read time, no status write.
/// An UN-attested one leaves it, flagged `disputed` (ADR-0078).
#[tokio::test]
async fn only_an_attested_unlink_clears_the_entry_and_an_unattested_one_is_a_dispute() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "pending").await;
    let agent = link_assertion_event(&kid, a, b, LinkVerb::Unlink, now_ms(), 0, "peer", false);
    apply_remote_raw(&c, &sk, agent).await.unwrap();
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(w.total, 1);
    assert!(w.items[0].entry.disputed);
    let human = link_assertion_event(&kid_h, a, b, LinkVerb::Unlink, now_ms() + 1, 0, "peer", true);
    apply_remote_attested(&c, &sk_h, human, &sk_h, &kid_h).await.unwrap();
    assert_eq!(worklist_count(&c).await.unwrap(), 0);
    assert_eq!(worklist(&c, 20).await.unwrap().total, 0);
}

/// `accepted` and `vetoed` each come from their own fixture; a pair in one record is never counted.
#[tokio::test]
async fn flags_and_the_one_record_case() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (v1, v2) = vetoed_pair(&c, &sk, &kid).await;
    seed_proposal(&c, v1, v2, "review").await;
    let (p, q) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, p, q).await;
    seed_proposal(&c, p, q, "accepted").await;
    let (s, t) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, s, t).await;
    submit_link_event(&c, &sk, &kid, s, t, 10, true).await;
    seed_proposal(&c, s, t, "pending").await; // one record: never counted
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(worklist_count(&c).await.unwrap(), 2);
    assert_eq!(w.total, 2);
    let of = |chart: Uuid| w.items.iter().find(|i| i.entry.pairs.iter().any(|p| p.0 == chart || p.1 == chart)).unwrap();
    assert!(of(v1).entry.vetoed && !of(v1).entry.accepted);
    assert!(of(p).entry.accepted && !of(p).entry.vetoed);
}

/// `limit` bounds the entries READ IN FULL, never `total`.
#[tokio::test]
async fn the_limit_bounds_the_items_not_the_total() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    for _ in 0..3 {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk, &kid, a, b).await;
        seed_proposal(&c, a, b, "pending").await;
    }
    let w = worklist(&c, 2).await.unwrap();
    assert_eq!((w.items.len(), w.total), (2, 3));
    assert_eq!(worklist_count(&c).await.unwrap(), 3);
}
```

- [ ] **Step 7: Run them to see them fail**

  Run: `cargo test -p cairn-node --test duplicate_worklist -- --nocapture`

  Expected: compile error (`worklist`, `worklist_count` not defined).

- [ ] **Step 8: Implement the reads** — in `worklist.rs`.

```rust
/// The open proposals with both sides' record keys — the ONE spelling of a record key, shared
/// by the count and the list. A record key is `COALESCE(person_member.person_id, chart)`.
const RECORDS_FROM: &str = "\
      FROM match_proposal_open \
      LEFT JOIN person_member a ON a.patient_id = patient_low \
      LEFT JOIN person_member b ON b.patient_id = patient_high";
const LOW_RECORD: &str = "COALESCE(a.person_id, patient_low)";
const HIGH_RECORD: &str = "COALESCE(b.person_id, patient_high)";

/// "Possible duplicates (N)": the number of distinct pairs of RECORDS with an open proposal.
/// ONE statement, no per-chart reads, and deliberately NO veto or dispute column — the front
/// door pays for this on every show, and `cairn_match_veto` per row is the list's cost, never
/// the count's.
pub async fn worklist_count(client: &(impl GenericClient + Sync)) -> anyhow::Result<usize> {
    let sql = format!(
        "SELECT count(*) FROM (SELECT DISTINCT LEAST(lr, hr), GREATEST(lr, hr) \
           FROM (SELECT {LOW_RECORD} AS lr, {HIGH_RECORD} AS hr {RECORDS_FROM}) r \
          WHERE lr <> hr) s"
    );
    let n: i64 = client
        .query_one(&sql, &[])
        .await
        .context("counting the open possible duplicates")?
        .get(0);
    Ok(n as usize)
}

/// The worklist: every entry grouped, the newest `limit` read in full (both records).
///
/// `total` is the number of entries at read time. An entry whose two records have meanwhile
/// come to share a chart (a link landing between this function's reads) is not shown; `total`
/// still counts it, so "N more" can be one high in that race — the next read is exact.
pub async fn worklist(
    client: &(impl GenericClient + Sync),
    limit: usize,
) -> anyhow::Result<Worklist> {
    let sql = format!(
        "SELECT patient_low::text AS low, patient_high::text AS high, \
                ({LOW_RECORD})::text AS low_record, ({HIGH_RECORD})::text AS high_record, \
                band, status, \
                EXISTS (SELECT 1 FROM cairn_match_veto(patient_low, patient_high)) AS vetoed, \
                {DISPUTED_SQL} AS disputed, \
                (extract(epoch FROM created_at) * 1000)::bigint AS created_ms \
         {RECORDS_FROM} \
         ORDER BY created_at DESC, patient_low, patient_high"
    );
    let rows = client
        .query(&sql, &[])
        .await
        .context("reading the open possible duplicates")?;
    let parse = |r: &tokio_postgres::Row, col: &str| -> anyhow::Result<Uuid> {
        Ok(r.get::<_, String>(col).parse()?)
    };
    let rows: Vec<ProposalRow> = rows
        .iter()
        .map(|r| {
            Ok(ProposalRow {
                low: parse(r, "low")?,
                high: parse(r, "high")?,
                low_record: parse(r, "low_record")?,
                high_record: parse(r, "high_record")?,
                band: r.get("band"),
                status: r.get("status"),
                vetoed: r.get("vetoed"),
                disputed: r.get("disputed"),
                created_ms: r.get("created_ms"),
            })
        })
        .collect::<anyhow::Result<_>>()?;
    let entries = group_by_record_pair(&rows);
    let total = entries.len();
    let mut items = Vec::new();
    for entry in entries.into_iter().take(limit) {
        let newer = person_charts(client, entry.open_chart).await?;
        let older = person_charts(client, entry.older_chart).await?;
        if is_another_record(&older, &newer) {
            items.push(WorklistItem { entry, newer, older });
        }
    }
    Ok(Worklist { items, total })
}
```

  ⚠️ `format!` inlines the `const`s by name (`{LOW_RECORD}` etc.; Rust 2021 captures any identifier
  in scope, consts included). The count and the list share `RECORDS_FROM`, `LOW_RECORD` and
  `HIGH_RECORD`, so they cannot disagree about who is one record.
  `two_members_against_one_record_count_once` and `the_limit_bounds_the_items_not_the_total` pin
  `worklist_count == worklist(..).total`. Check that the column names `band`,
  `status` and `created_at` are unambiguous with `person_member` joined (`person_member` has
  `patient_id`, `person_id`, `updated_at`; `match_proposal_open` has no `updated_at` clash because
  `updated_at` is not selected). If Postgres reports an ambiguity, alias the view
  `match_proposal_open mp` and qualify **every** column, then rewrite `DISPUTED_SQL`'s
  `patient_low`/`patient_high` the same way in BOTH places. One spelling must stay one spelling.

- [ ] **Step 9: Run the DB tests to see them pass**

  Run: `cargo test -p cairn-node --test duplicate_worklist --test duplicate_review --test match_proposal_open -- --nocapture`

  Expected: PASS, no `skipped:` line.

- [ ] **Step 10: Commit**

```bash
git add crates/cairn-node/src/duplicate_review crates/cairn-node/tests/duplicate_worklist.rs
# message: "feat(R5b): the worklist's node reads — one count statement, entries per pair of records (Refs #680)"
```

---

### Task 6: the banner's flags, and "Different people" refuses an accepted pair (#736)

**Files:**
- Modify: `crates/cairn-node/src/duplicate_review/mod.rs`
- Test: `crates/cairn-node/tests/duplicate_review_flags.rs` (create)

**Interfaces:**
- Produces: `OpenProposal { …, accepted: bool, disputed: bool }`;
  `PossibleDuplicate { …, accepted: bool, disputed: bool }`;
  `DifferentPeople::AcceptedAsSame`;
  `pub async fn accepted_pairs_between(client, left: &ChartSet, right: &ChartSet) -> anyhow::Result<Vec<(Uuid, Uuid)>>`.

- [ ] **Step 1: Write the failing tests** — `crates/cairn-node/tests/duplicate_review_flags.rs`.

```rust
//! R5b (#680, #736, ADR-0078): the banner's `accepted` and `disputed` flags, and "Different
//! people" refusing to overrule an earlier human's "same person". DB-gated on $CAIRN_TEST_PG.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::duplicate_review::{possible_duplicates, record_different_people, DifferentPeople};
use common::{
    apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair, seed_proposal, setup,
};
use uuid::Uuid;

const TABLES: [&str; 5] = [
    "patient_link", "person_member", "identity_projection_flag", "link_veto_flag", "match_proposal",
];

#[tokio::test]
async fn the_banner_carries_accepted_and_disputed() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b, d) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    register_pair(&c, &sk, &kid, d, Uuid::now_v7()).await;
    seed_proposal(&c, a, b, "accepted").await;
    seed_proposal(&c, a, d, "pending").await;
    let unlink = link_assertion_event(&kid, a, d, LinkVerb::Unlink, 50, 0, "peer", false);
    apply_remote_raw(&c, &sk, unlink).await.unwrap();
    let got = possible_duplicates(&c, &ChartSet::single(a)).await.unwrap();
    let to = |chart: Uuid| got.iter().find(|e| e.other_record.contains(&chart)).unwrap();
    assert!(to(b).accepted && !to(b).disputed);
    assert!(to(d).disputed && !to(d).accepted);
}

/// Review Focus 3: even if the webview sent it, "Different people" never overrules an accepted
/// "same person" — nothing is signed.
#[tokio::test]
async fn different_people_refuses_an_accepted_pair_and_signs_nothing() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "accepted").await;
    let before: i64 = c.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);
    let reviewer = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let out = record_different_people(
        &mut c, &ChartSet::single(a), &ChartSet::single(b), &reviewer, "testnode",
    )
    .await
    .unwrap();
    assert!(matches!(out, DifferentPeople::AcceptedAsSame));
    let after: i64 = c.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);
    assert_eq!(after, before);
}
```

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test -p cairn-node --test duplicate_review_flags`

  Expected: compile errors (no `accepted`, `disputed` or `AcceptedAsSame`).

- [ ] **Step 3: Implement** — `duplicate_review/mod.rs`.
  - `OpenProposal`: add `pub accepted: bool` ("a human already said 'same person' through C2;
    `apply_accepted_proposal` has not yet run (#736)") and `pub disputed: bool` ("another writer's
    un-attested unlink stands for the pair — [`DISPUTED_SQL`], ADR-0078").
  - `open_proposals_touching`'s SELECT gains `status = 'accepted' AS accepted,` and
    `{DISPUTED_SQL} AS disputed,`. Build the SQL with `format!` or `replace`, as Task 5 does; reuse
    the constant, never re-spell it. Then read both into the struct.
  - `PossibleDuplicate`: add `pub accepted: bool`, `pub disputed: bool`. In `group_by_other_record`,
    initialise them from the first proposal and `|=` them in the `Some(g)` arm, as `vetoed` is.
  - The unit test helper `prop(...)` in this file's tests: add `accepted: false, disputed: false`.
  - Add:

```rust
/// The open pairs between two records that a human has already ACCEPTED as the same person
/// (status `accepted`, #736). "Different people" must never overrule one.
pub async fn accepted_pairs_between(
    client: &(impl GenericClient + Sync),
    left: &ChartSet,
    right: &ChartSet,
) -> anyhow::Result<Vec<(Uuid, Uuid)>> {
    let rows = client
        .query(
            "SELECT patient_low::text, patient_high::text FROM match_proposal_open \
              WHERE status = 'accepted' AND \
                ((patient_low = ANY($1::text[]::uuid[]) AND patient_high = ANY($2::text[]::uuid[])) \
              OR (patient_low = ANY($2::text[]::uuid[]) AND patient_high = ANY($1::text[]::uuid[]))) \
              ORDER BY 1, 2",
            &[&ids(left), &ids(right)],
        )
        .await
        .context("reading whether a human already accepted these as the same person")?;
    rows.iter()
        .map(|r| Ok((r.get::<_, String>(0).parse()?, r.get::<_, String>(1).parse()?)))
        .collect()
}
```

  - `DifferentPeople`: add

```rust
    /// A human has already accepted (some of) these pairs as the SAME person (#736); that
    /// judgement awaits linking and is not overruled from here. Nothing was signed.
    AcceptedAsSame,
```

  - `record_different_people`: right after `open_pairs_between`'s empty check, add

```rust
    if !accepted_pairs_between(&*client, left, right).await?.is_empty() {
        return Ok(DifferentPeople::AcceptedAsSame);
    }
```

    This happens before any signature, so the `Err`-before-first-signature contract holds.

- [ ] **Step 4: Run them to see them pass, and the R5a suites**

  Run: `cargo test -p cairn-node --test duplicate_review_flags --test duplicate_review --test duplicate_worklist -- --nocapture` and `cargo test -p cairn-node --lib duplicate_review`

  Expected: PASS. The window tree will not compile until Task 7 (`DifferentPeople` is matched
  exhaustively there). That is expected; Task 7 follows directly.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/duplicate_review crates/cairn-node/tests/duplicate_review_flags.rs
# message: "feat(R5b): the banner's accepted and disputed flags; Different people never overrules an accepted pair (Refs #680, Refs #736)"
```

---

### Task 7: the banner's wording — #736 and the dispute note

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/duplicates/view.rs`, `view_tests.rs`, `mod.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/duplicates.js`

**Interfaces:**
- Produces:
  - `pub struct EntryFlags { pub vetoed: bool, pub accepted: bool, pub disputed: bool }`.
  - `entry_view(review_chart, flags: EntryFlags, identities, meds)`, replacing the `vetoed: bool`
    argument.
  - `DuplicateEntryView::offers_different_people: bool`.
  - `pub const ACCEPTED_HEADING`, `pub const DISPUTED_NOTE`, `pub const ACCEPTED_NOT_OVERRULED`.
    Task 8 reuses the first two.

- [ ] **Step 1: Write the failing goldens** — in `view_tests.rs` (find how existing tests call
  `entry_view` and follow that style):

```rust
#[test]
fn an_accepted_entry_is_worded_by_its_status_and_offers_no_different_people() {
    let flags = EntryFlags { vetoed: false, accepted: true, disputed: false };
    let v = entry_view(Uuid::from_u128(9), flags, Ok(vec![]), Err("x".into()));
    assert_eq!(v.heading, "Accepted as the same person — not yet linked");
    assert!(!v.offers_different_people);
}

#[test]
fn a_disputed_entry_says_so_and_still_offers_both_judgements() {
    let flags = EntryFlags { vetoed: false, accepted: false, disputed: true };
    let v = entry_view(Uuid::from_u128(9), flags, Ok(vec![]), Err("x".into()));
    assert_eq!(v.heading, "Possible duplicate — not yet reviewed");
    assert!(v.notes.contains(
        &"Recorded as not the same person, without a clinician's confirmation on record here."
            .to_string()
    ));
    assert!(v.offers_different_people);
}
```

  Update every existing `entry_view(…, true|false, …)` call in `view_tests.rs` and `mod.rs`'s tests
  to `EntryFlags { vetoed: <same bool>, accepted: false, disputed: false }`. Their expected text must
  not change. That is the byte-identical pin for a plain entry.

  In `mod.rs`'s tests, add a fixture-mode test that `DifferentPeople::AcceptedAsSame` maps to
  `refused(ACCEPTED_NOT_OVERRULED)` with `Retry::Never`. If the mapping is a private match arm with
  no seam, extract `fn different_people_view(outcome: DifferentPeople) -> Result<LinkReportView,
  ErrorView>` (pure) from `different_people_impl` and test that instead.

- [ ] **Step 2: Run them to see them fail**

  Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri duplicates`

  Expected: compile errors.

- [ ] **Step 3: Implement** — `view.rs`.

```rust
/// An accepted pair's heading (#736): a human already said "same person" through C2, and the
/// link has not been applied yet. "Not yet reviewed" would be false.
pub const ACCEPTED_HEADING: &str = "Accepted as the same person — not yet linked";
/// Another writer's un-attested unlink stands for the pair (ADR-0078). Shown, never hidden: the
/// disagreement is exactly what the human should see (principle 4).
pub const DISPUTED_NOTE: &str =
    "Recorded as not the same person, without a clinician's confirmation on record here.";
/// "Different people" on a pair a human already accepted as the same person (#736).
pub const ACCEPTED_NOT_OVERRULED: &str = "a clinician has already accepted these as the same \
     person, and that judgement is not overruled from here — nothing was done";

/// What the node read says about one entry's pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntryFlags {
    pub vetoed: bool,
    pub accepted: bool,
    pub disputed: bool,
}
```

  - `DuplicateEntryView`: add `pub offers_different_people: bool` with the doc *"`false` for an
    accepted pair: "Different people" would silently overrule an earlier human (#736). The backend
    refuses it anyway (`AcceptedAsSame`); this only hides the button."*
  - `entry_view(review_chart: Uuid, flags: EntryFlags, identities, meds)`:
    - `if flags.vetoed { notes.push(VETO_NOTE.into()) }`;
    - `if flags.disputed { notes.push(DISPUTED_NOTE.into()) }`;
    - `heading: if flags.accepted { ACCEPTED_HEADING } else { HEADING }.into()`;
    - `offers_different_people: !flags.accepted`.
  - `mod.rs` `duplicate_section`: `entry_view(entry.review_chart, EntryFlags { vetoed: entry.vetoed,
    accepted: entry.accepted, disputed: entry.disputed }, ids, meds)`.
  - `mod.rs` `different_people_impl`'s match: add
    `DifferentPeople::AcceptedAsSame => Err(refused(ACCEPTED_NOT_OVERRULED)),`.
  - `duplicates.js`:
    - `duplicateItem`'s Review click calls `reviewDuplicate(entry)` (pass the whole entry).
    - `reviewDuplicate(entry)` uses `entry.review_chart` where it used `otherId`.
    - The last line becomes
      `el("link-different").hidden = !(entry.offers_different_people && token === compareToken && compared !== null);`.
    - Update the function's doc comment: *"…shown only when the entry offers it (not on an accepted
      pair, #736) and THIS comparison…"*.

- [ ] **Step 4: Run them to see them pass**

  Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri`

  Expected: PASS, including `duplicates_js_reads_no_field_the_backend_does_not_send`
  (`offers_different_people` is now read and sent).

- [ ] **Step 5: Commit**

```bash
git add cairn-gui/cairn-gui-tauri/src/duplicates cairn-gui/cairn-gui-tauri/src-ui/duplicates.js
# message: "feat(R5b): the banner words an accepted pair by its status and shows a dispute (Refs #680, Refs #736)"
```

---

### Task 8: the tray's Rust — `worklist/view.rs` and `worklist/mod.rs`

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/worklist/view.rs`, `view_tests.rs`, `mod.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs`

**Interfaces:**
- Consumes: `cairn_node::duplicate_review::worklist::{worklist, worklist_count, Worklist}`;
  `cairn_node::patient::candidate_read::candidates_by_id`;
  `cairn_node::duplicate_check::{read_snapshot, classify, status_line, CheckState, STALLED_AFTER_SECS}`;
  `crate::funnel::rows::{person_row_view, PersonRowView}`;
  `crate::duplicates::view::{ACCEPTED_HEADING, DISPUTED_NOTE, HEADING, VETO_NOTE}`.
- Produces:
  - Tauri commands `duplicate_tray_count() -> TrayCountView` and
    `duplicate_worklist() -> WorklistView`.
  - Payloads:

```rust
pub struct TrayCountView { pub summary: Option<String>, pub status_line: Option<String> }
pub struct WorklistView { pub entries: Vec<WorklistEntryView>, pub more: Option<String>, pub error: Option<String> }
pub struct WorklistEntryView { pub heading: String, pub newer_label: String, pub newer: SideView,
                               pub older_label: String, pub older: SideView, pub notes: Vec<String>,
                               pub open_chart: Option<String> }
pub struct SideView { pub row: Option<PersonRowView>, pub error: Option<String> }
```

- [ ] **Step 1: Write the failing goldens** — `worklist/view_tests.rs`.

```rust
use super::*;
use cairn_node::duplicate_check::CheckState;

#[test]
fn zero_on_a_current_node_hides_the_tray() {
    let v = tray_count_view(Ok(0), Ok(CheckState::Current { last_ran: None }));
    assert_eq!(v, TrayCountView { summary: None, status_line: None });
}

#[test]
fn zero_on_a_node_that_never_ran_is_shown_with_the_reason() {
    let state = CheckState::NeverRun { waiting: 0 };
    let v = tray_count_view(Ok(0), Ok(state.clone()));
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates (0)"));
    assert_eq!(v.status_line, Some(cairn_node::duplicate_check::status_line(&state)));
}

#[test]
fn a_count_on_a_current_node_has_no_status_line() {
    let v = tray_count_view(Ok(3), Ok(CheckState::Current { last_ran: None }));
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates (3)"));
    assert_eq!(v.status_line, None);
}

#[test]
fn a_failed_count_is_worded_never_hidden() {
    let v = tray_count_view(Err("boom".into()), Ok(CheckState::Current { last_ran: None }));
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates — could not be checked: boom"));
}

#[test]
fn an_unreadable_status_is_worded() {
    let v = tray_count_view(Ok(0), Err("gone".into()));
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates (0)"));
    assert_eq!(v.status_line.as_deref(), Some("Duplicate check status unknown: gone"));
}

#[test]
fn more_counts_the_entries_not_shown() {
    assert_eq!(more_line(20, 20), None);
    assert_eq!(more_line(20, 21).as_deref(), Some("1 more possible duplicate, older than these."));
    assert_eq!(more_line(20, 25).as_deref(), Some("5 more possible duplicates, older than these."));
}

#[test]
fn an_entry_names_both_sides_and_its_notes() {
    let flags = crate::duplicates::view::EntryFlags { vetoed: true, accepted: false, disputed: true };
    let v = entry_view(flags, "auto_candidate", Ok(None), Ok(None), None);
    assert_eq!(v.heading, "Possible duplicate — not yet reviewed");
    assert_eq!(v.newer_label, "Registered more recently");
    assert_eq!(v.older_label, "Already on file");
    assert_eq!(
        v.notes,
        vec![
            "The matcher rates this a strong match.".to_string(),
            crate::duplicates::view::VETO_NOTE.to_string(),
            crate::duplicates::view::DISPUTED_NOTE.to_string(),
        ]
    );
    assert_eq!(v.open_chart, None);
}

#[test]
fn an_accepted_entry_uses_the_banners_heading() {
    let flags = crate::duplicates::view::EntryFlags { accepted: true, ..Default::default() };
    let v = entry_view(flags, "review", Ok(None), Ok(None), None);
    assert_eq!(v.heading, crate::duplicates::view::ACCEPTED_HEADING);
}

#[test]
fn an_unreadable_side_is_worded_and_never_dropped() {
    let v = entry_view(Default::default(), "review", Err("nope".into()), Ok(None), None);
    assert_eq!(v.newer.row, None);
    assert_eq!(v.newer.error.as_deref(), Some("This record could not be read here: nope"));
}

#[test]
fn a_failed_list_is_an_error_line() {
    let v = worklist_view(Err("down".into()));
    assert_eq!(v.error.as_deref(), Some("Could not read the possible duplicates: down"));
    assert!(v.entries.is_empty());
}
```

  `VETO_NOTE` says "Review shows which". From the tray, Review opens the chart whose banner's Review
  shows them, so the sentence stays true. `entry_view`'s `Ok(None)` means "the side was not read"
  (only tests pass it). Production passes `Ok(Some(row))` or `Err(e)`.

- [ ] **Step 2: Run them to see them fail**

  Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri worklist`

  Expected: compile errors (module missing).

- [ ] **Step 3: Implement `view.rs`**

```rust
//! Every sentence the front door's possible-duplicate tray shows, as pure functions (repair path
//! R5b, #680; design page "R5b — the worklist, designed 2026-10-08").
//!
//! The tray is the records clerk's possible-duplicate tray: a collapsed `<details>` whose summary
//! counts the open pairs of records. It may be HIDDEN only when that is the truth — none open on a
//! node whose check is current; every failed read is a worded line (R5a's rule, one tray over).
use crate::duplicates::view::{EntryFlags, ACCEPTED_HEADING, DISPUTED_NOTE, HEADING, VETO_NOTE};
use crate::funnel::rows::PersonRowView;
use cairn_node::duplicate_check::{status_line, CheckState};
use serde::Serialize;

/// At most this many entries are drawn (each costs two record reads); the rest are counted.
pub const MAX_SHOWN: usize = 20;
pub const NEWER_LABEL: &str = "Registered more recently";
pub const OLDER_LABEL: &str = "Already on file";
pub const STRONG_NOTE: &str = "The matcher rates this a strong match.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrayCountView {
    /// The `<summary>` text; `None` hides the tray (checked, none open, node current).
    pub summary: Option<String>,
    /// Why "0" or a count may be incomplete (R4's sentence); `None` on a Current node.
    pub status_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SideView {
    pub row: Option<PersonRowView>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorklistEntryView {
    pub heading: String,
    pub newer_label: String,
    pub newer: SideView,
    pub older_label: String,
    pub older: SideView,
    pub notes: Vec<String>,
    /// The chart Review opens (the newer record's); `None` when that side could not be read, so
    /// it is not in `AppState::shown` and Review could only be refused.
    pub open_chart: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorklistView {
    pub entries: Vec<WorklistEntryView>,
    pub more: Option<String>,
    pub error: Option<String>,
}

/// The summary and status line. **Pure.**
pub fn tray_count_view(count: Result<usize, String>, status: Result<CheckState, String>) -> TrayCountView {
    let status_line = match &status {
        Ok(CheckState::Current { .. }) => None,
        Ok(state) => Some(status_line(state)),
        Err(e) => Some(format!("Duplicate check status unknown: {e}")),
    };
    match count {
        Err(e) => TrayCountView {
            summary: Some(format!("Possible duplicates — could not be checked: {e}")),
            status_line,
        },
        Ok(0) if status_line.is_none() => TrayCountView { summary: None, status_line: None },
        Ok(n) => TrayCountView { summary: Some(format!("Possible duplicates ({n})")), status_line },
    }
}

/// "N more …" for entries beyond those shown. **Pure.**
pub fn more_line(shown: usize, total: usize) -> Option<String> {
    match total.saturating_sub(shown) {
        0 => None,
        1 => Some("1 more possible duplicate, older than these.".into()),
        n => Some(format!("{n} more possible duplicates, older than these.")),
    }
}

fn side(read: Result<Option<PersonRowView>, String>) -> SideView {
    match read {
        Ok(row) => SideView { row, error: None },
        Err(e) => SideView { row: None, error: Some(format!("This record could not be read here: {e}")) },
    }
}

/// One entry. **Pure.** `open_chart` is `Some` only when the newer side was read.
pub fn entry_view(
    flags: EntryFlags,
    band: &str,
    newer: Result<Option<PersonRowView>, String>,
    older: Result<Option<PersonRowView>, String>,
    open_chart: Option<String>,
) -> WorklistEntryView {
    let mut notes = vec![];
    if band == "auto_candidate" {
        notes.push(STRONG_NOTE.into());
    }
    if flags.vetoed {
        notes.push(VETO_NOTE.into());
    }
    if flags.disputed {
        notes.push(DISPUTED_NOTE.into());
    }
    WorklistEntryView {
        heading: if flags.accepted { ACCEPTED_HEADING } else { HEADING }.into(),
        newer_label: NEWER_LABEL.into(),
        newer: side(newer),
        older_label: OLDER_LABEL.into(),
        older: side(older),
        notes,
        open_chart,
    }
}

/// The list, or why it could not be read. **Pure.** `Ok` carries `(entries, total)`.
pub fn worklist_view(read: Result<(Vec<WorklistEntryView>, usize), String>) -> WorklistView {
    match read {
        Err(e) => WorklistView {
            entries: vec![],
            more: None,
            error: Some(format!("Could not read the possible duplicates: {e}")),
        },
        Ok((entries, total)) => WorklistView { more: more_line(entries.len(), total), entries, error: None },
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
```

  `PersonRowView` must derive `PartialEq, Eq` for these derives. Add them in `funnel/rows.rs` if
  missing (`CandidateView` already has them).

- [ ] **Step 4: Run the goldens to see them pass**

  Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri worklist::view`

  Expected: PASS.

- [ ] **Step 5: Write the failing fixture tests** — in `worklist/mod.rs`'s `#[cfg(test)] mod tests`
  (look at `duplicates/mod.rs`'s tests for how an `AppState` in fixture mode is built, and reuse that
  helper or pattern):

```rust
#[tokio::test]
async fn fixture_mode_counts_one_and_says_the_check_never_ran() {
    let state = fixture_state();
    let v = tray_count_impl(&state).await;
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates (1)"));
    assert!(v.status_line.is_some());
}

/// Review Focus 5: the list re-admits its charts every time it is read — `close_chart` cleared
/// `shown`, and the tray's Review must still open the newer record.
#[tokio::test]
async fn reading_the_list_admits_review_s_chart_again_after_close() {
    let state = fixture_state();
    let list = worklist_impl(&state).await;
    let open = list.entries[0].open_chart.clone().expect("fixture newer side is read");
    crate::funnel::commands::close_chart_impl(&state).await; // clears `shown`
    assert!(crate::funnel::commands::open_chart_impl(&state, &open).await.is_err());
    worklist_impl(&state).await;
    assert!(crate::funnel::commands::open_chart_impl(&state, &open).await.is_ok());
}
```

- [ ] **Step 6: Implement `mod.rs`**

```rust
//! The front door's possible-duplicate tray (repair path R5b, #680): its two commands. Every
//! sentence is in `view.rs`; every DB rule in `cairn_node::duplicate_review::worklist` (DB-tested
//! there). This module orders the reads, builds each side's person row through the SAME candidate
//! read the search uses (`candidate_read::candidates_by_id`), and admits every shown chart to
//! `AppState::shown` — the worklist IS a list on screen, so the funnel's "only a chart a list
//! showed can be opened" rule holds unchanged. Review is the existing `open_chart`.
//!
//! LOCKING: one hold of `state.db` per command; nothing here calls `read_chart_of` /
//! `chart_set_of` (which take the lock themselves).
pub mod view;

use crate::duplicates::view::EntryFlags;
use crate::funnel::rows::person_row_view;
use crate::state::AppState;
use cairn_node::db_diagnosis::operator_chain;
use cairn_node::duplicate_check::{classify, read_snapshot, CheckState, STALLED_AFTER_SECS};
use cairn_node::duplicate_review::worklist::{worklist, worklist_count};
use cairn_node::patient::candidate_read::candidates_by_id;
use cairn_patient_search::{Candidate, PersonRow, TrustState};
use uuid::Uuid;
use view::{entry_view, tray_count_view, worklist_view, TrayCountView, WorklistView, MAX_SHOWN};

pub async fn tray_count_impl(state: &AppState) -> TrayCountView {
    let Some(db) = state.db.as_ref() else {
        return tray_count_view(Ok(1), Ok(CheckState::NeverRun { waiting: 0 }));
    };
    let db = db.lock().await;
    let count = worklist_count(&*db).await.map_err(|e| operator_chain(&e));
    let status = read_snapshot(&db)
        .await
        .map(|s| classify(&s, STALLED_AFTER_SECS))
        .map_err(|e| operator_chain(&e));
    tray_count_view(count, status)
}

pub async fn worklist_impl(state: &AppState) -> WorklistView {
    let Some(db) = state.db.as_ref() else {
        let (newer, older) = fixture_pair();
        admit(state, newer.iter().chain(older.iter())).await;
        let entry = entry_view(
            EntryFlags::default(),
            "review",
            Ok(PersonRow::new(newer.clone()).map(|r| person_row_view(&r))),
            Ok(PersonRow::new(older).map(|r| person_row_view(&r))),
            Some(newer[0].patient_id.to_string()),
        );
        return worklist_view(Ok((vec![entry], 1)));
    };
    let db = db.lock().await;
    let read = async {
        let today: String = db.query_one("SELECT current_date::text", &[]).await?.get(0);
        let list = worklist(&*db, MAX_SHOWN).await?;
        let mut entries = vec![];
        let mut admitted: Vec<Candidate> = vec![];
        for item in &list.items {
            let newer = read_side(&db, &item.newer, &today)
                .await
                .map_err(|e| operator_chain(&e));
            let older = read_side(&db, &item.older, &today)
                .await
                .map_err(|e| operator_chain(&e));
            let open_chart = newer.is_ok().then(|| item.entry.open_chart.to_string());
            for side in [&newer, &older] {
                if let Ok(cands) = side {
                    admitted.extend(cands.iter().cloned());
                }
            }
            let flags = EntryFlags {
                vetoed: item.entry.vetoed,
                accepted: item.entry.accepted,
                disputed: item.entry.disputed,
            };
            entries.push(entry_view(
                flags,
                &item.entry.band,
                newer.map(|c| PersonRow::new(c).map(|r| person_row_view(&r))),
                older.map(|c| PersonRow::new(c).map(|r| person_row_view(&r))),
                open_chart,
            ));
        }
        Ok::<_, anyhow::Error>((entries, list.total, admitted))
    }
    .await;
    drop(db);
    match read {
        Err(e) => worklist_view(Err(operator_chain(&e))),
        Ok((entries, total, admitted)) => {
            admit(state, admitted.iter()).await;
            worklist_view(Ok((entries, total)))
        }
    }
}

/// One side of an entry: every chart of that record, as the search would show it. A failure is
/// that side's alone — the caller words it, and the other side and the other entries stand.
async fn read_side(
    db: &tokio_postgres::Client,
    set: &cairn_medication_view::ChartSet,
    today: &str,
) -> anyhow::Result<Vec<Candidate>> {
    candidates_by_id(db, set.members(), today).await
}

/// Put the tray's charts on the list of charts `open_chart` will open.
async fn admit<'a>(state: &AppState, cands: impl Iterator<Item = &'a Candidate>) {
    let mut shown = state.shown.lock().await;
    for c in cands {
        shown.insert(c.patient_id, c.clone());
    }
}

/// `--mock`: one entry from the fixture population — FIXTURE_UUID as the newer record (it opens to
/// the fixture chart), the next fixture as the one on file. The mock has no link model (#722).
fn fixture_pair() -> (Vec<Candidate>, Vec<Candidate>) {
    let pop = cairn_gui_data::mock::fixtures::starting_population();
    let cand = |i: usize| Candidate {
        patient_id: pop[i].uuid,
        display_name: pop[i].display_name.clone(),
        age: None,
        trust: pop[i].trust,
        last_activity: None,
        locale: None,
        photo_ref: None,
    };
    (vec![cand(0)], vec![cand(1)])
}

#[tauri::command]
pub async fn duplicate_tray_count(state: tauri::State<'_, AppState>) -> Result<TrayCountView, ()> {
    Ok(tray_count_impl(&state).await)
}

#[tauri::command]
pub async fn duplicate_worklist(state: tauri::State<'_, AppState>) -> Result<WorklistView, ()> {
    Ok(worklist_impl(&state).await)
}
```

  Notes for the implementer:
  - `db` is a `tokio::sync::MutexGuard<tokio_postgres::Client>`; `&db` derefs to `&Client` where
    `read_side` asks for one (write `&*db` if inference balks). Never `?` out of the whole list for
    one side's failure: each side's `Err` is worded in its own `SideView`.
  - `PersonRow::new(Vec<Candidate>) -> Option<PersonRow>` (`None` for an empty vec). A read set is
    never empty (`person_charts` always contains the chart).
  - `TrustState` is `Copy` if `pop[i].trust` moves; otherwise `.clone()`.
  - `fixture_state()` in the tests: use whatever helper `duplicates/mod.rs`'s or
    `funnel/commands.rs`'s tests use to build a fixture-mode `AppState`.
  - `main.rs`: `mod worklist;` beside `mod duplicates;`, and add
    `worklist::duplicate_tray_count, worklist::duplicate_worklist,` to `generate_handler!`.

- [ ] **Step 7: Run all window tests to see them pass**

  Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri && cargo clippy -p cairn-gui-tauri --all-targets -- -D warnings`

  Expected: PASS, clippy clean.

- [ ] **Step 8: Commit**

```bash
git add cairn-gui/cairn-gui-tauri/src
# message: "feat(R5b): the tray's commands and every sentence it shows (Refs #680)"
```

---

### Task 9: the webview — the tray

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/index.html`, `src-ui/funnel.js`
- Create: `cairn-gui/cairn-gui-tauri/src-ui/worklist.js`
- Modify: `cairn-gui/cairn-gui-tauri/src/worklist/mod.rs` (the JS field guard)

**Interfaces:**
- Consumes: the `duplicate_tray_count` / `duplicate_worklist` payloads (Task 8); funnel.js's
  `openChart(patientId, statusId)` and `failureText`; main.js's `el`, `cell`, `setMessage`.
- Produces: `refreshTray()` (global), which funnel.js calls.

- [ ] **Step 1: Write the failing JS field guard** — in `worklist/mod.rs`'s tests, following
  `duplicates_js_reads_no_field_the_backend_does_not_send`:

```rust
/// worklist.js is untyped: a Rust field rename would draw an empty tray, not break the build.
#[test]
fn worklist_js_reads_no_field_the_backend_does_not_send() {
    use crate::commands::tests::fields_read_in;
    let js = include_str!("../../src-ui/worklist.js");
    let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
        v.as_object().unwrap().keys().cloned().collect()
    };
    let (newer, older) = fixture_pair();
    let row = crate::funnel::rows::person_row_view(&PersonRow::new(newer.clone()).unwrap());
    let entry = view::entry_view(EntryFlags::default(), "review", Ok(Some(row.clone())),
                                 Ok(PersonRow::new(older).map(|r| person_row_view(&r))), None);
    let counted = tray_count_view(Ok(1), Ok(CheckState::NeverRun { waiting: 0 }));
    let list = worklist_view(Ok((vec![entry.clone()], 1)));
    let member = crate::funnel::view::candidate_view(&newer[0]);
    for (binding, available) in [
        ("counted", keys(serde_json::to_value(&counted).unwrap())),
        ("list", keys(serde_json::to_value(&list).unwrap())),
        ("entry", keys(serde_json::to_value(&entry).unwrap())),
        ("side", keys(serde_json::to_value(&entry.newer).unwrap())),
        ("row", keys(serde_json::to_value(&row).unwrap())),
        ("member", keys(serde_json::to_value(&member).unwrap())),
    ] {
        let read = fields_read_in(js, binding);
        assert!(!read.is_empty(), "worklist.js no longer reads `{binding}` — rename it here");
        for field in read {
            assert!(available.contains(&field), "worklist.js reads `{binding}.{field}`, not sent");
        }
    }
}
```

  Run it. Expected: FAIL (`worklist.js` does not exist, so `include_str!` fails to compile).

- [ ] **Step 2: Write `src-ui/worklist.js`**

```js
// The front door's possible-duplicate tray (repair path R5b, #680). Words nothing and decides
// nothing: every sentence comes from Rust (`worklist/view.rs`), every rule from the backend. Classic
// script, loaded after duplicates.js and BEFORE funnel.js, whose `boot()` calls `refreshTray`
// (load order, never a typeof guard — R5a's rule). It uses funnel.js's `openChart` and
// `failureText` only at call time.
//
// The tray is a native <details>: closed by default, its <summary> the count. It stays open across
// visits to a chart (the element is never rebuilt), and an open tray re-reads its list on every
// return to the front door — `close_chart` cleared the backend's list of openable charts, and the
// list read admits them again.
"use strict";

/** Re-read the count (front-door show, and every return to it); re-read the list if open. */
async function refreshTray() {
  let counted;
  try {
    counted = await invoke("duplicate_tray_count");
  } catch (failure) {
    counted = { summary: failureText(failure), status_line: null };
  }
  const tray = el("duplicate-tray");
  tray.hidden = !counted.summary;
  el("duplicate-tray-summary").textContent = counted.summary || "";
  setMessage(el("duplicate-tray-status"), counted.status_line || "");
  if (!tray.hidden && tray.open) await loadTray();
}

/** Read and draw the list. */
async function loadTray() {
  let list;
  try {
    list = await invoke("duplicate_worklist");
  } catch (failure) {
    list = { entries: [], more: null, error: failureText(failure) };
  }
  setMessage(el("duplicate-tray-error"), list.error || "");
  el("duplicate-tray-list").replaceChildren(...list.entries.map(trayItem));
  setMessage(el("duplicate-tray-more"), list.more || "");
}

let trayLineCount = 0; // makes each side's first-line id unique

/**
 * One side of an entry: its label, then each chart as text. It is NOT an open target (the
 * maintainer's decision: Review opens the newer record). Returns the block and the id of its first
 * chart line, which describes Review: every entry's heading is the same sentence, so a heading
 * label would not tell entries apart (R5a's lesson, R3's person-row pattern).
 */
function traySide(label, side) {
  const div = document.createElement("div");
  div.append(cell("p", label));
  let firstId = null;
  if (side.error) div.append(cell("p", side.error));
  if (side.row) {
    const row = side.row;
    if (row.label) div.append(cell("p", row.label));
    const ul = document.createElement("ul");
    for (const member of row.members) {
      const li = cell("li", member.name + " — " + member.age + " — identity " + member.trust);
      if (firstId === null) {
        firstId = "tray-line-" + ++trayLineCount;
        li.id = firstId;
      }
      ul.append(li);
    }
    div.append(ul);
  }
  return { div, firstId };
}

/** One possible duplicate: both records, the notes, and Review. */
function trayItem(entry) {
  const li = document.createElement("li");
  const newer = traySide(entry.newer_label, entry.newer);
  const older = traySide(entry.older_label, entry.older);
  li.append(cell("h3", entry.heading), newer.div, older.div);
  for (const note of entry.notes) li.append(cell("p", note));
  if (entry.open_chart) {
    const review = document.createElement("button");
    review.type = "button";
    review.textContent = "Review";
    // A screen reader says "Review, <the newer record's first chart>".
    if (newer.firstId) review.setAttribute("aria-describedby", newer.firstId);
    review.addEventListener("click", () => openChart(entry.open_chart, "duplicate-tray-error"));
    li.append(review);
  }
  return li;
}

el("duplicate-tray").addEventListener("toggle", () => {
  if (el("duplicate-tray").open) void loadTray();
});
```

- [ ] **Step 3: `index.html` and `funnel.js`**

  In `index.html`, directly after `</form>` of `#register-form` and before `</section>` of
  `#front-door`:

```html
        <!-- The records clerk's possible-duplicate tray (R5b, #680). Below the find and register
             forms (DOM order is clinical: the desk's work comes first); closed by default, its
             summary the count. Never role="alert", never takes focus. Hidden only when nothing is
             open AND the duplicate check is current (worklist/view.rs). -->
        <details id="duplicate-tray" hidden>
          <summary id="duplicate-tray-summary"></summary>
          <p id="duplicate-tray-status" hidden></p>
          <p id="duplicate-tray-error" role="status" hidden></p>
          <ol id="duplicate-tray-list" aria-label="Possible duplicates, newest first"></ol>
          <p id="duplicate-tray-more" hidden></p>
        </details>
```

  Script tags: insert `<script src="worklist.js"></script>` between `duplicates.js` and `funnel.js`.

  In `funnel.js`:
  - `boot()`'s `else` branch, after `el("browse-name").focus();`: `void refreshTray();`
  - `closeChart()`, after `el("browse-name").focus();`: `void refreshTray();`

  (`setMessage` hides an empty node. Check `main.js:58` that it toggles `hidden` as R5a relies on.)

- [ ] **Step 4: Run the guard and the window suite**

  Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri`

  Expected: PASS, including the new guard and the existing `funnel.js` guards.

- [ ] **Step 5: Headless walk (controller; nothing committed)**

  Follow the webview mock-walk recipe:
  - Copy `src-ui` to the scratchpad.
  - Stub `window.__TAURI__.core.invoke` before `main.js`. Return Rust-shaped payloads:
    `duplicate_tray_count` → `{summary: "Possible duplicates (1)", status_line: null}`;
    `duplicate_worklist` → one entry; `funnel_status` → front door; `open_chart` → a header.
  - Serve it and drive it with Playwright MCP.

  Assert **visibility** (computed `display` with no hidden ancestor), never text alone:
  1. the tray is visible and comes after `#register-form` in DOM order;
  2. it is closed, and `#duplicate-tray-list` is not visible;
  3. opening it shows the entry, with "Registered more recently" before "Already on file";
  4. Review invokes `open_chart` with the entry's `open_chart`, and `#chart-view` becomes visible;
  5. "Find another patient" returns to the front door, and with the tray still open,
     `duplicate_worklist` is invoked again (count the stub's calls);
  6. with `summary: null` the tray is not visible;
  7. no element in the tray has `role="alert"`, and focus is never moved into it.

  Record the result in the as-built note (Task 10).

- [ ] **Step 6: Commit**

```bash
git add cairn-gui/cairn-gui-tauri/src-ui cairn-gui/cairn-gui-tauri/src/worklist/mod.rs
# message: "feat(R5b): the front door's possible-duplicate tray (Refs #680)"
```

---

### Task 10: runbook, docs, follow-ons, full gates, PR

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `results/TEMPLATE.md`
- Modify: the design page (R5b as-built note), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: RUNBOOK §12 and TEMPLATE.md**

  Add **"## 12. Clear the possible-duplicate tray (R5b, #680)"**, modelled on §11:
  - **Setup:** two near-duplicate charts proposed by `cairn-matcher watch` (or seeded), node Current.
  - **Measure:** from the front door with the tray closed, the time to a recorded judgement (expand,
    Review, read the banner, Same person / Different people). Budget **≤ 25 s per entry**. Repeat
    with the tray already open from a previous entry.
  - **Check that** find ≤ 5 s is unchanged with the tray present (section 8's measurement, re-run).
  - **VoiceOver and keyboard:**
    - the summary is announced with its count;
    - expanding needs no mouse;
    - Review announces the newer record's first identity line;
    - nothing in the tray takes focus or is announced as an alert.

  `TEMPLATE.md` gains a matching *Clear the possible-duplicate tray* section. A miss is a finding to
  file, never a budget to adjust.

- [ ] **Step 2: Docs**
  - **Design page:** add "#### R5b — as built (2026-10-…)" after the R5b designed section. Cover
    what was built per layer, any deviation and why, the headless walk's result, what a human still
    owes (RUNBOOK §12), and the follow-ons filed.
  - **HANDOVER:**
    - ⇒ NEXT: R5b built on PR #…; next #716 + #723 (the small slice), #742, #743 part 2.
    - A "⇒ R5b'S DURABLE RULES" block:
      - the skip rule is db/057's (drift test);
      - `DISPUTED_SQL` is the one spelling;
      - the count and the list share `ROWS_SQL`;
      - the tray re-reads its list on return, because `close_chart` clears `shown`;
      - "Different people" refuses `accepted` in the backend;
      - `candidate_read` is the one candidate read.
    - Prune per the 500-line guideline, losing no issue number.
  - **ROADMAP:** the R5b entry, newest first, brief.

- [ ] **Step 3: Follow-on issues.** File every review finding that is not fixed in place (house
  rule 5), naming R5b and `Refs #680`. Then run `python3 scripts/check_closing_keywords.py` over the
  PR body.

- [ ] **Step 4: Full gates, in CI's order, AFTER the final edit** (trap 18). Run each and read the
  exit code, never `| tail`.
  1. `cargo fmt --all -- --check` (root, and `cairn-gui/`).
  2. `cargo clippy --workspace --all-targets -- -D warnings` (root), then in `cairn-gui/`:
     `cargo clippy --workspace --all-targets --locked -- -D warnings`.
  3. `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps` (root and `cairn-gui/`).
  4. `scripts/run-db-gated-tests.sh` (in the background; about 2 h). Meanwhile do steps 5 and 6.
  5. Matcher: `cd matcher && uv run --extra pipeline pytest -q` and
     `CAIRN_ALLOW_DB_SKIP=1 uv run --isolated pytest -q`.
  6. Docs: `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`.
  7. `cargo test -p cairn-node --test paper_parity_plan_section` (this plan's §1.2 labels).

- [ ] **Step 5: PR.** Push with
  `git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push -u origin feat/r5b-worklist-build`.
  Open the PR against `main`:
  - Body: what was built, the decisions (#741 → ADR-0078, #743 part 1, #736), the tests, what a human
    owes (RUNBOOK §12), and the follow-ons.
  - `Refs #680 #741 #743 #736`: `#741` may close with this PR. Ask the maintainer before writing a
    closing keyword.
  - **Draft** if any gate is red or any review is unfinished.

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk's possible-duplicate tray. Take a pair from the tray, fetch
  both folders, lay them side by side, then clip them together or mark them "different".
- **Steps:**
  - Paper **4** human acts (take the pair, fetch both folders, lay them side by side, clip or mark).
  - Architecture-forced **4** (expand the tray, Review, read the banner, *Same person* / *Different
    people*).
  - UI bundling target **3** (the tray stays open across returns once expanded, so "expand" is paid
    once per session).
  - `M ≤ N`. The front door's find and register gain **0** acts. The tray is closed by default and
    sits below both forms.
- **Time + cognitive load:**
  - Budget **≤ 25 s per entry**, from the tray to a recorded judgement. The load is the side-by-side
    read on R5a's banner, which shows the older record's full active medication list.
  - The front door's find **≤ 5 s** is unaffected: the count is one statement, and the list is read
    only on expand.
  - Measurement is owed by RUNBOOK §12 (a human act; this slice exposes the runnable surface).
