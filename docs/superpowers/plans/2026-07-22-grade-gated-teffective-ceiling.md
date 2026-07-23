# Grade-gated `t_effective` ceiling + born clock-confidence grade — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the point-clock `t_effective ≤ hlc_wall` ceiling with a grade-gated ceiling — a born `clock_grade` decides how much rejecting power the check has — closing both the principle-4 violation (a slow/dead clock rejecting a truthful clinician) and a latent remote-door sync-wedge DoS.

**Architecture:** A mandatory `clock_grade` field is born on every signed `EventBody`. A pure DB helper `cairn_ceiling_classify(hlc_wall, grade, t_effective)` returns `ok | flag | reject`; at `self-asserted`/`unknown` (every node this slice) the upper bound is open, so it is **flag-never-reject**. The strict door (db/005) rejects only a `reject` verdict; the lenient door (db/020) never rejects on the ceiling and instead records an advisory `t_effective_ceiling_flag` row (a cross-type door-side write, not an ADR-0057 projection). A `cairn_clock_health()` read surfaces "clock provably behind its own HLC".

**Tech Stack:** Rust (`cairn-event`, `cairn-node`, `cairn-sync`), PostgreSQL 18 + `cairn_pgx`, PL/pgSQL floor, CBOR/COSE_Sign1 wire. Design doc: [`docs/superpowers/specs/2026-07-22-grade-gated-teffective-ceiling-design.md`](../specs/2026-07-22-grade-gated-teffective-ceiling-design.md). Outcome ADR: **ADR-0058** (refines ADR-0003/0027).

## Global Constraints

- **Licence:** AGPL-3.0; every dependency AGPL-compatible. No new deps expected.
- **TDD:** failing test first, then minimal code. No production code without a driving test.
- **Safety-critical surface** (§9.1): the doors + wire are Rust / in-DB, reviewer-legible.
- **No hard-coded crypto material in tests** (house rule 6): keys/seeds derived at runtime (`std::array::from_fn`), never literals — CodeQL `rust/hard-coded-cryptographic-value`.
- **Full-workspace tests, not per-crate:** the `EventBody` field change fans out to `cairn-sync`; run `cargo test --workspace`, never `-p cairn-node` alone (a per-crate run misses the arity break; and never `cargo test | tail` — it masks the exit code).
- **DB-gated tests need** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`); multi-node convergence additionally needs `CAIRN_TEST_PG2`/`PG3` on `cairn_test2`/`cairn_test3` (self-skip locally without them).
- **SQL mirror tests** run via `scripts/run-db-sql-tests.sh` (CI runs them since #251).
- **Wipe dev/PoC rigs** before/after: the mandatory born field makes pre-slice events read `unknown`; never sync pre-slice logs through.
- Keep files < 500 lines where feasible; `db/040` is new and self-contained.
- Branch `design/216-grade-gated-teffective-ceiling` already exists with the committed design doc. Work on it.

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `crates/cairn-event/src/lib.rs` | `ClockGrade` enum + `clock_grade` field on `EventBody` | Modify |
| `crates/cairn-event/src/schema_generation.rs` | `SCHEMA_GENERATION` constant | Modify (39→40) |
| `crates/cairn-event/tests/clock_grade.rs` | wire round-trip + additive-only verify | Create |
| `db/040_clock_confidence_grade.sql` | grade rank + ceiling helpers + flag table + `cairn_clock_health` + `event_log` ALTER | Create |
| `db/tests/040_clock_confidence_grade_test.sql` | SQL mirror: classify truth table, dedup, clock_health | Create |
| `db/005_submit.sql` | strict door: require+validate grade, classify, reject/flag, INSERT col | Modify |
| `db/020_apply_remote_event.sql` | lenient door: delete RAISE, classify, flag, never reject, INSERT col | Modify |
| `crates/cairn-sync/src/main.rs` | add `db/040` to `SCHEMA`; mint `clock_grade` in emit paths | Modify |
| `crates/cairn-node/**` + all test fixtures | add `clock_grade:` to every `EventBody {…}` literal (compiler-guided) | Modify |
| `crates/cairn-node/tests/teffective_ceiling.rs` | door behavior + the wedge regression | Create |
| `docs/spec/decisions/0058-*.md` + `docs/spec/data-model.md` | ADR + §3.6/§3.17 prose | Create/Modify |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | state update | Modify |

---

## Task 1: `clock_grade` born on `EventBody` (the wire field + the fan-out)

**Files:**
- Modify: `crates/cairn-event/src/lib.rs:227-259` (add enum near `Hlc`; add field to `EventBody`)
- Test: `crates/cairn-event/tests/clock_grade.rs` (create)
- Modify: every file with an `EventBody {…}` literal (compiler lists them)

**Interfaces:**
- Produces: `cairn_event::ClockGrade` (enum, `Default = Unknown`, serde kebab-case); `EventBody.clock_grade: ClockGrade`.

- [ ] **Step 1: Write the failing test** — `crates/cairn-event/tests/clock_grade.rs`:

```rust
//! Issue #216 — the born clock-confidence grade on the wire (ADR-0058).
use cairn_event::{canonical_cbor, generate_key, sign, verify_self_described, ClockGrade, EventBody, Hlc};

/// A minimal body helper — derives its own key material (house rule 6: no literals).
fn body(grade: ClockGrade) -> EventBody {
    EventBody {
        event_id: "018f00000000000000000000000000aa".into(),
        patient_id: "018f00000000000000000000000000bb".into(),
        event_type: "patient.created".into(),
        schema_version: "1".into(),
        hlc: Hlc { wall: 1_700_000_000_000, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: String::new(),
        contributors: serde_json::json!([{"actor_id":"k","role":"recorded"}]),
        payload: serde_json::json!({}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: grade,
    }
}

#[test]
fn clock_grade_round_trips_through_cbor() {
    let b = body(ClockGrade::SelfAsserted);
    let bytes = canonical_cbor(&b).unwrap();
    let back: EventBody = ciborium::from_reader(&bytes[..]).unwrap();
    assert_eq!(back.clock_grade, ClockGrade::SelfAsserted);
}

#[test]
fn clock_grade_serializes_as_kebab_string() {
    let v = serde_json::to_value(ClockGrade::MultiAnchorCorroborated).unwrap();
    assert_eq!(v, serde_json::json!("multi-anchor-corroborated"));
}

#[test]
fn absent_grade_deserializes_to_unknown() {
    // A legacy/foreign body encoded WITHOUT clock_grade must read as Unknown, and
    // must still verify (additive-only: existing signed bytes are never re-encoded).
    let (sk, _kid) = generate_key().unwrap();
    // Build a legacy body by round-tripping through a map that omits clock_grade.
    let b = body(ClockGrade::SelfAsserted);
    let mut map: serde_json::Value = serde_json::to_value(&b).unwrap();
    map.as_object_mut().unwrap().remove("clock_grade");
    let legacy: EventBody = serde_json::from_value(map).unwrap();
    assert_eq!(legacy.clock_grade, ClockGrade::Unknown, "absent → Unknown default");
    // And a real signed legacy blob still verifies after the field is added.
    let signed = sign(&b, &sk).unwrap();
    assert!(verify_self_described(&signed.signed_bytes).is_ok());
}
```

- [ ] **Step 2: Run it — expect a COMPILE failure** (`ClockGrade` unknown, field missing):

Run: `cargo test -p cairn-event --test clock_grade 2>&1 | head -20`
Expected: `error[E0432]: unresolved import ... ClockGrade` / `missing field clock_grade`.

- [ ] **Step 3: Add the enum + field** in `crates/cairn-event/src/lib.rs` immediately after the `Hlc` struct (line ~233):

```rust
/// The ADR-0027 clock-confidence ladder (issue #216, ADR-0058): how far this node's
/// wall clock can be trusted as wall-clock truth for `t_recorded`. Ordered,
/// best-corroboration-wins. `Unknown` is the honest read of an event that declares no
/// grade (foreign / pre-slice); `SelfAsserted` (RTC only) is the sole *minted* value
/// until a verified clock source lands (deferred). Serialized as its kebab-case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ClockGrade {
    #[default]
    Unknown,
    SelfAsserted,
    NetworkSynced,
    HardwareSourced,
    ExternallyAnchored,
    MultiAnchorCorroborated,
}
```

Then append to `EventBody` after `plaintext_twin` (line ~258). It is **mandatory** (no `skip_serializing_if`) but `#[serde(default)]` so legacy bodies read `Unknown`:

```rust
    /// ADR-0027 clock-confidence grade — born on every event (issue #216 / ADR-0058).
    /// Mandatory on new mints; `#[serde(default)]` reads a legacy/foreign body lacking it
    /// as `Unknown`. Appended (trailing) so existing signed bytes are never re-encoded and
    /// still verify (additive-only, principle 11 / ADR-0012).
    #[serde(default)]
    pub clock_grade: ClockGrade,
```

- [ ] **Step 4: Fix every `EventBody {…}` construction site** the compiler now rejects. Production/emit sites mint `SelfAsserted`; test fixtures use `SelfAsserted` unless the test simulates a foreign/legacy event (then `Unknown`). Iterate:

Run: `cargo build --workspace 2>&1 | grep -E "missing field .clock_grade|-->" | head -60`
For each `--> file:line`, add `clock_grade: cairn_event::ClockGrade::SelfAsserted,` (or `ClockGrade::SelfAsserted,` where imported) to the literal. Repeat until:

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: `Finished`.

- [ ] **Step 5: Run the tests — expect PASS:**

Run: `cargo test -p cairn-event --test clock_grade`
Expected: 3 passed.

- [ ] **Step 6: Full-workspace build+test green (the cross-crate gate):**

Run: `cargo test --workspace --no-run 2>&1 | tail -3`
Expected: `Finished` (all fixtures compile). Then `cargo test -p cairn-event` — Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-event crates/cairn-node crates/cairn-sync
git commit -m "feat(#216): born clock_grade on EventBody (ADR-0058 wire floor)"
```

---

## Task 2: `db/040` — grade rank + the pure ceiling classifier + SCHEMA_GENERATION bump

**Files:**
- Create: `db/040_clock_confidence_grade.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs:44` (39 → 40)
- Modify: `crates/cairn-sync/src/main.rs:40-115` (add `db/040` to `SCHEMA`)
- Test: `db/tests/040_clock_confidence_grade_test.sql` (create)

**Interfaces:**
- Produces (SQL): `cairn_clock_grade_is_ratified(text)→bool`, `cairn_clock_grade_rank(text)→int`, `cairn_ceiling_upper_ms(bigint,text)→bigint`, `cairn_ceiling_classify(bigint,text,timestamptz)→text` (`'ok'|'flag'|'reject'`).

- [ ] **Step 1: Write the failing SQL mirror test** — `db/tests/040_clock_confidence_grade_test.sql`:

```sql
-- Issue #216 / ADR-0058 — the grade-gated ceiling classifier truth table.
DO $$
DECLARE w bigint := 1_700_000_000_000;  -- an arbitrary hlc wall (ms)
BEGIN
    -- backdating is always ok, at any grade
    IF cairn_ceiling_classify(w, 'self-asserted', to_timestamp((w-60000)/1000.0)) <> 'ok'
        THEN RAISE EXCEPTION 'FAIL: backdate must be ok'; END IF;
    -- NULL t_effective is ok
    IF cairn_ceiling_classify(w, 'self-asserted', NULL) <> 'ok'
        THEN RAISE EXCEPTION 'FAIL: null t_eff must be ok'; END IF;
    -- self-asserted forward = FLAG, never reject (the principle-4 fix: even decades ahead)
    IF cairn_ceiling_classify(w, 'self-asserted', to_timestamp((w + 60000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: self-asserted forward must flag'; END IF;
    IF cairn_ceiling_classify(w, 'self-asserted', to_timestamp((w::numeric + 2e12)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: self-asserted far-forward must still flag, never reject'; END IF;
    -- unknown behaves like self-asserted (open above)
    IF cairn_ceiling_classify(w, 'unknown', to_timestamp((w + 60000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: unknown forward must flag'; END IF;
    -- an UNRECOGNIZED (future) grade is treated as rank 0 → open above → flag (never reject)
    IF cairn_ceiling_classify(w, 'quantum-entangled', to_timestamp((w + 60000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: unknown future grade must flag, not reject'; END IF;
    -- a high grade (DORMANT this slice) re-arms reject above its bound
    IF cairn_ceiling_classify(w, 'hardware-sourced', to_timestamp((w + 1000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: hardware within W must flag'; END IF;
    IF cairn_ceiling_classify(w, 'hardware-sourced', to_timestamp((w + 3_600_000)/1000.0)) <> 'reject'
        THEN RAISE EXCEPTION 'FAIL: hardware far past W must reject'; END IF;
    RAISE NOTICE 'PASS: cairn_ceiling_classify truth table';
END $$;
```

- [ ] **Step 2: Run it — expect FAIL** (function undefined):

Run: `psql "$CAIRN_TEST_PG" -v ON_ERROR_STOP=1 -f db/tests/040_clock_confidence_grade_test.sql`
Expected: `ERROR: function cairn_ceiling_classify(...) does not exist`.

- [ ] **Step 3: Create `db/040_clock_confidence_grade.sql`** (helpers half — the flag table + clock_health land in Task 3, same file):

```sql
-- db/040 — the ADR-0027 clock-confidence grade + the grade-gated t_effective ceiling
-- (issue #216, ADR-0058). Loaded by BOTH replayers (cairn-node full + cairn-sync subset):
-- db/020 references these helpers and the flag table via PL/pgSQL late-binding, so this
-- file MUST be in cairn-sync's SCHEMA list. Additive-only; adding this file bumps
-- SCHEMA_GENERATION to 40 (the #188 downgrade guard).

-- The ratified grade vocabulary (STRICT-door gate). The COLUMN is plain text, never a
-- closed CHECK domain: a FUTURE grade value from an upgraded peer must be ADMITTED
-- verbatim at the lenient door (additive-only, principle 11), while a node may not AUTHOR
-- one it does not know (strict-submit / lenient-apply, ADR-0051).
CREATE OR REPLACE FUNCTION cairn_clock_grade_is_ratified(g text)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT g IN ('unknown','self-asserted','network-synced',
                 'hardware-sourced','externally-anchored','multi-anchor-corroborated');
$$;

-- Ordered rank (0 = least trusted). An unrecognized/future value ranks 0 — the SAFE
-- direction: it can only WITHHOLD reject power, never grant it.
CREATE OR REPLACE FUNCTION cairn_clock_grade_rank(g text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE g
        WHEN 'self-asserted' THEN 1
        WHEN 'network-synced' THEN 2
        WHEN 'hardware-sourced' THEN 3
        WHEN 'externally-anchored' THEN 4
        WHEN 'multi-anchor-corroborated' THEN 5
        ELSE 0 END;  -- 'unknown' and every unrecognized value
$$;

-- The forward-slowness tolerance upper bound (ms). Ranks 0-1 (unknown / self-asserted)
-- return NULL = OPEN ABOVE: a node that cannot prove what time it is has no standing to
-- call a timestamp impossible (principle 4 applied recursively). The higher-grade widths
-- are DORMANT this slice — no node mints above self-asserted until a verified clock source
-- lands (ADR-0058 deferred) — and are placeholders to be replaced with real per-source
-- uncertainty then.
CREATE OR REPLACE FUNCTION cairn_ceiling_upper_ms(p_hlc_wall bigint, p_grade text)
RETURNS bigint LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN cairn_clock_grade_rank(p_grade) <= 1 THEN NULL
        ELSE p_hlc_wall + CASE p_grade
            WHEN 'network-synced'            THEN 3600000
            WHEN 'hardware-sourced'          THEN 5000
            WHEN 'externally-anchored'       THEN 2000
            WHEN 'multi-anchor-corroborated' THEN 1000
            ELSE 0 END
    END;
$$;

-- The single classification rule both doors call. 'ok' (admit clean) | 'flag' (admit +
-- advisory record) | 'reject' (strict door only; production-unreachable this slice).
CREATE OR REPLACE FUNCTION cairn_ceiling_classify(p_hlc_wall bigint, p_grade text, p_t_eff timestamptz)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN p_t_eff IS NULL THEN 'ok'
        WHEN p_t_eff <= to_timestamp(p_hlc_wall / 1000.0) THEN 'ok'
        WHEN cairn_ceiling_upper_ms(p_hlc_wall, p_grade) IS NULL THEN 'flag'
        WHEN p_t_eff <= to_timestamp(cairn_ceiling_upper_ms(p_hlc_wall, p_grade) / 1000.0) THEN 'flag'
        ELSE 'reject'
    END;
$$;
```

- [ ] **Step 4: Run the SQL test against a freshly-loaded DB — expect PASS.** (Load the schema so db/040 is present. Use the project's loader; if a manual load is needed: `psql "$CAIRN_TEST_PG" -f db/040_clock_confidence_grade.sql` after its deps, but the canonical path is a full reload via the node/sync loader.)

Run: `scripts/run-db-sql-tests.sh 040_clock_confidence_grade_test` (or the repo's SQL-test entrypoint)
Expected: `PASS: cairn_ceiling_classify truth table`.

- [ ] **Step 5: Bump `SCHEMA_GENERATION`** — `crates/cairn-event/src/schema_generation.rs:44`:

```rust
pub const SCHEMA_GENERATION: i32 = 40;
```

- [ ] **Step 6: Add `db/040` to cairn-sync's subset** — `crates/cairn-sync/src/main.rs`, append after the `037_born_sealed` tuple (~line 106):

```rust
    // db/040 (issue #216): the grade-gated ceiling helpers + t_effective_ceiling_flag +
    // cairn_clock_health. db/020 in this subset references cairn_ceiling_classify /
    // cairn_record_ceiling_flag via late-binding — omitting this file would fail the FIRST
    // apply of a forward-dated event on a fresh `cairn-sync init` DB (the #198 trap).
    (
        "040_clock_confidence_grade",
        include_str!("../../../db/040_clock_confidence_grade.sql"),
    ),
```

- [ ] **Step 7: Run the schema-generation guard — expect PASS:**

Run: `cargo test -p cairn-event --test schema_generation`
Expected: PASS (`SCHEMA_GENERATION` 40 == newest `db/` prefix 40).

- [ ] **Step 8: Commit**

```bash
git add db/040_clock_confidence_grade.sql db/tests/040_clock_confidence_grade_test.sql \
        crates/cairn-event/src/schema_generation.rs crates/cairn-sync/src/main.rs
git commit -m "feat(#216): db/040 grade-gated ceiling classifier + schema gen 40"
```

---

## Task 3: `db/040` — the `event_log` column, the flag table + recorder, and `cairn_clock_health`

**Files:**
- Modify: `db/040_clock_confidence_grade.sql` (append)
- Modify: `db/tests/040_clock_confidence_grade_test.sql` (append dedup + clock_health checks)

**Interfaces:**
- Produces (SQL): `event_log.clock_grade text NOT NULL DEFAULT 'unknown'`; table `t_effective_ceiling_flag`; `cairn_record_ceiling_flag(bytea,bigint,timestamptz,text,text)→void`; `cairn_clock_health()→TABLE(...)`.

- [ ] **Step 1: Append failing checks** to `db/tests/040_clock_confidence_grade_test.sql`:

```sql
DO $$
DECLARE ca bytea := '\x1220'::bytea || digest('ceiling-flag-test', 'sha256'); n int;
BEGIN
    PERFORM cairn_record_ceiling_flag(ca, 1_700_000_000_000, to_timestamp(1_700_000_060.0), 'self-asserted', 'flag');
    PERFORM cairn_record_ceiling_flag(ca, 1_700_000_000_000, to_timestamp(1_700_000_060.0), 'self-asserted', 'flag');
    SELECT count(*) INTO n FROM t_effective_ceiling_flag WHERE content_address = ca;
    IF n <> 1 THEN RAISE EXCEPTION 'FAIL: content_address dedup expected 1 row, got %', n; END IF;
    -- clock_health returns exactly one row with the expected column set
    PERFORM 1 FROM cairn_clock_health();
    IF NOT FOUND THEN RAISE EXCEPTION 'FAIL: cairn_clock_health returned no row'; END IF;
    RAISE NOTICE 'PASS: flag dedup + clock_health';
END $$;
```

- [ ] **Step 2: Run — expect FAIL** (`t_effective_ceiling_flag` / functions undefined).

Run: `scripts/run-db-sql-tests.sh 040_clock_confidence_grade_test`
Expected: `ERROR: ... does not exist`.

- [ ] **Step 3: Append to `db/040_clock_confidence_grade.sql`:**

```sql
-- The born grade column on the append-only log. Plain text (admits future grades), NOT
-- NULL DEFAULT 'unknown' so existing rows + any omitting writer read honestly as unknown.
-- Added by ALTER for existing DBs (#207 paired-ALTER discipline); event_log's canonical
-- CREATE lives in db/001.
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS clock_grade text NOT NULL DEFAULT 'unknown';

-- The advisory clash record — a cross-type DOOR-side write, NOT an ADR-0057 projection
-- (cairn_projection_dispatch keys on event_type; the ceiling is type-independent). Both
-- doors call cairn_record_ceiling_flag on a flag/reject verdict. Append-only; keyed by the
-- flagged event's content_address so set-union re-delivery is idempotent. Survives a
-- cairn_reproject rebuild untouched (rebuild replays through the dispatch, never the doors,
-- and the inputs are immutable). A future grade we do not recognize is still recorded.
CREATE TABLE IF NOT EXISTS t_effective_ceiling_flag (
    flag_id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    content_address BYTEA,                          -- flagged event identity (dedup key)
    hlc_wall        BIGINT      NOT NULL,
    t_effective     TIMESTAMPTZ NOT NULL,
    clock_grade     TEXT        NOT NULL,
    verdict         TEXT        NOT NULL,           -- 'flag' | 'reject'
    flagged_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- NULLS DISTINCT (default): a pre-column NULL never collides; a real address is unique.
CREATE UNIQUE INDEX IF NOT EXISTS t_effective_ceiling_flag_ca_idx
    ON t_effective_ceiling_flag (content_address);
GRANT SELECT ON t_effective_ceiling_flag TO cairn_agent;

CREATE OR REPLACE FUNCTION cairn_record_ceiling_flag(
    p_ca bytea, p_hlc_wall bigint, p_t_eff timestamptz, p_grade text, p_verdict text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO t_effective_ceiling_flag (content_address, hlc_wall, t_effective, clock_grade, verdict)
    VALUES (p_ca, p_hlc_wall, p_t_eff, p_grade, p_verdict)
    ON CONFLICT (content_address) DO NOTHING;
$$;

-- The honest-assembly clock-health read (ADR-0027 §7): compares the RTC to hlc_state.wall,
-- which the HLC A3 merge keeps >= every accepted event. A live derived read — never stored,
-- never an event, never synced. The CLI `status` renders it; any client reads the row.
CREATE OR REPLACE FUNCTION cairn_clock_health()
RETURNS TABLE(rtc_now timestamptz, hlc_floor timestamptz, behind_by_ms bigint,
              is_behind boolean, effective_lower_bound timestamptz, default_grade text)
LANGUAGE sql STABLE AS $$
    SELECT
        clock_timestamp(),
        to_timestamp(s.hlc_wall / 1000.0),
        GREATEST(0, s.hlc_wall - (extract(epoch FROM clock_timestamp()) * 1000)::bigint),
        (s.hlc_wall - (extract(epoch FROM clock_timestamp()) * 1000)::bigint) > 1000,
        GREATEST(clock_timestamp(), to_timestamp(s.hlc_wall / 1000.0)),
        'self-asserted'::text
    FROM hlc_state s WHERE s.id;
$$;
GRANT EXECUTE ON FUNCTION cairn_clock_health() TO cairn_agent;
```

- [ ] **Step 4: Run the SQL test — expect PASS:**

Run: `scripts/run-db-sql-tests.sh 040_clock_confidence_grade_test`
Expected: `PASS: cairn_ceiling_classify truth table` and `PASS: flag dedup + clock_health`.

- [ ] **Step 5: Add the #207 paired-ALTER guard test** — append to the same SQL test file: assert the column exists after replay:

```sql
DO $$
DECLARE t text;
BEGIN
    SELECT data_type INTO t FROM information_schema.columns
      WHERE table_name='event_log' AND column_name='clock_grade';
    IF t IS NULL THEN RAISE EXCEPTION 'FAIL: event_log.clock_grade missing after replay (#207)'; END IF;
    RAISE NOTICE 'PASS: event_log.clock_grade present';
END $$;
```

Run: `scripts/run-db-sql-tests.sh 040_clock_confidence_grade_test` — Expected: all `PASS`.

- [ ] **Step 6: Commit**

```bash
git add db/040_clock_confidence_grade.sql db/tests/040_clock_confidence_grade_test.sql
git commit -m "feat(#216): event_log.clock_grade column, ceiling flag table, clock_health"
```

---

## Task 4: strict door (db/005) — grade-gated ceiling, classify + reject/flag

> **Refinement (ratified mid-build):** the strict door does NOT RAISE on an absent/unratified
> `clock_grade`. `clock_grade` is a mandatory Rust `EventBody` field, so a conforming client can never
> omit it (compile-time born-grade guarantee); a raw/hostile submitter omitting it is handled safely by
> defaulting to `unknown` (rank 0 → open-above → never rejects). The door **gates effect, not presence**
> (ADR-0056). This also removes the need for a raw-CBOR signing seam in the safety-critical crate.

**Files:**
- Modify: `db/005_submit.sql` (the tier-1 ceiling block — **locate by content**: the `IF v_t_eff IS NOT NULL AND v_t_eff > to_timestamp(...hlc...wall...)` block that `RAISE`s "prima-facie forward-dating"; line numbers may have drifted) + the DECLARE block + the `INSERT INTO event_log` column list (ends `... actor_id, sealed)`)
- Test: `crates/cairn-node/tests/teffective_ceiling.rs` (create; strict-door cases)

**Interfaces:**
- Consumes: `cairn_ceiling_classify`, `cairn_record_ceiling_flag` (Task 2/3). (`cairn_clock_grade_is_ratified` is intentionally NOT consumed here — the born-grade invariant is type-enforced; the predicate stays as a public tooling helper.)

- [ ] **Step 1: Write the failing Rust db-gated test** — `crates/cairn-node/tests/teffective_ceiling.rs` (strict-door half; use the crate's existing submit test helpers — mirror `crates/cairn-node/tests/hlc_drift.rs` for wiring, deriving keys at runtime per house rule 6):

```rust
//! Issue #216 / ADR-0058 — the grade-gated ceiling at the STRICT door (db/005).
// (Wiring: reuse the submit helper pattern from tests/hlc_drift.rs — a fresh keypair via
// generate_key(), a signed EventBody, submit_event over $CAIRN_TEST_PG. Full helper omitted
// here for brevity; copy the harness from hlc_drift.rs verbatim.)

#[test]
fn self_asserted_forward_t_effective_is_admitted_and_flagged() {
    // RED today: db/005 RAISEs on t_effective > hlc_wall. After the fix it must ADMIT and
    // write a t_effective_ceiling_flag row (the honest slow-clock clinician). Principle-4.
    let (client, event_id, ca) = submit_forward_effective(ClockGrade::SelfAsserted, 3600); // hlc_wall + 1h
    assert_event_present(&client, event_id);
    let flags: i64 = client.query_one(
        "SELECT count(*) FROM t_effective_ceiling_flag WHERE content_address = $1", &[&ca]
    ).unwrap().get(0);
    assert_eq!(flags, 1, "self-asserted forward must be admitted + flagged, never rejected");
}

#[test]
fn clean_backdate_writes_no_flag() {
    // t_effective before hlc_wall (normal backdating) → 'ok', no flag row.
    let (client, _event_id, ca) = submit_forward_effective(ClockGrade::SelfAsserted, -3600); // hlc_wall - 1h
    let flags: i64 = client.query_one(
        "SELECT count(*) FROM t_effective_ceiling_flag WHERE content_address = $1", &[&ca]
    ).unwrap().get(0);
    assert_eq!(flags, 0, "a clean backdate must not be flagged");
}

#[test]
fn high_grade_far_forward_is_rejected_at_the_strict_door() {
    // The dormant reject arm, exercised by SYNTHESIS: a hardware-sourced clock genuinely knows
    // the time, so a t_effective far past its tight bound (W=5s) IS prima-facie forward-dating.
    // (No emit path mints hardware-sourced this slice; the door still classifies whatever grade
    // the signed body carries.)
    let err = try_submit_forward_effective(ClockGrade::HardwareSourced, 3600).unwrap_err(); // +1h ≫ 5s
    assert!(err.to_string().contains("grade-gated"), "got: {err}");
}
```

(Helper contract for this file: `submit_forward_effective(grade, offset_secs) -> (Client, event_id, content_address)` signs an `EventBody` whose `t_effective = hlc_wall + offset_secs` with the given `clock_grade`, submits it through `submit_event`, and returns the row identity; `try_submit_forward_effective(...)` is the same but returns the `Result` so a reject can be asserted. Build the signed body + `submit_event` call by mirroring `crates/cairn-node/tests/hlc_drift.rs` verbatim; derive keys at runtime via `generate_key()` (house rule 6); connect via `db::connect_and_load_schema(&cs)`.)

- [ ] **Step 2: Run — expect FAIL** (the first test fails because db/005 currently rejects):

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test teffective_ceiling`
Expected: FAIL — submit RAISEs "prima-facie forward-dating".

- [ ] **Step 3: Edit `db/005_submit.sql`.** In the DECLARE block add `v_grade text; v_verdict text;`. Replace the existing tier-1 ceiling block (the `IF v_t_eff IS NOT NULL AND v_t_eff > to_timestamp((b -> 'hlc' ->> 'wall')::bigint / 1000.0) THEN RAISE EXCEPTION '...prima-facie forward-dating...'` block) with:

```sql
    -- 1b'. Grade-gated bitemporal ceiling (ADR-0058 refines ADR-0003 §3.6). The born clock_grade
    --      (a mandatory EventBody field — compile-time guaranteed for conforming clients; an absent
    --      or unrecognized value reads as the safe 'unknown', rank 0) gates the ceiling's rejecting
    --      power: at unknown/self-asserted the upper bound is OPEN, so a forward t_effective is
    --      FLAGGED, never rejected (principle 4 — a slow/dead clock must not force fabrication). The
    --      'reject' arm fires only for a credible high grade — production-unreachable this slice (no
    --      node mints above self-asserted; exercised by synthesis in the tests). The door gates
    --      EFFECT not PRESENCE (ADR-0056): a missing grade is admitted as 'unknown', never refused.
    v_grade := COALESCE(b ->> 'clock_grade', 'unknown');
    v_verdict := cairn_ceiling_classify((b -> 'hlc' ->> 'wall')::bigint, v_grade, v_t_eff);
    IF v_verdict = 'reject' THEN
        RAISE EXCEPTION 'submit_event: t_effective (%) exceeds the ceiling for a "%" clock (ADR-0058 grade-gated)',
            b ->> 't_effective', v_grade;
    ELSIF v_verdict = 'flag' THEN
        PERFORM cairn_record_ceiling_flag(v_ca, (b -> 'hlc' ->> 'wall')::bigint, v_t_eff, v_grade, 'flag');
    END IF;
```

Then add `clock_grade` to the `INSERT INTO event_log` — append it to the column list (after `sealed`) and append `v_grade` to the matching VALUES tuple:

```sql
         signer_key_id, plaintext_twin, attachments, attestation, attester_key, actor_id, sealed, clock_grade)
```
…and the matching `, v_grade` in the VALUES tuple (same ordinal position).

- [ ] **Step 4: Run the strict-door tests — expect PASS:**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test teffective_ceiling`
Expected: all three strict-door tests PASS (forward→flag, backdate→no-flag, high-grade→reject).

- [ ] **Step 5: Regression — existing submit tests still green:**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test admission --test seal_submit`
Expected: PASS (grade now minted `self-asserted` by the fixtures from Task 1).

- [ ] **Step 6: Commit**

```bash
git add db/005_submit.sql crates/cairn-node/tests/teffective_ceiling.rs
git commit -m "feat(#216): strict door requires+validates grade, grade-gated ceiling"
```

---

## Task 5: lenient door (db/020) — delete the RAISE, flag, never reject (the DoS fix)

**Files:**
- Modify: `db/020_apply_remote_event.sql:121-129` (the ceiling block) + DECLARE + INSERT col (`:292-295`)
- Test: `crates/cairn-node/tests/teffective_ceiling.rs` (append remote-door + wedge cases) + `crates/cairn-sync/tests/clinical_pull.rs` (the do_pull wedge regression)

**Interfaces:**
- Consumes: `cairn_ceiling_classify`, `cairn_record_ceiling_flag`.

- [ ] **Step 1: Write the failing remote-door + wedge tests.** Append to `teffective_ceiling.rs`:

```rust
#[test]
fn remote_forward_t_effective_is_admitted_and_flagged_never_rejected() {
    // A SIGNED foreign event with t_effective > hlc_wall must APPLY (no RAISE) and flag.
    let (client, event_id, ca) = apply_remote_forward_effective(ClockGrade::Unknown);
    assert_event_present(&client, event_id);
    let flags: i64 = client.query_one(
        "SELECT count(*) FROM t_effective_ceiling_flag WHERE content_address = $1", &[&ca]
    ).unwrap().get(0);
    assert_eq!(flags, 1, "remote forward must admit + flag");
}
```

And the headline regression in `crates/cairn-sync/tests/clinical_pull.rs` (reuse its two-node harness; mirror `pull_integrity_err`/`do_pull` usage there):

```rust
#[test]
fn forward_dated_event_does_not_wedge_the_pull() {
    // Issue #216 F1/F2: a verifiable event with t_effective > hlc_wall previously froze the
    // seq cursor (frozen=true). It must now apply and the cursor advance — no freeze, no DoS.
    let (mut c, addr, peer) = two_node_rig_with_forward_dated_event();
    let m = do_pull(&mut c, &addr, peer, false, None).unwrap();
    assert_eq!(m["watermark_frozen"], false, "a forward-dated event must not wedge the pull");
    assert!(m["applied_new"].as_i64().unwrap() >= 1, "the event applied");
}
```

- [ ] **Step 2: Run — expect FAIL** (remote apply RAISEs; the pull freezes):

Run: `CAIRN_TEST_PG=… CAIRN_TEST_PG2=… cargo test -p cairn-node --test teffective_ceiling remote; cargo test -p cairn-sync --test clinical_pull forward_dated`
Expected: FAIL (RAISE / `watermark_frozen == true`).

- [ ] **Step 3: Edit `db/020_apply_remote_event.sql`.** Add `v_grade text; v_verdict text;` to DECLARE. Replace the ceiling block at lines 121-129 with:

```sql
    -- 1b'. Grade-gated bitemporal ceiling (ADR-0058 refines ADR-0003 §3.6). LENIENT door:
    --      NEVER reject on the ceiling. A refusal of a verifiable event freezes the puller's
    --      seq watermark and WEDGES clinical sync (issue #216 F1/F2) — exactly the rule the
    --      HLC-drift clamp below already honors (availability over consistency). Admit the
    --      event UNCHANGED; a flag/reject verdict is recorded as an advisory clash row. An
    --      absent grade (foreign / pre-slice) reads as 'unknown'; a future grade is admitted
    --      verbatim (additive-only) and ranks 0 in the classifier (safe).
    v_grade := COALESCE(b ->> 'clock_grade', 'unknown');
    v_verdict := cairn_ceiling_classify((b -> 'hlc' ->> 'wall')::bigint, v_grade, v_t_eff);
    IF v_verdict IN ('flag', 'reject') THEN
        PERFORM cairn_record_ceiling_flag(v_ca, (b -> 'hlc' ->> 'wall')::bigint, v_t_eff, v_grade, v_verdict);
    END IF;
```

Add `clock_grade` to the INSERT column list (after `sealed`, line 295) and a normalized value in VALUES — store verbatim (plain-text column admits it):

```sql
         signer_key_id, plaintext_twin, attachments, attestation, attester_key, actor_id, sealed, clock_grade)
```
…and `, v_grade` appended to the VALUES tuple (matching position).

- [ ] **Step 4: Run — expect PASS** (both doors + the wedge regression):

Run: `CAIRN_TEST_PG=… CAIRN_TEST_PG2=… cargo test -p cairn-node --test teffective_ceiling; cargo test -p cairn-sync --test clinical_pull`
Expected: all PASS — `watermark_frozen == false`.

- [ ] **Step 5: Full-workspace regression:**

Run: `CAIRN_TEST_PG=… CAIRN_TEST_PG2=… CAIRN_TEST_PG3=… cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add db/020_apply_remote_event.sql crates/cairn-node/tests/teffective_ceiling.rs crates/cairn-sync/tests/clinical_pull.rs
git commit -m "fix(#216): lenient door admits+flags, never rejects — closes the sync-wedge DoS"
```

---

## Task 6: `emit_event` stores `clock_grade` on the author's own row

> **Scope refinement:** Task 1 already mints `clock_grade: SelfAsserted` in every `EventBody` literal
> (incl. `emit_event`'s body at `main.rs:843`), so the *signed body* and every submit-door path
> (db/005, Task 4) are already correct. The one remaining gap: `emit_event` (`main.rs:794`) does a
> **direct** `INSERT INTO event_log` (line ~855) whose column list omits `clock_grade`, so the
> author's OWN row stores the default `'unknown'` while peers (via db/020) store the signed
> `'self-asserted'` — a cross-node metadata inconsistency. Fix that INSERT. **The twin grade-line is
> DEFERRED** (a follow-on issue — see the design doc §9.6): it needs the Rust `plaintext_twin` and the
> SQL `cairn_twin_skeleton`/`cairn_event_twin` floor changed in lockstep or the demographic twin-match
> floor refuses, for a cosmetic gain (the grade is already legible via the column + `cairn_clock_health`).

**Files:**
- Modify: `crates/cairn-sync/src/main.rs` (`emit_event`'s direct `event_log` INSERT, ~line 855)
- Test: extend `crates/cairn-sync/tests/clinical_pull.rs` (or the nearest emit-path test) — author via `emit_event`/`write` and assert the stored `clock_grade`

- [ ] **Step 1: Failing test.** Add a focused test that authors an event through the same path `emit_event` uses (mirror an existing `emit_event`/`cmd_write` test in the crate) and asserts the stored column:

```rust
// After authoring one event via emit_event on node A:
let grade: String = client
    .query_one("SELECT clock_grade FROM event_log WHERE event_id = $1::text::uuid", &[&event_id])
    .unwrap().get(0);
assert_eq!(grade, "self-asserted", "the author's own row must store the minted grade, not the 'unknown' default");
```

- [ ] **Step 2: Run — expect FAIL** (the direct INSERT omits `clock_grade`, so the column defaults to `'unknown'`).

- [ ] **Step 3:** In `emit_event`'s `INSERT INTO event_log (...)` (`main.rs:855`), add `clock_grade` to the column list (after `attachments` or anywhere — column order in an explicit-column INSERT is free) and bind a value. Since the body carries the grade, serialize it: add `clock_grade` to the columns and pass the grade string as a new bind parameter, e.g. `let grade = serde_json::to_value(&body.clock_grade)?.as_str().unwrap().to_string();` then bind `&grade` at the new `$15` (renumber the `'[]'::jsonb` attachments literal is unaffected — it's a literal, not a bind). Simplest: append `,clock_grade` to the column list and `,$15` to VALUES with `&grade` appended to the params array.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-sync/src/main.rs crates/cairn-sync/tests/clinical_pull.rs
git commit -m "feat(#216): emit_event stores clock_grade on the author's own row"
```

---

## Task 7: `status` renders `cairn_clock_health`

**Files:**
- Modify: `crates/cairn-node/src/` status command path (find the `status` handler; it prints honest node state)
- Test: `crates/cairn-node/tests/teffective_ceiling.rs` (append the behind-detection case)

- [ ] **Step 1: Failing test** — a DB whose `hlc_state.wall` is forced ahead of the RTC reports `is_behind`:

```rust
#[test]
fn clock_health_flags_a_clock_behind_its_own_hlc() {
    let client = fresh_node_db();
    // Force hlc_state.wall far ahead of the real clock (simulate a dead-RTC node that has
    // synced a later event). hlc_state is a single-row table (db/001, WHERE id).
    client.execute("UPDATE hlc_state SET hlc_wall = (extract(epoch FROM clock_timestamp())*1000)::bigint + 3_600_000 WHERE id", &[]).unwrap();
    let row = client.query_one("SELECT is_behind, behind_by_ms FROM cairn_clock_health()", &[]).unwrap();
    let is_behind: bool = row.get(0);
    assert!(is_behind, "a clock an hour behind its own HLC must report is_behind");
}
```

- [ ] **Step 2: Run — expect FAIL** if `cairn_clock_health` unreachable from the test wiring; otherwise this validates Task 3. Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test teffective_ceiling clock_health`. Expected: PASS once db/040 is loaded (this mainly asserts the read; the `status` wiring is Step 3).

- [ ] **Step 3:** In the `status` command handler, `SELECT * FROM cairn_clock_health()` and print one line, e.g. `clock: self-asserted · behind_by 3600000ms (BEHIND its own HLC — check the node clock)` when `is_behind`, else `clock: self-asserted · ok`.

- [ ] **Step 4: Run — expect PASS.** Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test teffective_ceiling`. Expected: PASS. Optionally eyeball `cargo run -p cairn-node -- status`.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src crates/cairn-node/tests/teffective_ceiling.rs
git commit -m "feat(#216): status surfaces cairn_clock_health (clock-behind honesty)"
```

---

## Task 8: ADR-0058 + spec prose + HANDOVER/ROADMAP + follow-on issues

**Files:**
- Create: `docs/spec/decisions/0058-grade-gated-teffective-ceiling.md`
- Modify: `docs/spec/data-model.md` (§3.6 + §3.17), `docs/spec/index.md` (spec version + ADR count if listed)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0058** — status Accepted, dated 2026-07-22, Refines ADR-0003/0027. Content: the grade gates the ceiling's rejecting power; `self-asserted`/`unknown` → flag-never-reject (the three §3 failure modes); the remote door admits-and-flags (F1/F2 DoS); grade-only representation (interval derived); mint constrained to `self-asserted`; **the ADR-0027 §6 refinement** (`upper = RTC → RTC + W`, open above at low grades); `cairn_clock_health` as the honest-assembly read; deferred anchor planes. Lift prose from the design doc §4/§5/§9.

- [ ] **Step 2: Update `data-model.md`** §3.6 (ceiling is grade-gated; reject only when grade credible and above `hlc_wall + W`) and §3.17 (the grade gates rejecting power; interval derived from grade this slice; the §6 refinement; `cairn_clock_health`). Bump the spec version in `index.md` (v0.59 → v0.60) if that is where it lives.

- [ ] **Step 3: Rebuild the docs site to verify it compiles** (never commit `site/`):

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: build succeeds, no broken cross-refs.

- [ ] **Step 4: File the deferred follow-on issues** (design §9): anchor/notary planes + overlay grade-upgrade tokens; causal lower-bound tightening; UI clock-sanity alert; auto-downgrade grade on detected clock failure; exercise the numeric `W(grade)` when sources land. Use `gh issue create`; capture the numbers.

- [ ] **Step 5: Update HANDOVER + ROADMAP** — record the slice as done, the new ADR-0058, spec version, the filed follow-ons, and the rig-wipe caveat. Prune to keep both concise.

- [ ] **Step 6: Commit**

```bash
git add docs/
git commit -m "docs(#216): ADR-0058 + spec §3.6/§3.17 + HANDOVER/ROADMAP + follow-ons"
```

---

## Task 9: full verification, fmt/clippy/deny, SQL mirrors

- [ ] **Step 1: Formatting + lints:**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 2: cargo-deny (licence/advisory gate):**

Run: `cargo deny check`
Expected: no new violations (no new deps added).

- [ ] **Step 3: Full workspace + SQL mirror suites:**

Run: `CAIRN_TEST_PG=… CAIRN_TEST_PG2=… CAIRN_TEST_PG3=… cargo test --workspace`
Run: `scripts/run-db-sql-tests.sh`
Expected: all PASS.

- [ ] **Step 4:** Push the branch and open the PR to `main`, linking #216, summarizing the grade-gated ceiling + the closed DoS, and listing the filed follow-ons and the rig-wipe caveat.

```bash
git push -u origin design/216-grade-gated-teffective-ceiling
gh pr create --base main --title "#216/ADR-0058: grade-gated t_effective ceiling + born clock-confidence grade" --body "…"
```

---

## Self-Review

**Spec coverage** (design §5 items → tasks): (a) born grade field → T1; (b) grade-gated classify → T2/T4/T5; (c) strict/lenient door split → T4/T5; (d) no absolute cap → T2 truth-table (far-forward flags); (e) `cairn_clock_health` → T3/T7; (f) door-side flag → T3/T4/T5. Wire additive-only → T1 legacy-verify test. SCHEMA_GENERATION bump → T2. cairn-sync subset → T2. Twin → T6. ADR/spec/follow-ons/rig-wipe → T8. Verification/mirrors → T9. **No gaps.**

**Placeholder scan:** the Rust door-test harness in T4/T5/T7 says "copy the harness from hlc_drift.rs / clinical_pull.rs" rather than inlining ~80 lines of two-node rig boilerplate — this is a deliberate pointer to an exact existing pattern in-repo, not a TODO; the assertions and the DB effects are fully specified. All SQL and door edits are complete and exact.

**Type consistency:** `cairn_ceiling_classify(bigint,text,timestamptz)→text`, `cairn_record_ceiling_flag(bytea,bigint,timestamptz,text,text)`, `cairn_clock_health()` columns, and `ClockGrade` variants/serde names are used identically across T2–T7. `v_grade`/`v_verdict` declared in both door DECLAREs. `clock_grade` added to both INSERTs. Consistent.
