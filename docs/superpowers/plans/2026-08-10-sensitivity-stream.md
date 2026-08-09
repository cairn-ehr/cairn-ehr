# Sensitivity Stream Implementation Plan (§5.9 slice A, #232 part A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the graded, append-only sensitivity assertion stream and its effective-grade projection — ADR-0006 decision 3 — so a later slice can coarsen a safety projection (B) and narrow custody (C) against a grade that already exists.

**Architecture:** Two new plaintext clinical event types (`sensitivity.grade.asserted`, `sensitivity.grade-withdrawal.asserted`) land through the existing validated doors into two retained-set projection tables. The effective grade of an event is the **max by rank** over standing assertions on {the event, its thread, its patient}; standing = asserted minus withdrawn, evaluated at read so a withdrawal may arrive first. This slice **enforces nothing** — it computes and reports.

**Tech Stack:** PostgreSQL 18 + PL/pgSQL (safety-critical, §9) · Rust 1.96.0 (`cairn-event` pure wire types, `cairn-node` orchestration/CLI) · `cairn_pgx` ≥ 0.3.0.

**Design doc:** [`docs/superpowers/specs/2026-08-09-sensitivity-stream-design.md`](../specs/2026-08-09-sensitivity-stream-design.md) — read it first; every "why" below is short because the design carries it.

## Global Constraints

- **AGPL-3.0.** No new dependencies in this slice. If one becomes necessary, its licence is checked *before* it is added.
- **TDD, always.** Failing test first, watch it fail, then the minimal code. No production code without a test that drove it.
- **`SCHEMA_GENERATION` 47 → 48.** One bump for this slice, in `crates/cairn-event/src/schema_generation.rs`.
- **`cargo test --workspace`, never `-p cairn-node` alone.** This slice edits `cairn-sync`; per-crate runs miss exactly that.
- **DB-gated tests need** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`; the convergence test additionally needs `CAIRN_TEST_PG2`. Without them suites **self-skip and cargo counts them as passed** — a green count is not proof they ran.
- **Guard before connect.** `db::test_serial_guard(&base)` *before* `connect_and_load_schema`, in execution order.
- **UUIDs bind as text.** `cairn-node` does not enable tokio-postgres `with-uuid-1`: bind `&uuid.to_string()`, cast `$1::text::uuid`.
- **Registry rows use `ON CONFLICT … DO UPDATE` with an `IS DISTINCT FROM` guard** (#214), never `DO NOTHING` (#254).
- **Every `db/tests/*.sql` mirror opens with the scratch-database guard** (#169) and runs only via `scripts/run-db-sql-tests.sh`.
- **Inline documentation for a junior developer** — *why* and *how it fits*, not *what the next line does*. Files under 500 lines.
- **No hard-coded cryptographic material in tests** — derive keys at runtime (`std::array::from_fn`), never byte literals (#146).

---

## File Structure

| File | Responsibility |
|---|---|
| `db/048_sensitivity_stream.sql` | **new** — the whole in-DB surface: ladder, tables, floor checks, apply fns, read model, registry rows |
| `db/tests/048_sensitivity_stream_test.sql` | **new** — SQL mirror of the DB-gated Rust assertions |
| `crates/cairn-event/src/sensitivity.rs` | **new** — pure wire types, body builders, twin renderers. No I/O, no policy |
| `crates/cairn-event/src/lib.rs` | expose `pub mod sensitivity;` |
| `crates/cairn-event/src/schema_generation.rs` | 47 → 48 |
| `crates/cairn-node/src/sensitivity.rs` | **new** — orchestration: author an assertion / withdrawal, read the report |
| `crates/cairn-node/src/lib.rs` | expose `pub mod sensitivity;` |
| `crates/cairn-node/src/db.rs` | SCHEMA list + `db/048` |
| `crates/cairn-sync/src/main.rs` | SCHEMA **subset** + `db/048` — §10a, a disclosure if omitted |
| `crates/cairn-node/src/main.rs` | three CLI verbs |
| `crates/cairn-node/tests/sensitivity_ladder.rs` | the rank fn + effective grade |
| `crates/cairn-node/tests/sensitivity_floor.rs` | structural floor, both doors, hex legibility |
| `crates/cairn-node/tests/sensitivity_ceremony.rs` | local-door ceremony + remote leniency asymmetry |
| `crates/cairn-node/tests/sensitivity_convergence.rs` | two-node convergence *given equal custody* |
| `crates/cairn-node/tests/twin_registry.rs` | count **+2** |
| `db/tests/034_twin_registry_test.sql` | count **+2** — the count lives in BOTH places |
| `docs/spec/decisions/0062-*.md`, `docs/spec/identity.md`, `docs/spec/index.md` | ADR-0062, §5.9 prose, v0.64 |

---

## Task 1: The migration skeleton, the ladder, and both loaders

**Files:**
- Create: `db/048_sensitivity_stream.sql`
- Create: `db/tests/048_sensitivity_stream_test.sql`
- Create: `crates/cairn-node/tests/sensitivity_ladder.rs`
- Modify: `crates/cairn-event/src/schema_generation.rs` (47 → 48)
- Modify: `crates/cairn-node/src/db.rs` (SCHEMA list)
- Modify: `crates/cairn-sync/src/main.rs` (SCHEMA subset)

**Interfaces:**
- Produces: `cairn_sensitivity_rank(text) -> int` (IMMUTABLE); `db/048` loaded by **both** loaders.

- [ ] **Step 1: Write the failing test**

`crates/cairn-node/tests/sensitivity_ladder.rs`:

```rust
//! The §5.9 sensitivity ladder (ADR-0062).
//!
//! The one thing to understand before editing: an UNRECOGNISED grade ranks MAX here,
//! which is the exact opposite of `cairn_clock_grade_rank`'s `ELSE 0`. See the comment
//! on `cairn_sensitivity_rank` in db/048 — a "fix" that aligns the two is a leak.
mod common;
use common::{cs, db_msg};

#[tokio::test]
async fn the_ladder_orders_the_named_grades_and_ranks_the_unknown_maximum() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();

    let rank = |g: &'static str| {
        let c = &c;
        async move {
            c.query_one("SELECT cairn_sensitivity_rank($1)", &[&g])
                .await
                .map(|r| r.get::<_, i32>(0))
                .map_err(|e| db_msg(&e))
                .unwrap()
        }
    };

    assert_eq!(rank("routine").await, 0, "no protection asserted");
    assert!(rank("sensitive").await < rank("restricted").await);
    assert!(rank("restricted").await < rank("sequestered").await);

    // The inverted unknown. A future peer's grade must coarsen, never expose.
    assert_eq!(
        rank("grade:protected-witness").await,
        i32::MAX,
        "an unrecognised grade must rank MAX: ranking it 0 would let an older node read a \
         peer's newer grade as 'not sensitive' and render the body in the clear"
    );

    // NULL lands on the same safe side (a NOT NULL column makes this unreachable, but the
    // function is public API and must not have an unsafe corner).
    let null_rank: i32 = c
        .query_one("SELECT cairn_sensitivity_rank(NULL)", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(null_rank, i32::MAX);
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_ladder -- --nocapture
```

Expected: FAIL — `function cairn_sensitivity_rank(unknown) does not exist`.

- [ ] **Step 3: Create the migration with the ladder**

`db/048_sensitivity_stream.sql`:

```sql
-- Cairn — the §5.9 sensitivity stream (ADR-0006 decision 3, ADR-0062; issue #232 part A).
--
-- Sensitivity is not a boolean on a body: it is an append-only stream of graded assertions
-- whose EFFECTIVE value is a projection (never merge, always overlay). This file ships the
-- stream, the projection, and nothing else. It ENFORCES NOTHING — a grade computed here
-- withholds no content. Sequester (custody narrowing) is #232 part C and is blocked on #231.
--
-- # Why these bodies are plaintext
--
-- ADR-0052 §2 lists what stays unsealed because the machinery binds on it. Sensitivity
-- assertions join that list: a node must READ the grade in order to coarsen, and coarsening
-- is exactly what a node holding no custody of the graded body must still do. Sealing the
-- grade under the key it governs is circular.
--
-- # What must never appear in these bodies
--
-- The matched blacklist CATEGORY. A plaintext, unconditionally-replicated body carrying
-- `category: "termination-of-pregnancy"` IS the disclosure this whole mechanism exists to
-- prevent (ADR-0006 decision 4).

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The ladder.
--
--    Open TEXT, no CHECK domain: a future grade from an upgraded peer is ADMITTED verbatim
--    (additive-only, principle 11). Gaps of 10 leave room to interpose deployment terms
--    later without renumbering.
--
--    !! READ THIS BEFORE "FIXING" THE ELSE BRANCH !!
--    ELSE is MAX, deliberately INVERTING cairn_clock_grade_rank's ELSE 0 (db/040). There,
--    an unrecognised value ranking 0 is safe because rank 0 WITHHOLDS REJECT POWER. Here,
--    an unrecognised value ranking 0 would WITHHOLD PROTECTION: an older node reading a
--    peer's newer `grade:protected-witness` as "not sensitive" emits an uncoarsened safety
--    projection and renders the body in the clear — a leak on exactly the events that most
--    needed protecting, in code that looks correct because it matches db/040's pattern.
--    The failure mode here must be over-coarsening (honest, repaired by upgrading the node),
--    never disclosure (unrecoverable).
--
--    ABSENCE IS NOT UNKNOWN. No assertion at all contributes nothing and reads as 'routine'
--    (see cairn_effective_sensitivity below); an unparseable or unrecognised GRADE VALUE
--    ranks MAX. Collapsing the two would make every event in the record maximally sensitive
--    — principle 4's not-yet-asked vs unknown.
CREATE OR REPLACE FUNCTION cairn_sensitivity_rank(g text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE g
        WHEN 'routine'     THEN 0
        WHEN 'sensitive'   THEN 10
        WHEN 'restricted'  THEN 20
        WHEN 'sequestered' THEN 30
        ELSE 2147483647    -- unknown / future / NULL: coarsen, never expose
    END;
$$;

COMMIT;
```

- [ ] **Step 4: Wire the migration into BOTH loaders**

In `crates/cairn-node/src/db.rs`, append to the `SCHEMA` list, after the `db/047` entry:

```rust
    (
        "048_sensitivity_stream",
        include_str!("../../../db/048_sensitivity_stream.sql"),
    ),
```

In `crates/cairn-sync/src/main.rs`, append the identical entry to `const SCHEMA`, after `db/047`.

> **Why the subset entry is not optional.** `cairn-sync` loads an explicit subset. Omit
> `db/048` and a node syncing through `cairn-sync` stores the assertion in `event_log` with
> **no projection row**, so `cairn_effective_sensitivity` returns `routine` and the body
> renders in the clear. Slice 64's lesson (re-check every subset that loads a shared file),
> except the failure is disclosure rather than a wedged door.

In `crates/cairn-event/src/schema_generation.rs`, change `pub const SCHEMA_GENERATION: i32 = 47;` to `= 48;`.

- [ ] **Step 5: Run the test and the loader guards**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_ladder -- --nocapture
cargo test --workspace 2>&1 | tail -20
```

Expected: the ladder test PASSES; the workspace stays green, including `cairn-sync`'s `schema_subset_satisfies_its_own_doors` and the `SCHEMA_GENERATION` guard tests.

- [ ] **Step 6: Add the SQL mirror**

`db/tests/048_sensitivity_stream_test.sql`:

```sql
-- SQL mirror of crates/cairn-node/tests/sensitivity_* (see db/tests/README.md).
-- DESTRUCTIVE: runs only against a database marked disposable (#169).
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql

DO $$
BEGIN
    ASSERT cairn_sensitivity_rank('routine') = 0, 'routine ranks 0';
    ASSERT cairn_sensitivity_rank('sensitive') < cairn_sensitivity_rank('restricted'),
        'the ladder is ordered';
    ASSERT cairn_sensitivity_rank('restricted') < cairn_sensitivity_rank('sequestered'),
        'the ladder is ordered';
    ASSERT cairn_sensitivity_rank('grade:protected-witness') = 2147483647,
        'an unrecognised grade ranks MAX (inverting db/040 deliberately — ADR-0062)';
    ASSERT cairn_sensitivity_rank(NULL) = 2147483647, 'NULL lands on the safe side';
END $$;
```

- [ ] **Step 7: Run the mirrors**

```bash
scripts/run-db-sql-tests.sh
```

Expected: all mirrors pass, including the new `048`.

- [ ] **Step 8: Commit**

```bash
git add db/048_sensitivity_stream.sql db/tests/048_sensitivity_stream_test.sql \
        crates/cairn-node/tests/sensitivity_ladder.rs \
        crates/cairn-event/src/schema_generation.rs \
        crates/cairn-node/src/db.rs crates/cairn-sync/src/main.rs
git commit -m "feat(#232): the §5.9 sensitivity ladder, with the unknown ranking MAX

An unrecognised grade ranks MAX, inverting cairn_clock_grade_rank's ELSE 0.
There, unknown withholding reject power is safe; here, unknown withholding
protection is a leak — an older node would read a peer's newer grade as
'not sensitive' and render the body in the clear.

db/048 is loaded by BOTH cairn-node and cairn-sync: omitting it from the
subset would leave a synced assertion with no projection row, so the
effective grade would read routine on that node."
```

---

## Task 2: The wire types

**Files:**
- Create: `crates/cairn-event/src/sensitivity.rs`
- Modify: `crates/cairn-event/src/lib.rs`

**Interfaces:**
- Produces: `SENSITIVITY_EVENT_TYPE`, `SENSITIVITY_SCHEMA_VERSION`, `WITHDRAWAL_EVENT_TYPE`, `WITHDRAWAL_SCHEMA_VERSION`, `SubjectKind`, `SensitivityAssertion<'a>`, `SensitivityWithdrawal<'a>`, `sensitivity_assertion_body`, `sensitivity_withdrawal_body`, `render_sensitivity_twin`, `render_withdrawal_twin`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-event/src/sensitivity.rs` (tests first, module below in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_assertion_carries_subject_grade_and_source_and_no_category() {
        let a = SensitivityAssertion {
            subject_kind: SubjectKind::Thread,
            subject_id: uuid::Uuid::nil(),
            grade: "restricted",
            source: "human",
            rationale: None,
        };
        let b = sensitivity_assertion_body(&a);
        assert_eq!(b["subject_kind"], "thread");
        assert_eq!(b["grade"], "restricted");
        assert_eq!(b["source"], "human");
        // The matched blacklist category must NEVER be on the wire: a plaintext,
        // unconditionally-replicated body naming the category IS the disclosure.
        assert!(b.get("category").is_none(), "category must never travel");
        assert!(b.get("rationale").is_none(), "absent, not null");
    }

    #[test]
    fn the_builder_can_construct_a_rationale_less_chart_wide_raise() {
        // Deliberate: rationale is a DOOR rule (db/005), never a builder invariant. The
        // remote-door leniency test needs exactly this body, so a builder that refused it
        // would make the door asymmetry untestable.
        let a = SensitivityAssertion {
            subject_kind: SubjectKind::Patient,
            subject_id: uuid::Uuid::nil(),
            grade: "sensitive",
            source: "human",
            rationale: None,
        };
        let b = sensitivity_assertion_body(&a);
        assert_eq!(b["subject_kind"], "patient");
        assert!(b.get("rationale").is_none());
    }

    #[test]
    fn a_withdrawal_names_the_assertion_it_withdraws_in_hex() {
        let w = SensitivityWithdrawal {
            withdraws_hex: "a1b2c3",
            rationale: "patient consent 2026-08-09, recorded in note E44",
        };
        let b = sensitivity_withdrawal_body(&w);
        assert_eq!(b["withdraws"], "a1b2c3");
        assert_eq!(b["rationale"], "patient consent 2026-08-09, recorded in note E44");
    }

    #[test]
    fn the_twins_read_without_a_schema_and_never_name_the_category() {
        let a = SensitivityAssertion {
            subject_kind: SubjectKind::Patient,
            subject_id: uuid::Uuid::nil(),
            grade: "restricted",
            source: "advisory",
            rationale: Some("staff member treated here"),
        };
        let t = render_sensitivity_twin(&a);
        assert!(t.contains("restricted"), "the grade is the point: {t}");
        assert!(t.contains("whole chart"), "the subject must be legible: {t}");

        let w = SensitivityWithdrawal { withdraws_hex: "a1b2c3", rationale: "consent" };
        let tw = render_withdrawal_twin(&w);
        assert!(tw.contains("consent"), "the audited why must be legible: {tw}");
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test -p cairn-event sensitivity
```

Expected: FAIL — `file not found for module` / unresolved names.

- [ ] **Step 3: Write the module**

Put this **above** the `mod tests` block in `crates/cairn-event/src/sensitivity.rs`:

```rust
//! §5.9 sensitivity — the wire shape of a graded confidentiality claim and of its
//! withdrawal (ADR-0006 decision 3, ADR-0062).
//!
//! # Why these bodies are plaintext
//!
//! A node must read the grade in order to COARSEN, and coarsening is exactly what a node
//! holding no custody of the graded body must still do. Sealing the grade under the key it
//! governs is circular — so sensitivity joins ADR-0052 §2's plaintext-by-necessity list.
//!
//! # What is deliberately absent
//!
//! The matched blacklist CATEGORY. These bodies replicate unconditionally in the clear, so
//! `category: "termination-of-pregnancy"` on the wire is the disclosure the grade exists to
//! prevent (ADR-0006 decision 4). The category stays node-local.
//!
//! # Why the builders permit bodies the doors refuse
//!
//! A chart-wide raise with no rationale, and a withdrawal with no rationale, are BUILDABLE
//! here and REFUSED at the local authoring door (db/005). That split is deliberate: the
//! ceremony is a local-authoring rule, never a wire rule (ADR-0060 — a door check at apply
//! would let a peer's rationale-less act fork the event set and wedge replication), and the
//! tests that pin the remote door's leniency need to construct exactly those bodies.
use serde_json::{json, Value};
use uuid::Uuid;

/// Registered in `event_type_class` and the twin-check registry (db/048).
pub const SENSITIVITY_EVENT_TYPE: &str = "sensitivity.grade.asserted";
/// Wire schema version. Bumping it is an ADDITIVE act (ADR-0012).
pub const SENSITIVITY_SCHEMA_VERSION: &str = "sensitivity.grade.asserted/1";
pub const WITHDRAWAL_EVENT_TYPE: &str = "sensitivity.grade-withdrawal.asserted";
pub const WITHDRAWAL_SCHEMA_VERSION: &str = "sensitivity.grade-withdrawal.asserted/1";

/// What an assertion names. Adding a member here means adding it to db/048's
/// `cairn_check_sensitivity_grade` in the same commit — and note that db/048 does NOT
/// refuse an unknown kind: an unrecognised subject kind from a future peer is admitted and
/// interpreted CONSERVATIVELY as chart-wide (ADR-0062; the floor gates effect, not
/// presence — ADR-0056).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    /// One event.
    Event,
    /// A medication thread (`medication_id`). Later events on the thread inherit the grade
    /// automatically, because the effective grade is computed at READ.
    Thread,
    /// The whole chart. Deliberately the most effortful path: db/005 requires a rationale,
    /// and the blacklist can never author one (ADR-0062).
    Patient,
}

impl SubjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SubjectKind::Event => "event",
            SubjectKind::Thread => "thread",
            SubjectKind::Patient => "patient",
        }
    }
}

/// A single graded claim. Raising is frictionless by design — err toward confidential.
pub struct SensitivityAssertion<'a> {
    pub subject_kind: SubjectKind,
    pub subject_id: Uuid,
    /// Open vocabulary: db/048 ranks the named ladder and treats anything else as MAX.
    pub grade: &'a str,
    /// `human` | `advisory` — the provenance of the tag, never an authority claim.
    pub source: &'a str,
    /// Required by the local door when `subject_kind` is `Patient`; optional otherwise.
    pub rationale: Option<&'a str>,
}

/// Removing a claim from the standing set. Nothing is erased: the assertion stays in the
/// log, readable and re-assertable.
pub struct SensitivityWithdrawal<'a> {
    /// Hex `content_address` of the assertion being withdrawn. Hex because that is what the
    /// payload carries; db/048 decodes it through `cairn_decode_hex_or_raise` so a malformed
    /// value fails legibly with P0001 rather than stalling a pull (#228).
    pub withdraws_hex: &'a str,
    /// The audited why. **Clear text forever, and it replicates** — a rationale naming the
    /// condition leaks precisely what the grade protects. The UI must say so at entry.
    pub rationale: &'a str,
}

pub fn sensitivity_assertion_body(a: &SensitivityAssertion) -> Value {
    let mut body = json!({
        "subject_kind": a.subject_kind.as_str(),
        "subject_id": a.subject_id.to_string(),
        "grade": a.grade,
        "source": a.source,
    });
    // Absent, never `null`: an explicit null is an author asserting something about a
    // rationale, and absence is the honest "none given".
    if let Some(r) = a.rationale {
        body["rationale"] = json!(r);
    }
    body
}

pub fn sensitivity_withdrawal_body(w: &SensitivityWithdrawal) -> Value {
    json!({ "withdraws": w.withdraws_hex, "rationale": w.rationale })
}

/// The mandatory §3.13 legibility twin — this act in plain language, for a reader with no
/// schema at all (principle 11).
pub fn render_sensitivity_twin(a: &SensitivityAssertion) -> String {
    let subject = match a.subject_kind {
        SubjectKind::Event => "one event",
        SubjectKind::Thread => "one medication thread",
        SubjectKind::Patient => "this whole chart",
    };
    let mut out = format!(
        "Confidentiality grade \"{}\" asserted over {} ({}), source: {}",
        a.grade, subject, a.subject_id, a.source
    );
    if let Some(r) = a.rationale {
        out.push_str(&format!("; reason: {r}"));
    }
    out
}

pub fn render_withdrawal_twin(w: &SensitivityWithdrawal) -> String {
    format!(
        "Confidentiality grade withdrawn (assertion {}); reason: {}. \
         The withdrawn assertion remains on the record.",
        w.withdraws_hex, w.rationale
    )
}
```

Add to `crates/cairn-event/src/lib.rs`, in module order:

```rust
pub mod sensitivity;
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p cairn-event sensitivity
```

Expected: 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/sensitivity.rs crates/cairn-event/src/lib.rs
git commit -m "feat(#232): the sensitivity wire types

Plaintext by necessity (ADR-0052 §2): a node must read the grade to coarsen,
and coarsening is what a node with no custody must still do.

The matched blacklist category is deliberately absent from both bodies — a
plaintext replicated body naming the category IS the disclosure.

The builders PERMIT a rationale-less chart-wide raise and a rationale-less
withdrawal. The ceremony is a local-door rule, never a wire rule, and the
remote-door leniency tests need exactly those bodies."
```

---

## Task 3: The structural floor

**Files:**
- Modify: `db/048_sensitivity_stream.sql`
- Create: `crates/cairn-node/tests/sensitivity_floor.rs`
- Modify: `crates/cairn-node/tests/twin_registry.rs` (+2)
- Modify: `db/tests/034_twin_registry_test.sql` (+2)

**Interfaces:**
- Consumes: Task 2's `SENSITIVITY_EVENT_TYPE`, `sensitivity_assertion_body`, `render_sensitivity_twin`.
- Produces: `cairn_check_sensitivity_grade(text, jsonb)`, `cairn_check_sensitivity_withdrawal(text, jsonb)`; both types classified and twin-registered.

- [ ] **Step 1: Write the failing test**

`crates/cairn-node/tests/sensitivity_floor.rs`:

```rust
//! The db/048 structural floor. Every rule is a judgement about the SHAPE of the claim, so
//! it is safe at BOTH doors — a peer that produced one of these shapes produced something
//! no conformant door of any version could have minted.
mod common;
use cairn_event::sensitivity::*;
use common::{cs, db_msg, setup, submit_registration, submit_signed, EventSpec};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn the_floor_refuses_a_malformed_assertion_and_admits_a_well_formed_one() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    // The precedence rule (#345): a chart's FIRST event must be its registration, and a
    // sensitivity assertion bears patient_id — so the chart is registered first.
    submit_registration(&c, &sk, &kid, p, 1).await;

    // Well-formed: accepted.
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Thread,
        subject_id: Uuid::now_v7(),
        grade: "restricted",
        source: "human",
        rationale: None,
    };
    submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall: 10,
        },
    )
    .await
    .expect("a well-formed assertion is accepted");

    // A non-uuid subject_id is refused, legibly.
    let err = submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "thread", "subject_id": "not-a-uuid",
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("x".into()),
            wall: 11,
        },
    )
    .await
    .expect_err("a non-uuid subject_id must be refused");
    assert!(err.contains("subject_id"), "the refusal names the field: {err}");

    // A blank grade is refused: "" would rank MAX and coarsen everything, so it looks safe
    // while being a shape no author meant to write.
    let err = submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "thread", "subject_id": Uuid::now_v7().to_string(),
                "grade": "  ", "source": "human"
            }),
            plaintext_twin: Some("x".into()),
            wall: 12,
        },
    )
    .await
    .expect_err("a blank grade must be refused");
    assert!(err.contains("grade"), "the refusal names the field: {err}");
}

#[tokio::test]
async fn an_unknown_subject_kind_is_admitted_because_the_floor_gates_effect_not_presence() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // A future peer's `episode` subject must be ADMITTED (ADR-0056) — a closed CHECK here
    // would wedge the apply door on honest traffic. Task 5 pins that it is then INTERPRETED
    // conservatively, as chart-wide.
    submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "episode", "subject_id": Uuid::now_v7().to_string(),
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("future kind".into()),
            wall: 10,
        },
    )
    .await
    .expect("an unknown subject_kind is admitted, not refused");
}

#[tokio::test]
async fn a_malformed_withdraws_hex_fails_legibly_with_p0001() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let err = submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: WITHDRAWAL_EVENT_TYPE,
            schema_version: WITHDRAWAL_SCHEMA_VERSION,
            payload: json!({ "withdraws": "0xNOTHEX", "rationale": "consent" }),
            plaintext_twin: Some("x".into()),
            wall: 10,
        },
    )
    .await
    .expect_err("malformed hex must be refused");

    // Asserted on the SQLSTATE, not only the message: cairn-sync reads P0001 as "deliberate,
    // skip and re-offer" and ANYTHING ELSE as a transient fault it freezes the cursor on. A
    // message-only assertion would stay green through a well-meaning
    // `USING ERRCODE = SQLSTATE` and reintroduce the #228 permanent stall.
    assert!(err.contains("P0001"), "must raise P0001, got: {err}");
    assert!(err.contains("withdraws"), "the refusal names the field: {err}");
}
```

> `common::submit_signed` returns `Result<_, String>` carrying the formatted DB error
> (see `common::db_msg`); if the SQLSTATE is not already in that string, extend `db_msg`
> to prefix it rather than weakening the assertion.

- [ ] **Step 2: Run it and watch it fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_floor -- --nocapture
```

Expected: FAIL — the event type is unclassified, so both doors refuse everything.

- [ ] **Step 3: Add the classification, checks and twin registrations**

Insert into `db/048_sensitivity_stream.sql` before `COMMIT;`:

```sql
-- ---------------------------------------------------------------------------
-- 2. Classify both verbs. 'additive' with targets_other_author = FALSE.
--
--    A WITHDRAWAL is cross-author BY DESIGN: ADR-0006 decision 3 requires declassification
--    by AUTHORITY, not the ADR-0043 self-only suppression rule, because the self-only rule
--    deadlocks every real case (the asserting clinician retired; the patient who asserted
--    has left the practice). So it must NOT be routed through the suppression owner-gate.
--    The substitute control is the §6 ceremony in db/005: a bound human author plus a
--    rationale, enforced at the LOCAL door only.
INSERT INTO event_type_class AS r (event_type, mode, targets_other_author) VALUES
    ('sensitivity.grade.asserted',            'additive', FALSE),
    ('sensitivity.grade-withdrawal.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO UPDATE SET
    mode                 = EXCLUDED.mode,
    targets_other_author = EXCLUDED.targets_other_author
WHERE (r.mode, r.targets_other_author)
      IS DISTINCT FROM (EXCLUDED.mode, EXCLUDED.targets_other_author);

-- ---------------------------------------------------------------------------
-- 3. The structural floor for an assertion.
--
--    Note what is NOT refused: an unrecognised `subject_kind`. A closed set here would
--    wedge the apply door the first time an upgraded peer sent `episode` (ADR-0056 — the
--    floor gates EFFECT, not presence). The projection interprets an unknown kind
--    conservatively instead (section 6).
CREATE OR REPLACE FUNCTION cairn_check_sensitivity_grade(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'sensitivity assertion: missing payload';
    END IF;

    IF jsonb_typeof(p -> 'subject_kind') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'subject_kind')) = 0 THEN
        RAISE EXCEPTION 'sensitivity assertion: subject_kind must be a non-empty string';
    END IF;

    -- jsonb_typeof(NULL) is NULL, and `NULL IS DISTINCT FROM 'string'` is TRUE, so an
    -- ABSENT key lands in this branch rather than falling through (the #346 fail-OPEN
    -- pattern, avoided deliberately).
    IF jsonb_typeof(p -> 'subject_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'sensitivity assertion: subject_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'subject_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'sensitivity assertion: subject_id "%" is not a valid uuid',
            p ->> 'subject_id';
    END;

    -- A blank grade would rank MAX and coarsen everything — safe-looking, but a shape no
    -- author meant to write, and it would mask a UI bug forever (append-only: no UPDATE).
    IF jsonb_typeof(p -> 'grade') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'grade')) = 0 THEN
        RAISE EXCEPTION 'sensitivity assertion: grade must be a non-empty string';
    END IF;

    IF jsonb_typeof(p -> 'source') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'source')) = 0 THEN
        RAISE EXCEPTION 'sensitivity assertion: source must be a non-empty string (human | advisory)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 4. The structural floor for a withdrawal.
--
--    `withdraws` is decoded through cairn_decode_hex_or_raise (db/001, issue #228) so a
--    malformed value fails with the door named AND with SQLSTATE P0001. That code is a
--    CONTRACT with cairn-sync's pull loop: P0001 means "deliberate, skip and re-offer",
--    while any other SQLSTATE is read as a transient fault the cursor FREEZES on. A bare
--    decode() raises in class 22 and would stall sync from that peer permanently.
CREATE OR REPLACE FUNCTION cairn_check_sensitivity_withdrawal(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'sensitivity withdrawal: missing payload';
    END IF;

    IF jsonb_typeof(p -> 'withdraws') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'sensitivity withdrawal: withdraws must be the hex content_address of the assertion being withdrawn';
    END IF;
    PERFORM cairn_decode_hex_or_raise('withdraws', p ->> 'withdraws', 'sensitivity withdrawal');

    -- The rationale is the whole ceremony's evidence. Structural (non-empty) here; the
    -- LOCAL door additionally requires a bound human author (section 8).
    IF jsonb_typeof(p -> 'rationale') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'rationale')) = 0 THEN
        RAISE EXCEPTION 'sensitivity withdrawal: rationale must be a non-empty string (the audited why — ADR-0006 decision 3)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 5. Twin-check registrations (ADR-0048). ADDING A ROW HERE MEANS BUMPING THE EXPECTED
--    COUNT IN **BOTH** crates/cairn-node/tests/twin_registry.rs AND
--    db/tests/034_twin_registry_test.sql — the count lives in two places on purpose.
INSERT INTO cairn_event_twin_check AS r (event_type, check_fn, twin_required_msg) VALUES
    ('sensitivity.grade.asserted', 'cairn_check_sensitivity_grade',
     'sensitivity assertion requires a non-empty authored twin (a grade must be legible without a schema — principle 11)'),
    ('sensitivity.grade-withdrawal.asserted', 'cairn_check_sensitivity_withdrawal',
     'sensitivity withdrawal requires a non-empty authored twin (the audited why must be legible)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (r.check_fn, r.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);
```

- [ ] **Step 4: Bump the twin-registry counts in both places**

In `crates/cairn-node/tests/twin_registry.rs`, increase the expected row count by 2 and add the two new `event_type` names to the expected list. In `db/tests/034_twin_registry_test.sql`, make the identical change.

- [ ] **Step 5: Run the tests**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_floor --test twin_registry -- --nocapture
scripts/run-db-sql-tests.sh
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add db/048_sensitivity_stream.sql crates/cairn-node/tests/sensitivity_floor.rs \
        crates/cairn-node/tests/twin_registry.rs db/tests/034_twin_registry_test.sql
git commit -m "feat(#232): the sensitivity structural floor

Both verbs classified additive/targets_other_author=FALSE. A withdrawal is
cross-author BY DESIGN — ADR-0006 requires declassification by authority, and
the ADR-0043 self-only rule deadlocks every real case — so it must not route
through the suppression owner-gate. The ceremony in db/005 is the substitute.

An unrecognised subject_kind is ADMITTED (ADR-0056: gate effect, not
presence); a closed CHECK would wedge the apply door on an upgraded peer.

The withdraws field decodes through cairn_decode_hex_or_raise so a malformed
value raises P0001 rather than class 22 — the difference between 'skip and
re-offer' and a permanently frozen pull cursor (#228)."
```

---

## Task 4: Projection tables and apply functions

**Files:**
- Modify: `db/048_sensitivity_stream.sql`
- Modify: `crates/cairn-node/tests/sensitivity_floor.rs` (add the projection test)

**Interfaces:**
- Produces: tables `sensitivity_assertion`, `sensitivity_withdrawal`; `sensitivity_assertion_apply(event_log)`, `sensitivity_withdrawal_apply(event_log)`; both registered in `cairn_projection_apply`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/sensitivity_floor.rs`:

```rust
#[tokio::test]
async fn an_assertion_projects_and_a_withdrawal_projects_independently_of_arrival_order() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // The withdrawal is authored FIRST, naming an assertion that does not exist yet. Set-
    // union sync has no ordering, so this is normal traffic, and no FK may forbid it.
    let ghost = "aa".repeat(34); // a syntactically valid multihash-shaped hex value
    submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: WITHDRAWAL_EVENT_TYPE,
            schema_version: WITHDRAWAL_SCHEMA_VERSION,
            payload: json!({ "withdraws": ghost, "rationale": "consent" }),
            plaintext_twin: Some("withdrawn".into()),
            wall: 10,
        },
    )
    .await
    .expect("a withdrawal naming an unseen assertion must be accepted");

    let rows: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 1, "the withdrawal projects even with no target present");

    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: p,
        grade: "sensitive",
        source: "human",
        rationale: Some("staff member treated here"),
    };
    submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall: 11,
        },
    )
    .await
    .expect("assertion accepted");

    let row = c
        .query_one(
            "SELECT subject_kind, grade, source FROM sensitivity_assertion
              WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .map_err(|e| db_msg(&e))
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "patient");
    assert_eq!(row.get::<_, String>(1), "sensitive");
    assert_eq!(row.get::<_, String>(2), "human");
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_floor -- --nocapture
```

Expected: FAIL — `relation "sensitivity_withdrawal" does not exist`.

- [ ] **Step 3: Add the tables, apply fns and registry rows**

Insert into `db/048_sensitivity_stream.sql` before `COMMIT;`:

```sql
-- ---------------------------------------------------------------------------
-- 6. The retained sets.
--
--    patient_id is on EVERY row regardless of subject kind: it makes the whole effective-
--    grade computation one indexed scan per chart, instead of repeating #336 (the med-list
--    read path is O(all medications on the node) per chart open).
--
--    NO CHECK on subject_kind or grade: both are open vocabularies (ADR-0056/principle 11).
CREATE TABLE IF NOT EXISTS sensitivity_assertion (
    content_address BYTEA   PRIMARY KEY,   -- the producing event; provenance-precise
    event_id        UUID    NOT NULL,
    patient_id      UUID    NOT NULL,
    subject_kind    TEXT    NOT NULL,
    subject_id      UUID    NOT NULL,
    grade           TEXT    NOT NULL,
    source          TEXT    NOT NULL,
    rationale       TEXT,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    node_origin     TEXT    NOT NULL,
    first_seen      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS sensitivity_assertion_patient_idx
    ON sensitivity_assertion (patient_id);

--    NO FOREIGN KEY from `withdraws` to sensitivity_assertion. A withdrawal can arrive
--    BEFORE the assertion it withdraws (set-union sync has no ordering) and must still take
--    effect when the assertion lands — so "standing" is a set difference evaluated at READ
--    (section 7), never a row deletion at apply. Same arrival-order independence as
--    ADR-0059's "a strike NULLs the anchor rather than deleting the row".
CREATE TABLE IF NOT EXISTS sensitivity_withdrawal (
    content_address BYTEA   PRIMARY KEY,
    event_id        UUID    NOT NULL,
    withdraws       BYTEA   NOT NULL,
    patient_id      UUID    NOT NULL,
    rationale       TEXT    NOT NULL,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    node_origin     TEXT    NOT NULL,
    first_seen      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS sensitivity_withdrawal_target_idx
    ON sensitivity_withdrawal (withdraws);

GRANT SELECT ON sensitivity_assertion, sensitivity_withdrawal TO cairn_agent;

-- ---------------------------------------------------------------------------
-- 7. Apply. ON CONFLICT DO NOTHING is genuinely idempotent here (not the #254 bug): the PK
--    IS the content address, so a conflict means the SAME event applying twice and the
--    existing row is byte-for-byte what we would write.
--
--    The `e.sealed` guard mirrors every other non-clinical projection: only clinical.* is
--    born-sealed and db/005 refuses a sealed sensitivity body, but the APPLY door stays
--    lenient, so such a row can still reach here. Reading its ciphertext would drive NULLs
--    into NOT NULL columns and wedge the watermark; projecting nothing is harmless noise.
CREATE OR REPLACE FUNCTION sensitivity_assertion_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := e.body;
BEGIN
    IF e.sealed THEN RETURN; END IF;
    INSERT INTO sensitivity_assertion
        (content_address, event_id, patient_id, subject_kind, subject_id,
         grade, source, rationale, hlc_wall, hlc_counter, node_origin)
    VALUES (
        e.content_address, e.event_id, e.patient_id,
        p ->> 'subject_kind', (p ->> 'subject_id')::uuid,
        p ->> 'grade', p ->> 'source', p ->> 'rationale',
        e.hlc_wall, e.hlc_counter, e.node_origin)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION sensitivity_assertion_apply(event_log) FROM PUBLIC;

CREATE OR REPLACE FUNCTION sensitivity_withdrawal_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := e.body;
BEGIN
    IF e.sealed THEN RETURN; END IF;
    INSERT INTO sensitivity_withdrawal
        (content_address, event_id, withdraws, patient_id, rationale,
         hlc_wall, hlc_counter, node_origin)
    VALUES (
        e.content_address, e.event_id,
        cairn_decode_hex_or_raise('withdraws', p ->> 'withdraws', 'sensitivity withdrawal apply'),
        e.patient_id, p ->> 'rationale',
        e.hlc_wall, e.hlc_counter, e.node_origin)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION sensitivity_withdrawal_apply(event_log) FROM PUBLIC;

-- ---------------------------------------------------------------------------
-- 8. Register both apply fns with the ADR-0057 dispatcher + cairn_reproject heal/rebuild.
--    heal_safe = TRUE: content-addressed PK + DO NOTHING makes replay a no-op.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('sensitivity.grade.asserted', 'sensitivity_assertion_apply',
        ARRAY['sensitivity_assertion'], 10, TRUE),
       ('sensitivity.grade-withdrawal.asserted', 'sensitivity_withdrawal_apply',
        ARRAY['sensitivity_withdrawal'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);
```

- [ ] **Step 4: Run the tests**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_floor -- --nocapture
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add db/048_sensitivity_stream.sql crates/cairn-node/tests/sensitivity_floor.rs
git commit -m "feat(#232): sensitivity projections, with no FK on the withdrawal target

A withdrawal may arrive BEFORE the assertion it withdraws — set-union sync
has no ordering — so no FK may forbid it, and standing is a set difference
evaluated at read rather than a deletion at apply (the ADR-0059 shape).

patient_id sits on every row regardless of subject kind, so the effective
grade is one indexed scan per chart rather than a repeat of #336."
```

---

## Task 5: Standing, thread resolution, and the effective grade

**Files:**
- Modify: `db/048_sensitivity_stream.sql`
- Modify: `crates/cairn-node/tests/sensitivity_ladder.rs`
- Modify: `db/tests/048_sensitivity_stream_test.sql`

**Interfaces:**
- Produces: `cairn_sensitivity_standing(uuid)`, `cairn_event_thread(uuid)`, `cairn_effective_sensitivity(uuid) -> TABLE(grade text, subject_kind text, content_address bytea)`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/sensitivity_ladder.rs` (add `mod common` imports as in `sensitivity_floor.rs`):

```rust
/// Helper: author one assertion and return nothing. Kept local — it is only meaningful
/// with this file's fixtures.
async fn assert_grade(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: uuid::Uuid,
    kind: SubjectKind,
    subject: uuid::Uuid,
    grade: &str,
    wall: i64,
) {
    let a = SensitivityAssertion {
        subject_kind: kind,
        subject_id: subject,
        grade,
        source: "human",
        rationale: Some("test fixture"),
    };
    submit_signed(
        c, sk, kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall,
        },
    )
    .await
    .expect("assertion accepted");
}

#[tokio::test]
async fn the_effective_grade_is_the_max_over_event_thread_and_chart() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // A plain event on this chart with no assertion of its own.
    let target = submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "routine note" }),
            plaintext_twin: Some("routine note".into()),
            wall: 10,
        },
    )
    .await
    .expect("note accepted");

    let effective = |ev: uuid::Uuid| {
        let c = &c;
        async move {
            c.query_one(
                "SELECT grade, subject_kind FROM cairn_effective_sensitivity($1::text::uuid)",
                &[&ev.to_string()],
            )
            .await
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .map_err(|e| db_msg(&e))
            .unwrap()
        }
    };

    // No assertions anywhere: absence reads as routine, NOT as unknown.
    assert_eq!(effective(target).await.0, "routine");

    // A chart-wide grade reaches an event that carries none of its own.
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sensitive", 11).await;
    assert_eq!(effective(target).await, ("sensitive".into(), "patient".into()));

    // An event-scoped grade outranks the chart-wide one: max, and the winner is named.
    assert_grade(&c, &sk, &kid, p, SubjectKind::Event, target, "restricted", 12).await;
    assert_eq!(effective(target).await, ("restricted".into(), "event".into()));
}

#[tokio::test]
async fn a_withdrawal_lowers_the_effective_grade_and_the_assertion_survives() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let target = submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "n" }),
            plaintext_twin: Some("n".into()),
            wall: 10,
        },
    )
    .await
    .unwrap();

    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sequestered", 11).await;
    let ca_hex: String = c
        .query_one(
            "SELECT encode(content_address, 'hex') FROM sensitivity_assertion
              WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: WITHDRAWAL_EVENT_TYPE,
            schema_version: WITHDRAWAL_SCHEMA_VERSION,
            payload: serde_json::json!({ "withdraws": ca_hex, "rationale": "patient consent" }),
            plaintext_twin: Some("withdrawn".into()),
            wall: 12,
        },
    )
    .await
    .expect("withdrawal accepted");

    let g: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&target.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(g, "routine", "the withdrawn assertion no longer stands");

    // Nothing was erased — the assertion is still on the record, still re-assertable.
    let still: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(still, 1, "declassification is an overlay, never an erasure");
}

#[tokio::test]
async fn an_unknown_subject_kind_is_read_as_chart_wide_and_never_crosses_charts() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    let other = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    submit_registration(&c, &sk, &kid, other, 1).await;

    let mine = submit_signed(
        &c, &sk, &kid,
        EventSpec { patient: p, event_type: "note.added", schema_version: "note.added/1",
                    payload: serde_json::json!({"text":"n"}), plaintext_twin: Some("n".into()), wall: 10 },
    ).await.unwrap();
    let theirs = submit_signed(
        &c, &sk, &kid,
        EventSpec { patient: other, event_type: "note.added", schema_version: "note.added/1",
                    payload: serde_json::json!({"text":"n"}), plaintext_twin: Some("n".into()), wall: 10 },
    ).await.unwrap();

    submit_signed(
        &c, &sk, &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: serde_json::json!({
                "subject_kind": "episode", "subject_id": uuid::Uuid::now_v7().to_string(),
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("future kind".into()),
            wall: 11,
        },
    ).await.expect("admitted");

    let g = |ev: uuid::Uuid| {
        let c = &c;
        async move {
            c.query_one("SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
                        &[&ev.to_string()])
                .await.unwrap().get::<_, String>(0)
        }
    };
    assert_eq!(g(mine).await, "restricted", "unknown kind is read conservatively, chart-wide");
    assert_eq!(g(theirs).await, "routine", "and the envelope bounds it to ITS OWN chart");
}

#[tokio::test]
async fn recall_marks_an_assertion_but_never_lowers_the_grade() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let target = submit_signed(
        &c, &sk, &kid,
        EventSpec { patient: p, event_type: "note.added", schema_version: "note.added/1",
                    payload: serde_json::json!({"text":"n"}), plaintext_twin: Some("n".into()), wall: 10 },
    ).await.unwrap();
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "restricted", 11).await;

    let assertion_event: uuid::Uuid = c
        .query_one("SELECT event_id FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
                   &[&p.to_string()])
        .await.unwrap().get(0);

    // Recall the assertion's own event. recall_overlay MARKS; it must never remove the
    // assertion from the standing set — otherwise recalling a bad actor would silently
    // strip protection from every patient they graded.
    c.execute(
        "INSERT INTO recall_overlay (event_id, reason) VALUES ($1::text::uuid, 'test recall')
         ON CONFLICT DO NOTHING",
        &[&assertion_event.to_string()],
    )
    .await
    .map_err(|e| db_msg(&e))
    .expect("recall_overlay insert");

    let g: String = c
        .query_one("SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
                   &[&target.to_string()])
        .await.unwrap().get(0);
    assert_eq!(g, "restricted", "recall marks; it must never lower a grade");
}
```

> If `recall_overlay`'s column names differ from `(event_id, reason)`, read `db/006_recall.sql`
> and use its actual columns — the assertion being pinned is the grade, not the insert shape.

- [ ] **Step 2: Run and watch fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_ladder -- --nocapture
```

Expected: FAIL — `function cairn_effective_sensitivity(uuid) does not exist`.

- [ ] **Step 3: Add standing, thread resolution and the effective grade**

Insert into `db/048_sensitivity_stream.sql` before `COMMIT;`:

```sql
-- ---------------------------------------------------------------------------
-- 9. Standing = asserted minus withdrawn. ONE definition, so nothing can disagree about
--    what "still applies" means. Matching on content_address alone is correct: a content
--    address is globally unique, so a withdrawal cannot name a different chart's assertion
--    by accident.
CREATE OR REPLACE FUNCTION cairn_sensitivity_standing(p_patient_id uuid)
RETURNS TABLE (content_address bytea, subject_kind text, subject_id uuid, grade text)
LANGUAGE sql STABLE AS $$
    SELECT a.content_address, a.subject_kind, a.subject_id, a.grade
    FROM sensitivity_assertion a
    WHERE a.patient_id = p_patient_id
      AND NOT EXISTS (SELECT 1 FROM sensitivity_withdrawal w
                       WHERE w.withdraws = a.content_address);
$$;

-- ---------------------------------------------------------------------------
-- 10. Event -> thread. Returns NULL when the thread cannot be determined HERE.
--
--     medication_id lives INSIDE the sealed payload, and every medication projection is
--     populated through cairn_clear_payload — so on a node holding no custody the rows are
--     absent and this returns NULL. That is not a bug to route around; section 11 turns the
--     NULL into a conservative bound. It also returns NULL for a SHREDDED event, whose
--     projection rows db/037 scrubbed — which is exactly why the bound is needed today and
--     not only after sequester lands.
CREATE OR REPLACE FUNCTION cairn_event_thread(p_event_id uuid)
RETURNS uuid LANGUAGE sql STABLE AS $$
    WITH ca AS (SELECT content_address FROM event_log WHERE event_id = p_event_id)
    SELECT medication_id FROM (
        SELECT medication_id, content_address FROM medication_statement
        UNION ALL SELECT medication_id, content_address FROM medication_cessation
        UNION ALL SELECT medication_id, content_address FROM medication_coding
        UNION ALL SELECT medication_id, content_address FROM medication_dose
        UNION ALL SELECT medication_id, content_address FROM medication_dose_correction
    ) t
    WHERE t.content_address = (SELECT content_address FROM ca)
    LIMIT 1;
$$;

-- ---------------------------------------------------------------------------
-- 11. The effective grade: max by rank over standing assertions on
--     {this event, its thread, its patient}, with the winning subject named.
--
--     THE THREAD BRANCH IS THE SUBTLE ONE (ADR-0062, design §10b):
--       * thread resolves            -> that thread's standing assertions
--       * unresolved, chart HAS any thread-scoped assertion
--                                    -> ALL of the chart's thread assertions. A precise
--                                       conservative bound, not a sentinel: the event
--                                       belongs to SOME thread here, so the tightest safe
--                                       answer is the max over the chart's thread grades.
--       * unresolved, chart has none -> nothing. Without this clause every medication event
--                                       on every custody-less node would coarsen maximally.
--
--     CONSEQUENCE, stated so nobody "fixes" it: the effective grade is NON-MONOTONE IN
--     CUSTODY — gaining custody can LOWER it, as the bound collapses to the true value. The
--     grade is a function of local custody, not a global fact. ADR-0052 §9 found the same
--     about ADR-0049's thread commitment. Any cross-node equality test must therefore hold
--     custody equal.
--
--     Absence of every assertion reads as 'routine' (the coalesce below), never as unknown.
CREATE OR REPLACE FUNCTION cairn_effective_sensitivity(p_event_id uuid)
RETURNS TABLE (grade text, subject_kind text, content_address bytea)
LANGUAGE sql STABLE AS $$
    WITH ev AS (
        SELECT e.event_id, e.patient_id, cairn_event_thread(e.event_id) AS thread
        FROM event_log e WHERE e.event_id = p_event_id
    ),
    standing AS (
        SELECT s.* FROM ev, LATERAL cairn_sensitivity_standing(ev.patient_id) s
    ),
    applicable AS (
        -- event-scoped
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'event' AND s.subject_id = ev.event_id
        UNION ALL
        -- chart-scoped
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'patient' AND s.subject_id = ev.patient_id
        UNION ALL
        -- thread-scoped, resolved
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'thread' AND ev.thread IS NOT NULL
          AND s.subject_id = ev.thread
        UNION ALL
        -- thread-scoped, UNRESOLVED: the conservative bound (design §10b)
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'thread' AND ev.thread IS NULL
        UNION ALL
        -- an UNRECOGNISED subject kind: read as chart-wide, bounded by this envelope's
        -- patient (over-select, never silently miss — db/006's recall discipline)
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s
        WHERE s.subject_kind NOT IN ('event', 'thread', 'patient')
    )
    -- The LEFT JOIN LATERAL over a one-row constant is what makes this return EXACTLY ONE
    -- row even when nothing applies — so every caller can use query_one and read 'routine'
    -- rather than having to distinguish "no row" from "not sensitive". Absence is not
    -- unknown (principle 4), and that distinction is easiest to get wrong at the call site.
    SELECT COALESCE(a.grade, 'routine'),
           COALESCE(a.subject_kind, 'none'),
           a.content_address
    FROM (SELECT 1) AS one_row
    LEFT JOIN LATERAL (
        SELECT ap.grade, ap.subject_kind, ap.content_address
        FROM applicable ap
        -- Rank first; content_address breaks a tie between two equally-ranked grades
        -- deterministically (BYTEA has no collation — ADR-0045/#115).
        ORDER BY cairn_sensitivity_rank(ap.grade) DESC, ap.content_address ASC
        LIMIT 1
    ) a ON TRUE;
$$;

GRANT EXECUTE ON FUNCTION cairn_effective_sensitivity(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_sensitivity_standing(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_event_thread(uuid) TO cairn_agent;
```

- [ ] **Step 4: Run the tests**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_ladder -- --nocapture
```

Expected: all five PASS. If the `episode` case returns `routine`, the `NOT IN` branch is
missing or is comparing against the wrong set.

- [ ] **Step 5: Mirror the effective-grade assertions in SQL**

Append the max-over-three-subjects case and the withdrawal case to
`db/tests/048_sensitivity_stream_test.sql`, using `submit_event` directly (see
`db/tests/045_patient_registration_test.sql` for the fixture shape), then run
`scripts/run-db-sql-tests.sh`.

- [ ] **Step 6: Commit**

```bash
git add db/048_sensitivity_stream.sql crates/cairn-node/tests/sensitivity_ladder.rs \
        db/tests/048_sensitivity_stream_test.sql
git commit -m "feat(#232): standing, thread resolution, and the effective grade

Effective = max by rank over {event, thread, chart}, winner named.

The thread branch carries the §10b bound: medication_id lives inside the
sealed payload, so thread resolution needs custody and would otherwise
compute a LOWER grade on the node with less custody. Unresolved + the chart
has thread assertions bounds to the max over the chart's thread grades — a
precise conservative bound rather than a sentinel; unresolved + none
contributes nothing, so ungraded charts are unaffected.

This also makes §5.9's coarsen-but-survive true after a rung-3 shred, whose
scrubbed projection rows make the thread unresolvable permanently.

Consequence recorded in the code: the effective grade is non-monotone in
custody, the ADR-0052 §9 pattern. A recall test pins that recall marks but
never lowers."
```

---

## Task 6: The local-door ceremony and the remote-door asymmetry

**Files:**
- Modify: `db/005_submit.sql`
- Create: `crates/cairn-node/tests/sensitivity_ceremony.rs`

**Interfaces:**
- Produces: `cairn_sensitivity_ceremony_ok(text, jsonb, bytea)` called from `db/005` only.

- [ ] **Step 1: Write the failing test**

`crates/cairn-node/tests/sensitivity_ceremony.rs`:

```rust
//! Raising is free; lowering is a ceremony — and the ceremony is a LOCAL-AUTHORING rule.
//!
//! The asymmetry is tested, never merely commented (the Slice 64 pattern). A door check at
//! apply would let a peer's rationale-less act be refused, forking the event set and
//! wedging replication (ADR-0060, the #342 trap). For a RAISE the asymmetry is doubly
//! forced: refusing a peer's protective assertion would leave this node computing a LOWER
//! grade than the peer's, so the refusal is itself a disclosure.
mod common;
use cairn_event::sensitivity::*;
use common::{cs, setup, submit_registration, submit_signed, apply_remote_event, EventSpec};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn the_local_door_requires_a_rationale_for_a_chart_wide_raise() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // A THREAD raise needs no ceremony — raising must stay frictionless.
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Thread,
        subject_id: Uuid::now_v7(),
        grade: "restricted",
        source: "human",
        rationale: None,
    };
    submit_signed(&c, &sk, &kid, EventSpec {
        patient: p, event_type: SENSITIVITY_EVENT_TYPE,
        schema_version: SENSITIVITY_SCHEMA_VERSION,
        payload: sensitivity_assertion_body(&a),
        plaintext_twin: Some(render_sensitivity_twin(&a)), wall: 10,
    }).await.expect("a thread raise carries no ceremony");

    // A CHART-WIDE raise does: it is the one act whose blast radius is the whole record.
    let err = submit_signed(&c, &sk, &kid, EventSpec {
        patient: p, event_type: SENSITIVITY_EVENT_TYPE,
        schema_version: SENSITIVITY_SCHEMA_VERSION,
        payload: json!({
            "subject_kind": "patient", "subject_id": p.to_string(),
            "grade": "restricted", "source": "human"
        }),
        plaintext_twin: Some("chart-wide".into()), wall: 11,
    }).await.expect_err("a chart-wide raise with no rationale must be refused locally");
    assert!(err.contains("P0001"), "deliberate refusal: {err}");
    assert!(err.contains("rationale"), "the refusal names what would repair it: {err}");
}

#[tokio::test]
async fn the_remote_door_admits_what_the_local_door_refuses() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // The same rationale-less chart-wide raise, arriving from a peer. It MUST apply: a
    // refusal would both wedge replication and leave us less protected than the peer.
    apply_remote_event(&c, &sk, &kid, EventSpec {
        patient: p, event_type: SENSITIVITY_EVENT_TYPE,
        schema_version: SENSITIVITY_SCHEMA_VERSION,
        payload: json!({
            "subject_kind": "patient", "subject_id": p.to_string(),
            "grade": "restricted", "source": "human"
        }),
        plaintext_twin: Some("chart-wide".into()), wall: 12,
    }).await.expect("the remote door is lenient BY DESIGN");

    let n: i64 = c.query_one(
        "SELECT count(*) FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
        &[&p.to_string()],
    ).await.unwrap().get(0);
    assert_eq!(n, 1, "the peer's protective assertion stands here too");
}
```

> `common::apply_remote_event` is the existing helper the medication and registration
> suites use for the lenient door; if its name differs in `tests/common/mod.rs`, use the
> one those suites use rather than adding a second helper (#327).
>
> **A new `pub fn` in `tests/common/mod.rs` must also be added to
> `identity_scaffolding_shared.rs`'s hand-written expected-helper array**, or
> `derivation_finds_the_expected_helpers` fails.

- [ ] **Step 2: Run and watch fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_ceremony -- --nocapture
```

Expected: FAIL — the chart-wide raise is currently accepted locally.

- [ ] **Step 3: Add the ceremony to the LOCAL door only**

Add to `db/048_sensitivity_stream.sql` (before `COMMIT;`):

```sql
-- ---------------------------------------------------------------------------
-- 12. The ceremony. Called from db/005 (LOCAL authoring) and from NOWHERE ELSE.
--
--     Raising is frictionless — err toward confidential — with ONE exception: a chart-wide
--     raise states why. It is the only act here whose blast radius is the entire record,
--     and once part B coarsens safety projections a chart-wide grade blurs every signal on
--     the chart, including the ones with nothing sensitive about them. The rationale is
--     what the person who later has to unwind it gets to read.
--
--     Lowering always costs: a bound human author (ADR-0053) plus a rationale. ADR-0061
--     decision 4 REFUSED an authorship gate on registration because that blocks CARE
--     DOCUMENTATION; a withdrawal is an administrative act with a consent basis, blocks
--     nothing clinical (the content stays readable to everyone who already has custody —
--     only the GRADE stays high), so the asymmetry is deliberate, not an oversight.
CREATE OR REPLACE FUNCTION cairn_sensitivity_ceremony_ok(
    p_type text, b jsonb, p_authorship_actor bytea
) RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p_type = 'sensitivity.grade.asserted'
       AND (p ->> 'subject_kind') = 'patient'
       AND (jsonb_typeof(p -> 'rationale') IS DISTINCT FROM 'string'
            OR length(trim(p ->> 'rationale')) = 0) THEN
        RAISE EXCEPTION 'sensitivity: a chart-wide grade states why — supply a rationale (it coarsens every signal on this chart; a thread- or event-scoped grade needs none)';
    END IF;

    IF p_type = 'sensitivity.grade-withdrawal.asserted' AND p_authorship_actor IS NULL THEN
        RAISE EXCEPTION 'sensitivity: withdrawing a grade requires a bound human author — removing protection is accountable (ADR-0053; raising one is not)';
    END IF;
END;
$$;
```

In `db/005_submit.sql`, call it from `submit_event` **after** the twin/structural dispatch
and alongside the other checks that judge the event itself — i.e. before the four refusals
that read the log's own state (custody, substitution, the two erasure-target checks):

```sql
    PERFORM cairn_sensitivity_ceremony_ok(v_type, b, v_authorship_actor);
```

> Use whatever local variable `db/005` already holds the bound human author in (the one
> `cairn_authorship_bound` populates at step 4b) — do not introduce a second lookup.
> **Do NOT add this call to `db/020`.** The remote door stays lenient, and the second test
> above fails if it does not.

- [ ] **Step 4: Run the tests**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_ceremony -- --nocapture
cargo test --workspace 2>&1 | tail -20
```

Expected: both PASS; workspace green.

- [ ] **Step 5: Commit**

```bash
git add db/005_submit.sql db/048_sensitivity_stream.sql \
        crates/cairn-node/tests/sensitivity_ceremony.rs
git commit -m "feat(#232): raising is free, lowering is a ceremony — at the local door only

A chart-wide raise states why; a withdrawal needs a bound human author and a
rationale. Both are LOCAL-AUTHORING rules: a check at the apply door would let
a peer's rationale-less act fork the event set and wedge replication (#342),
and for a raise it is doubly wrong — refusing a peer's protective assertion
leaves this node computing a lower grade than the peer, so the refusal is
itself a disclosure. The asymmetry is tested, not commented.

The authorship gate ADR-0061 decision 4 refused for registration is right
here: a withdrawal is administrative, not care documentation, and blocks
nothing clinical."
```

---

## Task 7: The category blacklist

**Files:**
- Modify: `db/048_sensitivity_stream.sql`
- Modify: `db/tests/048_sensitivity_stream_test.sql`

**Interfaces:**
- Produces: table `sensitivity_category_map`; `cairn_sensitivity_candidate(jsonb) -> TABLE(grade text, category text)`.

- [ ] **Step 1: Write the failing SQL test**

Append to `db/tests/048_sensitivity_stream_test.sql`:

```sql
DO $$
DECLARE r record; n int;
BEGIN
    -- Ships EMPTY. Cairn provides the lookup mechanism, never the list (ADR-0006 §3).
    SELECT count(*) INTO n FROM sensitivity_category_map;
    ASSERT n = 0, 'the category map ships empty — the list is deployment configuration';

    SELECT count(*) INTO n FROM cairn_sensitivity_candidate('{"category":"sti-screen"}'::jsonb);
    ASSERT n = 0, 'an unmapped category yields no candidate';

    INSERT INTO sensitivity_category_map (category, grade, note)
    VALUES ('sti-screen', 'restricted', 'test fixture');

    SELECT * INTO r FROM cairn_sensitivity_candidate('{"category":"sti-screen"}'::jsonb);
    ASSERT r.grade = 'restricted', 'a mapped category yields its grade';
    ASSERT r.category = 'sti-screen', 'and names what matched, for LOCAL audit only';

    -- The function authors nothing: policy decides whether a candidate becomes an event.
    SELECT count(*) INTO n FROM event_log WHERE event_type = 'sensitivity.grade.asserted';
    ASSERT n = 0, 'the lookup must never author an assertion by itself';

    DELETE FROM sensitivity_category_map WHERE category = 'sti-screen';
END $$;
```

- [ ] **Step 2: Run and watch fail**

```bash
scripts/run-db-sql-tests.sh
```

Expected: FAIL — `relation "sensitivity_category_map" does not exist`.

- [ ] **Step 3: Add the map and the lookup**

Insert into `db/048_sensitivity_stream.sql` before `COMMIT;`:

```sql
-- ---------------------------------------------------------------------------
-- 13. The category blacklist — the AUTOMATIC source (ADR-0006 §3).
--
--     Ships EMPTY. Cairn provides the lookup MECHANISM, never the list: what is sensitive
--     is cultural, regional and personal, and shipping a list would be Cairn making the
--     policy (principle 9).
CREATE TABLE IF NOT EXISTS sensitivity_category_map (
    category TEXT PRIMARY KEY,
    grade    TEXT NOT NULL,
    note     TEXT NOT NULL DEFAULT ''
);
GRANT SELECT ON sensitivity_category_map TO cairn_agent;
REVOKE INSERT, UPDATE, DELETE ON sensitivity_category_map FROM PUBLIC;

--     A PURE lookup that yields a CANDIDATE. It authors nothing — all three ADR-0006
--     workflows are the same call site with different callers:
--       silent apply     -> the caller authors the assertion as an advisory actor
--       acceptance first -> the caller shows the candidate, a human authors it
--       manual only      -> the caller never calls this
--
--     THE SUBJECT IS NEVER THE PATIENT. This function cannot express a chart-wide candidate
--     at all: a coded hit on one drug blanket-grading an entire chart is exactly
--     "chart-wide as the default for highly sensitive records", which is the thing the
--     friction in section 12 exists to prevent. The caller pairs the returned grade with
--     the event or thread that carried the coded field.
CREATE OR REPLACE FUNCTION cairn_sensitivity_candidate(p_coded jsonb)
RETURNS TABLE (grade text, category text)
LANGUAGE sql STABLE AS $$
    SELECT m.grade, m.category
    FROM sensitivity_category_map m
    WHERE m.category = (p_coded ->> 'category')
    ORDER BY cairn_sensitivity_rank(m.grade) DESC
    LIMIT 1;
$$;
GRANT EXECUTE ON FUNCTION cairn_sensitivity_candidate(jsonb) TO cairn_agent;
```

- [ ] **Step 4: Run the mirrors**

```bash
scripts/run-db-sql-tests.sh
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/048_sensitivity_stream.sql db/tests/048_sensitivity_stream_test.sql
git commit -m "feat(#232): the category blacklist — lookup only, never chart-wide

Ships empty: Cairn provides the mechanism, never the list. The function
authors nothing, so silent-apply, acceptance-required and manual-only are the
same call site with different callers.

It cannot express a chart-wide candidate at all. A coded hit on one drug
grading the whole chart is precisely chart-wide-as-the-default, which the
ceremony exists to prevent."
```

---

## Task 8: Orchestration and the three CLI verbs

**Files:**
- Create: `crates/cairn-node/src/sensitivity.rs`
- Modify: `crates/cairn-node/src/lib.rs`, `crates/cairn-node/src/main.rs`

**Interfaces:**
- Consumes: Task 2's builders; Task 5's `cairn_effective_sensitivity`.
- Produces: `assert_sensitivity(...)`, `withdraw_sensitivity(...)`, `chart_sensitivity(...) -> Vec<ChartGrade>`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/sensitivity_ladder.rs`:

```rust
#[tokio::test]
async fn the_chart_report_names_the_winning_subject_for_every_graded_thread() {
    let Some(base) = cs() else { return };
    cairn_node::db::test_serial_guard(&base).await;
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sensitive", 11).await;

    let report = cairn_node::sensitivity::chart_sensitivity(&mut c, p).await.unwrap();
    assert_eq!(report.chart_grade, "sensitive");
    assert_eq!(
        report.chart_source, "chart-wide",
        "the report must name WHICH subject won — otherwise nobody can tell why a whole \
         chart is blurred, and therefore nobody can fix it"
    );
}
```

- [ ] **Step 2: Run and watch fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test sensitivity_ladder -- --nocapture
```

Expected: FAIL — unresolved module `cairn_node::sensitivity`.

- [ ] **Step 3: Write the orchestrator**

`crates/cairn-node/src/sensitivity.rs` — follow `patient/register.rs`'s shape exactly:
tick one HLC per event authored (`crate::db::next_hlc`), build the body with Task 2's
builder, sign with `cairn_event::sign`, submit through `submit_event`. Public surface:

```rust
/// One chart's grades, as the report renders them.
pub struct ChartReport {
    pub chart_grade: String,
    /// Which subject won: "chart-wide" | "this thread" | "this event" | "none".
    pub chart_source: String,
    pub threads: Vec<(uuid::Uuid, String, String)>,
}

pub async fn assert_sensitivity(
    client: &mut tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    node_origin: &str,
    patient: uuid::Uuid,
    subject_kind: cairn_event::sensitivity::SubjectKind,
    subject_id: uuid::Uuid,
    grade: &str,
    rationale: Option<&str>,
) -> anyhow::Result<uuid::Uuid>;

pub async fn withdraw_sensitivity(
    client: &mut tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    node_origin: &str,
    patient: uuid::Uuid,
    withdraws_hex: &str,
    rationale: &str,
) -> anyhow::Result<uuid::Uuid>;

pub async fn chart_sensitivity(
    client: &mut tokio_postgres::Client,
    patient: uuid::Uuid,
) -> anyhow::Result<ChartReport>;
```

`chart_sensitivity` maps the `subject_kind` returned by `cairn_effective_sensitivity` to
the human phrase (`patient` → `"chart-wide"`, `thread` → `"this thread"`, `event` →
`"this event"`, `none` → `"none"`, anything else → `"an unrecognised scope (read
chart-wide)"`). Add `pub mod sensitivity;` to `crates/cairn-node/src/lib.rs`.

- [ ] **Step 4: Add the three CLI verbs**

In `crates/cairn-node/src/main.rs`, add to the `Cmd` enum and its dispatch, following the
`PatientRegister` arm:

```rust
    /// Assert a confidentiality grade over an event, a medication thread, or a whole chart.
    ///
    /// Raising is deliberately cheap for an event or a thread. A whole-chart grade requires
    /// --rationale: it coarsens every signal on that chart, and the person who later has to
    /// unwind it needs something to read.
    SensitivityAssert {
        #[arg(long)] patient: Uuid,
        #[arg(long, value_parser = ["event", "thread", "patient"])] subject_kind: String,
        #[arg(long)] subject_id: Uuid,
        #[arg(long)] grade: String,
        #[arg(long)] rationale: Option<String>,
    },
    /// Withdraw a standing grade. Requires a rationale and an unlocked human key: removing
    /// protection is accountable. The withdrawn assertion stays on the record.
    SensitivityWithdraw {
        #[arg(long)] patient: Uuid,
        /// Hex content_address of the assertion, as `patient-sensitivity` prints it.
        #[arg(long)] withdraws: String,
        #[arg(long)] rationale: String,
    },
    /// Report a chart's effective grades. Reports only — nothing is withheld by this slice.
    PatientSensitivity {
        #[arg(long)] patient: Uuid,
    },
```

- [ ] **Step 5: Run the tests**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test --workspace 2>&1 | tail -20
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check
```

Expected: all green.

- [ ] **Step 6: Exercise the CLI once by hand**

```bash
cargo run -p cairn-node -- patient-sensitivity --patient <a registered uuid>
```

Expected: a report naming the winning subject for each line; no withholding.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/sensitivity.rs crates/cairn-node/src/lib.rs \
        crates/cairn-node/src/main.rs crates/cairn-node/tests/sensitivity_ladder.rs
git commit -m "feat(#232): sensitivity orchestration and three CLI verbs

The report always names WHICH subject won — chart-wide vs this thread. Without
it nobody can tell why an entire chart is uniformly blurred, and therefore
nobody can fix it.

Reports only: this slice withholds nothing. Enforcement needs custody
narrowing (#232 part C), and a projection-layer filter with no floor beneath
it is theatre a raw-SQL reader walks past."
```

---

## Task 9: Convergence given equal custody

**Files:**
- Create: `crates/cairn-node/tests/sensitivity_convergence.rs`

- [ ] **Step 1: Write the test**

```rust
//! Two nodes, opposite arrival orders, same effective grade — GIVEN EQUAL CUSTODY.
//!
//! The custody qualifier is load-bearing and belongs in the name. §10b makes the effective
//! grade non-monotone in custody, so two honest nodes with DIFFERENT custody may
//! legitimately disagree. Stated loosely this test either fails spuriously or, far worse,
//! gets "fixed" by deleting the §10b bound — reopening the leak it exists to close.
//!
//! Needs CAIRN_TEST_PG2; without it the test self-skips and cargo counts it as passed.
mod common;
// … build the same three assertions on both nodes in opposite orders via the remote-apply
// door, then assert cairn_effective_sensitivity agrees on both.
```

Model the two-node fixture on the existing multi-node convergence suite (search
`CAIRN_TEST_PG2` in `crates/cairn-node/tests/`) rather than inventing a new harness.

- [ ] **Step 2: Run it**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test2" \
  cargo test -p cairn-node --test sensitivity_convergence -- --nocapture
```

Expected: PASS, and it must actually run — confirm it is not reported as `0 filtered out`.

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/sensitivity_convergence.rs
git commit -m "test(#232): convergence given equal custody

Max over assertions is a join-semilattice, so arrival order cannot matter. The
custody qualifier is in the test name because §10b makes the grade
non-monotone in custody — without it the test would eventually be 'fixed' by
deleting the bound."
```

---

## Task 10: ADR-0062, the spec, and the working docs

**Files:**
- Create: `docs/spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md`
- Modify: `docs/spec/decisions/README.md` (index row), `docs/spec/identity.md` (§5.9), `docs/spec/index.md` (v0.64), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0062**

Follow the template in `docs/spec/decisions/README.md`. Carry the six decisions from design
§2 plus the §10b corollary (the effective grade is node-relative — the ADR-0052 §9 pattern),
and record the rejected alternatives **in full**: unknown-ranks-0 (matching db/040), capping
chart-wide below `sequestered`, self-only withdrawal (the ADR-0043 shape), and a plaintext
thread reference on `event_log`.

- [ ] **Step 2: Update the spec**

Add the six decisions to §5.9's prose in `docs/spec/identity.md`, and bump
`**Spec version:** 0.63` → `0.64` in `docs/spec/index.md`. Add the ADR-0062 row to
`docs/spec/decisions/README.md`'s index.

- [ ] **Step 3: Build the docs**

```bash
uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -5
```

Expected: no new link warnings.

- [ ] **Step 4: Update HANDOVER and ROADMAP**

Add a ROADMAP slice entry; update HANDOVER's ⇒ NEXT to name parts B/C/D as the remaining
§5.9 work with their blockers (**C is blocked on #231**). **Keep both files under 500 lines**
(#368) — condense an older slice if needed.

- [ ] **Step 5: File the follow-on issues**

- **B** — safety-projection emission (carries #294).
- **C** — sequester / custody narrowing. **Blocked on #231**; say why in the issue body.
- **D** — break-glass.
- Sealed-rationale variant (a withdrawal rationale is clear text forever and replicates).
- Render the effective grade in the legibility twin (the #283 shape for `clock_grade`).
- Add the sensitivity gesture kinds to **#360**'s `ui_gesture_timing_kind_ck` widening.

- [ ] **Step 6: Full gate, then commit**

```bash
scripts/run-db-gated-tests.sh
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check
```

```bash
git add docs/
git commit -m "docs(#232): ADR-0062 — the sensitivity stream (spec v0.64)"
```

---

## Self-Review

**Spec coverage:** design §2 decisions 1–6 → Tasks 5, 1, 6, 3, 2, 6 respectively · §3 wire → Task 2 · §4 subjects/max → Task 5 · §5 ladder → Task 1 · §6 ceremony → Task 6 · §7 blacklist → Task 7 · §8 read surface → Task 8 · §9 data model → Task 4 · §10a subset leak → Task 1 · §10b bound → Task 5 · §10c recall → Task 5 · §11 paper-parity → no code (the benchmark is recorded in the design; #360 owns the measurement) · §12 tests → distributed · §13 files → all covered · §14 follow-ons → Task 10 Step 5.

**Placeholders:** two deliberate soft spots, both flagged inline rather than hidden — Task 9's two-node fixture body (points at the existing `CAIRN_TEST_PG2` harness to copy) and Task 8's orchestrator body (points at `patient/register.rs`, whose HLC-tick discipline it must match). Every SQL object, every event body, and every assertion is written out.

**Type consistency:** `SubjectKind`/`as_str` (Task 2) is used unchanged in Tasks 3, 5, 6, 8 · `cairn_sensitivity_rank` (Task 1) is used by Tasks 5 and 7 · `cairn_effective_sensitivity`'s three-column shape (Task 5) matches every caller · `withdraws_hex` is hex on the wire and `BYTEA` in the table, decoded once in `sensitivity_withdrawal_apply`.
