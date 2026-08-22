# "db error" — the two places a database failure says nothing, and the cycle that blames the WAN

**Goal:** close [#467](https://github.com/cairn-ehr/cairn-ehr/issues/467) and
[#469](https://github.com/cairn-ehr/cairn-ehr/issues/469), the two residues of PR #466's review that
need no decision. Both are the same species — issue #109's, twice more — and both were found by a
real failure, not by reading: #467 by a required CI job that flaked and could say only *"loading
031_medication: db error"*, #469 by the new `references_unlearnable` metric inheriting a propagation
path that throws the whole metrics object away and calls a local database fault a WAN partition.

**Architecture:** no new event type, no migration file, no ADR, no wire change.
`SCHEMA_GENERATION` stays **50**. One new pure leaf module in `cairn-node`, four new pure functions
in `cairn-sync`, one new error type, one new key in the per-cycle log line, and the Bet A harness
taught to count it.

**Tech Stack:** Rust (`cairn-node`, `cairn-sync`), Python (`poc/walking-skeleton/harness/bet_a.py`).
No new dependencies in any tree.

Paper-parity: not clinical-surface — every item is an operator-facing diagnostic string or a log-line
classification below the enforcement floor; no clinical workflow gains, loses or reorders a step, and
no clinician-visible surface changes.

## Global Constraints

- **AGPL-3.0**; no new dependencies (house rule 1).
- **TDD**: the failing test is written and *seen to fail* first (house rule 2). For a diagnostic
  string, "seen to fail" means the assertion is run against the OLD renderer and observed red — a
  test that would pass against `"db error"` is testing nothing.
- **Files under 500 lines** (house rule 4). `crates/cairn-node/src/db.rs` is already 665; the helper
  therefore lands in its own module rather than growing it further.
- **Guard before connect** for any DB-gated test: `db::test_serial_guard(&base)` *before*
  `connect_and_load_schema`.
- **Never a raw connection string in an error message** — it can carry a password.

---

### Task 1 — #467: a database failure that names itself (`cairn-node`)

`tokio_postgres::Error`'s `Display` is the literal string **`db error`**: a bare match on kind, with
no chaining to the source that holds the message, the DETAIL and the SQLSTATE. `db.rs` has **nine**
`anyhow!("…: {e}")` wrappings over it — each of which *also* discards the source, so `anyhow`'s own
chain printing has nothing left to show — plus **five** bare `?` sites that keep the chain but name
no context and never surface the SQLSTATE.

`cairn-sync` solved this twice (`ApplyError::from`, then `legible_db_error` in PR #466). This is the
same fix one crate over, with the composition split out as a pure function so it is testable with no
database at all — the DbError arm cannot be constructed by hand, but the *rendering* can.

**Files:** create `crates/cairn-node/src/db_diagnosis.rs`; register it in `lib.rs`; edit `db.rs`
(14 sites); create `crates/cairn-node/tests/db_diagnosis.rs`.

**Interface:**

```rust
pub fn compose_db_diagnosis(message: &str, sqlstate: &str, detail: Option<&str>) -> String;
pub fn legible_db_error(e: &tokio_postgres::Error) -> String;
```

The composed shape is **byte-identical to cairn-sync's** — `"{message} [{sqlstate}] — {detail}"` —
so an operator grepping two logs sees one format, and each renderer's doc comment names the other.
(A shared crate for twenty lines is not worth a new workspace member today; the duplication is
recorded in both comments so the next person who touches either meets it.)

**Tests (in this order):**

1. `compose_names_all_three_parts` — DB-free. Message, SQLSTATE and DETAIL all present in the
   output; SQLSTATE bracketed so it is greppable.
2. `compose_without_detail_has_no_dangling_separator` — DB-free. An absent DETAIL must not leave
   a trailing `—`.
3. `a_non_db_error_still_renders_legibly` — DB-free (`#[tokio::test]`, an unparseable connection
   string, which fails in the config parser and never touches the network). Falls back to `Display`
   and does NOT say `"db error"` — the kind's own text is the whole story for these.
4. `a_server_error_carries_message_sqlstate_and_detail` — DB-gated. A `RAISE … USING DETAIL` through
   a real connection, the shape `db.rs`'s failures actually take.
5. `a_failed_migration_names_the_migration_and_the_sqlstate` — DB-gated, and the one that pins the
   ACCEPTANCE CRITERION rather than the helper: drive `connect_and_load_schema` at a database where
   the replay must fail, and assert the message carries the migration name **and** the SQLSTATE.

**Site edits:** all nine `anyhow!("…: {e}")` become `anyhow!("…: {}", legible_db_error(&e))`. The
five bare `?` sites gain the same treatment plus the context they never had (which statement failed).
The `connect` site names no connection string.

---

### Task 2 — #469: the metrics that vanish, and the local fault logged as link downtime

`do_pull`'s cursor commit is the ONLY propagation point between the apply loop and the `metrics`
object (verified: every earlier return either precedes any apply or is routed deliberately; the two
`?` inside the decode closure are captured into `decoded`, not propagated). A bare `?` there means:

* events **are** applied and durable, flags **are** written to `attachment_reference_flag`;
* `do_pull` returns a raw `postgres::Error`, so **no metrics object at all**;
* the #465 unlearnable report never runs;
* the text is `"db error"` (Task 1's species);
* and `run` classifies anything that is not a `PullIntegrityError` as **`"partition": true`** — a
  write failure on THIS node's database, reported to the operator as link downtime, and charged to
  the availability figure as such.

**Files:** `crates/cairn-sync/src/main.rs`; `poc/walking-skeleton/harness/bet_a.py`.

**Shape:**

1. **A third failure class, not a reuse.** `PullIntegrityError` means *the peer answered and its DATA
   is the problem*; `partition` means *the link*. A cursor-commit failure is neither, and folding it
   into either just moves the wrong diagnosis. New `struct CursorCommitError { message, metrics }`,
   and a new log key **`"local_fault": true`**.
2. **Build `metrics` before the commit**, then fill the two fields that are only knowable after it
   (`elapsed_ms`, and `references_unlearnable` — whose read stays AFTER the commit attempt on
   purpose: it is a report about work already done, and must never cost the pull its progress).
3. **The commit-derived fields go UNKNOWN on failure, never to the value the write would have set.**
   `cursor_seq` and `floor_active` are claims *about that write*. Reporting them after it failed is
   a precise untruth in the reassuring direction — a monitor sees a cursor that advanced when it did
   not, and if the connection dropped mid-statement this node genuinely cannot know which happened.
   Both become `null` (principle 4, and exactly `references_unlearnable`'s null-never-zero rule one
   field over); the attempted seq is named in the message instead.
4. **`run` classifies through a pure function**, so the three-way mapping is unit-testable with
   constructed errors instead of only reachable through a live pull.
5. **The Bet A harness counts the new class.** A key nothing counts is the silence this fix exists to
   end: `bet_a.py` already sums `partition` and `integrity`; it now sums `local_fault` too and prints
   it, so a cycle lost to this node's database is visible in the same summary — and no longer
   inflates the partition figure.

**New pure functions (all unit-tested with no database):**

```rust
fn cursor_commit_failure_message(peer_name: &str, applied: usize, attempted_seq: i64,
                                 cause: &str) -> String;
fn mark_cursor_outcome_unknown(metrics: &mut serde_json::Value);
fn classify_pull_failure(e: &(dyn Error + 'static)) -> (&'static str, serde_json::Value);
```

**Tests (in this order):**

1. `a_cursor_commit_failure_is_not_a_partition` — DB-free over constructed errors: the three classes
   map to `integrity` / `local_fault` / `partition`, and the first two carry their metrics through.
2. `the_failure_message_names_the_node_not_the_link` — DB-free: the text says the database, names the
   peer, the applied count and the attempted seq, and carries the legible cause.
3. `an_uncommitted_cursor_reports_unknown_not_the_value_it_tried` — DB-free: `cursor_seq` and
   `floor_active` are `null` afterwards, and the fields that describe completed work are untouched.
4. `a_failed_cursor_commit_keeps_the_cycles_metrics` — DB-gated, end to end: force the `sync_state`
   UPDATE to fail after events have applied, and assert the returned error is a `CursorCommitError`
   whose metrics still carry `applied_new`, that it is classified `local_fault`, and that its message
   is not `"db error"`.

**Deliberately NOT done:** `cycle_is_loud` is not extended. Its four states are the ones reachable on
a cycle that DID commit; a commit failure is an `Err` return before that predicate is consulted, and
widening it would blur a test that is currently sharp ("the event set is COMPLETE and the loss is
declared elsewhere"). The new error type's doc comment states the relationship instead.

---

## Verification

- `cargo test --workspace --all-targets` with `CAIRN_TEST_PG`/`PG2`/`PG3` set (the full local gate;
  ~2 h, started in the background).
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` on both cargo trees.
- Every new test **mutation-checked**: revert the fix, watch the assertion go red, restore.
- `ruff` + the poc harness left importable (`bet_a.py` is not under a test suite; the change is a
  three-line sum-and-print mirroring the two beside it).


---

## Addendum — what the PR review changed (2026-08-22, same day)

The plan above was executed as written. Its review then found that **the plan's own central
interface was wrong**, so the shipped code differs from the `Interface:` blocks above. Recorded
here rather than edited in, so the gap between what was planned and what survives is visible.

1. **`compose_db_diagnosis` takes four fields, not three.** `hint` was missing. `DbError`'s own
   `Display` prints message + DETAIL + **HINT**, so at the five sites that had been bare `?` the
   HINT previously reached the operator through anyhow's chain and the "fix" removed it — on
   `42883`, the SQLSTATE this plan's own acceptance test pins, PostgreSQL's HINT is the most
   actionable line it sends.

2. **The fallback arm was the defect the plan existed to remove.** The plan says `Display` "is
   genuinely the whole story" for a non-`DbError`. It is not: `Display` is a bare kind match for
   *every* kind and never consults `source()`. The plan reasoned from `Kind::Db` and generalised
   without checking. This also made the change a **regression** at `db::connect`, whose failures
   are overwhelmingly the no-`DbError` kind.

3. **A test that cannot fail is not a test.** The plan's own constraint said *"a test that would
   pass against `"db error"` is testing nothing"* — and the fallback test asserted only
   `!= "db error"`, which the broken output satisfies. The rule was right and was not applied to
   the one arm the plan reasoned loosely about. That is the transferable lesson: the assertion has
   to name what the output must CONTAIN, not what it must not equal.

4. **Scope was drawn one function too tight.** The plan scoped #469 to the cursor commit, which is
   where the issue pointed. But `do_pull`'s first two statements are the same defect, before any
   network I/O, and `cmd_run` never reconnects — so the misclassification the plan set out to end
   survived in a form that never self-heals. A "fix the propagation point the issue names" scope
   is worth one deliberate look outward before it is accepted.

5. **Two conditions that can co-occur need a set, not a choice.** The early return for the new class
   sat above the loud-integrity check, silently dropping the second diagnosis.

Also corrected: the message asserted "the cursor did not advance" while the metric beside it
reported the same fact as unknown; `do_requeue` has three in-loop `?` sites, not two; and a source
scan (`db_errors_stay_legible.rs`) now guards the class rather than the fourteen instances.
