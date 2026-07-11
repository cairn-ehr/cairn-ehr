# §5.4 finishers (PR #1): node-local John-Doe ordinal + `--observed-year` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a node-local, non-replicated "this node's John Doe #N" display ordinal to the §5.4 John-Doe
registration front door, and a bounded `--observed-year` override for clinician-observed evidence.

**Architecture:** Finisher 1 is a read-only projection VIEW (`db/030`) over the immutable `event_log`,
partitioned by `node_origin`, surfaced by `register_john_doe` and printed by its CLI. It does **not**
touch the callsign identity string, so partition-safety is unchanged. Finisher 2 adds a pure
`resolve_observed_year` validator and a CLI flag that feeds the existing
`assert_observed_evidence(..., observed_year)` parameter.

**Tech Stack:** Rust (`cairn-node` crate, `tokio-postgres`, `clap`, `anyhow`), PostgreSQL 18 (+ `cairn_pgx`),
SQL VIEW. DB-gated tests run only when `$CAIRN_TEST_PG` is set.

## Global Constraints

- **No new event type; no floor/wire/SCHEMA change; no ADR; no spec version bump.** Finisher 1 is a
  node-local read-side projection; finisher 2 is a CLI + input-validation refinement of an existing path.
- **The callsign identity string is untouched** — the ordinal is a display aid only, never an identity
  or merge key (principle 2, partition-safety).
- **Licensing:** all code AGPL-3.0; no new dependencies.
- **TDD:** failing test first, then minimal code. **All tests must pass before each commit.**
- **Junior-readable inline comments** on every non-trivial function/VIEW (why + how it fits).
- **Files < 500 lines** where feasible.
- DB-gated tests connect via `CAIRN_TEST_PG` (e.g.
  `host=127.0.0.1 port=5532 user=hherb dbname=cairn_test`) and self-serialize via
  `db::test_serial_guard`. `cargo test --workspace` is safe (DB tests early-return when the var is unset).
- The observed-evidence library fn signature is already
  `assert_observed_evidence(client, sk, kid, node_origin, patient_id, ev, observed_year)` — finisher 2
  does **not** change it; it only feeds it a validated year.

---

## File Structure

- `db/030_john_doe_local_ordinal.sql` — **new.** The `john_doe_local_ordinal` VIEW (finisher 1).
- `crates/cairn-node/src/db.rs` — **modify.** Register `db/030` in the migration list (after `029`).
- `crates/cairn-node/src/john_doe.rs` — **modify.** `register_john_doe` returns the ordinal
  (`(Uuid, String, i64)`); query the VIEW post-commit.
- `crates/cairn-node/tests/john_doe.rs` — **modify.** Update call sites to the 3-tuple; add the ordinal
  DB-gated test.
- `crates/cairn-node/src/evidence.rs` — **modify.** Pure `resolve_observed_year` + unit tests (finisher 2).
- `crates/cairn-node/src/main.rs` — **modify.** Print the `local ref` line at registration (finisher 1);
  add `--observed-year` flag + resolve it (finisher 2); update the `register_john_doe` call site.

---

## Task 1: The `john_doe_local_ordinal` VIEW + `register_john_doe` returns the ordinal

**Files:**
- Create: `db/030_john_doe_local_ordinal.sql`
- Modify: `crates/cairn-node/src/db.rs` (migration list, after the `029_hlc_collision_log` entry)
- Modify: `crates/cairn-node/src/john_doe.rs` (`register_john_doe` signature + post-commit query)
- Modify: `crates/cairn-node/tests/john_doe.rs` (call sites + new test)

**Interfaces:**
- Produces: `register_john_doe(client, sk, kid, node_origin, class, site, date, basis) ->
  anyhow::Result<(Uuid, String, i64)>` — the third element is the node-local ordinal.
- Produces (SQL): VIEW `john_doe_local_ordinal(patient_id uuid, node_origin text, callsign text,
  ordinal bigint)`.

- [ ] **Step 1: Write the failing test** (append to `crates/cairn-node/tests/john_doe.rs`)

```rust
// --- finisher 1: node-local friendly ordinal ---

/// Registration returns a per-node_origin ordinal (1, 2, …) and a foreign node_origin's
/// registrations form their OWN partition, never shifting this node's numbers. Proves the
/// VIEW is node-scoped without any `local_node` dependency, and that only callsign
/// registrations count (the count equals the number of John Does, not their events).
#[tokio::test]
async fn ordinal_numbers_registrations_per_node_origin() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    // Two John Does first-recorded on node "n" → ordinals 1 then 2, in registration order.
    let (_p1, _c1, o1) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "n", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    let (p2, _c2, o2) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "n", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    assert_eq!(o1, 1);
    assert_eq!(o2, 2);

    // A registration first-recorded on a DIFFERENT node_origin starts its own sequence at
    // 1 and does not shift node "n"'s ordinals.
    let (_p3, _c3, o3) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "m", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    assert_eq!(o3, 1, "a different node_origin is a separate partition");

    // node "n"'s second John Doe still reads ordinal 2 via the VIEW.
    let n2: i64 = c
        .query_one(
            "SELECT ordinal FROM john_doe_local_ordinal WHERE patient_id = $1::text::uuid",
            &[&p2.to_string()],
        )
        .await
        .unwrap()
        .get("ordinal");
    assert_eq!(n2, 2);

    // Only callsign registrations are counted (each register authors ONE callsign name +
    // one pending marker; the pending marker is not a name → excluded). Three John Does
    // total across both partitions.
    let total: i64 = c
        .query_one("SELECT count(*) FROM john_doe_local_ordinal", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(total, 3, "only callsign name registrations appear in the VIEW");
}
```

- [ ] **Step 2: Run the test to verify it fails to compile** (the 3-tuple return and the VIEW do not exist yet)

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test john_doe ordinal_numbers_registrations_per_node_origin`
Expected: FAIL — compile error (`register_john_doe` returns a 2-tuple; `john_doe_local_ordinal` unknown).

- [ ] **Step 3: Create the VIEW migration** — `db/030_john_doe_local_ordinal.sql`

```sql
-- db/030_john_doe_local_ordinal.sql
-- §5.4 node-local friendly John-Doe ordinal (a display aid, nothing more).
--
-- WHY: the callsign identity string (Unknown-<class>-<site>-<date>-<uuid-tail>) is
-- globally unique and partition-safe, but a UUID tail like "dead00ab" is not something a
-- clinician can say at the bedside. This VIEW derives a short, human-sayable
-- "this node's John Doe #N" handle from the immutable event_log. It NEVER touches the
-- callsign string, is never signed, never travels the wire, and is never an identity or a
-- merge key — so it cannot regress partition-safety.
--
-- HOW: row_number() PARTITIONs BY node_origin (the node that FIRST recorded the
-- registration), so each node numbers only the John Does it authored; a replicated foreign
-- registration lands in its own partition and never shifts this node's sequence. Ordering
-- within a partition is the collation-free (hlc_wall, hlc_counter, content_address) spine
-- (#115/#69): append-only log + monotonic single-node HLC means ranks never renumber, and
-- content_address (a BYTEA multihash, byte-ordered, identical on every node) breaks any
-- degenerate tie deterministically. All-time (no daily reset) — no timezone semantics to
-- get wrong.
--
-- WHAT IT SELECTS: exactly the callsign name authored by register_john_doe — a demographic
-- name assertion whose `use` facet is 'callsign' and whose provenance is the system
-- john-doe-registration marker. Never an ordinary name; never the pending marker (a
-- different event_type). event_log.body holds the event payload (see db/005 submit_event).
CREATE OR REPLACE VIEW john_doe_local_ordinal AS
SELECT patient_id,
       node_origin,
       body->>'value' AS callsign,
       row_number() OVER (PARTITION BY node_origin
                          ORDER BY hlc_wall, hlc_counter, content_address) AS ordinal
FROM event_log
WHERE event_type = 'demographic.field.asserted'
  AND body->>'field' = 'name'
  AND body->'facets'->>'use' = 'callsign'
  AND body->>'provenance' = 'system:john-doe-registration';
```

- [ ] **Step 4: Register the migration** in `crates/cairn-node/src/db.rs` — add this entry immediately
  after the `029_hlc_collision_log` tuple (before the closing `];`):

```rust
    // §5.4 node-local friendly John-Doe ordinal (display aid): a read-only VIEW ranking
    // each node's own callsign registrations, surfaced as "this node's John Doe #N" at
    // registration. The callsign identity string is untouched (partition-safety unchanged);
    // pure read-side, no floor/wire/event change.
    (
        "030_john_doe_local_ordinal",
        include_str!("../../../db/030_john_doe_local_ordinal.sql"),
    ),
```

- [ ] **Step 5: Change `register_john_doe` to return the ordinal** in `crates/cairn-node/src/john_doe.rs`.
  Change the return type on the signature from `anyhow::Result<(Uuid, String)>` to
  `anyhow::Result<(Uuid, String, i64)>`, and replace the final `tx.commit().await?;` / `Ok((patient_id, call))`
  tail with:

```rust
    tx.commit().await?;

    // Read the node-local friendly ordinal for this registration (finisher 1). Queried
    // post-commit against the same client: the callsign row is durably present, and
    // post-commit avoids threading the read through the just-consumed transaction handle.
    // The VIEW partitions by node_origin, so this row's ordinal is its rank within THIS
    // node's John-Doe registrations — i.e. "this node's John Doe #N".
    let ordinal: i64 = client
        .query_one(
            "SELECT ordinal FROM john_doe_local_ordinal WHERE patient_id = $1::text::uuid",
            &[&patient_id.to_string()],
        )
        .await?
        .get("ordinal");

    Ok((patient_id, call, ordinal))
```

  Also update the doc-comment's `Returns the minted (patient_id, callsign).` line to
  `Returns the minted (patient_id, callsign, node-local ordinal).`

- [ ] **Step 6: Update the existing `register_john_doe` call sites in the test file** so the crate compiles.
  In `crates/cairn-node/tests/john_doe.rs`, change each destructuring to the 3-tuple (add a `, _ord`
  element — keep the existing bindings):
  - `let (pid, _call) = john_doe::register_john_doe(` → `let (pid, _call, _ord) = john_doe::register_john_doe(`
  - `let (pid, call) = john_doe::register_john_doe(` → `let (pid, call, _ord) = john_doe::register_john_doe(`
  - `let (p1, c1) = john_doe::register_john_doe(` → `let (p1, c1, _ord) = john_doe::register_john_doe(`
  - `let (p2, c2) = john_doe::register_john_doe(` → `let (p2, c2, _ord) = john_doe::register_john_doe(`

  (The `main.rs` call site is updated in Task 2; if you run a full `cargo build` before Task 2, temporarily
  destructure there too — but Task 2 gives the final form. To keep Task 1 self-contained, also apply the
  Task 2 Step 1 edit to `main.rs` now.)

- [ ] **Step 7: Update the `main.rs` call site** (`crates/cairn-node/src/main.rs`, the `Cmd::RegisterJohnDoe`
  handler) so the workspace compiles. Change:

```rust
            let (pid, call) = cairn_node::john_doe::register_john_doe(
```
  to:
```rust
            let (pid, call, ordinal) = cairn_node::john_doe::register_john_doe(
```
  and change the trailing `println!` from:
```rust
            println!("registered John Doe {pid}\ncallsign {call}");
```
  to:
```rust
            println!("registered John Doe {pid}\ncallsign {call}\nlocal ref: John Doe #{ordinal} (this node)");
```

- [ ] **Step 8: Run the new test to verify it passes**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test john_doe ordinal_numbers_registrations_per_node_origin -- --nocapture`
Expected: PASS.

- [ ] **Step 9: Run the whole `john_doe` test file + build the workspace**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test john_doe && cargo build --workspace`
Expected: all `john_doe` tests PASS; workspace builds.

- [ ] **Step 10: fmt + clippy**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no diffs, no warnings.

- [ ] **Step 11: Commit**

```bash
git add db/030_john_doe_local_ordinal.sql crates/cairn-node/src/db.rs \
        crates/cairn-node/src/john_doe.rs crates/cairn-node/tests/john_doe.rs \
        crates/cairn-node/src/main.rs
git commit -m "feat(john-doe): node-local friendly ordinal via db/030 VIEW (§5.4 finisher 1)

register_john_doe now returns and prints 'this node's John Doe #N', a per-node_origin
display ordinal derived read-side from event_log. The callsign identity string is
untouched, so partition-safety is unchanged; no floor/wire/event/SCHEMA change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Pure `resolve_observed_year` validator

**Files:**
- Modify: `crates/cairn-node/src/evidence.rs` (add the pure fn + unit tests)

**Interfaces:**
- Produces: `resolve_observed_year(provided: Option<i32>, current_year: i32) -> anyhow::Result<i32>`.

- [ ] **Step 1: Write the failing tests** — add to the `#[cfg(test)] mod tests` block in
  `crates/cairn-node/src/evidence.rs`:

```rust
    #[test]
    fn resolve_observed_year_defaults_to_current_when_absent() {
        assert_eq!(resolve_observed_year(None, 2026).unwrap(), 2026);
    }

    #[test]
    fn resolve_observed_year_passes_a_plausible_past_year() {
        assert_eq!(resolve_observed_year(Some(2010), 2026).unwrap(), 2010);
    }

    #[test]
    fn resolve_observed_year_accepts_the_boundaries() {
        assert_eq!(resolve_observed_year(Some(1900), 2026).unwrap(), 1900);
        assert_eq!(resolve_observed_year(Some(2026), 2026).unwrap(), 2026);
    }

    #[test]
    fn resolve_observed_year_rejects_the_future() {
        // You cannot have observed a patient in a year that has not happened.
        assert!(resolve_observed_year(Some(2027), 2026).is_err());
    }

    #[test]
    fn resolve_observed_year_rejects_absurdly_historical() {
        assert!(resolve_observed_year(Some(1899), 2026).is_err());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-node --lib evidence::tests::resolve_observed_year`
Expected: FAIL — `resolve_observed_year` not found.

- [ ] **Step 3: Implement the pure fn** — add near the top of `crates/cairn-node/src/evidence.rs`
  (after the `use` block, before `build_estimated_dob_event`):

```rust
/// Resolve the year an age estimate was observed. `None` ⇒ default to `current_year`
/// (today, supplied by the caller from the node's DB clock). A supplied year must be
/// plausible: not in the future (you cannot have observed a patient in a year that has not
/// happened yet) and not absurdly historical (which would make the downstream
/// `observed_year − age` birth-range arithmetic nonsensical). Honest reject rather than
/// compute a garbage range (principle 4). Pure and DB-free so it is fully unit-tested.
pub fn resolve_observed_year(provided: Option<i32>, current_year: i32) -> anyhow::Result<i32> {
    const MIN_OBSERVED_YEAR: i32 = 1900;
    match provided {
        None => Ok(current_year),
        Some(y) if y < MIN_OBSERVED_YEAR => {
            anyhow::bail!("--observed-year must be >= {MIN_OBSERVED_YEAR} (implausibly historical)")
        }
        Some(y) if y > current_year => {
            anyhow::bail!("--observed-year {y} is in the future (current year is {current_year})")
        }
        Some(y) => Ok(y),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --lib evidence::tests::resolve_observed_year`
Expected: PASS (5 tests).

- [ ] **Step 5: fmt + clippy**

Run: `cargo fmt --all && cargo clippy -p cairn-node --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/evidence.rs
git commit -m "feat(evidence): pure resolve_observed_year validator (§5.4 finisher 2)

Bounds a supplied observed year to 1900..=current_year (reject the future and the
absurdly historical), defaulting to the current year when absent. Pure/DB-free.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Wire the `--observed-year` CLI flag through the handler

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (`Cmd::AssertObservedEvidence` variant + its handler)

**Interfaces:**
- Consumes: `evidence::resolve_observed_year` (Task 2);
  `evidence::assert_observed_evidence(..., observed_year)` (existing, unchanged).

- [ ] **Step 1: Add the flag to the `AssertObservedEvidence` variant** in `crates/cairn-node/src/main.rs`.
  Add this field to the struct variant (after `sex_basis`):

```rust
        /// The year the age was observed (defaults to the node's current year). Lets a
        /// clinician record evidence about a PAST observation. Bounded 1900..=current year.
        #[arg(long)]
        observed_year: Option<i32>,
```

- [ ] **Step 2: Destructure and use it in the handler.** In the `Cmd::AssertObservedEvidence { … }` match
  arm, add `observed_year,` to the destructured field list, and replace the current line:

```rust
            // Observation year comes from the node's own DB clock (the DB is the clock).
            let observed_year: i32 = db
                .query_one("SELECT extract(year FROM current_date)::int", &[])
                .await?
                .get(0);
```
  with:
```rust
            // Default the observation year to the node's own DB clock (the DB is the
            // clock), but let --observed-year override it for a past observation. The
            // pure validator rejects a future or absurdly-historical year (principle 4).
            let current_year: i32 = db
                .query_one("SELECT extract(year FROM current_date)::int", &[])
                .await?
                .get(0);
            let observed_year =
                cairn_node::evidence::resolve_observed_year(observed_year, current_year)?;
```

- [ ] **Step 3: Build + run the full cairn-node test suite (DB-gated) to confirm no regression**

Run: `cargo build --workspace && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node`
Expected: workspace builds; all cairn-node tests PASS (the existing `observed_evidence.rs` DB tests that
pass an explicit `observed_year` still pass — the library fn is unchanged).

- [ ] **Step 4: Manual CLI smoke (optional but recommended)** — verify the flag parses and the bound bites:

Run: `cargo run -p cairn-node -- --help 2>/dev/null | grep -i observed-year || cargo run -p cairn-node -- assert-observed-evidence --help`
Expected: `--observed-year <OBSERVED_YEAR>` appears in the help.

- [ ] **Step 5: fmt + clippy**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cli): --observed-year override for assert-observed-evidence (§5.4 finisher 2)

Defaults to the node's current DB year; a supplied year is validated by
resolve_observed_year (1900..=current). Parameterizes the computed DOB range only,
not t_effective (deliberate scope boundary).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Final verification + docs currency

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (mark PR #1 finishers done; note finisher 3 still open)

- [ ] **Step 1: Full workspace test + lint gate**

Run:
```bash
cargo fmt --all --check && \
cargo clippy --workspace --all-targets -- -D warnings && \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
```
Expected: fmt clean; clippy clean; all tests PASS (cairn-node DB-gated all pass, cairn-event + cairn-sync pass).

- [ ] **Step 2: mkdocs builds** (docs sanity; only touched superpowers/ + will touch HANDOVER/ROADMAP)

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: builds without error.

- [ ] **Step 3: Update HANDOVER.md + ROADMAP.md** — add a concise "this session" entry recording:
  finisher 1 (node-local John-Doe ordinal, `db/030` VIEW) + finisher 2 (`--observed-year`) done; the
  callsign identity string / partition-safety deliberately unchanged; **finisher 3 (identify→optional-link)
  still open, its own spec/PR next.** Prune per the house rules (keep both files concise). Reference the
  spec + this plan under `docs/superpowers/{specs,plans}/2026-07-11-john-doe-ordinal-and-observed-year*`.

- [ ] **Step 4: Commit the docs update**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — §5.4 finishers PR#1 done (ordinal + --observed-year)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 5: Push + open the PR to main**

```bash
git push -u origin feat/john-doe-ordinal-and-observed-year
gh pr create --base main --title "§5.4 finishers (1/2): node-local John-Doe ordinal + --observed-year" \
  --body "$(cat <<'EOF'
## Summary
Two small, self-contained §5.4 finishers (PR #1 of the finishers thread).

- **Finisher 1 — node-local John-Doe ordinal.** New read-only `john_doe_local_ordinal` VIEW (`db/030`)
  ranks each node's own callsign registrations by `node_origin` partition; `register_john_doe` returns
  and prints "this node's John Doe #N". The callsign **identity string is untouched** (still
  UUID-suffixed → partition-safe); the ordinal is a node-local display aid, never signed, never on the
  wire, never an identity. No floor/wire/event/SCHEMA change.
- **Finisher 2 — `--observed-year` override.** Pure `resolve_observed_year` (bounded 1900..=current
  year; rejects the future and absurdly-historical) + a CLI flag feeding the existing
  `assert_observed_evidence(..., observed_year)`. Parameterizes the computed DOB range only, not
  `t_effective`.

No new event type, no ADR, no spec bump.

## Design / plan
- Spec: `docs/superpowers/specs/2026-07-11-john-doe-ordinal-and-observed-year-design.md`
- Plan: `docs/superpowers/plans/2026-07-11-john-doe-ordinal-and-observed-year.md`

## Deferred
Finisher 3 (identify→optional-link) gets its own spec/PR — it needs the `identify` authoring surface
built from scratch + attestation-from-CLI.

## Test plan
- TDD throughout. New DB-gated test: per-node_origin ordinal + partition isolation + callsign-only.
- New pure unit tests: `resolve_observed_year` bounds.
- Full workspace green; fmt + clippy + mkdocs clean.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**1. Spec coverage:**
- Finisher 1 VIEW (partition-by-node_origin, callsign-only predicates) → Task 1 Step 3. ✓
- Migration registration → Task 1 Step 4. ✓
- `register_john_doe` returns ordinal (post-commit query) → Task 1 Step 5. ✓
- CLI prints `local ref: John Doe #N (this node)` → Task 1 Step 7. ✓
- Finisher 1 DB-gated test (two → 1,2; foreign partition; callsign-only count) → Task 1 Step 1. ✓
- Finisher 2 pure `resolve_observed_year` + bounds `1900..=current_year` → Task 2. ✓
- Finisher 2 pure tests (None→current; past passthrough; boundaries; future err; <1900 err) → Task 2 Step 1. ✓
- Finisher 2 `--observed-year` flag wired to existing `assert_observed_evidence` → Task 3. ✓
- Deliberate scope boundary (no `t_effective` change) → honored (library fn untouched). ✓
- Finisher 2 DB behavior (dob range from a chosen year) → already covered by existing
  `observed_evidence.rs` tests (which pass explicit years 2026/2020); Task 3 Step 3 re-runs them. ✓
- Docs currency (HANDOVER/ROADMAP) + finisher 3 deferral recorded → Task 4. ✓

**2. Placeholder scan:** No TBD/TODO/"add error handling"/"similar to". Every code step shows full code. ✓

**3. Type consistency:** `register_john_doe` return `(Uuid, String, i64)` used consistently (VIEW `ordinal`
is `bigint` → `i64`; `.get("ordinal")` as `i64`). `resolve_observed_year(Option<i32>, i32) -> Result<i32>`
consistent across Task 2 definition and Task 3 call. Call-site destructuring updated in every location
(`main.rs` + 4 in `tests/john_doe.rs`). ✓
