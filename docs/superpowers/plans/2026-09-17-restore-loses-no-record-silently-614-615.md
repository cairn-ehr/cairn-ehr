# A restore loses no record silently (#614, #615) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two states in which `cairn-node restore` still exits **0** having left a record
behind — a node event silently discarded by a substitution (#615) and a clinical record admitted
*deferred* and reported nowhere (#614).

**Architecture:** One new pure PL/pgSQL helper, `cairn_refuse_substitution`, in a new
`db/053_substitution_guard.sql`, called by all three write doors — `submit_event` (db/005),
`apply_remote_event` (db/020) and, for the first time, `restore_node_event` (db/009). It compares with
`IS DISTINCT FROM`, so #608's fail-open is fixed in the one place it now lives instead of being copied
a third time. Separately, `ClinicalRestoreReport` gains a `deferred` count filled by one aggregate
query, printed by a pure notice function, with the exit code deliberately unchanged.

**Tech Stack:** Rust 1.96.0 (pinned in `rust-toolchain.toml`), PostgreSQL ≥ 18, PL/pgSQL, `tokio-postgres`.

**Spec:** [`docs/superpowers/specs/2026-09-17-restore-loses-no-record-silently-614-615-design.md`](../specs/2026-09-17-restore-loses-no-record-silently-614-615-design.md) — read it first; this plan argues from it and does not restate its reasoning.

## Global Constraints

- **AGPL-3.0.** No new dependency is added by this plan. If one becomes tempting, it is a blocker, not a cleanup item.
- **TDD.** Every task writes the failing test first and *runs it to see it fail for the stated reason*. A test that fails for a different reason than predicted has not been seen to fail.
- **`SCHEMA_GENERATION` 52 → 53** — `crates/cairn-event/src/schema_generation.rs:45`. `crates/cairn-event/tests/schema_generation.rs` reads `db/` at test time and fails if the constant is not the newest prefix, so the bump and the file land in one commit.
- **Two loader lists, both mandatory here.** `crates/cairn-node/src/db.rs` (`SCHEMA`, the FULL list — `full_schema_list_carries_the_repo_generation` forces db/053 into it) **and** `crates/cairn-sync/src/main.rs:66` (`SCHEMA`, a deliberate SUBSET that legitimately lags). The subset carries db/005 and db/020, which will call the helper, so it **must** carry db/053. PL/pgSQL binds function names at *execution*, so omitting it loads cleanly and fails at the first write — a total write outage (#198, review finding B3). `schema_subset_tests` at the bottom of `cairn-sync/src/main.rs` is the guard that catches it.
- **Message text is a contract.** The two existing refusals must come out of the helper byte-identical: `submit_event: event_id <uuid> already exists with different content (substitution refused)` and the same with `apply_remote_event`.
- **Reviewer-legibility (§9).** All three doors are safety-critical surface. Comments explain *why*, for a junior developer, per house rule 3.
- **Never hard-code cryptographic material in tests, and never give a non-cryptographic value a cryptographic NAME.** Derive at runtime; call a discriminator a `lineage`/`variant`/`seed`, never a `salt`/`nonce`/`iv` (house rule 6; #146, #527).
- **Run tests with `scripts/run-db-gated-tests.sh`** for the DB tier. A plain `cargo test` without `CAIRN_TEST_PG*` fails `db_gate_actually_ran` unless `CAIRN_ALLOW_DB_SKIP=1` is exported (#450). Never pipe cargo through `| tail` — it masks the exit code.
- **A live IDE contends for `target/`.** If a narrow `cargo test` blocks before compiling, use `CARGO_TARGET_DIR=/tmp/cairn-614` rather than killing the IDE.

---

### Task 1: The shared refusal helper (`db/053`), wired into both loader lists

**Files:**
- Create: `db/053_substitution_guard.sql`
- Create: `crates/cairn-node/tests/substitution_guard.rs`
- Modify: `crates/cairn-event/src/schema_generation.rs:41-45`
- Modify: `crates/cairn-node/src/db.rs` (append to `SCHEMA`, after the `052_restore_doors` entry ending line ~335)
- Modify: `crates/cairn-sync/src/main.rs` (append to `SCHEMA`, after the `052_restore_doors` entry ending line ~223)

**Interfaces:**
- Produces: `cairn_refuse_substitution(p_found_ca BYTEA, p_new_ca BYTEA, p_event_id UUID, p_door TEXT) RETURNS VOID` — raises when `p_found_ca IS DISTINCT FROM p_new_ca`, message `'<door>: event_id <uuid> already exists with different content (substitution refused)'`. Tasks 2 and 3 call it.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/substitution_guard.rs`:

```rust
//! #615 / #608 — the ONE refusal all three write doors share.
//!
//! Before this file the guard was written twice, inline, in `db/005_submit.sql` and
//! `db/020_apply_remote_event.sql`, and not at all in `db/009_node_supersede_and_restore.sql`.
//! Both copies compared with `<>`, which yields NULL — and therefore does NOT fire — when the
//! sub-select feeding it returns no row (#608). Writing a third copy of that into the restore
//! door was the obvious way to fix #615 and would have put a known fail-open into the floor a
//! third time.
//!
//! What the helper is: a PURE raiser. It reads no table; both content-addresses arrive as
//! arguments. That is what lets one function serve `event_log` (db/005, db/020) and `node_event`
//! (db/009) without knowing about either.

use cairn_node::db;

mod common;
#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::cs;

/// A deterministic 32-byte content-address stand-in. Derived at runtime, never a literal, and
/// named `lineage` rather than `seed`/`salt` — house rule 6(b): the parameter a value flows into
/// is what CodeQL picks its sink by, and nothing here is cryptographic.
fn address(lineage: u8) -> Vec<u8> {
    (0..32u8).map(|i| i.wrapping_mul(7).wrapping_add(lineage)).collect()
}

/// Run the helper and give back the refusal message, or `None` if it allowed the write.
async fn refuse(c: &tokio_postgres::Client, found: Option<Vec<u8>>, new: Vec<u8>) -> Option<String> {
    let id = uuid::Uuid::now_v7();
    c.execute(
        "SELECT cairn_refuse_substitution($1, $2, $3, 'test_door')",
        &[&found, &new, &id],
    )
    .await
    .err()
    .map(|e| e.as_db_error().map(|d| d.message().to_string()).unwrap_or_default())
}

#[tokio::test]
async fn the_same_content_address_is_not_a_substitution() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    assert_eq!(
        refuse(&c, Some(address(1)), address(1)).await,
        None,
        "an idempotent re-write of the SAME event must stay a silent no-op (set-union)"
    );
}

#[tokio::test]
async fn a_different_content_address_is_refused_and_names_its_door() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let msg = refuse(&c, Some(address(1)), address(2))
        .await
        .expect("two different bodies under one event_id must be refused");
    assert!(
        msg.starts_with("test_door: event_id ")
            && msg.ends_with("already exists with different content (substitution refused)"),
        "the refusal must name the DOOR that raised it, so an operator reading a log knows \
         which write path refused; got: {msg}"
    );
}

/// The arm `<>` gets wrong, and the whole reason this helper exists rather than a third copy.
///
/// A caller reaches the guard only when its INSERT was a no-op — i.e. when a row with that id
/// exists. If the read-back nonetheless finds nothing, the honest answer is "this floor cannot
/// tell whether it is about to lose a record", and on the §9 surface that is a refusal, not a
/// pass. `<>` would yield NULL here and let the write through.
#[tokio::test]
async fn an_absent_row_fails_closed_rather_than_passing_silently() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    assert!(
        refuse(&c, None, address(1)).await.is_some(),
        "a NULL found-address means the guard could not establish what is stored: refuse. \
         This is the #608 fail-open, and it must not be reachable through the helper."
    );
}
```

- [ ] **Step 2: Run it to see it fail for the right reason**

```bash
scripts/run-db-gated-tests.sh 2>&1 | grep -A3 substitution_guard
```

Expected: all three FAIL with PostgreSQL `42883` — `function cairn_refuse_substitution(...) does not exist`. **If any test fails for another reason, stop and diagnose before writing SQL.**

- [ ] **Step 3: Write `db/053_substitution_guard.sql`**

```sql
-- Cairn — the one substitution refusal all three write doors share (#615, #608).
--
-- WHY THIS FILE EXISTS. A substitution is a SECOND, different event filed under an event_id
-- the log already holds. Every door inserts `ON CONFLICT (…) DO NOTHING`, because an
-- idempotent re-write of the SAME event must stay a silent no-op — that is set-union, and it
-- is what makes sync safe (principle 1). But the identical no-op is what a substitution looks
-- like, so without a comparison the two are indistinguishable and the rival is DISCARDED in
-- silence: two nodes then hold different bytes under one event_id, forever, with no alarm.
--
-- Two of the three doors already refused it, each with its own inline copy of the same four
-- lines. The third — `restore_node_event` (db/009) — did not, which is #615: an attacker who
-- can append to a sneakernet medium reuses the event_id of the clinic's `peer.revoked`, the
-- genuine revocation is dropped, and the node comes back TRUSTING A PEER THE CLINIC REVOKED,
-- at exit 0. That door is self-trusting by design (db/009's own comment at :68-77 says the
-- medium "can contain OTHER signers' events and is attacker-appendable"), so it is the door
-- where the guard matters most and the one that had none.
--
-- WHY A HELPER RATHER THAN A THIRD COPY. Both existing copies compare with `<>`. The branch is
-- reached only when a row with that id exists, so the read-back should always find one — but
-- if it ever does not, `<>` yields NULL, the IF does not fire, and THE GUARD PASSES SILENTLY
-- (#608). Copying that into the floor a third time is not a defensible way to fix a door, and
-- three doors spelling one invariant two ways is exactly the drift #159 needed a byte-identical
-- source guard to catch. One function, compared with IS DISTINCT FROM, fixes it once.
--
-- WHY IT IS PURE, AND WHY THAT MATTERS. It reads no table: both content-addresses arrive as
-- arguments. That is what lets the same function serve `event_log` (db/005, db/020) and
-- `node_event` (db/009) without knowing about either, and it is why each door keeps its own
-- read — db/005 and db/020 are on the 100k-event clinical path and read only when their INSERT
-- was a no-op, while db/009 (tens of node events per medium) reads unconditionally and is
-- thereby robust to a later edit disarming a ROW_COUNT it no longer sets.
--
-- ⚠️ NO `REVOKE EXECUTE … FROM PUBLIC`, AND THAT IS DELIBERATE — NOT AN OVERSIGHT OF #382.
-- The convention `floor_execute_grants.rs` checks covers four families: the per-event-type
-- structural validators, the registered projection appliers, and two registry triggers. This
-- belongs to none of them. It reads nothing, writes nothing and grants nothing, so a PUBLIC
-- caller invoking it learns strictly less than the door already tells it by refusing — the
-- same reasoning that leaves `cairn_decode_hex_or_raise` (db/001), its closest sibling,
-- unrevoked. #382's point is that a missing REVOKE a reader cannot classify as deliberate is
-- worse than either extreme; this paragraph is that classification. If this function ever
-- starts reading a table, revisit it.

BEGIN;

CREATE OR REPLACE FUNCTION cairn_refuse_substitution(
    p_found_ca  BYTEA,
    p_new_ca    BYTEA,
    p_event_id  UUID,
    p_door      TEXT
) RETURNS VOID
LANGUAGE plpgsql
IMMUTABLE
SET search_path = public, pg_temp
AS $$
BEGIN
    -- IS DISTINCT FROM, never `<>`: a NULL p_found_ca means the caller could not establish
    -- what is stored under this id, and on the safety-critical floor that is a refusal.
    IF p_found_ca IS DISTINCT FROM p_new_ca THEN
        -- The door name is interpolated so this reproduces both pre-existing messages
        -- byte-for-byte; `substitution_guard_is_single_source.rs` pins that nobody
        -- reintroduces an inline copy, and the door tests pin the text itself.
        RAISE EXCEPTION '%: event_id % already exists with different content (substitution refused)',
            p_door, p_event_id;
    END IF;
END;
$$;

COMMENT ON FUNCTION cairn_refuse_substitution(BYTEA, BYTEA, UUID, TEXT) IS
    'Refuse a second, different event filed under an event_id the log already holds. Called by '
    'submit_event (db/005), apply_remote_event (db/020) and restore_node_event (db/009). Pure: '
    'reads no table.';

COMMIT;
```

- [ ] **Step 4: Bump the generation and append to BOTH loader lists**

`crates/cairn-event/src/schema_generation.rs` — update the doc line and the constant:

```rust
/// The numeric prefix of the newest migration in `db/`
/// (`db/053_substitution_guard.sql` → 53).
///
/// Bump this in the same commit that adds a `db/*.sql` file; the guard test enforces it.
pub const SCHEMA_GENERATION: i32 = 53;
```

`crates/cairn-node/src/db.rs` — append to `SCHEMA` after the `052_restore_doors` entry:

```rust
    // db/053 (#615/#608): cairn_refuse_substitution — the ONE substitution refusal all three
    // write doors share. In BOTH lists: db/005 and db/020 CALL it, and PL/pgSQL binds function
    // names at execution, so a loader that omits this file loads cleanly and fails at the first
    // write.
    (
        "053_substitution_guard",
        include_str!("../../../db/053_substitution_guard.sql"),
    ),
```

`crates/cairn-sync/src/main.rs` — append the identical entry to its `SCHEMA` subset. The subset
legitimately lags `db/`'s newest file for node-only and medication migrations (#284), but **not for
this one**: it carries both doors that call the helper.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
scripts/run-db-gated-tests.sh
```

Expected: the three new tests PASS; `full_schema_list_carries_the_repo_generation`,
`crates/cairn-event/tests/schema_generation.rs` and `schema_subset_tests` all PASS.

- [ ] **Step 6: Commit**

```bash
git add db/053_substitution_guard.sql crates/cairn-node/tests/substitution_guard.rs \
        crates/cairn-event/src/schema_generation.rs crates/cairn-node/src/db.rs \
        crates/cairn-sync/src/main.rs
git commit -m "feat(#615,#608): one substitution refusal, compared with IS DISTINCT FROM

Both existing guards compare with <>, which yields NULL — and so does NOT
fire — if the read-back feeding it finds no row (#608). Porting that into
db/009 to fix #615 would have put a known fail-open into the floor a third
time. One pure helper instead, in db/053, taking both addresses as arguments
so it serves event_log and node_event alike.

SCHEMA_GENERATION 52 -> 53, and db/053 lands in BOTH loader lists: cairn-sync's
subset legitimately lags db/ for node-only migrations, but it carries db/005
and db/020, which call this. PL/pgSQL binds at execution, so omitting it would
load cleanly and fail at the first write.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: db/005 and db/020 call the helper, and a source guard keeps it single-source

**Files:**
- Create: `crates/cairn-node/tests/substitution_guard_is_single_source.rs`
- Modify: `db/005_submit.sql:1505-1514`
- Modify: `db/020_apply_remote_event.sql:468-483`

**Interfaces:**
- Consumes: `cairn_refuse_substitution(...)` from Task 1.
- Produces: nothing new. This task's deliverable is that **no behaviour changed** and that the inline copies are gone.

> **Why the test here is a source guard, not a behaviour test.** This task is a refactor: the doors
> must refuse exactly what they refused before, with exactly the text they used before. A behaviour
> test would be green before the change and green after, proving nothing. What *is* newly true is the
> single-source property — and that is what can silently regress, the same shape as
> `twin_dispatch_single_source.rs` (ADR-0048) and `name_winner_order_drift.rs` (#159).

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/substitution_guard_is_single_source.rs`:

```rust
//! #615/#608 — the substitution refusal is raised in exactly ONE place.
//!
//! No database: this reads `db/*.sql` as text.
//!
//! The defect this prevents is not hypothetical — it is the state the repo was in until #615.
//! db/005 and db/020 each carried their own copy of the same four lines, and when db/009 needed
//! the guard, the obvious move was a third copy. Both existing copies compared with `<>`, which
//! fails open on a NULL (#608), so the third copy would have inherited the bug — and fixing the
//! bug afterwards would have meant finding all three.
//!
//! A door may READ its own stored content-address however it likes (db/005 and db/020 do it
//! under a ROW_COUNT check because they are on the clinical hot path; db/009 reads
//! unconditionally). What no door may do is decide the question itself.

use std::fs;
use std::path::Path;

/// The refusal sentence, which must appear in `db/053` and nowhere else.
const SENTENCE: &str = "already exists with different content (substitution refused)";

/// The only file allowed to contain it.
const HOME: &str = "053_substitution_guard.sql";

#[test]
fn only_db_053_raises_the_substitution_refusal() {
    // CARGO_MANIFEST_DIR is crates/cairn-node; db/ is two levels up.
    let db_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db");
    let mut offenders = Vec::new();
    let mut found_home = false;

    for entry in fs::read_dir(&db_dir).expect("db/ is readable") {
        let path = entry.expect("a readable dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".sql") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("a readable migration");
        if !text.contains(SENTENCE) {
            continue;
        }
        if name == HOME {
            found_home = true;
        } else {
            offenders.push(name.to_string());
        }
    }

    assert!(
        found_home,
        "db/{HOME} must contain the refusal sentence — if it was renamed, update HOME here \
         rather than deleting this guard"
    );
    assert!(
        offenders.is_empty(),
        "the substitution refusal must be raised ONLY by cairn_refuse_substitution in \
         db/{HOME}. A door may read its own stored content-address however it likes, but it \
         must not decide the question itself — an inline copy is how #608's `<>` fail-open \
         came to exist in two places at once. Offenders: {offenders:?}"
    );
}
```

- [ ] **Step 2: Run it to see it fail**

```bash
cargo test -p cairn-node --test substitution_guard_is_single_source
```

Expected: FAIL, listing `["005_submit.sql", "020_apply_remote_event.sql"]` as offenders.

- [ ] **Step 3: Before editing — verify the `PERFORM` is safe in both doors**

`PERFORM` overwrites `FOUND` **and** `ROW_COUNT`. Both doors already capture the INSERT outcome
into a local first (db/020's comment says why in as many words), so the swap is safe *provided
nothing below the guard reads `ROW_COUNT` or `FOUND` expecting the INSERT's value*. Check it, do not
assume it:

```bash
sed -n '1505,1620p' db/005_submit.sql   | grep -n "GET DIAGNOSTICS\|FOUND"
sed -n '468,620p'   db/020_apply_remote_event.sql | grep -n "GET DIAGNOSTICS\|FOUND"
```

If either reads `FOUND`/`ROW_COUNT` below the guard without re-capturing, re-capture it into a local
immediately after the `PERFORM`. **Do not respond by dropping the helper and keeping the inline copy**
— that is the outcome this slice exists to end.

- [ ] **Step 4: Replace db/005's inline comparison**

`db/005_submit.sql`, replacing lines 1510-1514 (the `IF v_log_rows = 0` block), keeping the existing
comment above it:

```sql
    -- Idempotent re-submit of the SAME event is a silent no-op (set-union).
    -- But a DIFFERENT event reusing this event_id (substitution) must not pass
    -- silently: compare the stored content-address to what we just verified.
    --
    -- The comparison itself lives in cairn_refuse_substitution (db/053) since #615, shared with
    -- apply_remote_event and restore_node_event. The read stays HERE, under the ROW_COUNT check,
    -- because this door is on the 100k-event clinical path and must not pay a SELECT per event.
    IF v_log_rows = 0 THEN
        PERFORM cairn_refuse_substitution(
            (SELECT content_address FROM event_log WHERE event_id = v_event_id),
            v_ca, v_event_id, 'submit_event');
    END IF;
```

- [ ] **Step 5: Replace db/020's inline comparison**

`db/020_apply_remote_event.sql`, replacing the `IF v_rows = 0` block at 478-483. **The guard's
POSITION does not move** — it stays above the `cairn.remote_apply` marker clear and above the
`cairn_project_late_custody` call, so a rival body never reaches an applier (ADR-0070 decision 1,
trap 10; pinned by `late_custody_reaches_the_chart.rs::a_rival_body_never_reaches_an_applier`). Keep
every existing comment in place and change only the four lines that compare:

```sql
    IF v_rows = 0 THEN
        PERFORM cairn_refuse_substitution(
            (SELECT content_address FROM event_log WHERE event_id = v_event_id),
            v_ca, v_event_id, 'apply_remote_event');
    END IF;
```

- [ ] **Step 6: Run the source guard and every existing door test**

```bash
cargo test -p cairn-node --test substitution_guard_is_single_source
scripts/run-db-gated-tests.sh
```

Expected: the source guard PASSES. Every pre-existing test asserting the two refusal messages
passes **unchanged** — in particular `restore_one_event_id_one_body.rs` (whose case 2 pens a rival
clinical body with the door's reason) and `late_custody_reaches_the_chart.rs`. A changed expected
string anywhere is a bug in this task, not in the test: the message must be byte-identical.

- [ ] **Step 7: Commit**

```bash
git add db/005_submit.sql db/020_apply_remote_event.sql \
        crates/cairn-node/tests/substitution_guard_is_single_source.rs
git commit -m "refactor(#608): both existing doors raise through the shared guard

No behaviour change and no message change — the door name interpolates to the
byte-identical text both doors already raised. What IS new is that the
comparison exists once, so #608's <> can no longer be fixed in one copy and
left in another.

Each door keeps its own read under its own ROW_COUNT check: db/005 and db/020
are on the 100k-event clinical path and must not pay a SELECT per event.
db/020's guard POSITION is unchanged and load-bearing (ADR-0070 decision 1).

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: The restore door gets the guard (#615)

**Files:**
- Create: `crates/cairn-node/tests/restore_one_node_event_id_one_body.rs`
- Modify: `db/009_node_supersede_and_restore.sql` (declare `v_found`; add the guard after the `IF/ELSE` that ends at ~line 139)

**Interfaces:**
- Consumes: `cairn_refuse_substitution(...)` from Task 1.
- Produces: `restore_node_event` now raises `restore_node_event: event_id <uuid> already exists with different content (substitution refused)`. `apply_medium` propagates it with `?`, so the whole restore aborts.

> The clinical plane already has this test — `restore_one_event_id_one_body.rs`, case 2. The node
> plane has none, and that asymmetry **is** #615. The new file is deliberately named to sit beside it.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/restore_one_node_event_id_one_body.rs`:

```rust
//! #615 — one node_event_id, one body, through the RESTORE door.
//!
//! The clinical-plane sibling of this question is `restore_one_event_id_one_body.rs` case 2,
//! which has passed since slice 2d. The node plane had no such test because it had no such
//! guard: `db/009`'s two `INSERT … ON CONFLICT (node_event_id) DO NOTHING` sites carried no
//! comparison at all, so a second, different event under an id already present was DISCARDED
//! IN SILENCE and the restore exited 0.
//!
//! Why that is a security defect and not a tidiness one: the node plane IS the trust set. The
//! restore door is self-trusting by design — any validly-signed `node.enrolled` is admitted
//! without a trust check, because a fresh node has no trust set to check against — and db/009's
//! own comment at :68-77 already argues that "the medium can contain OTHER signers' events and
//! is attacker-appendable" (it is why the HLC drift ceiling is there). So the attack is cheap:
//! append an event carrying the event_id of the clinic's `peer.revoked`, earlier in file order.
//! The genuine revocation is dropped. The node comes back TRUSTING A PEER THE CLINIC REVOKED,
//! and the summary says `restored N event(s)`.
//!
//! The count cannot catch it: `apply_medium` returns `events.len()` — what was OFFERED, not what
//! landed — and its own doc says so. The guard is the load-bearing half.

use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_node::{db, identity};

mod common;
#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::cs;

/// A signed `node.enrolled` from `sk`. The node's own genesis restores first, which is how a
/// later event's author key resolves.
fn synth_enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    sign(
        &EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: identity::NIL_PATIENT.into(),
            event_type: "node.enrolled".into(),
            schema_version: "node/1".into(),
            hlc: Hlc { wall: 1, counter: 0, node_origin: name.into() },
            t_effective: None,
            signer_key_id: hex::encode(sk.verifying_key().to_bytes()),
            contributors: serde_json::json!([]),
            payload: serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
            attachments: vec![],
            plaintext_twin: None,
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
            safety: None,
        },
        sk,
    )
    .unwrap()
    .signed_bytes
}

/// A signed peer event under a CALLER-CHOSEN `event_id`, so two rivals can share one.
/// `peer_hex` is what makes the two bodies differ.
fn synth_peer_with_id(
    sk: &SigningKey,
    name: &str,
    event_id: uuid::Uuid,
    event_type: &str,
    peer_hex: &str,
) -> Vec<u8> {
    sign(
        &EventBody {
            event_id: event_id.to_string(),
            patient_id: identity::NIL_PATIENT.into(),
            event_type: event_type.into(),
            schema_version: "node/1".into(),
            hlc: Hlc { wall: 2, counter: 0, node_origin: name.into() },
            t_effective: None,
            signer_key_id: hex::encode(sk.verifying_key().to_bytes()),
            contributors: serde_json::json!([]),
            payload: serde_json::json!({ "peer_node_id_hex": peer_hex, "role": "peer" }),
            attachments: vec![],
            plaintext_twin: None,
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
            safety: None,
        },
        sk,
    )
    .unwrap()
    .signed_bytes
}

/// A 32-byte node id as lowercase hex. Derived, never a literal; `lineage` is a discriminator
/// and deliberately not called a seed/salt (house rule 6b).
fn node_id_hex(lineage: u8) -> String {
    hex::encode((0..32u8).map(|i| i.wrapping_mul(11).wrapping_add(lineage)).collect::<Vec<u8>>())
}

/// THE ATTACK: a rival node event under an id the log already holds is refused by name.
#[tokio::test]
async fn a_rival_node_event_under_one_id_is_refused_not_discarded() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.ok();

    let (sk, _kid) = cairn_event::generate_key().unwrap();
    c.execute("SELECT restore_node_event($1)", &[&synth_enroll(&sk, "Restored")])
        .await
        .expect("the medium's own genesis restores first, so its key resolves");

    // The clinic's genuine revocation of a peer it no longer trusts.
    let contested = uuid::Uuid::now_v7();
    let genuine =
        synth_peer_with_id(&sk, "Restored", contested, "peer.revoked", &node_id_hex(1));
    // The attacker's event, reusing that id with different content. On a real medium it is
    // positioned EARLIER in file order, so it lands first and the revocation becomes the rival;
    // here the order is irrelevant — either way one of the two must not vanish in silence.
    let rival = synth_peer_with_id(&sk, "Restored", contested, "peer.added", &node_id_hex(2));

    c.execute("SELECT restore_node_event($1)", &[&rival])
        .await
        .expect("the first event under a fresh id applies normally");

    let err = c
        .execute("SELECT restore_node_event($1)", &[&genuine])
        .await
        .expect_err(
            "a SECOND, different node event under one event_id must be refused. Discarding it \
             silently is #615: the clinic's peer.revoked is dropped and the node comes back \
             trusting a revoked peer, at exit 0",
        );
    let msg = err.as_db_error().map(|e| e.message().to_string()).unwrap_or_default();
    assert!(
        msg.contains("restore_node_event")
            && msg.contains("already exists with different content (substitution refused)"),
        "the restore door's refusal must name ITSELF, so an operator reading a failed restore \
         knows which door refused and that it was a substitution; got: {msg}"
    );
}

/// The guard must refuse a RIVAL, never a REPEAT.
///
/// `apply_medium`'s doc promises re-applying the same medium is a no-op, and that promise is
/// load-bearing on the resume path — a restore interrupted halfway is restarted over the same
/// file. A guard that refused an identical re-offer would make a resumable ceremony unresumable.
#[tokio::test]
async fn re_restoring_the_identical_event_is_still_a_silent_no_op() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.ok();

    let (sk, _kid) = cairn_event::generate_key().unwrap();
    let genesis = synth_enroll(&sk, "Restored");
    let peer = synth_peer_with_id(
        &sk, "Restored", uuid::Uuid::now_v7(), "peer.added", &node_id_hex(3),
    );

    for pass in 1..=2 {
        for ev in [&genesis, &peer] {
            c.execute("SELECT restore_node_event($1)", &[ev]).await.unwrap_or_else(|e| {
                panic!("pass {pass}: re-applying the SAME medium must stay a no-op, not raise: {e}")
            });
        }
    }
}
```

- [ ] **Step 2: Run it to see it fail for the right reason**

```bash
cargo test -p cairn-node --test restore_one_node_event_id_one_body
```

Expected: `a_rival_node_event_under_one_id_is_refused_not_discarded` FAILS at `expect_err` — the
second call returns `Ok`, which is #615 exactly. `re_restoring_the_identical_event_is_still_a_silent_no_op`
already PASSES and must keep passing.

- [ ] **Step 3: Add the guard to db/009**

Add `v_found BYTEA;` to the `DECLARE` block, then insert this immediately after the `END IF;` that
closes the enroll/non-enroll branch (~line 139), **before** the `cairn_node_hlc_merge` call:

```sql
    -- SUBSTITUTION REFUSAL (#615). Both branches above insert ON CONFLICT DO NOTHING, which is
    -- what makes a re-restore of the same medium a no-op — and is also what made a SECOND,
    -- DIFFERENT event under one node_event_id vanish without a word. The node plane is the
    -- TRUST SET: the silently-dropped event can be the clinic's own `peer.revoked`, and this
    -- door is self-trusting (see the drift-ceiling comment above: the medium is attacker-
    -- appendable). db/005 and db/020 have refused this since their first review; this door is
    -- the one that did not.
    --
    -- THREE THINGS ABOUT THE SHAPE, each chosen rather than inherited:
    --   * No GET DIAGNOSTICS. A ROW_COUNT check placed here would be correct only because the
    --     last statement of BOTH branches happens to be the INSERT. Someone later adding a
    --     statement inside either branch would disarm the guard SILENTLY — the exact failure
    --     db/020's own comment warns about. Reading the row back has no such coupling.
    --   * Fail-closed on an absent row. If the read finds nothing, v_found is NULL and
    --     cairn_refuse_substitution's IS DISTINCT FROM refuses. That state should be
    --     unreachable; on the §9 surface "should be unreachable" is not a reason to pass.
    --   * Once, not twice. Both branches write node_event under the same key, so one site
    --     covers both and there is no second copy to drift (#608's lesson, one file earlier).
    --
    -- COST: one extra SELECT per NODE-plane event. A medium carries tens of those (enrolls,
    -- peers, revokes, supersedes), not the 100 003 clinical records the §1.2 budget was measured
    -- against — which is why db/005 and db/020 keep their ROW_COUNT check and this door does not.
    --
    -- A REFUSAL ABORTS THE WHOLE RESTORE, because apply_medium propagates with `?`. That is not
    -- a new posture: this door already aborts on an unknown node event type, an over-ceiling
    -- event, an HLC wall past the drift ceiling, and an author key resolving to no restored
    -- enroll. A medium carrying two rival events under one id is a compromised or corrupt
    -- medium, and restoring a node whose peer list was decided by whoever appended last is a
    -- worse outcome than refusing and telling the operator to find another copy.
    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'restore_node_event');
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p cairn-node --test restore_one_node_event_id_one_body
scripts/run-db-gated-tests.sh
```

Expected: both new tests PASS. Every existing restore suite still passes — especially
`restore.rs`, `restore_ceremony_order.rs`, `restore_cli_surface.rs` and
`restore_needs_nothing_about_the_dead_node.rs`, which drive real media end to end. A failure there
means a genuine medium is tripping the guard, which would be a real defect in this task.

- [ ] **Step 5: Commit**

```bash
git add db/009_node_supersede_and_restore.sql \
        crates/cairn-node/tests/restore_one_node_event_id_one_body.rs
git commit -m "fix(#615): the restore door refuses a substitution, like the other two

db/009's two ON CONFLICT DO NOTHING sites carried no comparison, so a second,
different node event under an id already present was discarded in silence at
exit 0. The node plane is the trust set and this door is self-trusting, so the
dropped event can be the clinic's own peer.revoked — the restored node comes
back trusting a peer it had revoked, and the summary says 'restored N event(s)'.
The count cannot catch it: apply_medium returns what was OFFERED.

Guarded once after the IF/ELSE and deliberately WITHOUT GET DIAGNOSTICS: a
ROW_COUNT check there is correct only while the INSERT stays the last statement
of both branches, and would be disarmed silently by a later edit. One extra
SELECT per node-plane event — tens per medium, not the clinical path.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: A deferred record stops being silent (#614)

**Files:**
- Modify: `crates/cairn-node/src/restore/clinical.rs` (add `deferred` to `ClinicalRestoreReport` at :87-117; add `deferred_notice` and `deferred_count`; fill the field at the tail of `apply_clinical_plane`)
- Modify: `crates/cairn-node/src/main.rs:3580-3591` (print it inside the `counts.clinical > 0` block)
- Create: `crates/cairn-node/tests/restore_reports_deferred_records.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks. Independent of #615 and can be reviewed alone.
- Produces: `ClinicalRestoreReport.deferred: usize`; `pub fn deferred_notice(deferred: usize) -> Option<String>`; `async fn deferred_count(db: &Client) -> anyhow::Result<usize>`.

- [ ] **Step 1: Write the failing pure test**

Append to `crates/cairn-node/tests/restore_reports_deferred_records.rs` (new file):

```rust
//! #614 — a record this build cannot CLASSIFY is no longer silent.
//!
//! db/020 admits an event whose `event_type` is absent from `event_type_class` *uninterpreted*:
//! its own words are "It yields NO projection rows and confers NO power". It returns Ok, custody
//! is orthogonal to classification, so `apply_clinical_plane` counted it `applied` and the
//! summary said `N applied … (of N on the medium)` with every `Unrestored` field zero — exit 0.
//!
//! Per ADR-0012's additive schema evolution, a NEW CLINICAL EVENT TYPE is the case that actually
//! happens; a whole new plane (which ADR-0071 gives exit 3) is rare. The realistic DR box — a
//! spare laptop one release behind the live node — hits this one.
//!
//! ⚠️ THE VERDICT DELIBERATELY DOES NOT MOVE. The record IS in the log, which is exactly what
//! ADR-0071's exit-0 rule claims, and `connect_and_load_schema` re-adjudicates on upgrade, so no
//! second restore is needed and nothing is left on the medium. The defect was the SILENCE. This
//! is the decision `Unrestored`'s doc asks for ("a sixth cause gets a deliberate decision, not a
//! silent widening") and the answer is no — `the_cause_list_is_exactly_five` stays green.

use cairn_node::restore::clinical::deferred_notice;

#[test]
fn a_clean_restore_says_nothing_about_deferral() {
    assert_eq!(
        deferred_notice(0),
        None,
        "a restore with nothing deferred must print no line at all — an operator who reads a \
         deferral note on every clean run stops reading it"
    );
}

#[test]
fn the_notice_names_the_count_the_remedy_and_the_command() {
    let n = deferred_notice(7).expect("a nonzero deferral must be reported");
    assert!(n.contains('7'), "the operator needs the number, not just the fact: {n}");
    assert!(
        n.contains("cairn-node deferred"),
        "the notice must name the subcommand that LISTS them, or it tells an operator a \
         problem exists and not how to look at it: {n}"
    );
    assert!(
        n.contains("upgrade"),
        "the remedy is to upgrade this node, and it must be in the text — not inferrable: {n}"
    );
    assert!(
        n.contains("no second restore"),
        "the notice must say the medium holds nothing back, or an operator reading it will \
         reasonably re-run the whole ceremony looking for records that are already here: {n}"
    );
}
```

- [ ] **Step 2: Run it to see it fail**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test restore_reports_deferred_records
```

Expected: FAIL to compile — `deferred_notice` is not found in `cairn_node::restore::clinical`.

- [ ] **Step 3: Add the field, the pure notice and the count**

In `crates/cairn-node/src/restore/clinical.rs`, add to `ClinicalRestoreReport` (after
`skipped_no_registry`):

```rust
    /// Records this build admitted but cannot CLASSIFY — a newer Cairn's event type (#614).
    ///
    /// db/020 admits an unclassifiable type *uninterpreted*: it yields no projection rows and
    /// confers no power, and it returns `Ok`. Custody is orthogonal to classification, so
    /// nothing above distinguished it from a record that came fully back, and it was counted in
    /// [`applied`](ClinicalRestoreReport::applied) — which it genuinely is, in the log.
    ///
    /// ⚠️ **Deliberately NOT an [`Unrestored`](crate::restore::completeness::Unrestored) cause,
    /// so the exit code does not move.** The record IS in this node's log, which is precisely
    /// what ADR-0071's exit-0 rule claims, and `connect_and_load_schema` re-adjudicates deferred
    /// events, so an upgrade heals this with nothing left on the medium and no second restore.
    /// The defect #614 names is the SILENCE, and this field ends it.
    pub deferred: usize,
```

Then the two functions:

```rust
/// How many records this node holds that it cannot yet interpret.
///
/// **One aggregate query, not a probe per record.** A per-record probe would double the
/// round-trips on the path whose §1.2 budget was measured at 1.17 ms/event over 100 003 events;
/// this is O(1) and cannot move it.
///
/// **Why counting the whole table honestly answers a question about THIS medium.** `restore`
/// runs against a fresh, un-enrolled database *before* `finalize_identity` — the door fences
/// itself closed once a genesis exists — and nothing else writes in that window, so every
/// `event_deferred` row present came from this medium. That stays true on a RESUMED restore:
/// rows left by the earlier attempt are this same medium's. If a future caller runs this against
/// a database with other history, the count stops meaning what its caller says it means.
async fn deferred_count(db: &Client) -> anyhow::Result<usize> {
    let n: i64 = db
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .context("counting the records this build cannot classify")?
        .get(0);
    Ok(n as usize)
}

/// The operator's line about records that came back but cannot be read yet. **Pure.**
///
/// Pure so its wording is testable without a database — the same discipline as
/// [`Unrestored::notice`](crate::restore::completeness::Unrestored::notice), for the same reason:
/// this text is the entire difference between an operator who knows and one who does not.
///
/// It says four things, and each is load-bearing: the COUNT (a fact to check against the chart),
/// that the records ARE in the log (so the medium is not short), that the remedy is an UPGRADE
/// and not a second restore (or the operator re-runs the whole ceremony hunting records already
/// here), and the COMMAND that lists them.
pub fn deferred_notice(deferred: usize) -> Option<String> {
    if deferred == 0 {
        return None;
    }
    Some(format!(
        "  · {deferred} of them carry an event type this build cannot classify. They ARE in \
         the log and will project once this node is upgraded — no second restore is needed, \
         and nothing is left on the medium. List them with `cairn-node deferred`."
    ))
}
```

At the tail of `apply_clinical_plane`, immediately before `Ok(report)`:

```rust
    // Asked once, after the loop — see deferred_count's doc for why the whole table is the
    // honest answer here and why this is not a per-record probe.
    report.deferred = deferred_count(db).await?;

    Ok(report)
```

- [ ] **Step 4: Print it**

In `crates/cairn-node/src/main.rs`, inside the `if counts.clinical > 0 {` block at :3580, after the
`for (reason, n) in &clinical.refusals` loop:

```rust
                // #614 — a record admitted but unclassifiable is IN the log and counted above
                // as `applied`, which it is. Saying only that would leave an operator reading
                // "N applied" at exit 0 and finding those charts empty; the verdict is right
                // and the silence was not.
                if let Some(line) = cairn_node::restore::clinical::deferred_notice(clinical.deferred)
                {
                    println!("{line}");
                }
```

- [ ] **Step 5: Run the pure tests to verify they pass**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test restore_reports_deferred_records
```

Expected: both PASS.

- [ ] **Step 6: Add the DB-gated end-to-end arm**

Append to `crates/cairn-node/tests/restore_reports_deferred_records.rs`. It restores a medium
carrying an event of a type absent from `event_type_class` and asserts three things together —
the count, the line, and that the verdict did **not** move:

```rust
use cairn_node::restore::clinical::apply_clinical_plane;
use cairn_node::restore::completeness::Unrestored;
use cairn_node::db;

mod common;
#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::cs;

/// A medium carrying a newer Cairn's event type reports it, and still exits 0.
///
/// Both halves matter and they are asserted in one test on purpose: a test that only checked
/// the line would stay green if someone "fixed" #614 by adding a sixth `Unrestored` cause, and
/// a test that only checked the exit code would stay green if the line were dropped.
#[tokio::test]
async fn an_unclassifiable_type_is_reported_and_still_exits_zero() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Build a clinical record whose event_type is absent from event_type_class, capture it to a
    // medium and restore it. Follow restore_one_event_id_one_body.rs's fixture shape:
    // provisioned_clinic -> author -> capture -> wipe_to_a_fresh_dr_machine -> apply_clinical_plane.
    // The unclassifiable type is the ONLY thing that differs from that file's happy path.
    // (Implementer: reuse restore_kit::{provisioned_clinic, capture, wipe_to_a_fresh_dr_machine}
    //  and author the event with a type such as "clinical.from.a.newer.cairn"; assert with
    //  in_event_log that it really is admitted, not refused — a refused record would pen and
    //  make this test pass for the wrong reason.)

    let report = apply_clinical_plane(/* per the kit's signature */).await.unwrap();

    assert_eq!(report.deferred, 1, "the unclassifiable record must be counted as deferred");
    assert_eq!(report.penned(), 0, "it must be ADMITTED, not refused — otherwise this test \
                                    proves nothing about #614");
    assert!(
        deferred_notice(report.deferred).is_some(),
        "a deferred record must produce an operator line"
    );
    assert!(
        Unrestored::default().is_complete(),
        "the verdict does not move: a deferred record leaves nothing on the medium, so no \
         Unrestored cause holds and the run exits 0 (ADR-0071's rule is about the LOG)"
    );
}
```

> **Implementer note:** the fixture comment above is the one place this plan does not spell out
> every line, because `restore_kit`'s signatures are the authority and they change. Read
> `restore_one_event_id_one_body.rs`'s setup and mirror it; do not invent a second fixture path
> (#598 is the open issue about exactly that duplication).

- [ ] **Step 7: Run it**

```bash
scripts/run-db-gated-tests.sh
```

Expected: PASS, including `restore_exit_vocabulary.rs::the_cause_list_is_exactly_five` unchanged.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-node/src/restore/clinical.rs crates/cairn-node/src/main.rs \
        crates/cairn-node/tests/restore_reports_deferred_records.rs
git commit -m "fix(#614): a record this build cannot classify stops being silent

db/020 admits an unclassifiable event type uninterpreted and returns Ok, so
apply_clinical_plane counted it 'applied' and the summary said 'N applied (of N
on the medium)' with every Unrestored field zero — exit 0, total silence. Per
ADR-0012 this is the case that ACTUALLY happens; a whole new plane, which
ADR-0071 gives exit 3, is rare.

Reported, not re-verdicted: the record IS in the log, which is what ADR-0071's
exit-0 rule claims, and connect_and_load_schema re-adjudicates on upgrade, so
nothing is left on the medium. Unrestored's doc asks for a deliberate decision
rather than a silent widening before a sixth cause is added; this is it, and it
is no.

One aggregate query, not a probe per record — the §1.2 budget is 1.17 ms/event
over 100 003 events and must not double its round-trips.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: The published contract moves with the code, and ADR-0072 records why

**Files:**
- Modify: `crates/cairn-node/src/main.rs:1470-1492` (the `Restore` `EXIT STATUS` doc comment)
- Modify: `crates/cairn-node/tests/restore_cli_surface.rs` (assert the new wording against the SPAWNED help)
- Create: `docs/spec/decisions/0072-a-restore-loses-no-record-silently.md`
- Modify: `mkdocs.yml` (nav entry, **same commit**)
- Modify: `docs/spec/index.md` (spec version v0.73 → v0.74)
- Modify: `docs/spec/decisions/README.md` (ADR index)

**Interfaces:**
- Consumes: the behaviour built in Tasks 3 and 4.

> `--help` is part of the contract, and clap assembles it at runtime. PR #612 caught the same
> pattern three times: a fix written under the pressure of a finding is itself unreviewed code, and
> **round 2's fix wrote round 3's contradiction**. Limit (b) in that block currently *discloses* #614
> as a known limit of exit 0. After Task 4 it is *reported*, and leaving the old text standing would
> be the fourth instance.

- [ ] **Step 1: Write the failing test**

In `crates/cairn-node/tests/restore_cli_surface.rs`, add to the existing `--help` test (or add one
beside it, matching the file's style):

```rust
/// `--help`'s exit-0 contract tells the truth about deferred records (#614).
///
/// Asserted against the SPAWNED help, never the source text: clap assembles this at runtime
/// under `verbatim_doc_comment`, and a source-text assertion passes while the help a human reads
/// stays silent. The status is asserted too — `cmd --help || exit 1` is a normal pre-flight.
#[test]
fn the_help_says_a_deferred_record_is_reported_not_merely_admitted() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["restore", "--help"])
        .output()
        .expect("cairn-node restore --help runs");
    assert!(out.status.success(), "`restore --help` must exit 0");
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        help.contains("cairn-node deferred"),
        "exit 0's stated limit (b) must name the command that lists the records it is about, \
         now that the summary reports them; got:\n{help}"
    );
}
```

- [ ] **Step 2: Run it to see it fail**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test restore_cli_surface -- the_help_says
```

Expected: FAIL — the current text names `#614` but not the command.

- [ ] **Step 3: Rewrite limit (b)**

Replace the `(b)` clause in the `Restore` doc comment. **Every line is hand-wrapped to 80 columns**:
`verbatim_doc_comment` applies to the whole doc comment, so clap prints each line exactly as
written, here and in `cairn-node --help`'s subcommand list, and one over-long line wraps into a
two-word orphan that breaks the status table's alignment.

```
    ///   exit 0 = every record the medium carried reached this node's log, and
    ///            none of the five causes below holds. ONE KNOWN LIMIT, named
    ///            rather than implied: it is a claim about RECORDS, not
    ///            provisioning — a restore whose local-state export degraded
    ///            still exits 0 on a medium carrying no clinical records,
    ///            having installed no custody key (#613). A record this build
    ///            cannot CLASSIFY is in the log and counted, and projects into
    ///            no chart until this node is upgraded — that is NOT a limit of
    ///            exit 0 any more: the summary names how many, and
    ///            `cairn-node deferred` lists them (#614).
```

- [ ] **Step 4: Run it to verify it passes**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test restore_cli_surface
cargo run -p cairn-node -- restore --help | cat -A | grep -n '.\{81,\}'
```

Expected: the test PASSES, and the second command prints **nothing** (no line over 80 columns).

- [ ] **Step 5: Write ADR-0072**

`docs/spec/decisions/0072-a-restore-loses-no-record-silently.md`, following the house ADR shape
(Status / Context / Decision / Consequences). It must record, at minimum:

1. **The substitution refusal is one function, not three copies**, and *why* `IS DISTINCT FROM`:
   a third copy of a known fail-open is not a fix.
2. **The restore door refuses like the other two, and a refusal aborts the ceremony** — with the
   reasoning that db/009 already aborts on four lesser things, and that the node plane is the trust
   set.
3. **A deferred record is reported, not re-verdicted** — the explicit answer to `Unrestored`'s
   "a sixth cause gets a deliberate decision", and why exit 0 is *correct* here rather than tolerated.
4. **What it deliberately leaves open:** #608's `cairn_project_late_custody` half, #605, node-plane
   completeness accounting, #613.

> ⚠️ **An ADR is immutable once merged, so check every factual claim against `git show main:<file>`,
> not against memory of the diff.** PR #612 shipped a false sentence into ADR-0071's draft and caught
> it only in round 3. Give the ADR's facts a pass of their own.

- [ ] **Step 6: Nav, index and version — in this same commit**

The docs build runs `--strict`; a file absent from `mkdocs.yml`'s nav is a WARNING that **aborts**
it, and no local Rust gate sees this:

```bash
uv run --with-requirements docs/requirements.txt -- mkdocs build --strict
```

Expected: builds clean. Bump `docs/spec/index.md` to **v0.74** and add the ADR-index row.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/main.rs crates/cairn-node/tests/restore_cli_surface.rs \
        docs/spec/decisions/0072-a-restore-loses-no-record-silently.md \
        docs/spec/decisions/README.md docs/spec/index.md mkdocs.yml
git commit -m "docs(#614,#615): ADR-0072, and --help stops calling #614 a limit of exit 0

The EXIT STATUS block disclosed #614 as a known limit of exit 0. It is now
REPORTED, so leaving that text standing would be the fourth instance of the
pattern PR #612 caught three times: a fix written under the pressure of a
finding is itself unreviewed code, and --help is part of the contract.

Asserted against the SPAWNED help, never the source text.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Mutation run, full gate, and the tracking documents

**Files:**
- Create: `scripts/mutations/2026-09-17-614-615.sh` (throwaway harness, committed with the plan's ledger)
- Modify: this plan (fill the mutation table below with outcomes)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write the mutation table's EXPECTED outcomes before running anything**

Predicting a survivor *before* the run is what distinguishes a reasoned survivor from one
rationalised afterwards (#594's M9 lesson).

| # | Mutation | Expected | Actual | Killed by |
|---|----------|----------|--------|-----------|
| M1 | `IS DISTINCT FROM` → `<>` in db/053 | KILLED | **KILLED** | `substitution_guard::an_absent_row_fails_closed_rather_than_passing_silently` |
| M2 | Delete db/009's `PERFORM cairn_refuse_substitution` | KILLED | **KILLED** | `restore_one_node_event_id_one_body::a_rival_node_event_under_one_id_is_refused_not_discarded` |
| M3 | Delete db/005's `PERFORM` (leave db/020's) | KILLED | **KILLED** | `late_custody_reaches_the_chart` — **not** `seal_submit`, where the plan first guessed it |
| M4 | Delete db/020's `PERFORM` (leave db/005's) | KILLED | **KILLED** | `restore_one_event_id_one_body` case 2 |
| M5 | `report.deferred` hard-wired to `0` | KILLED | **KILLED** | `an_unclassifiable_type_is_reported_and_still_exits_zero` (reports `applied: 1, deferred: 0` — #614's exact pre-fix state) |
| M6 | Strip `cairn-node deferred` from the notice | KILLED | **KILLED** | `the_notice_names_the_count_the_remedy_and_the_command` (pure, no database) |
| M7 | Move db/009's guard **above** the `IF/ELSE` | KILLED | **KILLED** | `re_restoring_the_identical_medium_is_still_a_silent_no_op` — the idempotence arm, not the attack arm: `v_found` is always NULL there, so a CLEAN restore refuses |
| M8 | Add `deferred` to `Unrestored` | KILLED (compile) | **KILLED (E0063 × 3)** | `the_cause_list_is_exactly_five`'s full struct literal. **Recorded as saying nothing about runtime** — it is the guard working as designed, not evidence about the code under test (#594's M9 lesson) |

**Two harness defects, both found by the harness's own positive control on its first run** — and
the reason that control exists at all:

1. **The compiler-kill probe matched a bare leading `error:`** — which is also what cargo prints
   for an *ordinary runtime failure* (`error: test failed, to rerun pass …`). All five kills in
   the first run were misreported as compiler kills, i.e. as saying nothing about runtime. Now
   matched on `could not compile` or an error **code**.
2. **M6 swapped the remedy sentence to the EMPTY STRING**, so the revert's anchor was `''` —
   37 628 occurrences. **This is #594's exact defect**, the one that discarded a whole run there.
   Here the uniqueness check stopped the run instead of letting M7 execute on top of an unreverted
   M6. The tree was left dirty by the abort, inspected, and restored from the committed file.

- [ ] **Step 2: Build the harness with its own positive control**

The harness **must**: refuse to start on a dirty tree (`git diff --quiet`), swap whole blocks both
ways (never anchor on `""` — the empty string is not a unique anchor and a silent failed revert
makes every later mutation run on top of its predecessor), include a block's **leading comment** in
its anchor, and fail loudly if its own revert did not land. A first M2–M6 run was discarded in #594
for exactly these two defects.

- [ ] **Step 3: Run the mutations, record actual vs expected**

Any divergence from the table is a finding: either the test is weaker than believed or the code does
less than believed. Record it here either way.

- [ ] **Step 4: Full local gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
scripts/run-db-gated-tests.sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
uv run --with-requirements docs/requirements.txt -- mkdocs build --strict
scripts/codeql-alerts.sh
```

⚠️ A healthy full local gate takes **~2 hours** (macOS re-assesses each freshly-linked binary once
via Gatekeeper). Start it in the background and do the docs pass while it runs. `run-db-gated-tests.sh`
does **not** run `cargo doc`, and CI's `RUSTDOCFLAGS=-D warnings` makes an intra-doc link to a private
item fail two jobs — so run it explicitly. Never `| tail`.

Also run the three Cargo trees' lockfiles if any dependency moved (none should):
`cairn-gui/Cargo.lock` and `extensions/cairn_pgx/Cargo.lock` are not seen by any root gate, only by
CI's `--locked` clippy.

- [ ] **Step 5: Update HANDOVER and ROADMAP**

HANDOVER: rewrite ⇒ NEXT (#614/#615 are built; #594's issue is closed; what remains on the DR path is
the three open decisions, the two races, and the review wave). Add a **trap 12** for the durable rule:
*the substitution refusal has ONE home, and a door that needs it calls it — a fourth inline copy is
the #608 shape returning.* Add a *Recent sessions* entry with what generalises past the slice.
ROADMAP: log the slice and carry every open issue number forward; **never drop one while condensing**.
Keep both under 500 lines where it serves the reader, not the number.

- [ ] **Step 6: Commit and open the PR**

```bash
git add -A && git commit -m "docs(#614,#615): mutation ledger, HANDOVER trap 12, ROADMAP

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
git push -u origin feat/614-615-restore-loses-no-record-silently
gh pr create --title "A restore loses no record silently (#614, #615, ADR-0072)" --body "..."
```

The PR body links **#614 and #615** without a closing keyword adjacent to the reference
(`scripts/check_closing_keywords.py` guards this; `fix(#615):` in a commit subject is safe because
the parenthesis breaks the adjacency). Close both issues **by hand** after merge.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the disaster-recovery ceremony — a clinic restoring its record from the backup
it keeps off-site. On paper that is carrying the box of charts back from storage and putting them on
the shelf.

**Steps:** paper *N* = 1 human act. Architecture-forced *M* = 3 (insert medium · run `restore` ·
supply the recovery code) — **unchanged by this slice, which adds no human act on the success path.**
UI bundling target *K* = 3. `M > N` stands and is tracked as
[#512](https://github.com/cairn-ehr/cairn-ehr/issues/512); this slice neither widens nor narrows it.

**Time + cognitive load:** the measured budget is unchanged and is not re-run — 100 003 events in
**116.7 s** against **600 s**, linear at 1.17 ms/event
(`crates/cairn-node/results/2026-09-10-macos-m3max.md`). #614's count is one aggregate query, O(1).
#615's read is one `SELECT` per **node**-plane event, of which a medium carries tens; neither is on the
per-clinical-record path. Cognitive load **falls**: an operator who previously had to already know that
deferred records exist in order to go looking for them is now told, with the command that lists them.

On paper, a chart that came back from storage in a filing system nobody in the building can read is
not a chart that came back, and a box that came back one folder short is not a box that came back.
Saying both on the restore's own last screen restores the paper affordance — the visibly short box —
that a silent exit 0 removed.

---

## Self-review

**Spec coverage.** §2.1 → Task 1. §2.2 → Task 2. §2.3 → Task 3. §2.4 → Task 4. §2.5 → Task 5. §3
(what is not built) → recorded in Task 5's ADR item 4. §4 tests 1–4 → Tasks 1–3; tests 5–6 → Task 4;
test 7 → Task 5; mutations → Task 6. §5 risks: `PERFORM` clobbering → Task 2 step 3; the two loader
lists → Task 1 step 4 and Global Constraints; the mkdocs nav → Task 5 step 6. No gaps.

**Placeholders.** One deliberate and flagged: Task 4 step 6's fixture setup names the kit helpers to
reuse rather than transcribing signatures that change, with the reason given inline and #598 cited.
Every other step carries its actual content.

**Type consistency.** `deferred_notice(usize) -> Option<String>` and `ClinicalRestoreReport.deferred:
usize` are used identically in Tasks 4 and 5. `cairn_refuse_substitution(BYTEA, BYTEA, UUID, TEXT)`
has the same four-argument order at all four call sites (test, db/005, db/020, db/009).
