# Implementation plan — DR slice 2d: restore reads the clinical plane back

- **Date:** 2026-09-10
- **Design authority:**
  [`2026-09-09-dr-slice-2d-restore-reads-the-clinical-plane-design.md`](../specs/2026-09-09-dr-slice-2d-restore-reads-the-clinical-plane-design.md)
  — reviewed, corrected and merged as PR #565. **This plan does not re-decide anything the
  design decided.** Where a task below is terse, the design section it names carries the
  argument; read that, do not re-derive it.
- **Closes:** [#554](https://github.com/cairn-ehr/cairn-ehr/issues/554).
- **Crates touched:** `cairn-medium`, `cairn-wire`, `cairn-node`, `cairn-sync` — four, so
  **the gate is the full workspace**, never `-p cairn-node` (#503's lesson;
  `cairn-sync/tests/clinical_pull.rs` is the suite a per-crate run misses).
- **Migration:** `db/052`, `SCHEMA_GENERATION` 51 → 52.

---

## The order, and why it is this order

The tasks are sequenced so that **every task's tests can run the moment it lands**, and so
that the two findings that would ship a green-but-broken slice (design §2.1's double-wrap and
§2.2's dropped custody) are each fenced by a test *before* the code that could commit them
exists.

Two ordering constraints are hard:

1. **T3 (the migration) precedes every task that calls into the database.** A test that
   `SELECT`s a function the schema does not define fails for the wrong reason, and a wrong
   reason is how a red test gets "fixed" by weakening the assertion.
2. **T8 (the inverted pin) is LAST.** `nothing_yet_restores_a_clinical_event_from_a_medium`
   is the guard that this slice is not yet done. Inverting it before the guarantee holds
   deletes the only signal that would say so. **Invert it, never delete it** (#554).

---

## T1 — `cairn-medium`: one derivation of the servable clinical set

**Design §5.0.** `MediumTransport::new` derives *"clinical records within `verified_through`,
ascending by `source_seq`, deduped"*. `cairn-node`'s restore needs exactly that set. Writing a
second one loses the trust gate (2a invariant 5) — the review's finding.

- **New pure function in `crates/cairn-medium/src/chain.rs`** (the module that owns
  `verified_through`, and already builds the identical prefix/plane/flat_map pipeline in
  `seq_gaps`):

  ```rust
  pub fn plane_records(m: &MediumV3, report: &ChainReport, plane: Plane) -> Vec<MediumRecord>
  ```

  Prefix-gated on `report.verified_through` (`None` ⇒ empty, never "all"), filtered to
  `plane`, sorted by `source_seq`, `dedup()`ed over the WHOLE `MediumRecord` (all five
  fields — the sidecar reasoning in `MediumTransport::new`'s comment moves with it).
  Exported from `lib.rs` beside `seq_gaps`.
- **`MediumTransport::new` is re-expressed on it** and keeps its own comments about *why*
  each step exists; only the derivation moves. Its existing tests are the proof and must not
  be touched.

**Tests (in `cairn-medium`, pure, no DB):**
- the prefix gate: a medium with a broken link mid-file yields records only from the verified
  prefix (design test 14's unit half);
- the sort: a record with a lower `source_seq` in a LATER segment comes back first (test 15's
  unit half);
- `None` verified_through yields empty, not all;
- a byte-identical duplicate collapses; a record differing in `dek_wrapped` alone does **not**.

---

## T2 — `cairn-node`: `backup::clinical_plane_records`

**Design §5.1.** A thin adapter over T1 — it must not re-implement the gate or the sort.

```rust
pub fn clinical_plane_records(image: &MediumImage) -> Result<Vec<MediumRecord>, BackupError>
```

`MediumImage::Legacy` ⇒ `Ok(vec![])` (§5.2: a legacy medium has no clinical plane; the
*naming* of that outcome is the caller's job in T7, not a silent zero here).
`node_plane_events` keeps its exact current shape and tests — untouched.

**Tests:** the legacy arm returns empty; the V3 arm returns exactly what
`chain::plane_records(.., Plane::Clinical)` returns for the same image (so the adapter cannot
drift onto a second answer).

---

## T3 — `db/052`: the registry door, the shared pen, the pen's custody column

**Design §4 and §6.1.** One migration, three things.

1. **`restore_actor_registry(p_rows JSONB) RETURNS integer`** — set-shaped, resumable,
   SECURITY DEFINER, `SET search_path = public, pg_temp`, `REVOKE EXECUTE FROM PUBLIC`,
   `GRANT` to `cairn_node` **only** (never `cairn_agent`). Fenced twice: refuses a non-empty
   `local_node`; refuses if `actor_event` holds any `actor_event_id` **not** among `p_rows`.
   Inserts in ascending source-`seq` order, letting `GENERATED ALWAYS AS IDENTITY` assign
   fresh values (no `OVERRIDING SYSTEM VALUE`); `recorded_at` carried verbatim and
   **required**. Validates shape only — it deliberately does not re-adjudicate #152/#166.
2. **`cairn_quarantine_event(...)`** — the pen, lifted from `cairn-sync`'s Rust into the
   database so `cairn-node` can reach it (`cairn-sync` is binary-only). The dedupe-bump,
   the quota probe and the full-pen-vs-lost-race distinction move verbatim; **the quota
   becomes two caller-supplied parameters** rather than two constants, and a restore passes
   unbounded (§6.2). Grants to `cairn_node`.
3. **`ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS dek_wrapped BYTEA;`** — additive,
   nullable (§2.2). **`db/021`'s `CREATE TABLE` is NOT widened**; if a later tidy-up widens
   it, that pair owes an entry in `migration_replay_widening.rs` (#207).

**Migration bookkeeping — each of these has bitten before:**
- `db/tests/052_*.sql` mirror;
- `SCHEMA_GENERATION` 51 → 52 in `crates/cairn-event/src/schema_generation.rs`;
- the hand-written migration list in `crates/cairn-node/src/db.rs`;
- `crates/cairn-node/tests/schema_version_guard.rs`.

**Tests:** the SQL mirror covers the two fences, the resume path, the ordering property and
the identity counter (design tests 8, 9, 11, 12); the privilege (test 10) is a Rust test in
`cairn-node` because it needs a role switch.

---

## T4 — `localstate.rs`: the serde contract

**Design §2.4 and test 13.** Drop `#[serde(default)]` from `ActorRegistryRow::recorded_at`
and rewrite the doc paragraph that justifies it — the justification ("degrading rather than
refusing over one audit field") is false now that the field is `actor_current`'s primary
ordering key and the door installs it.

**Tests:** a CBOR `ActorRegistryRow` with no `recorded_at` fails to decode; an `EpisodeDek`
missing a field fails to decode; **`deny_unknown_fields` is pinned on both** — deleting it
must redden something, which today it does not.

---

## T5 — the registry install

`apply_local_state` calls `restore_actor_registry` with the decoded rows and reports the
inserted count through `AppliedLocalState`. The "carried but not yet applied" note in
`main.rs` becomes a real count of what landed.

**Tests:** design 8–12, driven from Rust against a live DB; test 10 (the `cairn_agent` /
`PUBLIC` refusal) lives here.

---

## T6 — the clinical apply

**Design §2.1, §5, §6.** New module `crates/cairn-node/src/restore/clinical.rs` (`restore.rs`
becomes a directory) so neither `main.rs` (6068 lines) nor `restore.rs` grows further — house
rule 4.

- For each record from T2, in order: unwrap `dek_wrapped` in **Rust** with the inherited
  unwrap secret and pass the **plaintext** to `apply_remote_event`'s `p_dek` (§2.1 — the door
  re-wraps). Mirrors what `cairn-sync`'s `do_pull` already does.
- **Per-event failure is skip + count + pen, never abort** (§6): pen through
  `cairn_quarantine_event` with `peer = '(restore)'`, the record's `dek_wrapped` preserved,
  an unbounded quota, and **no `quarantine_floor_seq` pinned** (§2.3).
- **No usable export** (§6): a record whose `dek_wrapped` is `None` restores normally with a
  NULL `p_dek`; a record that **carries** one is refused here and penned with that custody
  intact. **The test is the record's custody, not whether the event is sealed.**
- Counts by reason, returned as a report struct for T7 to print.

**Tests:** design 2 (the end-to-end guarantee — seal a body on node A, capture, restore into
fresh node B, read the payload back in clear), 3 (the double-wrap regression, named), 4
(custody survives the pen, via requeue), 5 (a pre-capture-shredded sealed body restores
custody-less), 6 (the no-export path, keyed on custody), 7 (the pen is not capped — **and
this fixture must run above `MAX_QUARANTINE_ROWS_PER_PEER`, or it proves nothing**), 16 (a
duplicate `source_seq` is a no-op; a substitution is refused).

---

## T7 — the ceremony and the operator surface

**Design §3.** `main.rs`'s `Cmd::Restore` arm reorders to:

```
mint new key → apply node plane → apply_local_state_export → apply CLINICAL plane → finalize_identity
```

and the two temporary ORDERING NOTE comments are replaced by the real reasoning. The scope
notes stop saying "not restored yet" and start reporting what landed, what was penned and
why, the legacy-medium named outcome, the `Unknown(tag)` note with its record count, the
provenance gate for clinical segments, the pen's row/byte totals with the over-ordinary-quota
sentence, and the AEAD caveat. **Non-zero exit if any clinical event was refused, after the
full summary has printed.**

**Tests:** design 17 (legacy named outcome, unknown-plane note), 18 (provenance gates the
clinical plane), 19 (the order is load-bearing — a failed clinical apply leaves `local_node`
empty and the same database re-restores), 20 (the two §3 preconditions pinned as source
guards: zero `local_node` references in `db/020`, none in `cairn_register_unwrap_key`), 21
(no floor pinned), 22 (the summary survives the failure; refusals counted by reason), 23 (the
AEAD caveat is printed).

---

## T8 — invert the pin

`dr_clinical_guarantee_gap.rs::nothing_yet_restores_a_clinical_event_from_a_medium` becomes
`a_clinical_event_restores_from_a_medium`: its two count assertions (`event_log`,
`event_dek`) invert, its **leg-1 equality** (`node_plane_events(&image) == federation`) stays
exactly as it is (§5.1 keeps `node_plane_events` unchanged), and its doc is rewritten to say
it is now a GUARANTEE and what reddens it. **Inverted, never deleted** (design test 1).

---

## T9 — `cairn-sync` becomes a caller

`quarantine_event` becomes a thin caller of `cairn_quarantine_event`, passing the existing
constants as the quota so **its behaviour on the sync path is unchanged** — which its
existing tests pin. `do_requeue` unwraps and passes `dek_wrapped` (§2.2), which is what makes
design test 4 pass.

---

## T10 — ADR-0067, the spec bump, and the docs

**Design §8.** `docs/spec/decisions/0067-*.md` carrying all five decisions — the AEAD
exception, erasure-does-not-propagate-backwards **in as many words**, the load-bearing
ceremony order, the **supersession of ADR-0026 decision 2's implementation wording**, and the
resource-budget carve-out. Spec version in `docs/spec/index.md` moves with it. HANDOVER +
ROADMAP updated; "2e" retired as a label per §8.

---

## T11 — the measurement

**Design's benchmark section.** Restore a 100 000-event medium and record wall-clock against
#512's ≤ 10 min budget, with a per-event figure reported as evidence. **If it falls outside
the budget that is the finding — file it against #512, never adjust the budget.**

---

## Verification

The gate is `scripts/run-db-gated-tests.sh` (SQL mirrors, then the FULL workspace `cargo
test` with the DB env baked in). A cross-crate change relinks every test binary, so this run
is measured in hours, not minutes — start it in the background and do the docs pass while it
runs (HANDOVER's standing note). `cargo clippy --workspace --all-targets -D warnings` and
`cargo doc` with `RUSTDOCFLAGS=-D warnings` are both part of CI and both must be clean.

---

## Paper-parity benchmark (§1.2)

**Inherited unchanged from the design, which inherits it unchanged from
[#512](https://github.com/cairn-ehr/cairn-ehr/issues/512).** House rule 7 permits filing an
`M > N` defect, never arguing one away, and redefining the baseline inside the slice being
measured is how a falsifiable benchmark stops being falsifiable.

**Paper counterpart:** the off-site duplicate chart — the practice that copies its records,
keeps the copy in another building, and carries the box back after a fire.

**Steps:** paper *N* = **2** (fetch the box; shelve it) → architecture-forced *M* = **3**
(attach the medium; run `cairn-node restore` and answer its prompts; **confirm the echoed
identity when provenance is not sole-enroll-signed**) → UI bundling target *K* = **2**.
`M > N`, **filed as an architecture defect (#512), not argued away** — the extra act is the
identity confirmation, which has no paper counterpart because a paper box carries no
cryptographic identity to mis-assign.

**This plan changes none of the three numbers.** It changes what those steps recover. T7 owes
the `Provenance` gate for clinical segments (the third act); a plan that dropped it would be
reporting `M = 2` by deleting a safety step, not by bundling one.

**Time + cognitive load:** cognitive load is unchanged by construction — the operator types
the same command and reads a longer scope line. The time budget is #512's and is **not
adjusted here**: *a restore of a 100 000-event medium completes in ≤ 10 min, and the operator
needs one secret and no knowledge of the dead node's configuration.* **T11 measures it.** If
the measurement falls outside the budget, that is the finding — file it against #512.
