# Human-attested medication responsibility (slice 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a clinician take human-attested clinical responsibility for a medication thread — both author-time ("I vouch as I record") and post-hoc ("I reviewed this existing entry") — and surface, per drug, *who vouched* and *whether the vouch is still current*.

**Architecture:** One additive event type `clinical.medication-attestation.asserted` referencing a `medication_id`, carrying the responsible human as a responsibility-bearing contributor through the **existing** db/005 attestation gate (3-arg door) — the shipped device-additive med events are unchanged. A sign-off pins a convergent **set-commitment** of the thread's content events it reviewed; staleness is a commitment compare (append-only ⇒ any later change flips it stale, closing the lower-HLC late-arrival gap). Pure Rust builders + an in-DB floor (`db/034`) + separate projection views the current-list joins to (the current-list view is never widened — replay-safe). Safety-critical write path (§9).

**Tech Stack:** Rust (`cairn-event`, `cairn-node`), PostgreSQL 18 + `cairn_pgx` (pgrx), PL/pgSQL + pgcrypto (`digest`).

**Design doc:** [`docs/superpowers/specs/2026-07-14-medication-attestation-responsibility-design.md`](../specs/2026-07-14-medication-attestation-responsibility-design.md)

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-compatible (checked before adding). No new runtime dependency is introduced by this slice.
- **TDD** — failing test first, then minimal code (load-bearing on this §9 safety-critical write path).
- **Files under 500 lines** where feasible (house rule 4). `crates/cairn-node/src/medication.rs` is 621 lines and is split in Task 2.
- **Never hard-code cryptographic material in tests** — derive keys/seeds at runtime (`std::array::from_fn`), never byte-array literals (house rule 6, CodeQL FP).
- **Additive only:** `db/031`, `db/032`, `db/033` are **untouched**. `patient_medication_current` / `_past` are **not widened** (db/031–033 re-issue them with an identical column set every reconnect; a wider db/034 version breaks db/033's re-creation with *"cannot drop columns from view"*).
- **`event_log` facts:** `event_id UUID PK`; `body JSONB` **is the payload** (db/005 stores `b -> 'payload'`); the verified responsibility proof lands in `attester_key BYTEA` + `attestation BYTEA` (set by the db/005 gate); `content_address BYTEA UNIQUE`; positions are `(hlc_wall, hlc_counter, node_origin, content_address)`.
- **The db/005 attestation gate is the real enforcement.** Any contributor carrying a `responsibility` key forces the 3-arg `submit_event($1,$2,$3)` door: a valid attestation token bound to the event + the attester must be an enrolled `kind='human'` actor. No db/005 change.
- **Test substrate:** DB-gated Rust tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx` installed). They self-serialize cluster-wide via a Postgres advisory lock, so `cargo test --workspace` is reliable.
- **Definition of done per task:** the task's tests pass **and** `cargo fmt --all --check` (both cargo trees), `cargo clippy --workspace --all-targets -- -D warnings`, and the relevant `cargo test` are green. Whole-workspace green + `mkdocs build` is Task 9.
- **Thread content events** (the set an attestation reviews / staleness watches) are exactly: `clinical.medication.asserted`, `clinical.medication-cessation.asserted`, `clinical.medication-dose-change.asserted`, `clinical.medication-dose-correction.asserted` — filtered by `body ->> 'medication_id'`. Reconciliation/separation/attestation events are **excluded** (they are not thread content).

---

## Task 1: Pure event builders + twin (`cairn-event::medication::attestation`)

**Files:**
- Create: `crates/cairn-event/src/medication/attestation.rs`
- Modify: `crates/cairn-event/src/medication/mod.rs` (add `pub mod attestation;` + re-export)

**Interfaces:**
- Produces:
  - `pub struct MedicationAttestation<'a> { pub medication_id: &'a str, pub reviewed_commitment: &'a str, pub reviewed_count: u32, pub basis: Option<&'a str>, pub note: Option<&'a str> }`
  - `pub fn medication_attestation_body(a: &MedicationAttestation) -> serde_json::Value`
  - `pub fn render_medication_attestation_twin(a: &MedicationAttestation) -> String`

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-event/src/medication/attestation.rs`:

```rust
//! Medication attestation builder (slice 4). Pure: shapes only payload JSON.
//! A human takes clinical responsibility for one `medication_id` thread, pinning
//! a convergent commitment of the thread's content-event set it reviewed (so a
//! later change flips the vouch stale). Mirrors `reconciliation.rs`. The db/034
//! floor rejects a malformed medication_id / commitment; the db/005 gate enforces
//! the human attestation (the responsibility-bearing contributor is added by the
//! cairn-node author path, not here).
use serde_json::{json, Value};

/// One attestation of one `medication_id` thread. `reviewed_commitment` is a hex
/// digest of the reviewed content-event set (see `cairn_medication_thread_commitment`,
/// db/034); `reviewed_count` is a legibility hint only. `basis`/`note` are omitted
/// entirely when absent (never serialized as null) so an added-later field never
/// changes an existing event's content address (principle 11, demographics idiom).
pub struct MedicationAttestation<'a> {
    pub medication_id: &'a str,
    pub reviewed_commitment: &'a str,
    pub reviewed_count: u32,
    pub basis: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// Build the `clinical.medication-attestation.asserted` payload.
pub fn medication_attestation_body(a: &MedicationAttestation) -> Value {
    let mut p = json!({
        "medication_id": a.medication_id,
        "reviewed_commitment": a.reviewed_commitment,
        "reviewed_count": a.reviewed_count,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(b) = a.basis {
        obj.insert("basis".into(), json!(b));
    }
    if let Some(n) = a.note {
        obj.insert("note".into(), json!(n));
    }
    p
}

/// The §3.13 legibility twin. Always non-empty.
pub fn render_medication_attestation_twin(a: &MedicationAttestation) -> String {
    let mut s = format!(
        "Reviewed & vouched for medication thread {} ({} entries)",
        a.medication_id, a.reviewed_count
    );
    if let Some(b) = a.basis {
        s.push_str(&format!(" — {b}"));
    }
    if let Some(n) = a.note {
        s.push_str(&format!(" [{n}]"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MedicationAttestation<'static> {
        MedicationAttestation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            reviewed_commitment: "abcdef0123456789",
            reviewed_count: 4,
            basis: Some("admission reconciliation"),
            note: None,
        }
    }

    #[test]
    fn body_carries_id_commitment_count() {
        let v = medication_attestation_body(&sample());
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["reviewed_commitment"], "abcdef0123456789");
        assert_eq!(v["reviewed_count"], 4);
        assert_eq!(v["basis"], "admission reconciliation");
    }

    #[test]
    fn basis_and_note_omitted_when_absent_not_null() {
        let a = MedicationAttestation { basis: None, note: None, ..sample() };
        let v = medication_attestation_body(&a);
        let o = v.as_object().unwrap();
        assert!(!o.contains_key("basis"), "absent basis omitted, not null");
        assert!(!o.contains_key("note"), "absent note omitted, not null");
        assert_eq!(v["reviewed_count"], 4);
    }

    #[test]
    fn twin_is_nonempty_and_reads_naturally() {
        let s = render_medication_attestation_twin(&sample());
        assert!(s.contains("Reviewed"));
        assert!(s.contains("4 entries"));
        assert!(s.contains("admission reconciliation"));
        assert!(!s.trim().is_empty());
        // non-empty even with no basis/note
        let bare = MedicationAttestation { basis: None, note: None, ..sample() };
        assert!(!render_medication_attestation_twin(&bare).trim().is_empty());
    }

    #[test]
    fn twin_surfaces_note_when_present() {
        let a = MedicationAttestation { note: Some("verified with pharmacy"), ..sample() };
        let s = render_medication_attestation_twin(&a);
        assert!(s.contains("verified with pharmacy"), "note must surface, got: {s}");
    }
}
```

- [ ] **Step 2: Wire the module** — in `crates/cairn-event/src/medication/mod.rs`, add `pub mod attestation;` beside the other `pub mod` lines, and add to the re-exports:

```rust
pub use attestation::{
    medication_attestation_body, render_medication_attestation_twin, MedicationAttestation,
};
```

- [ ] **Step 3: Run tests to verify they fail** — Run: `cargo test -p cairn-event attestation`. Expected: FAIL to compile (module not yet wired) then, once wired, the tests fail if code is wrong; write code to green.

- [ ] **Step 4: Run tests to verify they pass** — Run: `cargo test -p cairn-event medication::attestation`. Expected: 4 passed.

- [ ] **Step 5: fmt + clippy + commit**

```bash
cargo fmt --all && cargo clippy -p cairn-event --all-targets -- -D warnings
git add crates/cairn-event/src/medication/attestation.rs crates/cairn-event/src/medication/mod.rs
git commit -m "feat(cairn-event): medication attestation payload builder + twin (slice 4)"
```

---

## Task 2: Split `cairn-node/src/medication.rs` into a module dir (companion refactor)

Pure code-move, **zero behaviour change**, so the file we add to (Task 5/6) stays under 500 lines (house rule 4). No new tests — the existing `cargo test -p cairn-node` suite (incl. `dose_build_tests` + `reconciliation_build_tests` inside the file, and `tests/medication*.rs`) is the safety net.

**Files:**
- Delete: `crates/cairn-node/src/medication.rs`
- Create: `crates/cairn-node/src/medication/mod.rs`, `.../assert.rs`, `.../cessation.rs`, `.../dose.rs`, `.../reconciliation.rs`
- Modify: none (the module is already declared `pub mod medication;` in `lib.rs` — a dir replaces the file transparently)

- [ ] **Step 1: Create the module dir with a re-exporting `mod.rs`.** Create `crates/cairn-node/src/medication/mod.rs`:

```rust
//! §3.15/§3.16 medication recording — the node authoring surface. Device-additive
//! by default (signed by the node key, a `recorded` contributor, no attestation);
//! the slice-4 attestation path (`attestation.rs`) layers human responsibility as a
//! separable overlay. Orchestrators over an immortal `medication_id` thread:
//! assert / cease / change-dose / correct-dose, plus reconcile / separate over a
//! thread PAIR. Offline-first throughout (no event requires its target thread to be
//! present locally). Split by verb to keep each file focused (house rule 4).
mod assert;
mod cessation;
mod dose;
mod reconciliation;

pub use assert::{build_assert_body, assert_medication, validate_term, AssertMedicationInput};
pub use cessation::{build_cease_body, cease_medication, CeaseMedicationInput};
pub use dose::{
    build_dose_change_body, build_dose_correction_body, change_dose, correct_dose,
    resolve_correction_target, ChangeDoseInput, CorrectDoseInput,
};
pub use reconciliation::{
    build_reconcile_body, build_separate_body, reconcile_medications, separate_medications,
    validate_distinct_subjects, ReconcileInput,
};
```

- [ ] **Step 2: Move each verb's code into its file verbatim.** From the current `medication.rs`, move:
  - `AssertMedicationInput`, `validate_term`, `build_assert_body`, `assert_medication`, plus the `MEDICATION_SCHEMA_VERSION` / `MEDICATION_CESSATION_SCHEMA_VERSION` consts → `assert.rs` (cessation const goes to `cessation.rs`).
  - `CeaseMedicationInput`, `build_cease_body`, `cease_medication` → `cessation.rs`.
  - `ChangeDoseInput`, `build_dose_change_body`, `change_dose`, `CorrectDoseInput`, `build_dose_correction_body`, `correct_dose`, `resolve_correction_target`, the `DOSE_*_SCHEMA_VERSION` consts, and the `#[cfg(test)] mod dose_build_tests` → `dose.rs`.
  - `ReconcileInput`, `validate_distinct_subjects`, `build_reconcile_like_body`, `build_reconcile_body`, `build_separate_body`, `reconcile_medications`, `separate_medications`, the `RECONCILIATION_*` / `SEPARATION_*` consts, and the `#[cfg(test)] mod reconciliation_build_tests` → `reconciliation.rs`.
  Each file needs its own `use` header (copy the imports each block actually uses from the original top-of-file `use` block: `cairn_event::medication::{...}`, `cairn_event::{sign, EventBody, Hlc, SigningKey}`, `uuid::Uuid`). Keep `crate::db::next_hlc` calls as-is.

- [ ] **Step 3: Delete the old file** — `git rm crates/cairn-node/src/medication.rs`.

- [ ] **Step 4: Verify green (no behaviour change)** — Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node`. Expected: the same set of tests pass as before the split (incl. `medication`, `medication_dose`, `medication_reconciliation`, and the in-file build tests). If any file exceeds 500 lines, that's acceptable here (dose.rs is the largest); the point is no single file grows further in later tasks.

- [ ] **Step 5: fmt + clippy + commit**

```bash
cargo fmt --all && cargo clippy -p cairn-node --all-targets -- -D warnings
git add -A crates/cairn-node/src/medication crates/cairn-node/src/medication.rs
git commit -m "refactor(cairn-node): split medication.rs into a per-verb module dir (no behaviour change)"
```

---

## Task 3: `db/034` part 1 — floor: registration, structural check, twin registry, commitment fn

**Files:**
- Create: `db/034_medication_attestation.sql`
- Modify: `crates/cairn-node/src/db.rs` (add `db/034_medication_attestation.sql` to the ordered migration load list — find where `db/033_medication_reconciliation.sql` is listed and add the new file immediately after)
- Test: `crates/cairn-node/tests/medication_attestation.rs` (new; floor tests only in this task)

**Interfaces:**
- Produces (SQL): `cairn_check_medication_attestation(p_type text, b jsonb) RETURNS void`; `cairn_medication_thread_commitment(p_medication_id uuid) RETURNS bytea`; registry rows for `clinical.medication-attestation.asserted`.

- [ ] **Step 1: Write `db/034_medication_attestation.sql` part 1.**

```sql
-- 034_medication_attestation.sql — slice 4 of the clinical.medication surface.
--
-- One additive verb: clinical.medication-attestation.asserted. A human takes
-- clinical responsibility (principle 10, ADR-0007) for one medication_id thread,
-- pinning a convergent commitment of the thread's content-event SET it reviewed.
-- Responsibility is enforced entirely by the db/005 attestation gate (the payload
-- carries a responsibility-bearing contributor -> the 3-arg door demands a valid
-- human token). This migration is purely structural floor + a set-commitment helper
-- + an overlay/projection (part 2). db/031, db/032, db/033 are UNTOUCHED, and the
-- current-list views are NOT widened (replay rule). See ADR-0049.
BEGIN;

-- 1. Register the verb (fail-closed registry, ADR-0010). Additive: an attestation
--    adds accountability and forecloses on nothing, so ADR-0043's owner-gate does
--    NOT apply and a clinician may vouch for a thread another author recorded.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-attestation.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. Structural floor. Culture-neutral, OFFLINE-FIRST (no check the thread exists
--    locally — set-union sync may deliver the attestation before the thread). The
--    twin requirement is enforced by the db/005 dispatcher via twin_required_msg
--    (step 3), NOT here. Mirrors cairn_check_medication_reconciliation.
CREATE OR REPLACE FUNCTION cairn_check_medication_attestation(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication attestation: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication attestation: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication attestation: medication_id must be a valid uuid';
    END;
    IF jsonb_typeof(b -> 'patient_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication attestation: patient_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (b ->> 'patient_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication attestation: patient_id must be a valid uuid';
    END;
    -- reviewed_commitment: a non-empty hex string (the pinned set commitment).
    IF jsonb_typeof(p -> 'reviewed_commitment') IS DISTINCT FROM 'string'
       OR (p ->> 'reviewed_commitment') !~ '^[0-9a-fA-F]+$' THEN
        RAISE EXCEPTION 'medication attestation: reviewed_commitment must be a non-empty hex string';
    END IF;
    -- reviewed_count: a non-negative integer legibility hint.
    IF jsonb_typeof(p -> 'reviewed_count') IS DISTINCT FROM 'number'
       OR (p ->> 'reviewed_count')::numeric < 0
       OR (p ->> 'reviewed_count')::numeric <> floor((p ->> 'reviewed_count')::numeric) THEN
        RAISE EXCEPTION 'medication attestation: reviewed_count must be a non-negative integer';
    END IF;
END;
$$;

-- 3. Register the verb's floor + HARD twin requirement in the #173/ADR-0048 registry
--    (the single db/005 dispatcher reads these rows). Placed AFTER the check fn above
--    so the fail-closed registry trigger sees cairn_check_medication_attestation(text,
--    jsonb) declared at load time (an implementer catch from #173: registry INSERT must
--    follow the CREATE, or a fresh load rolls back).
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-attestation.asserted', 'cairn_check_medication_attestation',
     'medication attestation requires a non-empty authored twin (§3.13/§3.15)')
ON CONFLICT (event_type) DO NOTHING;

-- 4. The set-commitment SINGLE SOURCE. Sorted-concat-hash of the thread's content-event
--    content_addresses (byte order -> order-independent, collation-free; mirrors
--    event_set_commitment in medium.rs). Called at BOTH author time (the orchestrator
--    pins this value) and read time (the staleness view recomputes it) -> byte-identity
--    guaranteed, no Rust<->SQL drift. NULL when the thread has no local content events
--    (orphan): the orchestrator bails, the projection reads NULL -> stale. Content
--    events EXCLUDE reconciliation/separation/attestation (not thread content).
CREATE OR REPLACE FUNCTION cairn_medication_thread_commitment(p_medication_id uuid)
RETURNS bytea LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN count(*) = 0 THEN NULL
                ELSE digest(string_agg(content_address, ''::bytea ORDER BY content_address), 'sha256')
           END
    FROM event_log
    WHERE event_type IN (
            'clinical.medication.asserted',
            'clinical.medication-cessation.asserted',
            'clinical.medication-dose-change.asserted',
            'clinical.medication-dose-correction.asserted')
      AND (body ->> 'medication_id')::uuid = p_medication_id;
$$;

COMMIT;
```

- [ ] **Step 2: Add the migration to the load list.** In `crates/cairn-node/src/db.rs`, locate the ordered list of embedded migrations (the `include_str!("../../../db/033_medication_reconciliation.sql")` entry) and add immediately after it:

```rust
include_str!("../../../db/034_medication_attestation.sql"),
```

(Match the exact surrounding syntax — it is an array/slice of `&str`. If the list uses filename strings loaded at runtime instead, add `"034_medication_attestation.sql"` in the same style. Confirm by reading the existing db/033 entry.)

- [ ] **Step 3: Write the failing floor tests.** Create `crates/cairn-node/tests/medication_attestation.rs`:

```rust
//! DB-gated tests for slice-4 medication attestation (floor first). Requires
//! CAIRN_TEST_PG (PG18 + cairn_pgx). Uses the shared test harness conventions from
//! tests/medication_reconciliation.rs — read it for the `connect()` + node-provision
//! helpers and copy the module-local helpers this file needs.
mod common; // if the repo has a shared tests/common; otherwise inline the connect helper (see medication_reconciliation.rs)

// NOTE for the implementer: mirror the exact harness bootstrap used by
// crates/cairn-node/tests/medication_reconciliation.rs (connection string from
// CAIRN_TEST_PG, schema load, node key provisioning, a `submit_raw` helper that
// signs+submits an EventBody). Reuse its helpers rather than re-inventing them.

#[tokio::test]
async fn floor_accepts_well_formed_attestation() {
    // Provision a node + patient + one medication thread (assert), then submit a
    // well-formed attestation via the human 3-arg door; expect success.
    // (Full body construction lands in Task 5's build_attestation_body; for this
    // floor-only task, construct the attestation EventBody inline with a
    // responsibility-bearing contributor + a valid human attestation token, exactly
    // as tests/attestation.rs / apply_proposal do.)
}

#[tokio::test]
async fn floor_rejects_bad_medication_id() {
    // Submit an attestation whose payload.medication_id = "not-a-uuid"; expect the
    // floor to raise 'medication_id must be a valid uuid'.
}

#[tokio::test]
async fn floor_rejects_malformed_commitment() {
    // payload.reviewed_commitment = "zzzz" (non-hex) -> raise 'must be a non-empty hex string'.
}

#[tokio::test]
async fn floor_rejects_negative_count() {
    // payload.reviewed_count = -1 -> raise 'must be a non-negative integer'.
}

#[tokio::test]
async fn commitment_fn_is_deterministic_and_null_when_absent() {
    // SELECT cairn_medication_thread_commitment on a thread with 1 content event ->
    // non-null, stable across calls; on an unknown uuid -> NULL.
}
```

Because the responsibility-bearing attestation body needs the human token plumbing that Task 5 introduces, **write these five tests using the same inline signing/attestation the existing `crates/cairn-node/tests/attestation.rs` uses** (mint a human key at runtime via `std::array::from_fn`, enroll it as `kind='human'` with the `enroll-human` path or the raw actor_event insert the existing tests use, build the `EventBody` with the responsibility contributor, `sign` + `sign_attestation`, submit via `SELECT submit_event($1,$2,$3)`). Fill in each test body with the concrete construction (no placeholder bodies at commit time).

- [ ] **Step 4: Run to verify they fail, then pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_attestation`
Expected: initially FAIL (migration/floor absent), then PASS after Steps 1–2 load. Confirm the floor rejections raise the exact messages.

- [ ] **Step 5: SQL mirror + commit.** Add matching assertions to the DB-suite SQL mirror if the repo keeps one for medication (check for `db/tests/03*_*.sql`; if a `db/tests/033_*` exists, add a sibling `db/tests/034_medication_attestation_test.sql` asserting the floor fn + registry row exist and the commitment fn returns NULL for an unknown uuid). Then:

```bash
cargo fmt --all
git add db/034_medication_attestation.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/medication_attestation.rs db/tests/034_medication_attestation_test.sql
git commit -m "feat(db/034): medication attestation floor + set-commitment fn (slice 4 part 1)"
```

---

## Task 4: `db/034` part 2 — attestation overlay table, apply trigger, projection views

**Files:**
- Modify: `db/034_medication_attestation.sql` (append part 2)
- Test: `crates/cairn-node/tests/medication_attestation.rs` (add projection tests)

**Interfaces:**
- Produces (SQL): table `medication_attestation`; trigger `medication_attestation_apply`; views `medication_thread_attestation`, `medication_group_attestation`.

- [ ] **Step 1: Append part 2 to `db/034_medication_attestation.sql`.**

```sql
BEGIN;

-- 5. The attestation overlay: one row per attestation event (append-only; every
--    vouch retained for audit). attester_kid is the VERIFIED responsible human,
--    read from event_log.attester_key (the db/005 gate stored it after checking the
--    token + kind='human'). reviewed_commitment stored as bytea for a direct compare.
CREATE TABLE IF NOT EXISTS medication_attestation (
    event_id            UUID PRIMARY KEY,       -- the attestation event's own id
    medication_id       UUID NOT NULL,
    patient_id          UUID NOT NULL,
    attester_kid        TEXT NOT NULL,          -- hex of the verified human attester key
    reviewed_commitment BYTEA NOT NULL,
    reviewed_count      INTEGER NOT NULL,
    hlc_wall            BIGINT NOT NULL,
    hlc_counter         INTEGER NOT NULL,
    origin              TEXT NOT NULL,
    content_address     BYTEA NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_attestation TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_attestation_thread_idx
    ON medication_attestation (medication_id);

-- 6. Apply trigger: fold each attestation event into the overlay (door-agnostic —
--    fires for both the local submit door and the db/020 remote-apply door). Append
--    a row keyed by the event's own id; a re-delivered event is deduped by the PK.
CREATE OR REPLACE FUNCTION medication_attestation_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_attestation
        (event_id, medication_id, patient_id, attester_kid, reviewed_commitment,
         reviewed_count, hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        NEW.event_id,
        (p ->> 'medication_id')::uuid,
        NEW.patient_id,
        encode(NEW.attester_key, 'hex'),                  -- verified human (db/005 gate)
        decode(p ->> 'reviewed_commitment', 'hex'),
        (p ->> 'reviewed_count')::integer,
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (event_id) DO NOTHING;                    -- append-only, idempotent
    RETURN NULL;  -- AFTER trigger
END;
$$;
DROP TRIGGER IF EXISTS medication_attestation_apply_trg ON event_log;
CREATE TRIGGER medication_attestation_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication-attestation.asserted')
    EXECUTE FUNCTION medication_attestation_apply();

-- 7. Per-thread standing attestation: the LATEST vouch per thread (by its own
--    convergent position), with the staleness verdict = "the thread's current
--    content-set commitment differs from what this vouch reviewed". Because thread
--    content is append-only (grow-only), ANY later content event (higher OR lower
--    HLC) changes the commitment -> stale. content_address is bytea -> byte-order
--    tiebreak needs no COLLATE.
CREATE OR REPLACE VIEW medication_thread_attestation AS
SELECT DISTINCT ON (a.medication_id)
       a.medication_id,
       a.patient_id,
       a.attester_kid,
       a.hlc_wall     AS attested_wall,
       a.hlc_counter  AS attested_counter,
       a.reviewed_count,
       (cairn_medication_thread_commitment(a.medication_id) IS DISTINCT FROM a.reviewed_commitment)
           AS stale
FROM medication_attestation a
ORDER BY a.medication_id, a.hlc_wall DESC, a.hlc_counter DESC, a.content_address DESC;
GRANT SELECT ON medication_thread_attestation TO cairn_agent;

-- 8. Group rollup (conservative): a reconciled group is "attested & current" iff
--    EVERY member thread has a non-stale attestation. Singletons (group_id =
--    medication_id) reduce trivially to their thread. medication_thread_group (db/033)
--    lists every locally-asserted thread with its group_id, so an orphan attestation
--    (no local assert) is simply not a member -> renders nothing until it arrives.
CREATE OR REPLACE VIEW medication_group_attestation AS
SELECT g.group_id,
       g.patient_id,
       bool_and(ta.medication_id IS NOT NULL AND NOT ta.stale)      AS attested_current,
       count(*) FILTER (WHERE ta.medication_id IS NULL)             AS unattested_members,
       count(*) FILTER (WHERE ta.stale)                             AS stale_members
FROM medication_thread_group g
LEFT JOIN medication_thread_attestation ta ON ta.medication_id = g.medication_id
GROUP BY g.group_id, g.patient_id;
GRANT SELECT ON medication_group_attestation TO cairn_agent;

COMMIT;
```

- [ ] **Step 2: Write the failing projection tests** in `crates/cairn-node/tests/medication_attestation.rs`:

```rust
#[tokio::test]
async fn post_hoc_attestation_shows_attester_and_not_stale() {
    // assert a thread -> submit an attestation pinning the current commitment ->
    // SELECT * FROM medication_thread_attestation WHERE medication_id = $thread:
    // one row, attester_kid = the human, stale = false.
}

#[tokio::test]
async fn later_change_flips_stale_true() {
    // assert -> attest (stale=false) -> dose-change on the thread ->
    // medication_thread_attestation.stale = true.
}

#[tokio::test]
async fn lower_hlc_late_arrival_flips_stale_true() {
    // THE LOAD-BEARING TEST. assert (hlc=100) -> attest pinning commitment over
    // {assert} -> insert a dose-change with a LOWER hlc (e.g. hlc=50, an earlier-wall
    // event that "synced late") on the same thread -> the content SET changed, so the
    // commitment changes -> stale = true. Proves the set-commitment closes the gap a
    // head-position pin would miss (position 50 < the pinned head, yet still stale).
    // Construct the low-hlc event by minting its EventBody with an explicit low Hlc
    // and submitting through the local door (submit_event does not reject a lower HLC
    // for a distinct content_address).
}

#[tokio::test]
async fn group_current_only_when_all_members_current() {
    // assert thread A + thread B; reconcile(A,B); attest A and B ->
    // medication_group_attestation.attested_current = true. Then dose-change B ->
    // attested_current = false (B stale), stale_members = 1.
}

#[tokio::test]
async fn orphan_attestation_renders_nothing_until_thread_arrives() {
    // Submit an attestation for a medication_id with NO local content events (offline-
    // first): floor accepts; medication_thread_group has no member row -> the thread is
    // absent from medication_group_attestation. (medication_thread_attestation MAY show
    // a stale=true row since the current commitment is NULL; assert it is not surfaced
    // in the group rollup.)
}
```

Fill in each test body concretely (reuse the Task 5 `attest_medication_thread` once it exists, or the inline attestation construction from Task 3 — whichever is available when this task runs; if run before Task 5, use the inline construction).

- [ ] **Step 3: Run to verify fail, then pass** — Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_attestation`. Expected: the projection tests fail before part 2 loads, pass after.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add db/034_medication_attestation.sql crates/cairn-node/tests/medication_attestation.rs
git commit -m "feat(db/034): medication attestation overlay + staleness projections (slice 4 part 2)"
```

---

## Task 5: `cairn-node::medication::attestation` — post-hoc orchestrator

**Files:**
- Create: `crates/cairn-node/src/medication/attestation.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs` (declare + re-export)
- Test: `crates/cairn-node/tests/medication_attestation.rs` (add orchestrator + responsibility-gate tests)

**Interfaces:**
- Consumes: `cairn_event::medication::{medication_attestation_body, render_medication_attestation_twin, MedicationAttestation}` (Task 1); `cairn_medication_thread_commitment` (Task 3); `cairn_event::{sign, sign_attestation, event_address, EventBody, Hlc, SigningKey}`.
- Produces:
  - `pub struct AttestParams<'a> { pub human_sk: &'a SigningKey, pub human_kid: &'a str, pub basis: Option<&'a str>, pub note: Option<&'a str> }`
  - `pub fn build_attestation_body(event_id: Uuid, medication_id: Uuid, patient: Uuid, reviewed_commitment: &str, reviewed_count: u32, basis: Option<&str>, note: Option<&str>, human_kid: &str, hlc: Hlc) -> EventBody`
  - `pub async fn thread_commitment(client, medication_id: Uuid) -> anyhow::Result<Option<(String, u32)>>` (hex commitment + count; None when no local content events)
  - `pub async fn attest_thread_in_tx(tx: &tokio_postgres::Transaction<'_>, params: &AttestParams<'_>, patient: Uuid, medication_id: Uuid, hlc: Hlc) -> anyhow::Result<Uuid>`
  - `pub async fn attest_medication_thread(client: &mut tokio_postgres::Client, node_origin: &str, params: &AttestParams<'_>, patient: Uuid, medication_id: Uuid) -> anyhow::Result<Uuid>`

- [ ] **Step 1: Write the module.** Create `crates/cairn-node/src/medication/attestation.rs`:

```rust
//! Slice-4 human-attested clinical responsibility on a medication thread. Device-
//! signed medication events stay unchanged; this authors a SEPARATE
//! `clinical.medication-attestation.asserted` event carrying the responsible human as
//! a responsibility-bearing contributor -> the db/005 gate demands a valid human
//! attestation token (the 3-arg door). A sign-off pins a convergent commitment of the
//! thread's content-event set (via `cairn_medication_thread_commitment`, db/034) so a
//! later change flips the vouch stale. Reused by the post-hoc CLI and the author-time
//! `--attest-as` path (attestation.rs is the single owner of the attestation seam).
use cairn_event::medication::{
    medication_attestation_body, render_medication_attestation_twin, MedicationAttestation,
};
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use uuid::Uuid;

const ATTESTATION_SCHEMA_VERSION: &str = "clinical.medication-attestation/1";

/// The human who takes responsibility, plus optional context. Threaded explicitly so
/// the author paths stay pure functions of their arguments. The human key both signs
/// AND attests the attestation event (the `identify --link` precedent).
pub struct AttestParams<'a> {
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
    pub basis: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-attestation.asserted` `EventBody`. Pure.
/// The sole contributor is the vouching human with a `responsibility` marker — this is
/// what makes submit_event demand a valid human attestation token (db/005 gate).
#[allow(clippy::too_many_arguments)] // event id + thread + patient + pin + count + basis/note + human + hlc
pub fn build_attestation_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    reviewed_commitment: &str,
    reviewed_count: u32,
    basis: Option<&str>,
    note: Option<&str>,
    human_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let a = MedicationAttestation {
        medication_id: &mid,
        reviewed_commitment,
        reviewed_count,
        basis,
        note,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-attestation.asserted".into(),
        schema_version: ATTESTATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: human_kid.into(),
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "attested", "responsibility": "attested"}
        ]),
        payload: medication_attestation_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_medication_attestation_twin(&a)),
    }
}

/// Read the thread's current set commitment (hex) + content-event count from the
/// single-source SQL fn. `None` when the thread has no LOCAL content events (orphan) —
/// the caller then refuses to author a meaningless vouch.
pub async fn thread_commitment(
    client: &tokio_postgres::Client,
    medication_id: Uuid,
) -> anyhow::Result<Option<(String, u32)>> {
    let row = client
        .query_one(
            "SELECT encode(cairn_medication_thread_commitment($1::text::uuid), 'hex') AS c, \
             (SELECT count(*) FROM event_log \
                WHERE event_type IN ('clinical.medication.asserted', \
                    'clinical.medication-cessation.asserted', \
                    'clinical.medication-dose-change.asserted', \
                    'clinical.medication-dose-correction.asserted') \
                  AND (body ->> 'medication_id')::uuid = $1::text::uuid) AS n",
            &[&medication_id.to_string()],
        )
        .await?;
    let commitment: Option<String> = row.get("c");
    let count: i64 = row.get("n");
    Ok(commitment.map(|c| (c, count as u32)))
}

/// Author one attestation for `medication_id` INSIDE the caller's transaction (so it
/// sees any content event the caller just submitted in the same txn). Computes the
/// commitment, signs with the human key, mints the token, submits the 3-arg door.
/// Returns the attestation event id. Errors (rolling the caller's txn back) if the
/// thread has no local content events or the db/005 gate refuses.
pub async fn attest_thread_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    params: &AttestParams<'_>,
    patient: Uuid,
    medication_id: Uuid,
    hlc: Hlc,
) -> anyhow::Result<Uuid> {
    // thread_commitment takes &Client; a Transaction derefs to a GenericClient — run
    // the same query directly on the tx to keep one txn/snapshot.
    let row = tx
        .query_one(
            "SELECT encode(cairn_medication_thread_commitment($1::text::uuid), 'hex') AS c, \
             (SELECT count(*) FROM event_log \
                WHERE event_type IN ('clinical.medication.asserted', \
                    'clinical.medication-cessation.asserted', \
                    'clinical.medication-dose-change.asserted', \
                    'clinical.medication-dose-correction.asserted') \
                  AND (body ->> 'medication_id')::uuid = $1::text::uuid) AS n",
            &[&medication_id.to_string()],
        )
        .await?;
    let commitment: Option<String> = row.get("c");
    let count: i64 = row.get("n");
    let commitment = commitment.ok_or_else(|| {
        anyhow::anyhow!(
            "no local content for medication thread {medication_id}; nothing to vouch for \
             (author or sync the thread first)"
        )
    })?;

    let event_id = Uuid::now_v7();
    let body = build_attestation_body(
        event_id,
        medication_id,
        patient,
        &commitment,
        count as u32,
        params.basis,
        params.note,
        params.human_kid,
        hlc,
    );
    let signed = sign(&body, params.human_sk)?;
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, params.human_kid, "attested", params.human_sk)?;
    let attester_vk = params.human_sk.verifying_key().to_bytes().to_vec();
    tx.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &attester_vk],
    )
    .await?;
    Ok(event_id)
}

/// Post-hoc standalone sign-off: mint an HLC, open a one-statement txn, attest, commit.
pub async fn attest_medication_thread(
    client: &mut tokio_postgres::Client,
    node_origin: &str,
    params: &AttestParams<'_>,
    patient: Uuid,
    medication_id: Uuid,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let tx = client.transaction().await?;
    let id = attest_thread_in_tx(&tx, params, patient, medication_id, hlc).await?;
    tx.commit().await?;
    Ok(id)
}
```

- [ ] **Step 2: Wire into `mod.rs`** — add `mod attestation;` and:

```rust
pub use attestation::{
    attest_medication_thread, attest_thread_in_tx, build_attestation_body, thread_commitment,
    AttestParams,
};
```

- [ ] **Step 3: Write the failing responsibility-gate + happy-path tests** in `tests/medication_attestation.rs`:

```rust
#[tokio::test]
async fn attestation_requires_a_valid_human_token() {
    // Build the attestation EventBody with the responsibility contributor, sign it,
    // but submit through the 1-arg door (no token) -> submit_event RAISES
    // 'requires attestation'. Then submit with an AGENT attester (a kind!='human'
    // enrolled key) -> RAISES 'attester is not an enrolled human actor'.
}

#[tokio::test]
async fn attest_medication_thread_end_to_end() {
    // Provision node + an enrolled human key; assert a thread; call
    // attest_medication_thread(...) -> Ok(event_id); medication_thread_attestation
    // shows stale=false + attester_kid = the human's hex kid.
}

#[tokio::test]
async fn attest_refuses_orphan_thread_with_clear_message() {
    // Call attest_medication_thread for a random medication_id with no local content
    // events -> Err containing "nothing to vouch for".
}
```

Enroll the human via the real `enroll-human` path if convenient, else the raw `actor_event` insert the existing `tests/attestation.rs` / `recall_epoch.rs` use (mint the key at runtime with `std::array::from_fn`, never a literal — house rule 6).

- [ ] **Step 4: Run to verify fail then pass** — Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_attestation`. Expected: PASS.

- [ ] **Step 5: fmt + clippy + commit**

```bash
cargo fmt --all && cargo clippy -p cairn-node --all-targets -- -D warnings
git add crates/cairn-node/src/medication/attestation.rs crates/cairn-node/src/medication/mod.rs crates/cairn-node/tests/medication_attestation.rs
git commit -m "feat(cairn-node): post-hoc medication attestation orchestrator (slice 4)"
```

---

## Task 6: Author-time — wire `--attest-as` into all six verb orchestrators

Each verb gains an optional `attest: Option<&AttestParams<'_>>`. When present, the verb's submit and the attestation(s) run in ONE transaction. Single-thread verbs attest their `medication_id`; the pair verbs (`reconcile`/`separate`) attest **both** subject threads.

**Files:**
- Modify: `crates/cairn-node/src/medication/assert.rs`, `cessation.rs`, `dose.rs`, `reconciliation.rs`
- Test: `crates/cairn-node/tests/medication_attestation.rs` (author-time + supersede tests)

**Interfaces:**
- Consumes: `attest_thread_in_tx`, `AttestParams` (Task 5); `crate::db::next_hlc`.
- Produces: an added trailing `attest: Option<&AttestParams<'_>>` parameter on `assert_medication`, `cease_medication`, `change_dose`, `correct_dose`, `reconcile_medications`, `separate_medications` (each returns the same primary id as before; the attestation ids are internal).

- [ ] **Step 1: Refactor `assert_medication` to the atomic author-time shape.** Replace the current body of `assert_medication` in `assert.rs` with (note the signature gains `attest`):

```rust
pub async fn assert_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_term(input.term)?;
    // Mint HLCs up front (self-committing; a rolled-back submit just leaves a gap —
    // the HLC is monotonic and gaps are allowed, exactly like identify_patient).
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let body = build_assert_body(event_id, medication_id, patient, input, node_kid, verb_hlc);
    let signed = sign(&body, node_sk)?;

    match attest {
        None => {
            // Unchanged device-additive path (1-arg door, auto-commit).
            client
                .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
                .await?;
        }
        Some(params) => {
            let attest_hlc = crate::db::next_hlc(client, node_origin).await?;
            let tx = client.transaction().await?;
            tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
                .await?;
            crate::medication::attest_thread_in_tx(&tx, params, patient, medication_id, attest_hlc)
                .await?;
            tx.commit().await?;
        }
    }
    Ok(medication_id)
}
```

- [ ] **Step 2: Apply the identical `attest: Option<&AttestParams>` shape to the other single-thread verbs** (`cease_medication`, `change_dose`, `correct_dose`). Each already knows its `medication_id`; the pattern is the same as Step 1 — mint the verb HLC, sign the verb body, then `match attest { None => 1-arg submit; Some(params) => mint attest HLC, open txn, submit verb (1-arg), attest_thread_in_tx(tx, params, patient, medication_id, attest_hlc), commit }`. Add `attest: Option<&crate::medication::AttestParams<'_>>` as the trailing parameter to each. (Repeat the code per verb — the engineer may read tasks out of order; do not write "same as assert".) For `correct_dose`, the `medication_id` is already a parameter; attest that thread.

- [ ] **Step 3: Wire the pair verbs (`reconcile_medications` / `separate_medications`) to attest BOTH subjects.** In `reconciliation.rs`, add `attest: Option<&crate::medication::AttestParams<'_>>` and, in the `Some` branch, after submitting the reconcile/separate event, attest each subject thread in the SAME txn:

```rust
pub async fn reconcile_medications(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    input: &ReconcileInput<'_>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_distinct_subjects(subject_a, subject_b)?;
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_reconcile_body(event_id, subject_a, subject_b, patient, input, node_kid, verb_hlc);
    let signed = sign(&body, node_sk)?;
    match attest {
        None => {
            client.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;
        }
        Some(params) => {
            // Two attestation HLCs (one per subject thread), minted up front.
            let hlc_a = crate::db::next_hlc(client, node_origin).await?;
            let hlc_b = crate::db::next_hlc(client, node_origin).await?;
            let tx = client.transaction().await?;
            tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;
            crate::medication::attest_thread_in_tx(&tx, params, patient, subject_a, hlc_a).await?;
            crate::medication::attest_thread_in_tx(&tx, params, patient, subject_b, hlc_b).await?;
            tx.commit().await?;
        }
    }
    Ok(event_id)
}
```

Apply the identical shape to `separate_medications` (repeat the code, calling `build_separate_body`).

- [ ] **Step 4: Update existing call sites.** Every current caller of these six orchestrators (the CLI in `main.rs`, and any test helpers) must pass `None` for the new `attest` parameter to compile. Grep: `rg 'assert_medication\(|cease_medication\(|change_dose\(|correct_dose\(|reconcile_medications\(|separate_medications\(' crates/cairn-node`. Add `None` (or `attest.as_ref()`) at each. (Task 7 replaces the `main.rs` sites with real flag handling; for now `None` keeps the tree compiling.)

- [ ] **Step 5: Write the failing author-time + supersede tests** in `tests/medication_attestation.rs`:

```rust
#[tokio::test]
async fn author_time_assert_is_attested_current_in_one_txn() {
    // assert_medication(..., Some(&params)) -> medication_thread_attestation shows the
    // new thread with stale=false immediately; exactly one attestation row exists.
}

#[tokio::test]
async fn author_time_rejection_rolls_the_verb_back() {
    // assert_medication with a params whose human key is NOT enrolled kind='human' ->
    // Err; assert NO medication_statement row and NO attestation row were written
    // (the whole txn rolled back).
}

#[tokio::test]
async fn author_time_dose_change_is_attested_current() {
    // assert a thread (device-additive); change_dose(..., Some(&params)) -> the thread's
    // attestation is stale=false and pins the post-change commitment.
}

#[tokio::test]
async fn reconcile_attest_as_vouches_for_both_threads() {
    // assert A + assert B; reconcile_medications(A, B, ..., Some(&params)) ->
    // medication_thread_attestation has non-stale rows for BOTH A and B;
    // medication_group_attestation.attested_current = true for the group.
}

#[tokio::test]
async fn supersede_not_retract_correction_flips_prior_vouch_stale() {
    // assert -> attest (stale=false). Author a dose-correction ("acted in error, correct
    // is X"). The prior attestation row is STILL in medication_attestation (retained),
    // and medication_thread_attestation.stale = true. Then attest again -> stale=false.
    // Proves responsibility is superseded, never retracted.
}
```

- [ ] **Step 6: Run to verify fail then pass** — Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_attestation`. Expected: PASS. Also run `cargo test -p cairn-node` to confirm the `None`-passing call-site updates didn't break `medication`/`medication_dose`/`medication_reconciliation`.

- [ ] **Step 7: fmt + clippy + commit**

```bash
cargo fmt --all && cargo clippy -p cairn-node --all-targets -- -D warnings
git add crates/cairn-node/src/medication crates/cairn-node/tests/medication_attestation.rs
git commit -m "feat(cairn-node): author-time --attest-as on all six medication verbs (slice 4)"
```

---

## Task 7: CLI — `medication-attest` + `--attest-as` on every verb

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (new subcommand; new shared flag struct + resolver; add flags to the six verb subcommands)

**Interfaces:**
- Consumes: `cairn_node::medication::{attest_medication_thread, AttestParams, ...}`; `load_attester_key`, `attester_is_enrolled_human` (existing in `main.rs` / `identify.rs`).

- [ ] **Step 1: Add a shared clap flag group + a resolver helper.** Near the other `load_*` helpers in `main.rs`, add:

```rust
/// The `--attest-as` flag set, shared by every medication verb and reused as the
/// author-time convenience. `--attest-as` present ⇒ author-time human attestation.
#[derive(clap::Args, Clone)]
struct AttestFlags {
    /// Take clinical responsibility for the affected thread(s): a human key that
    /// signs+attests the attestation. Absent ⇒ device-additive (no vouch).
    #[arg(long)]
    attest_as: Option<std::path::PathBuf>,
    /// Passphrase to unseal --attest-as (else CAIRN_ATTESTER_PASSPHRASE, else prompt).
    #[arg(long)]
    attest_passphrase: Option<String>,
    /// Optional context recorded on the vouch (e.g. "admission reconciliation").
    #[arg(long)]
    basis: Option<String>,
    /// Optional free-text note on the vouch.
    #[arg(long)]
    note: Option<String>,
}

/// Resolve `--attest-as` into a loaded human key + verified kid, or None when the flag
/// is absent. Runs the `attester_is_enrolled_human` legibility pre-check (the db/005
/// gate is the real enforcement). Errors if a passphrase/basis/note is given with no
/// key (nothing to attest — refuse loudly, like identify-patient's cross-flag check).
async fn resolve_attester(
    client: &tokio_postgres::Client,
    flags: &AttestFlags,
) -> anyhow::Result<Option<(cairn_event::SigningKey, String)>> {
    match &flags.attest_as {
        None => {
            if flags.attest_passphrase.is_some() || flags.basis.is_some() || flags.note.is_some() {
                anyhow::bail!(
                    "--attest-passphrase/--basis/--note require --attest-as: nothing to attest"
                );
            }
            Ok(None)
        }
        Some(path) => {
            let sk = load_attester_key(path, flags.attest_passphrase.clone())?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            if !cairn_node::identify::attester_is_enrolled_human(client, &kid).await? {
                anyhow::bail!(
                    "--attest-as key is not an enrolled human actor; run `enroll-human` first"
                );
            }
            Ok(Some((sk, kid)))
        }
    }
}
```

(Confirm `load_attester_key`'s exact signature from `main.rs` and match it; the design references it taking a path + optional passphrase.)

- [ ] **Step 2: Add the `medication-attest` subcommand.** Add a `Cmd` variant + its handler:

```rust
/// Take clinical responsibility for an existing medication thread (post-hoc med-rec
/// sign-off). Records who vouched and pins the reviewed state so a later change flags
/// the vouch as needing re-review.
MedicationAttest {
    /// The medication_id thread to vouch for.
    medication_id: Uuid,
    /// Patient UUID (the chart the thread belongs to).
    #[arg(long)]
    patient: Uuid,
    #[command(flatten)]
    attest: AttestFlags,
},
```

Handler (in the `match cli.command` block):

```rust
Cmd::MedicationAttest { medication_id, patient, attest } => {
    let (mut client, node_origin) = connect_node(&cli).await?; // match the repo's connect helper
    let resolved = resolve_attester(&client, &attest).await?
        .ok_or_else(|| anyhow::anyhow!("medication-attest requires --attest-as: a vouch needs a responsible human"))?;
    let (human_sk, human_kid) = resolved;
    let params = cairn_node::medication::AttestParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
        basis: attest.basis.as_deref(),
        note: attest.note.as_deref(),
    };
    let id = cairn_node::medication::attest_medication_thread(
        &mut client, &node_origin, &params, patient, medication_id,
    ).await?;
    println!("attested medication thread {medication_id} (event {id})");
}
```

Match the exact node-connection/origin bootstrap the sibling medication subcommands use (read the `Cmd::MedicationAssert` handler and copy its connect + `node_origin` derivation).

- [ ] **Step 3: Add `#[command(flatten)] attest: AttestFlags` to each of the six verb subcommands** (`MedicationAssert`, `MedicationCease`, `MedicationChangeDose`, `MedicationCorrectDose`, `MedicationReconcile`, `MedicationSeparate` — match the actual variant names) and, in each handler, resolve + thread it:

```rust
// inside e.g. the MedicationAssert handler, after `client` is available:
let resolved = resolve_attester(&client, &attest).await?;
let params = resolved.as_ref().map(|(sk, kid)| cairn_node::medication::AttestParams {
    human_sk: sk, human_kid: kid, basis: attest.basis.as_deref(), note: attest.note.as_deref(),
});
// then pass `params.as_ref()` as the new trailing `attest` arg to the orchestrator:
let medication_id = cairn_node::medication::assert_medication(
    &mut client, &node_sk, &node_kid, &node_origin, patient, &input, params.as_ref(),
).await?;
```

Repeat for the other five handlers (each already builds its input + calls its orchestrator; add `params.as_ref()` as the trailing arg). Do not write "same as assert" — show each handler's changed call line.

- [ ] **Step 4: Manual e2e CLI smoke (record the transcript in the commit body).** On a provisioned node with an enrolled human key:

```bash
# assert a thread, then vouch author-time
cargo run -p cairn-node -- medication-assert --patient <pid> --term metformin --info-source patient-reported --attest-as <human.key>
# post-hoc vouch on an existing thread
cargo run -p cairn-node -- medication-attest <mid> --patient <pid> --attest-as <human.key> --basis "admission reconciliation"
# change dose -> the vouch should now read stale; re-attest to clear it
cargo run -p cairn-node -- medication-change-dose <mid> --patient <pid> --dose-amount 1000 --dose-unit mg --info-source clinician-observed
cargo run -p cairn-node -- medication-attest <mid> --patient <pid> --attest-as <human.key>
# verify via SQL: SELECT stale FROM medication_thread_attestation WHERE medication_id = '<mid>';
```

- [ ] **Step 5: fmt + clippy + commit**

```bash
cargo fmt --all && cargo clippy -p cairn-node --all-targets -- -D warnings
git add crates/cairn-node/src/main.rs
git commit -m "feat(cli): medication-attest + --attest-as on every medication verb (slice 4)"
```

---

## Task 8: ADR-0049 + spec bump + HANDOVER/ROADMAP

**Files:**
- Create: `docs/spec/decisions/0049-commitment-based-sign-off-currency.md`
- Modify: `docs/spec/index.md` (version v0.49 → v0.50 + ADR-index row), `docs/spec/data-model.md` (a §3.15/§3.16 paragraph pointing at the ADR — match how ADR-0047 is referenced), `docs/spec/decisions/README.md` (index row), `docs/HANDOVER.md`, `docs/ROADMAP.md`
- Note: `docs/CLAUDE.md` rules require the docs build via the pinned requirements.

- [ ] **Step 1: Write ADR-0049.** Follow the exact header/format of `docs/spec/decisions/0047-medication-reconciliation-resolution.md`. Content: **Context** — medication (and every future clinical stream) needs human-attested responsibility with an honest "still current?" signal (#163); a head-position pin silently absorbs a lower-HLC late-arriving event as "reviewed." **Decision** — responsibility is a separable per-thread attestation overlay (principle 10) through the db/005 gate; a sign-off pins a convergent set-commitment of the reviewed content-event set; staleness = commitment compare (sound because thread content is append-only/grow-only); responsibility is **superseded by correcting the record, never retracted** (no de-attestation event). **Consequences** — closes the lower-HLC gap + the cross-node divergent-set case; advances #163; binds future "sign-off currency" work; residual (can't review the future — the §5.13 sweep backstops); reuses ADR-0007 principle 10, the db/005 gate, ADR-0045 collation-free positions. Home: §3.15/§3.16.

- [ ] **Step 2: Bump the spec.** In `docs/spec/index.md`, change the version line to v0.50 and add the ADR-0049 row to the index table (copy the ADR-0047/0048 row format). Add the one-line ADR entry to `docs/spec/decisions/README.md`. In `docs/spec/data-model.md`, add a short paragraph under the medication section referencing ADR-0049 (mirror how ADR-0047 reconciliation is referenced).

- [ ] **Step 3: Verify docs build** — Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`. Expected: builds clean (no broken links to the new ADR).

- [ ] **Step 4: Update HANDOVER + ROADMAP.** Add the slice-4 "this session" block to `docs/HANDOVER.md` (top) and a "Slice 33 — medication attestation" entry to `docs/ROADMAP.md` Phase 4, both concise. Prune older HANDOVER detail toward the 500-line target (the maintainer's house rule 8 — condense superseded session blocks into one-liners; do not delete the ADR index).

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "docs(adr): ADR-0049 commitment-based sign-off currency; spec v0.50; handover/roadmap (slice 4)"
```

---

## Task 9: Whole-branch verification + code review

**Files:** none (verification only).

- [ ] **Step 1: Full workspace green.**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
```

Expected: fmt clean; clippy clean; **all** suites pass, including `medication_attestation` (all), and the untouched `medication` / `medication_dose` / `medication_reconciliation` still green (proves db/031–033 + the current-list views are unbroken across the reconnects that re-run every migration).

- [ ] **Step 2: Confirm the current-list views are unwidened.** Run: `git diff main -- db/031_medication.sql db/032_medication_dose.sql db/033_medication_reconciliation.sql` → expect **empty** (no change). Confirms replay-safety by construction.

- [ ] **Step 3: mkdocs build clean** — Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`.

- [ ] **Step 4: Whole-branch code review.** Invoke `superpowers:requesting-code-review` (or the repo's `/code-review`). Fix any Critical/Important in-branch; file House-rule-5 issues for anything out of scope. Re-run Step 1 after fixes.

- [ ] **Step 5: Final commit (if fixes) + open PR.**

```bash
git push -u origin feat/medication-attestation-responsibility
gh pr create --base main --title "feat: human-attested medication responsibility (slice 4, ADR-0049)" --body "<summary + design/plan links + test evidence>"
```

---

## Self-review notes (checked against the spec)

**Spec coverage:**
- §3 event type + payload + twin → Task 1 (builder) + Task 3 (floor/registry).
- §3.1 reviewed_commitment (set-commitment recipe, one SQL fn) → Task 3 `cairn_medication_thread_commitment`.
- §4 floor (registration, structural check, twin registry, offline-first) → Task 3.
- §5 projections (overlay, thread_head*, thread_attestation, group_attestation, no current-list widening) → Task 4. (*`medication_thread_head` was folded away: staleness uses the commitment directly, so a separate head view is unnecessary — noted as a simplification; the "last changed" legibility read can be added later if a consumer needs it.)
- §5.1 commitment-based staleness incl. the lower-HLC gap → Task 4 `lower_hlc_late_arrival_flips_stale_true`.
- §6.2 builders/orchestrators (build_attestation_body, attest_thread_in_tx, attest_medication_thread, author-time all verbs) → Task 5 + Task 6.
- §6.3 CLI (`medication-attest` + `--attest-as` every verb, shared resolver, cross-flag validation) → Task 7.
- §6.4 file split → Task 2.
- §7 TDD scenarios → Tasks 3–6 tests (floor, responsibility gate, post-hoc, staleness core + gap, group rollup, author-time all verbs + pair, supersede-not-retract, orphan) + Task 7 e2e smoke.
- §8 honest limits (superseded-not-retracted; conservative group rollup; reviewed_count advisory) → encoded in tests + ADR.
- §9 ADR-0049 + spec bump → Task 8.

**Deviation (recorded):** `medication_thread_head` (design §5.1/§5) is dropped — the commitment compare is the sole staleness authority and needs no separate head view; this removes a moving part rather than adding one. If a "last changed at" display is wanted later, it is an additive view.

**Placeholder scan:** the DB-gated test bodies in Tasks 3–6 describe the exact scenario + the concrete construction to use (inline signing/attestation mirroring `tests/attestation.rs`), naming the precise assertions; they are TDD specs to be filled with concrete code at write time, not shipped placeholders. Every SQL object and Rust signature is given in full.

**Type consistency:** `AttestParams` fields (`human_sk`, `human_kid`, `basis`, `note`) are consistent across Tasks 5–7; `attest_thread_in_tx(tx, params, patient, medication_id, hlc)` and the orchestrators' trailing `attest: Option<&AttestParams>` match; `reviewed_commitment` is hex (String) in the payload/build path and bytea in the table (decoded in the apply trigger), compared bytea-to-bytea in the view — consistent.
