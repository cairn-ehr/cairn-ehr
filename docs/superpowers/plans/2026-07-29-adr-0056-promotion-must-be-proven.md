# Promotion Must Be Proven — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the three defects found reviewing PR #302 — a promoted deferred event must have proven it can project, and an unverified carried attestation token must never count as a vouch.

**Architecture:** Both safety defects are one mistake: a state inferred from a proxy with the wrong lifetime. `event_deferred` was answering both *"has this been adjudicated?"* and *"is this token vouched?"*, and only has the right lifetime for the first — promotion deletes it. Fix: a second marker (`event_attestation_unvouched`) with the correct lifetime, read by the three readers of `event_log.attester_key`; plus two new gates in `cairn_readjudicate_deferred` — gate 0 re-runs the per-type structural floor db/020 step 8 skipped, gate 4 runs the type's heal-safe apply fns inside the promotion subtransaction so the marker is deleted only for an event that has *already projected cleanly*.

**Tech Stack:** PostgreSQL 18 (PL/pgSQL + SQL migrations under `db/`), Rust (tokio-postgres in `cairn-node`, blocking `postgres` in `cairn-sync`).

**Spec:** [docs/superpowers/specs/2026-07-29-adr-0056-promotion-must-be-proven-design.md](../specs/2026-07-29-adr-0056-promotion-must-be-proven-design.md)

**Branch:** `feat/adr-0056-admit-uninterpreted-floor-265-266` (PR #302, pre-merge). Do not create a new branch.

## Global Constraints

- **Licence:** AGPL-3.0. No new dependencies in this plan; if one becomes necessary, its licence must be AGPL-3.0-compatible and checked *before* adding.
- **TDD is mandatory.** Every task writes the failing test first and **verifies the failure** before writing the fix. A step that says "run it to verify it fails" is not optional bookkeeping — if it passes, the test does not discriminate and must be fixed before proceeding.
- **`SCHEMA_GENERATION` stays 43.** No new `db/*.sql` file is added. `crates/cairn-event/src/schema_generation.rs` is not touched. A guard test enforces constant == newest `db/*.sql`; adding a file would break it.
- **Never hard-code cryptographic material in tests** (house rule 6 / issue #146). Keys come from `generate_key()`; any fixture bytes are computed at runtime (e.g. `(0u8..64).map(|i| i.wrapping_mul(7)).collect()`), never written as literals.
- **Comment for a junior developer** (house rule 3). Every non-trivial block explains *why* it exists and how it fits, not what the next line does.
- **Test DB:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test"`. All DB-gated tests self-skip when unset.
- **Migrations replay on every connect.** Everything added to `db/*.sql` must be idempotent (`CREATE TABLE IF NOT EXISTS`, `CREATE OR REPLACE FUNCTION`).
- Paper-parity: not clinical-surface — these are floor corrections beneath the application layer; no human act is added, removed, or reordered at any layer, and the changes affect only which admitted events gain power and when. (Wording is load-bearing: `crates/cairn-node/tests/paper_parity_plan_section.rs` matches the literal `Paper-parity: not clinical-surface` and requires ≥30 characters of reason after it.)

---

### Task 1: The `event_attestation_unvouched` marker and its lifecycle

Creates the marker, has db/020 write it, and has db/043 clear it when a gate actually verifies the token. No reader consumes it yet — that is Tasks 2–4.

**Files:**
- Modify: `db/001_envelope.sql` (append, before the closing `COMMIT;`)
- Modify: `db/020_apply_remote_event.sql:233-236` (the deferred arm)
- Modify: `db/043_deferred_readjudication.sql:103-120` (gate 1's success path)
- Modify: `db/tests/043_deferred_readjudication_test.sql`
- Test: `crates/cairn-node/tests/deferred_admission.rs` (append; reuses the file's existing `cs`, `setup`, `peer_event`, `db_msg`, `WALL_2026`, `UNKNOWN_TYPE`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: table `event_attestation_unvouched (event_id UUID PRIMARY KEY REFERENCES event_log(event_id) ON DELETE CASCADE)`. Tasks 2–4 read it with the predicate `EXISTS (SELECT 1 FROM event_attestation_unvouched u WHERE u.event_id = <expr>)`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// The marker's LIFETIME is the whole point (design §3). `event_deferred` answers "has this
/// been adjudicated?"; this second marker answers "is the stored attester_key vouched?" — and
/// those two facts stop agreeing the moment promotion deletes the first. Three states:
/// carried-and-unverified, verified-at-promotion (cleared), promoted-but-never-gated (kept).
#[tokio::test]
async fn an_unvouched_marker_tracks_whether_a_token_was_ever_verified() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();

    // A token that WOULD verify, on an event that bears responsibility — so gate 1 runs.
    let mut b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    b.contributors = serde_json::json!([{
        "actor_id": kid_h, "role": "authored",
        "responsibility": {"held_by": kid_h}
    }]);
    let signed = sign(&b, &sk_h).unwrap();
    let token = cairn_event::sign_attestation(
        &cairn_event::event_address(&signed.signed_bytes),
        &kid_h,
        "attested",
        &sk_h,
    )
    .unwrap();
    let hkey = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &token, &hkey],
    )
    .await
    .unwrap();

    // State 1: carried, not vouched. The door stored a token it could not verify.
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        unvouched, 1,
        "a deferred event carrying a token must be marked unvouched — nothing verified it"
    );

    // State 2: gate 1 runs (the type bears responsibility) and the token verifies.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    c.execute("SELECT 1 FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        unvouched, 0,
        "gate 1 verified the token, so the unvouched marker must be CLEARED"
    );
}

/// The state that produced the F2 hole: an additive event bearing NO responsibility. No gate
/// ever demands its token, so promotion must leave the unvouched marker STANDING — the marker
/// outliving `event_deferred` is exactly what the fix depends on.
#[tokio::test]
async fn a_never_gated_token_stays_unvouched_after_promotion() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid_a, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk_a).unwrap();
    // Keys are DERIVED, never literals (house rule 6): a blob that could never verify.
    let bogus: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(7)).collect();
    let akey = hex::decode(&kid_a).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &bogus, &akey],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    c.execute("SELECT 1 FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();

    let deferred: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 0, "precondition: an additive event promotes");
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        unvouched, 1,
        "no gate demanded this token, so nothing verified it — the marker must OUTLIVE \
         event_deferred, which is the whole reason it is a separate table"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission unvouched -- --nocapture
```

Expected: FAIL. Both tests error with `relation "event_attestation_unvouched" does not exist`.

- [ ] **Step 3: Create the table**

In `db/001_envelope.sql`, immediately after the `event_deferred_type_idx` index and before the closing `COMMIT;`:

```sql
-- ---------------------------------------------------------------------------
-- The carried-not-vouched marker (ADR-0056, PR #302 review finding F2).
--
-- One row means: "this event_log row's attestation/attester_key were STORED
-- WITHOUT BEING VERIFIED." The remote door cannot verify a deferred event's
-- travelling token — the gate that would is deferred with the interpretation —
-- but it must store it, or cairn_readjudicate_deferred has nothing to verify
-- later and admit-and-defer degrades into a slower fail-closed.
--
-- WHY A SECOND TABLE, rather than reusing event_deferred: the two answer
-- DIFFERENT questions with DIFFERENT lifetimes.
--   event_deferred                 -> "has this event been adjudicated?"
--   event_attestation_unvouched    -> "is this event's stored token vouched?"
-- Promotion deletes the first. The second must SURVIVE it, because db/043's
-- gate 1 only verifies a token when the type's mode DEMANDS one — an additive
-- event bearing no responsibility promotes with its token never checked. Using
-- event_deferred as the proxy is precisely the F2 defect: it let a forged token
-- on an unknown-type event put any key inside the target's human-author set
-- once the type was classified.
--
-- Node-local derived state — never signed, never on the wire (principle 12),
-- like event_deferred, reproject_log (db/039) and node_schema (db/038).
--
-- Deleted, never marked resolved, for the same reason event_deferred is: its
-- presence IS the invariant, and a resolved-row history would be a second,
-- drift-prone source of truth for one fact. Its readers are documented at the
-- three call sites (db/005 cairn_suppression_author_ok, db/018
-- patient_link_apply, db/034 medication_attestation_apply); a new reader of
-- event_log.attestation / .attester_key owes the same exclusion.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS event_attestation_unvouched (
    event_id UUID PRIMARY KEY REFERENCES event_log(event_id) ON DELETE CASCADE
);
```

- [ ] **Step 4: Have db/020's deferred arm write it**

`db/020_apply_remote_event.sql` currently ends its deferred arm at lines 233-236 with the two assignments. Leave those; the marker itself must be written after the `event_log` INSERT because of the FK. Add it inside the existing `IF v_deferred THEN` block at line 453, so both markers are written together:

```sql
    IF v_deferred THEN
        INSERT INTO event_deferred (event_id, event_type)
        VALUES (v_event_id, v_type)
        ON CONFLICT (event_id) DO NOTHING;
        -- The token stored at step 4 above was never verified (nothing here COULD verify
        -- it — the gate is deferred with the interpretation), so name that state now.
        -- Only when a token actually travelled: an event that carried none has nothing
        -- unvouched about it, and a spurious row would make every reader needlessly
        -- exclude a row whose attester_key is NULL anyway.
        IF v_att_key IS NOT NULL OR v_att IS NOT NULL THEN
            INSERT INTO event_attestation_unvouched (event_id)
            VALUES (v_event_id)
            ON CONFLICT (event_id) DO NOTHING;
        END IF;
    END IF;
```

- [ ] **Step 5: Have db/043's gate 1 clear it on a verified token**

In `db/043_deferred_readjudication.sql`, at the end of the `IF r.mode = 'suppressing' OR v_bears THEN` block (after the `cairn_responsibility_bound` check at line 117-119, before the closing `END IF;` at line 120):

```sql
                -- VOUCHED, at last. Every check the door would have run has now run
                -- against this token, so it stops being "carried" and becomes a real
                -- vouch. Inside the per-row subtransaction deliberately: if a LATER
                -- gate refuses this event, this clear rolls back with it and the token
                -- stays honestly unvouched.
                DELETE FROM event_attestation_unvouched WHERE event_id = r.event_id;
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission -- --nocapture
```

Expected: PASS, all 13 tests (the 11 existing plus the 2 new).

- [ ] **Step 7: Add the SQL mirror assertion**

In `db/tests/043_deferred_readjudication_test.sql`, after assertion block 1 and before block 2:

```sql
-- 1b. The carried-not-vouched marker exists and is 1:1 with event_log. Without the PK a
--     double-admission could double-mark, and a single clearing DELETE would leave a row
--     behind — pinning a genuinely-vouched token as unvouched forever, which reads as
--     over-refusal on the ADR-0043 floor rather than the over-permission F2 was.
DO $$
BEGIN
    IF to_regclass('public.event_attestation_unvouched') IS NULL THEN
        RAISE EXCEPTION 'event_attestation_unvouched is missing — an unverified carried token would be indistinguishable from a verified vouch once promotion deletes the event_deferred marker (PR #302 review finding F2)';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class cl ON cl.oid = i.indrelid
        WHERE cl.relname = 'event_attestation_unvouched' AND i.indisprimary
    ) THEN
        RAISE EXCEPTION 'event_attestation_unvouched has no primary key — the marker must be 1:1 with event_log';
    END IF;
END $$;
```

- [ ] **Step 8: Run the SQL mirrors**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  ./scripts/run-db-sql-tests.sh
```

Expected: all mirrors pass, including `043_deferred_readjudication_test.sql`.

- [ ] **Step 9: Commit**

```bash
git add db/001_envelope.sql db/020_apply_remote_event.sql \
        db/043_deferred_readjudication.sql \
        db/tests/043_deferred_readjudication_test.sql \
        crates/cairn-node/tests/deferred_admission.rs
git commit -m "feat(#302-F2): name the carried-not-vouched state

event_deferred was answering two questions with one lifetime: 'has this
been adjudicated?' and 'is this token vouched?'. Promotion deletes it, and
gate 1 only verifies a token when the type's mode demands one — so an
additive event bearing no responsibility promotes with an unverified
attester_key and no marker saying so.

event_attestation_unvouched is written by the door beside event_deferred
and cleared only inside gate 1's success path, so it survives promotion.
No reader consumes it yet."
```

---

### Task 2: `cairn_suppression_author_ok` asks the question it means

Replaces PR #302's `event_deferred` exclusion with the unvouched predicate. This is the task that closes F2's confirmed exploit.

**Files:**
- Modify: `db/005_submit.sql:334-353` (the `human_authors` CTE's attester arm)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (append)

**Interfaces:**
- Consumes: `event_attestation_unvouched` from Task 1.
- Produces: no new symbols. `cairn_suppression_author_ok(p_target UUID, p_attester_key BYTEA) RETURNS boolean` keeps its signature.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// THE F2 REGRESSION PIN (PR #302 review). The sibling test
/// `a_carried_token_does_not_widen_the_owner_gate` covers the still-DEFERRED target. This
/// covers the target after PROMOTION — where the original fix stopped working, because it
/// keyed on the event_deferred marker that promotion deletes.
///
/// Scenario, measured before the fix: a hostile peer ships an unknown-type event signed by an
/// honest human, carrying a GARBAGE attestation blob naming Mallory. The node admits it
/// deferred and stores the blob unverified. The type is later classified ('additive', FALSE) —
/// no gate demands a token, so nothing ever checks it — and promotion deletes the marker. The
/// owner-gate then unioned Mallory's key into the target's human-author set, and she could
/// suppress another clinician's event on the strength of a blob nothing had ever looked at.
#[tokio::test]
async fn a_carried_token_never_widens_the_owner_gate_after_promotion() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    // A SECOND enrolled human — Mallory. The pinned determinants must differ from the
    // setup() human's, or enroll_actor refuses the pair as one actor (issue #152).
    let (_sk_m, kid_m) = cairn_event::generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"mallory\"}', $1)",
        &[&kid_m],
    )
    .await
    .unwrap();

    let p = Uuid::now_v7();
    // Signed by the HONEST human, so the target's author set is non-empty via the signer arm
    // and the gate is genuinely restrictive — not the vacuous "no human authors => anyone may
    // suppress" branch, which would make this test pass for the wrong reason.
    let b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    let target_id = b.event_id.clone();
    let signed = sign(&b, &sk_h).unwrap();
    // Derived at runtime, never a literal (house rule 6): a blob that could never verify.
    let bogus: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(7)).collect();
    let mkey = hex::decode(&kid_m).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &bogus, &mkey],
    )
    .await
    .expect("a deferred event carrying a token is still admitted");

    // The code plane arrives. 'additive' + no responsibility contributor = NO gate demands a
    // token, so promotion never verifies this one.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    c.execute("SELECT 1 FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();

    // Preconditions — without these the test proves nothing.
    let deferred: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 0, "precondition: the event was PROMOTED");
    let stored: Option<Vec<u8>> = c
        .query_one(
            "SELECT attester_key FROM event_log WHERE event_id = $1::text::uuid",
            &[&target_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stored.as_deref(),
        Some(mkey.as_slice()),
        "precondition: the unverified key is still on the row (event_log is append-only, \
         so it can never be scrubbed) — the hazard is not reproduced without it"
    );

    let widened: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&target_id, &mkey],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !widened,
        "a token NO gate ever demanded must not widen the ADR-0043 owner-gate after \
         promotion — Mallory never signed, authored, or attested anything"
    );

    // Sanity: the fix narrowed only the unvouched arm; the real signer still owns the event.
    let genuine: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&target_id, &hex::decode(&kid_h).unwrap()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        genuine,
        "the target's real human signer must still count as its author"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission after_promotion -- --nocapture
```

Expected: FAIL at the `!widened` assertion — `a token NO gate ever demanded must not widen the ADR-0043 owner-gate after promotion`.

- [ ] **Step 3: Swap the predicate**

In `db/005_submit.sql`, replace the whole attester arm of the `human_authors` CTE — the comment block PR #302 added plus its two-line predicate (lines 334-353) — with:

```sql
        -- ADR-0056 (issue #265, PR #302 review finding F2): count this arm only when the
        -- stored token has actually been VOUCHED.
        --
        -- The remote door stores a deferred event's travelling token without verifying it
        -- — it cannot, because the gate that verifies it is deferred with the
        -- interpretation. Unioning an unverified key here would let a hostile peer put ANY
        -- key it likes inside the target's human-author set simply by attaching a forged
        -- token to an unknown-type event, and its holder could then suppress that event.
        -- That is over-permission on the ADR-0043 floor, which this function's header
        -- forbids in exactly those words.
        --
        -- WHY NOT "is the target deferred?", which is what this originally asked: that is a
        -- PROXY with the wrong lifetime. cairn_readjudicate_deferred (db/043) verifies a
        -- token only when the type's mode DEMANDS one, so an additive event bearing no
        -- responsibility is promoted — event_deferred row deleted — with its token never
        -- checked. The proxy said "vouched" the instant the marker vanished. The marker
        -- below survives promotion and is cleared only by gate 1 actually verifying, so it
        -- answers the question this arm means to ask.
        --
        -- The fix is NEUTRAL, not merely stricter: for a target signed by an AGENT, dropping
        -- this arm empties human_authors and the gate OPENS (the agent-advisory-is-
        -- dismissable rule below). That is correct — an unverified token must not move the
        -- gate in EITHER direction.
        --
        -- Two other readers of event_log.attester_key carry the same exclusion:
        -- patient_link_apply (db/018) and medication_attestation_apply (db/034). A new
        -- reader of these columns owes the same choice.
        SELECT encode(t.attester_key, 'hex') FROM tgt t
        WHERE t.attester_key IS NOT NULL
          AND NOT EXISTS (SELECT 1 FROM event_attestation_unvouched u
                           WHERE u.event_id = p_target)
```

- [ ] **Step 4: Run the full deferred suite to verify it passes**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission -- --nocapture
```

Expected: PASS, 14 tests. `a_carried_token_does_not_widen_the_owner_gate` (the still-deferred case) must still pass — the new predicate covers it, because a deferred row is unvouched by construction.

- [ ] **Step 5: Commit**

```bash
git add db/005_submit.sql crates/cairn-node/tests/deferred_admission.rs
git commit -m "fix(#302-F2): the owner-gate asks whether a token is vouched, not whether the target is deferred

Measured before this commit: a garbage 64-byte blob naming an unrelated
enrolled human, attached to an unknown-type event, put that human inside
another clinician's author set once the type was classified additive —
cairn_suppression_author_ok returned true for a key that never signed,
authored, or attested anything.

The exclusion keyed on event_deferred, which promotion deletes; gate 1
verifies a token only when the mode demands one, so 'marker gone' never
meant 'token checked'. It now excludes on event_attestation_unvouched,
which survives promotion and is cleared only by an actual verification."
```

---

### Task 3: `patient_link_apply` stops reading an unvouched token as a human decision

**Files:**
- Modify: `db/018_identity_linkage.sql:374-375` (the local-door hard-veto refusal) and `db/018_identity_linkage.sql:416-420` (the flag-lifecycle read)
- Test: `crates/cairn-node/tests/link_veto_floor.rs` (append; reuses the file's existing `cs`, `setup`, `vetoed_pair`, `link_body`, `db_msg`)

**Interfaces:**
- Consumes: `event_attestation_unvouched` from Task 1.
- Produces: no new symbols.

The reachable exploit is the **second** read, not the first: the local-door refusal at line 374 is already skipped on the sync-apply path (`current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on'`), whereas the flag lifecycle at 416-420 is not — an unvouched token makes `v_win_attested` true, so a hard-vetoed merge stands with **no `link_veto_flag` row and no `under-review` trust state**. Both reads are fixed; only the second is testable end to end.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/link_veto_floor.rs`:

```rust
/// PR #302 review finding F2, second reader. A deferred event's attester_key is CARRIED, NOT
/// VOUCHED — and once promotion projects it (db/043 gate 4), this apply fn sees it.
///
/// `v_win_attested` drives the whole #190 flag lifecycle: an attested link is the human
/// decision the veto forces, so it raises no flag. Let an unverified token satisfy that and a
/// hostile peer suppresses the flag on a hard-vetoed merge with a blob nothing ever checked —
/// two charts silently merged, no worklist entry, both reading `confirmed`. That is strictly
/// worse than the un-attested case, which at least gets flagged.
#[tokio::test]
async fn an_unvouched_token_is_not_a_link_attestation() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    // Simulate "this node has no code for identity.link.asserted yet" the same way
    // deferred_admission.rs does for patient.created: remove BOTH things the migration
    // provides. Removing only the class row produces a registered-but-unclassified state no
    // real node can reach, and the AFTER-INSERT dispatcher would still fire. All three rows
    // are restored by the next connect's migration replay, so this is self-healing.
    for sql in [
        "DELETE FROM cairn_projection_apply WHERE event_type = 'identity.link.asserted'",
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'identity.link.asserted'",
        "DELETE FROM event_type_class WHERE event_type = 'identity.link.asserted'",
    ] {
        c.execute(sql, &[]).await.unwrap();
    }

    // An AGENT link — no human decision anywhere — carrying a garbage token. Derived at
    // runtime, never a literal (house rule 6).
    let body = link_body(&kid_a, a, b, true, 10, false);
    let signed = sign(&body, &sk_a).unwrap();
    let bogus: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(11)).collect();
    let akey = hex::decode(&kid_a).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes, &bogus, &akey],
    )
    .await
    .expect("an unclassifiable type is admitted uninterpreted");

    // A fresh connect replays the migrations (restoring all three rows), re-adjudicates, and
    // projects what it promoted.
    drop(c);
    let c2 = db::connect_and_load_schema(&base).await.unwrap();

    let deferred: i64 = c2
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 0, "precondition: the link was promoted");
    let merged: i64 = c2
        .query_one("SELECT count(*) FROM person_member", &[])
        .await
        .unwrap()
        .get(0);
    assert!(merged > 0, "precondition: the promoted link projected");

    let flags: i64 = c2
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 1,
        "an UNVOUCHED token is not the human decision the §5.2 veto forces — the vetoed \
         merge must still be flagged, never silently accepted"
    );
    assert_eq!(trust_state(&c2, a).await.as_deref(), Some("under-review"));
    assert_eq!(trust_state(&c2, b).await.as_deref(), Some("under-review"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test link_veto_floor unvouched -- --nocapture
```

Expected: FAIL. Either `assertion left == right` with `flags = 0` (the fix is not in yet), or — if Task 6 is not yet done — a panic from `connect_and_load_schema`. If the latter, note it: that is F1 showing through, and it confirms the two findings interlock. Proceed anyway; Task 6 removes that failure mode and this test must be re-run at the end of Task 6.

- [ ] **Step 3: Fix the local-door refusal (db/018 line 374)**

Replace:

```sql
    IF v_state = 'link' AND e.attester_key IS NULL AND cairn_has_hard_veto(lo, hi)
       AND current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
```

with:

```sql
    -- "e.attester_key set" means a human decided — but only if something VERIFIED it. A
    -- deferred event's token is carried, not vouched (ADR-0056 / PR #302 finding F2), so an
    -- unvouched row is treated exactly as an un-attested one: the veto still forces a human
    -- decision. Unreachable on this branch today (the remote_apply guard below skips it for
    -- every replicated event, and a deferred event only ever arrives replicated), but a
    -- reader must not have to prove that to trust the line.
    IF v_state = 'link'
       AND (e.attester_key IS NULL
            OR EXISTS (SELECT 1 FROM event_attestation_unvouched u
                        WHERE u.event_id = e.event_id))
       AND cairn_has_hard_veto(lo, hi)
       AND current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
```

- [ ] **Step 4: Fix the flag-lifecycle read (db/018 lines 416-420)**

Replace:

```sql
    SELECT pl.state, pl.content_address, el.attester_key IS NOT NULL
      INTO v_win_state, v_win_ca, v_win_attested
      FROM patient_link pl
      JOIN event_log el ON el.content_address = pl.content_address
      WHERE pl.low = lo AND pl.high = hi;
```

with:

```sql
    -- THE REACHABLE ONE (PR #302 finding F2). Unlike the door refusal above, nothing skips
    -- this on the sync path — so an unvouched token satisfying `v_win_attested` would
    -- SUPPRESS the flag on a hard-vetoed merge: two charts merged, no worklist entry, both
    -- charts reading `confirmed`. Strictly worse than the un-attested case, which is at
    -- least flagged. An unvouched vouch is not a vouch.
    SELECT pl.state, pl.content_address,
           el.attester_key IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM event_attestation_unvouched u
                              WHERE u.event_id = el.event_id)
      INTO v_win_state, v_win_ca, v_win_attested
      FROM patient_link pl
      JOIN event_log el ON el.content_address = pl.content_address
      WHERE pl.low = lo AND pl.high = hi;
```

- [ ] **Step 5: Run the veto suite to verify it passes**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test link_veto_floor -- --nocapture
```

Expected: PASS, every test in the file. `human_attested_link_with_hard_veto_still_passes` is the discriminator that proves the fix did not simply break attestation — a genuinely attested link has no unvouched marker and must still pass.

- [ ] **Step 6: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/tests/link_veto_floor.rs
git commit -m "fix(#302-F2): an unvouched token is not a link attestation (db/018)

patient_link_apply reads attester_key IS NOT NULL as 'a human decided',
which is what lets an attested link pass the #190 hard veto and raise no
flag. Once promotion projects a deferred event, that column can hold a
token nothing verified — so a hostile peer could suppress the veto flag on
a hard-vetoed merge: two charts merged, no worklist entry, both reading
confirmed. Worse than the un-attested case, which is at least flagged.

Both reads now exclude event_attestation_unvouched. The flag-lifecycle
read is the reachable one; the door refusal is fixed too so a reader need
not prove the remote_apply guard covers it."
```

---

### Task 4: `medication_attestation_apply` degrades honestly on an unvouched token

**Files:**
- Modify: `db/034_medication_attestation.sql:249-250` (the `IF p IS NULL THEN RETURN; END IF;` guard) and `db/034_medication_attestation.sql:258-269` (the INVARIANT comment)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (append)

**Interfaces:**
- Consumes: `event_attestation_unvouched` from Task 1.
- Produces: no new symbols.

This reader is **unreachable today** — `clinical.medication-attestation.asserted` always carries a responsibility contributor (the M1 floor check), so db/043's gate 1 always runs for it and either verifies the token or refuses promotion. The guard is defence in depth against a future type that reads this column without bearing responsibility, and the header's stated INVARIANT is currently wrong about *why* the column is trustworthy. The test forces the state directly rather than pretending it arrives naturally.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// PR #302 review finding F2, third reader — a WHITE-BOX test, and deliberately so.
///
/// `medication_attestation_apply` projects `encode(e.attester_key,'hex')` as `attester_kid`:
/// the responsible human, the thing the whole ADR-0049 sign-off surface reads. Its header
/// asserts that column is a verified vouch. That is true today only because a
/// `-attestation.asserted` event always bears responsibility, so db/043's gate 1 always runs
/// for it — a property of the EVENT TYPE, not of the column. This forces the state that
/// property currently rules out, so the guard is pinned before some future type reaches it.
#[tokio::test]
async fn an_unvouched_token_never_becomes_an_attester_kid() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();

    // A well-formed attested event, admitted through the normal (classified) door so its
    // token is genuinely verified and it projects a row.
    let med_id = Uuid::now_v7();
    let rows_before: i64 = c
        .query_one("SELECT count(*) FROM medication_attestation", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows_before, 0, "precondition: a clean projection table");

    // Drive the ordinary attestation flow via the deferred path so we own the row: defer,
    // classify, promote. UNKNOWN_TYPE stands in for any future type that reads attester_key.
    // EVERY NOT NULL column medication_attestation demands (medication_id, patient_id,
    // attester_kid, reviewed_commitment, reviewed_count) must be satisfiable, or the pre-fix
    // run fails on a constraint instead of on the defect — and a test that fails for the
    // wrong reason proves nothing. reviewed_commitment is `decode(..., 'hex')`, so the
    // payload carries hex; derived at runtime, never a literal (house rule 6).
    let commitment: String = (0u8..32).map(|i| format!("{:02x}", i.wrapping_mul(5))).collect();
    let mut b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    b.payload = serde_json::json!({
        "medication_id": med_id.to_string(),
        "reviewed_commitment": commitment,
        "reviewed_count": 1
    });
    let signed = sign(&b, &sk_h).unwrap();
    let bogus: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(13)).collect();
    let hkey = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &bogus, &hkey],
    )
    .await
    .unwrap();

    // The row now holds an attester_key nothing verified, and says so.
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(unvouched, 1, "precondition: the token is marked unvouched");

    // Call the apply fn directly on that row — the white-box part. It must decline to
    // project rather than mint an attester_kid from an unverified key.
    c.execute(
        "SELECT medication_attestation_apply(el) FROM event_log el WHERE el.event_type = $1",
        &[&UNKNOWN_TYPE],
    )
    .await
    .expect("the apply fn must DEGRADE (no row), never raise — a raise wedges the event");

    let rows_after: i64 = c
        .query_one("SELECT count(*) FROM medication_attestation", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        rows_after, 0,
        "an unvouched token must never become an attester_kid — that column IS the \
         responsible human on the ADR-0049 sign-off surface"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission attester_kid -- --nocapture
```

Expected: FAIL at the final assertion with `rows_after = 1` — the fn projected a row keyed on an unverified key.

- [ ] **Step 3: Add the guard and correct the header**

In `db/034_medication_attestation.sql`, replace:

```sql
BEGIN
    IF p IS NULL THEN RETURN; END IF;
```

with:

```sql
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- ADR-0056 / PR #302 finding F2: attester_key may hold a token the remote door CARRIED
    -- without verifying (a deferred event's gate is deferred with the interpretation). This
    -- fn's whole output keys on it as the responsible human, so an unvouched one must yield
    -- NO ROW rather than a false vouch — honest degradation, the §3.13 discipline, and the
    -- same shape as the p IS NULL arm above. RETURN, never RAISE: a raise here would abort
    -- the apply and wedge the event forever (the ADR-0058 hazard).
    IF EXISTS (SELECT 1 FROM event_attestation_unvouched u WHERE u.event_id = e.event_id) THEN
        RETURN;
    END IF;
```

Then correct the INVARIANT paragraph in the same file — replace the sentence beginning `-- INVARIANT:` and running to `-- turns a would-be NULL into a legible floor rejection long before this trigger.` with:

```sql
        -- INVARIANT, and its real basis: attester_key is non-NULL and VERIFIED here. A
        -- `-attestation.asserted` event always carries a responsibility contributor
        -- (enforced by the M1 floor check above), which trips the db/005 gate that populates
        -- attester_key at the local door AND db/043's gate 1 on the deferred path — so the
        -- token is verified on every route that reaches this fn. Note that is a property of
        -- the EVENT TYPE, not of the column: a type bearing no responsibility can be
        -- promoted with its carried token unchecked, which is what the unvouched guard at
        -- the top of this fn defends against.
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission -- --nocapture
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test medication_attestation -- --nocapture
```

Expected: PASS both. The existing medication-attestation suite is the discriminator — a genuinely attested event has no unvouched marker and must still project.

- [ ] **Step 5: Commit**

```bash
git add db/034_medication_attestation.sql crates/cairn-node/tests/deferred_admission.rs
git commit -m "fix(#302-F2): an unvouched token never becomes an attester_kid (db/034)

medication_attestation_apply projects attester_key as the responsible
human on the ADR-0049 sign-off surface. Unreachable today — the type
always bears responsibility, so gate 1 always verifies — but the header
asserted the column was trustworthy when what is actually trustworthy is
the event type. Guard added (RETURN, never RAISE: a raise wedges the
event), header corrected to name the real basis."
```

---

### Task 5: Gate 0 — re-run the per-type structural floor

**Files:**
- Modify: `db/043_deferred_readjudication.sql:66` (add `PERFORM set_config` at the function's `BEGIN`), `:56-65` (DECLARE), `:67-81` (the loop's SELECT), `:92-95` (after the body re-derivation)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (append)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: the loop record gains `el_row` (an `event_log` composite), which Task 6's gate 4 consumes as `r.el_row`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// PR #302 review finding F1, first half. db/020 step 8 — `cairn_event_twin`'s dispatch to the
/// type's `check_fn` and `twin_required_msg` — is skipped for a deferred event for exactly the
/// same reason the other three gates are: the type has no registry row. db/043 re-ran three
/// gates and not this one, so it was WAIVED rather than deferred.
///
/// Pinned with `clinical.medication.asserted`, which hard-requires an authored twin and has a
/// real `check_fn`. The event below would be refused by BOTH doors if the type were known.
#[tokio::test]
async fn promotion_refuses_an_event_its_type_floor_rejects() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();

    // Simulate "no code for this type yet" — all three rows the migration provides. Restored
    // by the next connect's replay, so this is self-healing even if the test dies partway.
    for sql in [
        "DELETE FROM cairn_projection_apply WHERE event_type = 'clinical.medication.asserted'",
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'clinical.medication.asserted'",
        "DELETE FROM event_type_class WHERE event_type = 'clinical.medication.asserted'",
    ] {
        c.execute(sql, &[]).await.unwrap();
    }

    let mut b = peer_event(&kid, p, "clinical.medication.asserted", WALL_2026);
    b.schema_version = "clinical.medication.asserted/1".into();
    b.payload = serde_json::json!({"nonsense": true});
    b.plaintext_twin = None; // the type hard-REQUIRES an authored twin
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .expect("an unclassifiable type is admitted uninterpreted");

    // The code plane lands — restore only the classification, so promotion is attempted while
    // the projection registration is still absent. That isolates gate 0 from gate 4.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ('clinical.medication.asserted', 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[],
    )
    .await
    .unwrap();
    c.batch_execute(include_str!("../../../db/031_medication.sql"))
        .await
        .unwrap(); // restores cairn_event_twin_check for the type

    let rows = c
        .query("SELECT promoted_type FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "an event its own type's structural floor rejects must NOT be promoted"
    );

    let err: Option<String> = c
        .query_one("SELECT max(adjudication_error) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    let err = err.expect("the refusal must be recorded");
    assert!(
        err.contains("twin") || err.contains("§3.13"),
        "the flag must name a CLINICAL reason, not a constraint violation; got: {err}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission type_floor -- --nocapture
```

Expected: FAIL at `an event its own type's structural floor rejects must NOT be promoted` — `rows` has one entry.

- [ ] **Step 3: Add `el_row`, `v_clear`, and the remote-apply marker**

In `db/043_deferred_readjudication.sql`, add to the `DECLARE` block:

```sql
    v_clear    jsonb;
```

Replace the loop's SELECT list (line 68-69) with — note `el` is selected *as a composite as well as* its individual columns, because PL/pgSQL cannot reliably reach `r.el_row.signed_bytes`:

```sql
        SELECT d.event_id, d.event_type, el AS el_row, el.signed_bytes, el.content_address,
               el.attestation, el.attester_key, c.mode, c.targets_other_author
```

Immediately after `BEGIN` (line 66), before the `FOR r IN` loop:

```sql
    -- These are PEER-ARRIVED events, so every check below must run on the LENIENT tier the
    -- door would have used. db/041's cairn_check_medication_coding reads this marker; without
    -- it, gate 0 would refuse a verifiable peer event outright — the sync-watermark freeze
    -- db/020's own step-8 comment warns about, and precisely what ADR-0056 forbids.
    -- SET LOCAL (is_local = true): scoped to this transaction, exactly as cairn_reproject
    -- (db/039) does for its whole run.
    PERFORM set_config('cairn.remote_apply', 'on', true);
```

- [ ] **Step 4: Add gate 0**

In the per-row subtransaction, immediately after the `IF b IS NULL THEN ... END IF;` block (line 95):

```sql
            -- Deferred gate 0 — the per-type STRUCTURAL floor (db/020 step 8). It was skipped
            -- at admission for the same reason gates 1-3 were: the type had no registry row,
            -- so cairn_event_twin found neither a check_fn nor a twin_required_msg and fell
            -- through to the skeleton. Now the row exists, so the check must run — otherwise
            -- this check is WAIVED rather than deferred, and this file's header is false.
            --
            -- cairn_clear_payload is reused rather than reimplementing db/020's
            -- sealed/unsealed branching, so the two paths cannot drift on what a readable
            -- body is. NULL = sealed with no custody here: skip, exactly as the door does —
            -- a structural check cannot run on ciphertext. Gate 4 still proves such an event
            -- can project.
            v_clear := cairn_clear_payload(r.el_row);
            IF v_clear IS NOT NULL THEN
                PERFORM cairn_event_twin(r.event_type, jsonb_set(b, '{payload}', v_clear));
            END IF;
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission -- --nocapture
```

Expected: PASS, every test in the file. `classification_promotes_a_passing_deferred_event` is the discriminator — a well-formed event of an unregistered type still has no `check_fn`, so gate 0 is a no-op for it and it must still promote.

- [ ] **Step 6: Commit**

```bash
git add db/043_deferred_readjudication.sql crates/cairn-node/tests/deferred_admission.rs
git commit -m "fix(#302-F1): gate 0 re-runs the per-type structural floor

db/020 step 8's cairn_event_twin dispatch is skipped for a deferred event
for the same reason gates 1-3 are — the type has no registry row. db/043
re-ran three gates and not this one, so it was waived rather than deferred
and this file's own header was false.

Gate 0 runs it, on the clear body via cairn_clear_payload so it cannot
drift from the door. The pass raises cairn.remote_apply for its duration:
these are peer-arrived events and db/041's coding check reads that marker.

Payoff beyond correctness: adjudication_error now reads 'medication
assertion requires a non-empty authored twin' instead of a NOT NULL
violation. 'Flagged legibly' means naming a clinical reason."
```

---

### Task 6: Gate 4 — prove the projection, and simplify the loader

The task that closes the node-bricking failure.

**Files:**
- Modify: `db/043_deferred_readjudication.sql` (DECLARE; end of the per-row subtransaction)
- Modify: `crates/cairn-node/src/db.rs:435-495` (the call + the `else`-branch removal)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (append)

**Interfaces:**
- Consumes: `r.el_row` from Task 5.
- Produces: `cairn_readjudicate_deferred()` keeps its `RETURNS TABLE(promoted_type text, promoted_count bigint)` signature. `connect_and_load_schema` keeps its signature.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// PR #302 review finding F1, the part that BRICKS THE NODE.
///
/// Measured before this fix: promotion deleted the marker for an event whose apply fn then
/// raised, and because event_log is append-only nothing could undo it. Three consecutive
/// connect_and_load_schema calls failed with `post-upgrade heal replay: db error` and
/// node_schema.version never advanced past 42. `cairn-node deferred` could not diagnose it —
/// it calls connect_and_load_schema itself.
///
/// The invariant this pins: a promoted event is one that has ALREADY projected cleanly.
#[tokio::test]
async fn a_promotion_that_cannot_project_never_promotes() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    for sql in [
        "DELETE FROM cairn_projection_apply WHERE event_type = 'clinical.medication.asserted'",
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'clinical.medication.asserted'",
        "DELETE FROM event_type_class WHERE event_type = 'clinical.medication.asserted'",
    ] {
        c.execute(sql, &[]).await.unwrap();
    }
    let mut b = peer_event(&kid, p, "clinical.medication.asserted", WALL_2026);
    b.schema_version = "clinical.medication.asserted/1".into();
    b.payload = serde_json::json!({"nonsense": true});
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .unwrap();
    // The code plane update also bumps the generation, so the loader takes the FULL-heal
    // branch — the realistic path, and the one that wedged permanently.
    c.execute("UPDATE node_schema SET version = version - 1", &[])
        .await
        .unwrap();
    drop(c);

    for attempt in 1..=3 {
        db::connect_and_load_schema(&base)
            .await
            .unwrap_or_else(|e| panic!("connect attempt {attempt} must succeed, got: {e}"));
    }

    let c2 = db::connect_and_load_schema(&base).await.unwrap();
    let kept: i64 = c2
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        kept, 1,
        "an event that cannot project must KEEP its marker — powerless, retryable, and \
         above all unable to take the loader down with it"
    );
    let embedded = db::embedded_schema_version();
    let recorded: i32 = c2
        .query_one("SELECT version FROM node_schema", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        recorded, embedded,
        "the generation must ADVANCE — a stuck stamp means every future connect retries \
         the same doomed heal forever"
    );

    // Leave the shared test database clean for the next test.
    c2.batch_execute("TRUNCATE event_log CASCADE").await.unwrap();
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission cannot_project -- --nocapture
```

Expected: FAIL at `connect attempt 1 must succeed, got: post-upgrade heal replay: db error`.

**If it passes, stop.** Task 5's gate 0 already refuses this payload, so the test does not reach gate 4 and does not discriminate. Change the payload to one gate 0 accepts but the apply fn rejects — add a valid authored twin (`b.plaintext_twin = Some("metformin 500mg".into())`) while leaving the payload without the fields `medication_statement` requires — and re-verify the failure before continuing.

- [ ] **Step 3: Recover the wedged test database**

The failing run leaves a poisoned event behind. Before implementing:

```bash
psql "host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  -c "TRUNCATE event_log CASCADE"
```

- [ ] **Step 4: Add gate 4**

In `db/043_deferred_readjudication.sql`, add to `DECLARE`:

```sql
    v_apply_fn text;
```

At the END of the per-row subtransaction — after the `IF r.targets_other_author THEN ... END IF;` block and immediately before `EXCEPTION WHEN OTHERS THEN`:

```sql
            -- Deferred gate 4 — PROVE IT TAKES EFFECT (PR #302 review finding F1).
            --
            -- Gates 0-3 answer "should this event have power?". This one answers "CAN it
            -- take power?", and skipping it bricked the node: the marker delete below
            -- commits, the event becomes replay-eligible, the loader's heal then raises on
            -- it, and because event_log is append-only nothing can undo that. Every
            -- subsequent connect repeated the same failure and the generation stamp never
            -- advanced. Measured: three consecutive connects failed, node_schema frozen.
            --
            -- Running the apply fns HERE, inside the per-row subtransaction, makes the
            -- marker delete conditional on them succeeding: a raise sets v_err, the
            -- subtransaction rolls back every projection write it made, and the marker stays.
            -- The invariant that buys: a PROMOTED EVENT IS ONE THAT HAS ALREADY PROJECTED
            -- CLEANLY. That holds for a stricter apply fn written years from now, which gate
            -- 0 alone would not cover.
            --
            -- WHY PER-EVENT DISPATCH IS AFFORDABLE HERE and not in cairn_reproject: db/039
            -- is deliberately set-based (one full-table pass per (type, fn)) because the
            -- per-event loop it replaced was ~25% of a 2M-event rebuild at the Pi target.
            -- That argument does not transfer — event_deferred is empty on a healthy node
            -- and tiny by construction otherwise.
            --
            -- heal_safe mirrors heal mode (db/039): a fn that only converges under a
            -- TRUNCATE cannot prove anything by running over live rows.
            FOR v_apply_fn IN
                SELECT apply_fn FROM cairn_projection_apply
                 WHERE event_type = r.event_type AND heal_safe
                 ORDER BY run_order, apply_fn
            LOOP
                EXECUTE format('SELECT %I($1)', v_apply_fn) USING r.el_row;
            END LOOP;
```

- [ ] **Step 5: Simplify the loader**

In `crates/cairn-node/src/db.rs`, replace the `let promoted: Vec<String> = ...` binding and the whole `if recorded != Some(embedded) { ... } else { ... }` block (lines 435-495) with:

```rust
    let promoted = client
        .query(
            "SELECT promoted_type, promoted_count FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .map_err(|e| anyhow::anyhow!("re-adjudicating deferred events: {e}"))?;
    // Granting power is never a silent event: an operator who upgrades a node wants to see
    // which types just became live. Empty on a healthy node, so this prints nothing.
    for row in &promoted {
        let ty: String = row.get(0);
        let n: i64 = row.get(1);
        eprintln!("re-adjudicated: promoted {n} deferred event(s) of type {ty}");
    }

    // #208/ADR-0057: heal replay on generation CHANGE only, and BEFORE the stamp
    // below. New projection capability (and any projection-logic fix) arrives only
    // via a code-plane update — i.e. a generation change — so an unchanged
    // generation means there is nothing to heal and the connect path does zero
    // reprojection work (the old db/013 every-connect backfill is retired by this
    // branch's demographics conversion). An UNKNOWN recorded generation (fresh DB:
    // free no-op; hand-built rig: converges once) errs toward healing. Runs inside
    // SCHEMA_LOAD_LOCK: concurrent loaders serialize, and the second sees the
    // stamped generation.
    //
    // NO targeted reproject for what the pass just promoted: db/043's gate 4 already
    // ran each promoted event's heal-safe apply fns inside its promotion
    // subtransaction — that is what "promoted" now MEANS. It also makes this full
    // heal safe by construction, which it was not before: a promoted event that
    // cannot project used to abort the load here, permanently, because the marker
    // was already gone and event_log is append-only (PR #302 review finding F1).
    //
    // Ordered BEFORE the stamp deliberately: if the heal query below errors, the
    // stamp never runs, so the recorded generation stays at its OLD (pre-upgrade)
    // value and the `?` propagates the failure up to the caller. The NEXT connect
    // attempt then sees the same stale `recorded`, so it retries the FULL
    // replay-then-heal — exactly the loud, self-retrying failure mode a broken
    // migration file already has in this loader (a bad `db/*.sql` blocks connect
    // until fixed; it never silently half-applies). Stamp-then-heal would invert
    // this: a heal failure AFTER the stamp leaves the generation already advanced,
    // so the next connect reads `recorded == embedded`, skips the heal entirely,
    // and the projections stay SILENTLY stale — the worst failure mode, and the
    // reason this order is load-bearing, not cosmetic.
    if recorded != Some(embedded) {
        client
            .execute(
                "SELECT count(*) FROM cairn_reproject('', false, 'loader')",
                &[],
            )
            .await
            .map_err(|e| anyhow::anyhow!("post-upgrade heal replay: {e}"))?;
    }
```

- [ ] **Step 6: Run the test to verify it passes**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission -- --nocapture
```

Expected: PASS, every test. `connect_promotes_and_reprojects_a_deferred_event` is the discriminator — it asserts a promoted `patient.created` reaches `patient_chart`, which now happens via gate 4 rather than the removed targeted reproject. If it fails, gate 4 is not running the right fns.

- [ ] **Step 7: Re-run Task 3's test, which was blocked on this**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-node --test link_veto_floor -- --nocapture
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add db/043_deferred_readjudication.sql crates/cairn-node/src/db.rs \
        crates/cairn-node/tests/deferred_admission.rs
git commit -m "fix(#302-F1): a promoted event is one that has already projected

Gates 0-3 ask whether an event should have power. Gate 4 asks whether it
CAN take power — and skipping it bricked the node. The marker delete
commits, the event becomes replay-eligible, the loader's heal raises on
it, and event_log is append-only so nothing undoes it. Measured: three
consecutive connects failed and node_schema stayed at 42. cairn-node
deferred could not even diagnose it; it connects too.

Gate 4 runs the type's heal-safe apply fns inside the promotion
subtransaction, so the marker delete is conditional on them succeeding.
Per-event dispatch is affordable here precisely where it is not in
cairn_reproject: the deferred set is empty on a healthy node.

The loader gets SMALLER: the targeted reproject is now redundant, and the
full heal is safe by construction."
```

---

### Task 7: cairn-sync runs the pass (F3)

Safe only now — before Task 6, adding this would have spread the wedge to the sync daemon.

**Files:**
- Modify: `crates/cairn-sync/src/main.rs:738-762` (inside `load_schema_under_lock`, after the migration loop, before the gated heal)
- Test: `crates/cairn-sync/src/main.rs` (the existing `#[cfg(test)] mod` around line 4900)

**Interfaces:**
- Consumes: `cairn_readjudicate_deferred()` from Tasks 5-6.
- Produces: no new symbols.

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `crates/cairn-sync/src/main.rs`, beside `load_schema_stamps_the_generation_and_refuses_a_newer_db`:

```rust
    /// PR #302 review finding F3. This crate embeds db/043 — and the comment beside that
    /// entry argues it MUST, because db/020 (also here) is the door that WRITES the
    /// event_deferred marker. The reasoning was right and only the function shipped: nothing
    /// in this binary called it, so a sync-only database — the phone-tier carrier node
    /// ADR-0056 exists for — accumulated markers that nothing could ever promote.
    #[test]
    fn load_schema_promotes_a_deferred_event() {
        let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut lock = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        lock.execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64])
            .unwrap();
        let mut c = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        load_schema(&mut c).expect("baseline replay must succeed");
        c.batch_execute("TRUNCATE event_log CASCADE").unwrap();

        // Hand-write a deferred row: this crate cannot sign, and the pass only needs a row
        // whose type has no event_type_class entry to leave it untouched-and-unflagged.
        c.batch_execute(
            "DO $$ DECLARE v_id uuid := uuidv7(); v_sb bytea; BEGIN \
               v_sb := ('sync-defer-' || v_id::text)::bytea; \
               INSERT INTO event_log (event_id, patient_id, event_type, schema_version, \
                 hlc_wall, hlc_counter, node_origin, signed_bytes, content_address, \
                 body, contributors, signer_key_id, plaintext_twin) \
               VALUES (v_id, v_id, 'sync.defer.probe', 'test-1', \
                 (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', v_sb, \
                 '\\x1220'::bytea || digest(v_sb, 'sha256'), \
                 '{}'::jsonb, '[]'::jsonb, 'test-key', 'probe'); \
               INSERT INTO event_deferred (event_id, event_type) \
                 VALUES (v_id, 'sync.defer.probe'); \
               INSERT INTO event_type_class (event_type, mode, targets_other_author) \
                 VALUES ('sync.defer.probe', 'additive', FALSE) ON CONFLICT DO NOTHING; \
             END $$;",
        )
        .unwrap();

        load_schema(&mut c).expect("replay must succeed");

        let kept: i64 = c
            .query_one("SELECT count(*) FROM event_deferred", &[])
            .unwrap()
            .get(0);
        assert_eq!(
            kept, 0,
            "this loader must re-adjudicate: a sync-only node that never promotes accumulates \
             admitted-but-permanently-powerless events with no mechanism to notice"
        );

        c.batch_execute(
            "TRUNCATE event_log CASCADE; \
             DELETE FROM event_type_class WHERE event_type = 'sync.defer.probe'",
        )
        .unwrap();
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-sync promotes_a_deferred -- --nocapture --test-threads=1
```

Expected: FAIL — `kept` is 1, nothing promoted it.

- [ ] **Step 3: Add the call**

In `crates/cairn-sync/src/main.rs`, in `load_schema_under_lock`, immediately after the `for (name, sql) in SCHEMA { ... }` loop and before the `if recorded != Some(embedded)` heal:

```rust
    // ADR-0056 decision 4 (#266) / PR #302 finding F3: RE-ADJUDICATE FIRST, REPROJECT
    // SECOND — the same pass, in the same position, as cairn-node's loader. This crate
    // carries db/020, the door that WRITES the event_deferred marker, so without this call
    // a sync-only database (the phone-tier carrier node the ADR exists for) accumulates
    // admitted-but-powerless events that nothing can ever promote.
    //
    // Safe to run unconditionally because db/043's gate 4 promotes only events that have
    // already projected cleanly: a bad event keeps its marker instead of taking the schema
    // load down with it. On THIS subset database gate 4 runs only the projections db/002
    // registers here, which is correct — the node projects what it knows how to project.
    client.execute("SELECT count(*) FROM cairn_readjudicate_deferred()", &[])?;
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test -p cairn-sync -- --nocapture --test-threads=1
```

Expected: PASS, the whole cairn-sync suite.

- [ ] **Step 5: Update the two SCHEMA-list comments that overstated what shipped**

Both `crates/cairn-node/src/db.rs:237-242` and `crates/cairn-sync/src/main.rs:133-138` claim db/043 must be in both lists — true, but they read as though shipping the file were sufficient. Append one sentence to each, adapting the wording to the file:

```rust
    // Shipping the file is necessary but not sufficient: the loader in this crate must also
    // CALL cairn_readjudicate_deferred, or the function sits unused and the markers pile up
    // anyway (PR #302 review finding F3).
```

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-sync/src/main.rs crates/cairn-node/src/db.rs
git commit -m "fix(#302-F3): cairn-sync's loader actually runs the pass

This crate embedded db/043 and the comment beside it argued correctly that
it must, because db/020 — also here — writes the event_deferred marker.
Only the function shipped. Nothing called it, so a sync-only database
accumulated admitted-but-permanently-powerless events with no mechanism to
notice.

Ordered after F1 deliberately: before gate 4 this call would have spread
the connect-wedge to the sync daemon.

Both SCHEMA-list comments now say shipping the file is necessary but not
sufficient."
```

---

### Task 8: Documentation currency and the full verification gate

**Files:**
- Modify: `docs/superpowers/specs/2026-07-29-adr-0056-admit-uninterpreted-floor-design.md` (§4 line ~80, §4.2 line ~124)
- Modify: `crates/cairn-node/tests/deferred_admission.rs` (the misleading assertion message in `a_travelling_token_survives_defer_then_promote`)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (Slice 58)

**Interfaces:**
- Consumes: everything above.
- Produces: nothing.

- [ ] **Step 1: Correct the predecessor design doc's two false claims**

In `docs/superpowers/specs/2026-07-29-adr-0056-admit-uninterpreted-floor-design.md`, replace the sentence beginning `The twin needs no work:` with:

```markdown
The twin needs work at PROMOTION, though not at admission — a claim this document originally got
wrong, and PR #302's review caught. At **admission** `cairn_event_twin` (db/005) finds no
`cairn_event_twin_check` row for an unregistered type, so `v_fn` and `v_msg` are both NULL and it
returns `cairn_twin_skeleton`; it never raises. But that reasoning was applied once and never
re-applied to **promotion**, when the registry row exists and the check *should* run. Left
unfixed, the type's `check_fn` and `twin_required_msg` were WAIVED rather than deferred, and the
resulting reprojection wedged `connect_and_load_schema` permanently. Now db/043's **gate 0**
re-runs it — see
[promotion must be proven](2026-07-29-adr-0056-promotion-must-be-proven-design.md). The
generalisable rule: every claim of the form *"X needs no work because the registry is empty"* has
a second lifetime in which the registry is no longer empty.
```

In §4.2, replace `Promotion deletes the marker, at which point the now-verified token counts normally.` with:

```markdown
Promotion deletes the marker — but that does **not** imply the token was verified, which this
document originally assumed. db/043's gate 1 verifies a token only when the type's mode demands
one, so an additive event bearing no responsibility is promoted with its token unchecked, and the
`event_deferred` proxy said "vouched" the instant the marker vanished. The exclusion is therefore
keyed on `event_attestation_unvouched`, a marker with the correct lifetime — see
[promotion must be proven](2026-07-29-adr-0056-promotion-must-be-proven-design.md) §3.
```

- [ ] **Step 2: Fix the misleading test assertion**

In `crates/cairn-node/tests/deferred_admission.rs`, in `a_travelling_token_survives_defer_then_promote`, replace the final assertion message `"the carried token must now VERIFY and promote the event"` with:

```rust
        "the carried token must SURVIVE defer→promote. Note this type is additive and bears \
         no responsibility, so no gate demands the token and nothing verifies it here — the \
         unvouched marker is what keeps it from counting as a vouch (PR #302 finding F2)",
```

- [ ] **Step 3: Run the full verification gate**

Run each, in order, and read the exit code of each — do **not** pipe to `tail`, which masks cargo's exit code:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  cargo test --workspace
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test" \
  ./scripts/run-db-sql-tests.sh
uv run --with-requirements docs/requirements.txt -- mkdocs build
```

Expected: all five exit 0. The workspace count should be **927 + 8 = 935 passed, 0 failed** (8 new tests: 2 in Task 1, 1 in Task 2, 1 in Task 3, 1 in Task 4, 1 in Task 5, 1 in Task 6, 1 in Task 7). If the count differs, reconcile it before proceeding — a missing test is a task that did not land.

**Run the workspace suite twice** against the same databases. The first run leaves projection state behind; a second green run is what proves replay safety and that no test poisons a shared database for its successors.

- [ ] **Step 4: Update HANDOVER and ROADMAP**

In `docs/HANDOVER.md`, in the Slice 58 session block, add a seventh carry-forward item after the existing six:

```markdown
7. **A review found the slice's own lesson applied one layer too shallow.** Item 3 above says
   *"when you store a value you have not verified, name the state and audit every reader"* — and
   the slice then used `event_deferred` as the name, which has the wrong lifetime: promotion
   deletes it, and gate 1 verifies a token only when the type's mode demands one. An additive
   event bearing no responsibility promoted with its token unchecked, and the gate re-opened.
   Separately, the same "re-adjudicate everything that was deferred" claim enumerated three
   gates when there were four — the per-type structural floor was waived, and the reprojection
   that followed **bricked the node**: three consecutive connects failed and the generation
   stamp never advanced. **Two rules:** a marker is only a valid proxy for a fact if it has the
   fact's lifetime; and a promotion must PROVE the event takes effect, never assume it. See
   ROADMAP Slice 58's review round.
```

In `docs/ROADMAP.md`, add a review-round paragraph to Slice 58 in the same style as Slice 57's, naming F1/F2/F3, the measured evidence, and the workspace count going 927→935.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-07-29-adr-0056-admit-uninterpreted-floor-design.md \
        docs/HANDOVER.md docs/ROADMAP.md \
        crates/cairn-node/tests/deferred_admission.rs
git commit -m "docs(ADR-0056): correct the two design claims that produced F1 and F2

'The twin needs no work' was true at admission and never re-asked for
promotion. 'Promotion deletes the marker, at which point the now-verified
token counts normally' assumed a verification that only happens when the
mode demands a token.

Both corrected in place with the reasoning failure recorded rather than
quietly overwritten. HANDOVER carries the two generalisable rules: a
marker is a valid proxy only if it has the fact's lifetime, and a
promotion must prove the event takes effect."
```

- [ ] **Step 6: Push and update the PR**

```bash
git push
gh pr comment 302 --body "Pushed fixes for the three review findings — see \`docs/superpowers/specs/2026-07-29-adr-0056-promotion-must-be-proven-design.md\`.

- **F1** \`cairn_readjudicate_deferred\` gains gate 0 (the per-type structural floor db/020 step 8 skipped) and gate 4 (run the type's heal-safe apply fns inside the promotion subtransaction). A promoted event is now, by construction, one that has already projected cleanly — so the loader's heal can no longer meet an event that wedges it. \`db.rs\` gets smaller: the targeted reproject is redundant.
- **F2** \`event_attestation_unvouched\` (db/001) names the carried-not-vouched state with the lifetime it actually needs. Three readers exclude on it: db/005's owner-gate (replacing the \`event_deferred\` proxy), db/018's link-veto flag lifecycle, db/034's attester_kid.
- **F3** cairn-sync's loader now calls the pass — ordered after F1, since before gate 4 it would have spread the wedge to the sync daemon.

Workspace 935/0; SQL mirrors, clippy, fmt, mkdocs all clean."
```

---

## Self-Review

**Spec coverage** — every section maps to a task: design §3 → Task 1; §3.1 → Tasks 2, 3, 4; §4 → Task 5; §5 → Task 6; §6 → Tasks 6 (loader) and 7 (cairn-sync); §7 → Task 8; §8 → the tests distributed across Tasks 1-7; §9 scope boundaries → no task (correctly, they are exclusions); §10 paper-parity → the Global Constraints heading.

**Type consistency** — `event_attestation_unvouched(event_id)` is created in Task 1 and read with the identical `EXISTS (SELECT 1 FROM event_attestation_unvouched u WHERE u.event_id = …)` shape in Tasks 2, 3, 4. `r.el_row` is introduced in Task 5 step 3 and consumed in Task 5 step 4 (`cairn_clear_payload`) and Task 6 step 4 (`EXECUTE … USING`). `cairn_readjudicate_deferred()` keeps `RETURNS TABLE(promoted_type text, promoted_count bigint)` throughout; Task 6's loader reads both columns, Task 7's caller reads neither.

**Ordering constraints, all load-bearing** — Task 1 before 2/3/4 (the table must exist). Task 5 before 6 (gate 4 consumes `r.el_row`). Task 6 before 7 (F3 would otherwise spread the wedge). Task 3's test is expected to fail *through* F1 until Task 6 lands, which is why Task 6 step 7 re-runs it.
