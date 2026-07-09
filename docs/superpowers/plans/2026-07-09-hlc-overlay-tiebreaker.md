# HLC-overlay deterministic tiebreaker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the silent cross-node divergence in the five uniform state-overlay projections by appending the event `content_address` as a deterministic final tiebreaker, applied once through a shared pure `cairn_hlc_overlay_wins()` predicate.

**Architecture:** Each standing-state overlay folds a new event in with an HLC-guarded upsert comparing `(hlc_wall, hlc_counter, origin)`. When two distinct events share that triple (a Byzantine/broken signer reusing its own triple), the strict-`>` guard is false both ways and the winner is decided by arrival order — two honest nodes diverge. We add the `content_address` (BYTEA multihash of the signed bytes: canonical, `UNIQUE`, byte-compared, collation-free) as the final tiebreaker via one shared `IMMUTABLE` SQL helper, and store it on each overlay row so the comparison has both sides. Projection-read-side only: no wire/event-format/floor-gate change.

**Tech Stack:** PostgreSQL 18 + `cairn_pgx` (PL/pgSQL + SQL migrations in `db/`, loaded by `crates/cairn-node/src/db.rs`), Rust integration tests (`tokio-postgres`) gated on `$CAIRN_TEST_PG`.

## Global Constraints

- **TDD** — failing test first, then the migration/code that makes it pass. No production SQL without a driving test.
- **Edit migrations in place** — pre-clinical posture (no deployed DBs; schema is recreated fresh per test via `db::connect_and_load_schema`). Add the new column to the `CREATE TABLE` body; **no `ALTER TABLE`**. Matches the #99/#152 hardening precedent.
- **No SCHEMA-array change** in `db.rs` (all in-place edits to already-loaded files).
- **No new event type, no wire/event-format change, no ADR/spec bump.** `content_address` already travels on every event; this only stores it in the projection and consults it.
- **Scope = Group A only** — the five uniform state overlays: `patient_chart` (db/002), `patient_link` (db/018), `chart_dispute` (db/023), `chart_identity_state` (db/024), `name_repudiation` (db/025). The demographic overlays (db/010–014) already converge modulo TEXT-collation and are **out of scope** (deferred to [#69](https://github.com/cairn-ehr/cairn-ehr/issues/69)).
- **Never hard-code cryptographic material in tests** (house rule 6) — test key material is runtime-derived via `generate_key()`. (Small non-crypto BYTEA literals like `'\x01'` used to exercise the pure predicate are digests-under-test, not key material, and are fine.)
- **Reviewer-legibility** — every non-trivial function/branch carries a comment explaining *why* for a junior contributor.
- Run the gated suite with `CAIRN_TEST_PG` set, e.g. `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`). Without it these tests self-skip.

---

### Task 1: The shared `cairn_hlc_overlay_wins()` predicate + pure unit tests

**Files:**
- Modify: `db/002_projection.sql` (insert the helper immediately after `BEGIN;`)
- Test: `crates/cairn-node/tests/overlay_tiebreaker.rs` (new)

**Interfaces:**
- Produces (SQL): `cairn_hlc_overlay_wins(new_wall bigint, new_counter int, new_origin text, new_addr bytea, cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea) RETURNS boolean` — `IMMUTABLE`, true iff the new event outranks the stored winner under `(wall, counter, origin, content_address)`, treating a null current side as "no winner yet".
- Produces (Rust test helpers, reused by later tasks): `cs() -> Option<String>`, `wins(&Client, i64, i32, &str, Vec<u8>, Option<i64>, Option<i32>, Option<&str>, Option<Vec<u8>>) -> bool`.

- [ ] **Step 1: Write the failing test** — create `crates/cairn-node/tests/overlay_tiebreaker.rs`:

```rust
//! Deterministic HLC-overlay tiebreaker (#115): the shared `cairn_hlc_overlay_wins()`
//! predicate, and arrival-order-independent convergence of the five state overlays when
//! two DISTINCT events share an identical (wall, counter, origin) triple (a Byzantine /
//! broken-signer collision). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Evaluate the pure predicate in the DB. `na`/`ca` are the content-address bytes (current
/// side nullable — the "no winner yet" case an overlay hits on its first insert).
async fn wins(
    c: &Client,
    nw: i64, nc: i32, no: &str, na: Vec<u8>,
    cw: Option<i64>, cc: Option<i32>, co: Option<&str>, ca: Option<Vec<u8>>,
) -> bool {
    c.query_one(
        "SELECT cairn_hlc_overlay_wins($1,$2,$3,$4,$5,$6,$7,$8)",
        &[&nw, &nc, &no, &na, &cw, &cc, &co, &ca],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn overlay_predicate_is_a_deterministic_total_order() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Higher wall wins regardless of everything after it.
    assert!(wins(&c, 2, 0, "a", vec![1], Some(1), Some(9), Some("z"), Some(vec![9])).await);
    // Lower wall loses regardless of everything after it.
    assert!(!wins(&c, 1, 9, "z", vec![9], Some(2), Some(0), Some("a"), Some(vec![1])).await);
    // Equal (wall, counter, origin): the content_address breaks the tie deterministically.
    assert!(wins(&c, 5, 3, "peer", vec![2], Some(5), Some(3), Some("peer"), Some(vec![1])).await);
    assert!(!wins(&c, 5, 3, "peer", vec![1], Some(5), Some(3), Some("peer"), Some(vec![2])).await);
    // A full tie (same address too) is NOT a win — strict-greater, so an idempotent re-apply
    // never churns the row.
    assert!(!wins(&c, 5, 3, "peer", vec![7], Some(5), Some(3), Some("peer"), Some(vec![7])).await);
    // No current winner yet (COALESCE path): a real event always beats the absent row.
    assert!(wins(&c, 0, 0, "", vec![0], None, None, None, None).await);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker overlay_predicate_is_a_deterministic_total_order -- --nocapture`
Expected: FAIL — Postgres error `function cairn_hlc_overlay_wins(...) does not exist`.

- [ ] **Step 3: Add the helper** — in `db/002_projection.sql`, replace:

```sql
BEGIN;

-- The projection Bet B times: one row per patient, kept current by overlay.
```

with:

```sql
BEGIN;

-- ── Shared overlay-winner predicate (#115) ──────────────────────────────────────────────
-- Every standing-state overlay (patient_chart below, plus patient_link/db-018,
-- chart_dispute/db-023, chart_identity_state/db-024, name_repudiation/db-025) folds a new
-- event in only when it OUTRANKS the stored winner. Ranking is the HLC total order (wall,
-- then counter, then origin) exactly as before — BUT a Byzantine or broken signer can reuse
-- its own (wall, counter, origin) triple across two DIFFERENT signed bodies. A plain
-- strict-`>` guard is then false in both directions, so the winner would be decided by
-- ARRIVAL ORDER and two honest nodes could converge to different standing state — a silent
-- cross-node divergence in the safety-critical projection layer, exactly what "sync = safe
-- set-union" must not allow. The event's content_address (the BYTEA multihash of its signed
-- bytes) is the deterministic final tiebreaker: canonical, UNIQUE, byte-compared (so it is
-- immune to the TEXT-collation concern that #69 tracks for the `origin` comparison), and
-- never shared by two distinct events. Appending it makes the overlay pick the SAME winner
-- on every node even under an HLC collision. COALESCE encodes "no current winner yet" (an
-- overlay's first insert, e.g. patient_chart's note-only row): -1 wall/counter and '' origin
-- sort below any real event, and an empty bytea sorts below any real \x1220… address, so a
-- real event always beats an absent one. Only the CURRENT side is COALESCEd — the NEW side is
-- always a real, fully-populated event.
CREATE OR REPLACE FUNCTION cairn_hlc_overlay_wins(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT (new_wall, new_counter, new_origin, new_addr)
         > (COALESCE(cur_wall, -1), COALESCE(cur_counter, -1),
            COALESCE(cur_origin, ''), COALESCE(cur_addr, '\x'::bytea));
$$;

-- The projection Bet B times: one row per patient, kept current by overlay.
```

- [ ] **Step 4: Run test to verify it passes**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker overlay_predicate_is_a_deterministic_total_order -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/002_projection.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "feat(floor): shared cairn_hlc_overlay_wins() overlay-winner predicate (#115)"
```

---

### Task 2: Route `patient_link` (db/018) through the helper + convergence test

**Files:**
- Modify: `db/018_identity_linkage.sql` (the `patient_link` table + `patient_link_apply()`)
- Test: `crates/cairn-node/tests/overlay_tiebreaker.rs` (append)

**Interfaces:**
- Consumes: `cairn_hlc_overlay_wins(...)` (Task 1).
- Produces (Rust test helpers, reused by later tasks): `db_msg`, `setup(&Client) -> (SigningKey, String, SigningKey, String)`, `apply(&Client, &[u8]) -> Result<u64, tokio_postgres::Error>`, `reset_between_orders(&Client)`, `link_event(kid, a, b, link: bool, wall, counter) -> EventBody`.

- [ ] **Step 1: Write the failing test** — append to `crates/cairn-node/tests/overlay_tiebreaker.rs`. First add the shared harness (imports at top of file, plus helpers), then the test:

Add these imports to the top of the file (merge with the existing `use` lines):

```rust
use cairn_event::identity::{link_assertion_body, render_link_twin, unlink_assertion_body,
    render_unlink_twin, LinkAssertion};
use cairn_event::{event_address, generate_key, sign, EventBody, Hlc, SigningKey};
use uuid::Uuid;
```

Add the shared harness helpers:

```rust
/// The Postgres RAISE text for a failed statement (Display renders only "db error"; the real
/// message lives in the DbError payload — project convention, see identity_linkage.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}

/// Truncate every clinical + Group-A overlay table and enroll one agent signer + one human
/// attester (distinct keys — the suppressing repudiation in Task 5 needs a human token).
/// Overlay tables from later migrations are truncated behind `to_regclass` guards so setup()
/// stays correct even on a DB migrated only to an earlier stage (the identity_*.rs pattern).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_link, person_member, \
         identity_projection_flag CASCADE",
    ).await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.name_repudiation') IS NOT NULL THEN TRUNCATE name_repudiation; END IF; \
         END $$;",
    ).await.unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0").await.unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"tb-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute("SELECT enroll_actor('human', '{\"role\":\"records-officer\"}', $1)", &[&kid_h])
        .await.unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Apply one validly-signed remote event through the in-DB clinical apply door (db/020),
/// which takes the wire HLC verbatim — the ONLY door that lets two events carry a colliding
/// (wall, counter, origin) triple, and the realistic foreign-node scenario.
async fn apply(c: &Client, signed: &[u8]) -> Result<u64, tokio_postgres::Error> {
    c.execute("SELECT apply_remote_event($1)", &[&signed.to_vec()]).await
}

/// Clean the event log + projections between the two arrival orders WITHOUT dropping the
/// actor enrollment (re-running setup() would mint new keys and un-enroll the pre-signed
/// events). hlc_state is reset so the local merge does not carry over.
async fn reset_between_orders(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, patient_chart, patient_identifier, patient_demographic, \
         patient_name, patient_link, person_member, identity_projection_flag CASCADE",
    ).await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.name_repudiation') IS NOT NULL THEN TRUNCATE name_repudiation; END IF; \
         END $$;",
    ).await.unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0").await.unwrap();
}

/// A signed link (or unlink) for the SAME (a, b) pair at a chosen HLC triple. link vs unlink
/// changes the event_type (and event_id), so the two events differ in signed bytes ⇒ differ
/// in content_address ⇒ they collide on (wall, counter, origin) but never on the tiebreak.
fn link_event(kid: &str, a: Uuid, b: Uuid, link: bool, wall: i64, counter: i32) -> EventBody {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let la = LinkAssertion { subject_a: &a_s, subject_b: &b_s, provenance: "tb:conv", confidence: None };
    let (etype, payload, twin) = if link {
        ("identity.link.asserted", link_assertion_body(&la), render_link_twin(&la))
    } else {
        ("identity.unlink.asserted", unlink_assertion_body(&la), render_unlink_twin(&la))
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: etype.into(),
        schema_version: "identity.link/1".into(),
        hlc: Hlc { wall, counter, node_origin: "peer".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}
```

Then the test:

```rust
#[tokio::test]
async fn patient_link_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // Two distinct events at an IDENTICAL (wall, counter, origin): a link and an unlink of the
    // same pair. Winner MUST be the higher content_address, both arrival orders.
    let e_link = sign(&link_event(&kid, a, b, true, 5000, 7), &sk).unwrap().signed_bytes;
    let e_unlink = sign(&link_event(&kid, a, b, false, 5000, 7), &sk).unwrap().signed_bytes;
    let expect = if event_address(&e_unlink) > event_address(&e_link) { "unlink" } else { "link" };
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };

    apply(&c, &e_link).await.expect("link applies");
    apply(&c, &e_unlink).await.expect("unlink applies");
    let state1: String = c
        .query_one("SELECT state FROM patient_link WHERE low = $1 AND high = $2", &[&lo, &hi])
        .await.unwrap().get(0);

    reset_between_orders(&c).await;
    apply(&c, &e_unlink).await.expect("unlink applies");
    apply(&c, &e_link).await.expect("link applies");
    let state2: String = c
        .query_one("SELECT state FROM patient_link WHERE low = $1 AND high = $2", &[&lo, &hi])
        .await.unwrap().get(0);

    assert_eq!(state1, state2, "arrival order must not change the winner (#115)");
    assert_eq!(state1, expect, "winner is the higher content_address, deterministically");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker patient_link_converges_under_hlc_collision -- --nocapture`
Expected: FAIL on `assert_eq!(state1, state2, …)` — the un-patched overlay keeps the first-applied event (arrival-order dependent), so `state1 == "link"` while `state2 == "unlink"`.

- [ ] **Step 3: Patch db/018** — three edits in `db/018_identity_linkage.sql`.

Edit A — add the column to the `patient_link` table. Replace:

```sql
    provenance  TEXT    NOT NULL,
    confidence  TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high),
```

with:

```sql
    provenance  TEXT    NOT NULL,
    confidence  TEXT,
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high),
```

Edit B — carry `content_address` through the upsert and route the guard through the helper. Replace:

```sql
    INSERT INTO patient_link
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, confidence)
    VALUES
        (lo, hi, v_state, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
         p ->> 'provenance', p ->> 'confidence')
    ON CONFLICT (low, high) DO UPDATE SET
        state       = EXCLUDED.state,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        provenance  = EXCLUDED.provenance,
        confidence  = EXCLUDED.confidence,
        updated_at  = clock_timestamp()
    WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin)
        > (patient_link.hlc_wall, patient_link.hlc_counter, patient_link.origin);
```

with:

```sql
    INSERT INTO patient_link
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, confidence,
         content_address)
    VALUES
        (lo, hi, v_state, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
         p ->> 'provenance', p ->> 'confidence', NEW.content_address)
    ON CONFLICT (low, high) DO UPDATE SET
        state       = EXCLUDED.state,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        provenance  = EXCLUDED.provenance,
        confidence  = EXCLUDED.confidence,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- Overlay only when the incoming event outranks the stored winner, with content_address
    -- as the deterministic final tiebreaker (#115) so an HLC-triple collision converges.
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        patient_link.hlc_wall, patient_link.hlc_counter, patient_link.origin,
        patient_link.content_address);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker patient_link_converges_under_hlc_collision -- --nocapture`
Expected: PASS. Also run `cargo test -p cairn-node --test identity_linkage` — expected PASS (no regression; the added column is populated on every insert).

- [ ] **Step 5: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "fix(floor): patient_link overlay converges under HLC collision via tiebreaker (#115)"
```

---

### Task 3: Route `chart_dispute` (db/023) through the helper + convergence test

**Files:**
- Modify: `db/023_identity_dispute.sql` (the `chart_dispute` table + `chart_dispute_apply()`)
- Test: `crates/cairn-node/tests/overlay_tiebreaker.rs` (append)

**Interfaces:**
- Consumes: `cairn_hlc_overlay_wins(...)`, `setup`, `apply`, `reset_between_orders` (Tasks 1–2).
- Produces: `dispute_event(kid, dispute_id, subject, open: bool, descriptive, wall, counter) -> EventBody`.

- [ ] **Step 1: Write the failing test** — append to `crates/cairn-node/tests/overlay_tiebreaker.rs`.

Add imports (merge with existing `use cairn_event::identity::{…}`):

```rust
use cairn_event::identity::{dispute_assertion_body, dispute_resolution_body,
    render_dispute_resolved_twin, render_dispute_twin, DisputeAssertion, DisputeResolution};
```

Add the builder + test:

```rust
/// A signed dispute-open (or dispute-resolve) for the SAME dispute_id + subject at a chosen
/// HLC triple. open vs resolve changes the event_type (and event_id) ⇒ different signed
/// bytes ⇒ different content_address; they collide on (wall, counter, origin) only.
fn dispute_event(
    kid: &str, dispute_id: Uuid, subject: Uuid, open: bool, descriptive: &str, wall: i64, counter: i32,
) -> EventBody {
    let did = dispute_id.to_string();
    let subj = subject.to_string();
    let (etype, payload, twin, sver) = if open {
        let d = DisputeAssertion { dispute_id: &did, subject: &subj, reason: descriptive };
        ("identity.dispute.asserted", dispute_assertion_body(&d), render_dispute_twin(&d),
         "identity.dispute.asserted/1")
    } else {
        let d = DisputeResolution { dispute_id: &did, subject: &subj, resolution: descriptive };
        ("identity.dispute.resolved", dispute_resolution_body(&d), render_dispute_resolved_twin(&d),
         "identity.dispute.resolved/1")
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subj.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
        hlc: Hlc { wall, counter, node_origin: "peer".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

#[tokio::test]
async fn chart_dispute_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let (did, subj) = (Uuid::now_v7(), Uuid::now_v7());
    let e_open = sign(&dispute_event(&kid, did, subj, true, "claims never here", 5000, 7), &sk)
        .unwrap().signed_bytes;
    let e_resolved = sign(&dispute_event(&kid, did, subj, false, "confirmed present", 5000, 7), &sk)
        .unwrap().signed_bytes;
    let expect = if event_address(&e_resolved) > event_address(&e_open) { "resolved" } else { "open" };

    apply(&c, &e_open).await.expect("open applies");
    apply(&c, &e_resolved).await.expect("resolve applies");
    let s1: String = c.query_one("SELECT state FROM chart_dispute WHERE dispute_id = $1", &[&did])
        .await.unwrap().get(0);

    reset_between_orders(&c).await;
    apply(&c, &e_resolved).await.expect("resolve applies");
    apply(&c, &e_open).await.expect("open applies");
    let s2: String = c.query_one("SELECT state FROM chart_dispute WHERE dispute_id = $1", &[&did])
        .await.unwrap().get(0);

    assert_eq!(s1, s2, "a dispute must not settle open-vs-resolved by arrival order (#115)");
    assert_eq!(s1, expect, "winner is the higher content_address, deterministically");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker chart_dispute_converges_under_hlc_collision -- --nocapture`
Expected: FAIL on `assert_eq!(s1, s2, …)` — un-patched overlay is arrival-order dependent (`s1 == "open"`, `s2 == "resolved"`).

- [ ] **Step 3: Patch db/023** — two edits in `db/023_identity_dispute.sql`.

Edit A — add the column. Replace:

```sql
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON chart_dispute TO cairn_agent;
```

with:

```sql
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON chart_dispute TO cairn_agent;
```

Edit B — carry `content_address` + route the guard. Replace:

```sql
    INSERT INTO chart_dispute
        (dispute_id, subject, state, detail, hlc_wall, hlc_counter, origin)
    VALUES
        ((p ->> 'dispute_id')::uuid, (p ->> 'subject')::uuid, v_state, v_detail,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    ON CONFLICT (dispute_id) DO UPDATE SET
        subject     = EXCLUDED.subject,
        state       = EXCLUDED.state,
        detail      = EXCLUDED.detail,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        updated_at  = clock_timestamp()
    WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin)
        > (chart_dispute.hlc_wall, chart_dispute.hlc_counter, chart_dispute.origin);
```

with:

```sql
    INSERT INTO chart_dispute
        (dispute_id, subject, state, detail, hlc_wall, hlc_counter, origin, content_address)
    VALUES
        ((p ->> 'dispute_id')::uuid, (p ->> 'subject')::uuid, v_state, v_detail,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (dispute_id) DO UPDATE SET
        subject     = EXCLUDED.subject,
        state       = EXCLUDED.state,
        detail      = EXCLUDED.detail,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- content_address is the deterministic final tiebreaker (#115) so an HLC-triple collision
    -- settles open-vs-resolved identically on every node.
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        chart_dispute.hlc_wall, chart_dispute.hlc_counter, chart_dispute.origin,
        chart_dispute.content_address);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker chart_dispute_converges_under_hlc_collision -- --nocapture`
Expected: PASS. Also `cargo test -p cairn-node --test identity_dispute` — expected PASS (no regression).

- [ ] **Step 5: Commit**

```bash
git add db/023_identity_dispute.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "fix(floor): chart_dispute overlay converges under HLC collision via tiebreaker (#115)"
```

---

### Task 4: Route `chart_identity_state` (db/024) through the helper + convergence test

**Files:**
- Modify: `db/024_identity_identify.sql` (the `chart_identity_state` table + `chart_identity_state_apply()`)
- Test: `crates/cairn-node/tests/overlay_tiebreaker.rs` (append)

**Interfaces:**
- Consumes: `cairn_hlc_overlay_wins(...)`, `setup`, `apply`, `reset_between_orders`.
- Produces: `identity_state_event(kid, subject, identified: bool, descriptive, wall, counter) -> EventBody`.

- [ ] **Step 1: Write the failing test** — append to `crates/cairn-node/tests/overlay_tiebreaker.rs`.

Add imports (merge with existing identity `use`):

```rust
use cairn_event::identity::{identify_assertion_body, pending_assertion_body,
    render_identify_twin, render_pending_twin, IdentifyAssertion, PendingAssertion};
```

> Field names are verified against `crates/cairn-event/src/identity.rs:151-179`: `PendingAssertion { subject, basis }` → `pending_assertion_body` / `render_pending_twin`; `IdentifyAssertion { subject, method }` → `identify_assertion_body` / `render_identify_twin`. The builder below uses them directly.

Add the builder + test:

```rust
/// A signed identity-pending (or identify) for the SAME subject at a chosen HLC triple.
/// pending vs identify changes the event_type (and event_id) ⇒ different content_address.
fn identity_state_event(
    kid: &str, subject: Uuid, identified: bool, descriptive: &str, wall: i64, counter: i32,
) -> EventBody {
    let subj = subject.to_string();
    let (etype, payload, twin, sver) = if identified {
        let a = IdentifyAssertion { subject: &subj, method: descriptive };
        ("identity.identify.asserted", identify_assertion_body(&a), render_identify_twin(&a),
         "identity.identify.asserted/1")
    } else {
        let a = PendingAssertion { subject: &subj, basis: descriptive };
        ("identity.pending.asserted", pending_assertion_body(&a), render_pending_twin(&a),
         "identity.pending.asserted/1")
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subj.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
        hlc: Hlc { wall, counter, node_origin: "peer".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

#[tokio::test]
async fn chart_identity_state_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let subj = Uuid::now_v7();
    let e_pending = sign(&identity_state_event(&kid, subj, false, "unconscious ED arrival", 5000, 7), &sk)
        .unwrap().signed_bytes;
    let e_identified = sign(&identity_state_event(&kid, subj, true, "photo id matched", 5000, 7), &sk)
        .unwrap().signed_bytes;
    let expect = if event_address(&e_identified) > event_address(&e_pending) { "identified" } else { "pending" };

    apply(&c, &e_pending).await.expect("pending applies");
    apply(&c, &e_identified).await.expect("identify applies");
    let s1: String = c.query_one("SELECT state FROM chart_identity_state WHERE subject = $1", &[&subj])
        .await.unwrap().get(0);

    reset_between_orders(&c).await;
    apply(&c, &e_identified).await.expect("identify applies");
    apply(&c, &e_pending).await.expect("pending applies");
    let s2: String = c.query_one("SELECT state FROM chart_identity_state WHERE subject = $1", &[&subj])
        .await.unwrap().get(0);

    assert_eq!(s1, s2, "identity-pending vs identified must not flip by arrival order (#115)");
    assert_eq!(s1, expect, "winner is the higher content_address, deterministically");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker chart_identity_state_converges_under_hlc_collision -- --nocapture`
Expected: FAIL on `assert_eq!(s1, s2, …)` (arrival-order dependent before the patch).

- [ ] **Step 3: Patch db/024** — two edits in `db/024_identity_identify.sql`.

Edit A — add the column. Replace:

```sql
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON chart_identity_state TO cairn_agent;
```

with:

```sql
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON chart_identity_state TO cairn_agent;
```

Edit B — carry `content_address` + route the guard. Replace:

```sql
    INSERT INTO chart_identity_state
        (subject, state, detail, hlc_wall, hlc_counter, origin)
    VALUES
        ((p ->> 'subject')::uuid, v_state, v_detail,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    ON CONFLICT (subject) DO UPDATE SET
        state       = EXCLUDED.state,
```

with:

```sql
    INSERT INTO chart_identity_state
        (subject, state, detail, hlc_wall, hlc_counter, origin, content_address)
    VALUES
        ((p ->> 'subject')::uuid, v_state, v_detail,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (subject) DO UPDATE SET
        state       = EXCLUDED.state,
```

Then replace:

```sql
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        updated_at  = clock_timestamp()
    WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin)
        > (chart_identity_state.hlc_wall, chart_identity_state.hlc_counter, chart_identity_state.origin);
```

with:

```sql
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- content_address is the deterministic final tiebreaker (#115) so an HLC-triple collision
    -- converges pending-vs-identified identically on every node.
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        chart_identity_state.hlc_wall, chart_identity_state.hlc_counter,
        chart_identity_state.origin, chart_identity_state.content_address);
```

> The `DO UPDATE SET` detail line between the two blocks (`detail = EXCLUDED.detail,`) is left untouched — the two replacements bracket it.

- [ ] **Step 4: Run test to verify it passes**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker chart_identity_state_converges_under_hlc_collision -- --nocapture`
Expected: PASS. Also `cargo test -p cairn-node --test identity_identify` — expected PASS.

- [ ] **Step 5: Commit**

```bash
git add db/024_identity_identify.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "fix(floor): chart_identity_state overlay converges under HLC collision (#115)"
```

---

### Task 5: Route `name_repudiation` (db/025) through the helper + attested convergence test

**Files:**
- Modify: `db/025_identity_repudiate.sql` (the `name_repudiation` table + `name_repudiation_apply()`, incl. the stale `#69`/"add content_address" comment)
- Test: `crates/cairn-node/tests/overlay_tiebreaker.rs` (append)

**Interfaces:**
- Consumes: `cairn_hlc_overlay_wins(...)`, `setup` (returns the human attester), `reset_between_orders`.
- Produces: `repudiation_event(kid, subject, value, reason, wall, counter) -> EventBody`, `apply_attested(&Client, &[u8], &[u8], &[u8]) -> Result<u64, tokio_postgres::Error>`.

`name_repudiation` is `mode='suppressing'`, so the floor always demands a human attestation token — the two colliding events are applied through the 3-arg apply door with per-event tokens.

- [ ] **Step 1: Write the failing test** — append to `crates/cairn-node/tests/overlay_tiebreaker.rs`.

Add imports:

```rust
use cairn_event::identity::{render_repudiate_twin, repudiation_assertion_body, RepudiationAssertion};
use cairn_event::sign_attestation;
```

Add the helpers + test:

```rust
/// A signed repudiation of (subject, value). Two repudiations of the SAME (subject, value)
/// with different `reason` differ in signed bytes ⇒ different content_address, colliding on
/// (wall, counter, origin) only — the #115 case for the suppressing overlay.
fn repudiation_event(
    kid: &str, subject: Uuid, value: &str, reason: &str, wall: i64, counter: i32,
) -> EventBody {
    let subj = subject.to_string();
    let a = RepudiationAssertion { subject: &subj, value, reason };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subj.clone(),
        event_type: "identity.repudiate.asserted".into(),
        schema_version: "identity.repudiate.asserted/1".into(),
        hlc: Hlc { wall, counter, node_origin: "peer".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: repudiation_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_repudiate_twin(&a)),
    }
}

/// Apply a validly-signed suppressing event through the attested apply door (db/020, 3-arg):
/// signed bytes + a human attestation token over its content_address + the attester's key.
async fn apply_attested(
    c: &Client, signed: &[u8], token: &[u8], hkey: &[u8],
) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.to_vec(), &token.to_vec(), &hkey.to_vec()],
    ).await
}

#[tokio::test]
async fn name_repudiation_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = setup(&c).await;
    let hkey = hex::decode(&kid_h).unwrap();

    let subj = Uuid::now_v7();
    let value = "Fabricated Persona";
    // Two repudiations of the same struck name at an IDENTICAL HLC triple; the winning `reason`
    // must be the higher content_address either arrival order.
    let e1 = sign(&repudiation_event(&kid, subj, value, "reason-one", 5000, 7), &sk).unwrap().signed_bytes;
    let e2 = sign(&repudiation_event(&kid, subj, value, "reason-two", 5000, 7), &sk).unwrap().signed_bytes;
    let t1 = sign_attestation(&event_address(&e1), &kid_h, "attested", &sk_h).unwrap();
    let t2 = sign_attestation(&event_address(&e2), &kid_h, "attested", &sk_h).unwrap();
    let expect = if event_address(&e2) > event_address(&e1) { "reason-two" } else { "reason-one" };

    apply_attested(&c, &e1, &t1, &hkey).await.expect("repudiation one applies");
    apply_attested(&c, &e2, &t2, &hkey).await.expect("repudiation two applies");
    let r1: String = c
        .query_one("SELECT reason FROM name_repudiation WHERE subject = $1 AND value = $2", &[&subj, &value])
        .await.unwrap().get(0);

    reset_between_orders(&c).await;
    apply_attested(&c, &e2, &t2, &hkey).await.expect("repudiation two applies");
    apply_attested(&c, &e1, &t1, &hkey).await.expect("repudiation one applies");
    let r2: String = c
        .query_one("SELECT reason FROM name_repudiation WHERE subject = $1 AND value = $2", &[&subj, &value])
        .await.unwrap().get(0);

    assert_eq!(r1, r2, "a repudiation must not settle by arrival order (#115)");
    assert_eq!(r1, expect, "winner is the higher content_address, deterministically");
}
```

Add `hex` to `crates/cairn-node/Cargo.toml` `[dev-dependencies]` only if not already present (the existing `apply_remote_event.rs` uses `hex::decode`, so it is already there — confirm before adding).

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker name_repudiation_converges_under_hlc_collision -- --nocapture`
Expected: FAIL on `assert_eq!(r1, r2, …)` (arrival-order dependent before the patch).

- [ ] **Step 3: Patch db/025** — two edits in `db/025_identity_repudiate.sql`.

Edit A — add the column. Replace:

```sql
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (subject, value)
```

with:

```sql
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (subject, value)
```

Edit B — carry `content_address`, route the guard, and update the now-stale comment. Replace:

```sql
    INSERT INTO name_repudiation
        (subject, value, reason, hlc_wall, hlc_counter, origin)
    VALUES
        ((p ->> 'subject')::uuid, p ->> 'value', p ->> 'reason',
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    ON CONFLICT (subject, value) DO UPDATE SET
        reason      = EXCLUDED.reason,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        updated_at  = clock_timestamp()
    -- The (wall, counter, origin) tuple mirrors db/012/024; `origin` is TEXT, so it carries
    -- the same collation-sensitivity tracked by #69 and the same "add content_address as a
```

with:

```sql
    INSERT INTO name_repudiation
        (subject, value, reason, hlc_wall, hlc_counter, origin, content_address)
    VALUES
        ((p ->> 'subject')::uuid, p ->> 'value', p ->> 'reason',
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (subject, value) DO UPDATE SET
        reason      = EXCLUDED.reason,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- content_address is the deterministic final tiebreaker (#115): a Byzantine (wall,counter,
    -- origin) collision now converges to the same winner on every node. The remaining TEXT
    -- collation-sensitivity of the intermediate `origin` comparison stays tracked by #69 (a
```

> Then read the two or three lines that follow in `db/025_identity_repudiate.sql` (the tail of the original `#69`/"add content_address" comment) and trim them so the comment reads coherently — the "as a [final tiebreak]" clause is now done, not pending. Keep the `WHERE (...) > (...)` predicate replacement: replace the `WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin) > (name_repudiation.hlc_wall, name_repudiation.hlc_counter, name_repudiation.origin);` line with:

```sql
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        name_repudiation.hlc_wall, name_repudiation.hlc_counter, name_repudiation.origin,
        name_repudiation.content_address);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker name_repudiation_converges_under_hlc_collision -- --nocapture`
Expected: PASS. Also `cargo test -p cairn-node --test identity_repudiate` — expected PASS (no regression).

- [ ] **Step 5: Commit**

```bash
git add db/025_identity_repudiate.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "fix(floor): name_repudiation overlay converges under HLC collision (#115)"
```

---

### Task 6: Route `patient_chart` (db/002, CASE-form) through the helper + convergence test

**Files:**
- Modify: `db/002_projection.sql` (the `patient_chart` table + `patient_chart_apply()`)
- Test: `crates/cairn-node/tests/overlay_tiebreaker.rs` (append)

**Interfaces:**
- Consumes: `cairn_hlc_overlay_wins(...)`, `setup`, `apply`, `reset_between_orders`.
- Produces: `amended_event(kid, patient, name, wall, counter) -> EventBody`.

`patient_chart` uses the inline `CASE`-form (not a `WHERE`-guard) and stores demographic provenance in `demo_hlc_wall/demo_hlc_count/demo_origin`; its overlay column is `demo_content_address` and it is **nullable** (a `note.added` row inserts no demographic winner). The helper's `COALESCE(cur_addr, '\x')` handles that null.

- [ ] **Step 1: Write the failing test** — append to `crates/cairn-node/tests/overlay_tiebreaker.rs`:

```rust
/// A signed patient.amended carrying a `name` at a chosen HLC triple. Two amendments with
/// different names differ in content_address, colliding on (wall, counter, origin) only.
fn amended_event(kid: &str, patient: Uuid, name: &str, wall: i64, counter: i32) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "patient.amended".into(),
        schema_version: "patient/1".into(),
        hlc: Hlc { wall, counter, node_origin: "peer".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name": name}),
        attachments: vec![],
        plaintext_twin: Some(format!("patient amended: {name}")),
    }
}

#[tokio::test]
async fn patient_chart_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let p = Uuid::now_v7();
    let e_a = sign(&amended_event(&kid, p, "Alice A", 5000, 7), &sk).unwrap().signed_bytes;
    let e_b = sign(&amended_event(&kid, p, "Bob B", 5000, 7), &sk).unwrap().signed_bytes;
    let expect = if event_address(&e_b) > event_address(&e_a) { "Bob B" } else { "Alice A" };

    apply(&c, &e_a).await.expect("amend A applies");
    apply(&c, &e_b).await.expect("amend B applies");
    let n1: String = c.query_one("SELECT name FROM patient_chart WHERE patient_id = $1", &[&p])
        .await.unwrap().get(0);

    reset_between_orders(&c).await;
    apply(&c, &e_b).await.expect("amend B applies");
    apply(&c, &e_a).await.expect("amend A applies");
    let n2: String = c.query_one("SELECT name FROM patient_chart WHERE patient_id = $1", &[&p])
        .await.unwrap().get(0);

    assert_eq!(n1, n2, "the demographic winner must not flip by arrival order (#115)");
    assert_eq!(n1, expect, "winner is the higher content_address, deterministically");
}
```

> If `apply` rejects `patient.amended` at the door (e.g. a schema_version or twin requirement), read `db/005_submit.sql:19-20` (the type is registered `additive`) and `crates/cairn-node/tests/apply_remote_event.rs:65-84` for the exact `note.added` envelope shape, and mirror it — `patient.amended` is a non-demographic additive type that degrades honestly on the twin floor (db/015), so a plain `plaintext_twin` as above is accepted.

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker patient_chart_converges_under_hlc_collision -- --nocapture`
Expected: FAIL on `assert_eq!(n1, n2, …)` (arrival-order dependent before the patch).

- [ ] **Step 3: Patch db/002** — three edits in `db/002_projection.sql`.

Edit A — add the nullable column after `demo_origin`. Replace:

```sql
    demo_hlc_wall  BIGINT,
    demo_hlc_count INTEGER,
    demo_origin    TEXT,
    note_count     INTEGER     NOT NULL DEFAULT 0,
```

with:

```sql
    demo_hlc_wall  BIGINT,
    demo_hlc_count INTEGER,
    demo_origin    TEXT,
    demo_content_address BYTEA,   -- winning demographic event's content address; #115 tiebreak
                                  -- (nullable: a note-only row has no demographic winner yet)
    note_count     INTEGER     NOT NULL DEFAULT 0,
```

Edit B — carry `demo_content_address` on the INSERT. Replace:

```sql
        INSERT INTO patient_chart AS pc (
            patient_id, name, dob, sex,
            demo_hlc_wall, demo_hlc_count, demo_origin,
            last_activity, updated_at)
        VALUES (
            NEW.patient_id,
            NEW.body ->> 'name', NEW.body ->> 'dob', NEW.body ->> 'sex',
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.recorded_at, clock_timestamp())
```

with:

```sql
        INSERT INTO patient_chart AS pc (
            patient_id, name, dob, sex,
            demo_hlc_wall, demo_hlc_count, demo_origin, demo_content_address,
            last_activity, updated_at)
        VALUES (
            NEW.patient_id,
            NEW.body ->> 'name', NEW.body ->> 'dob', NEW.body ->> 'sex',
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            NEW.recorded_at, clock_timestamp())
```

Edit C — route every `CASE` condition through the helper and set `demo_content_address` on win. First, `replace_all` the repeated condition string. Replace (all occurrences):

```sql
CASE WHEN (NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
                                     > (COALESCE(pc.demo_hlc_wall,-1), COALESCE(pc.demo_hlc_count,-1), COALESCE(pc.demo_origin,''))
```

with (all occurrences):

```sql
CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
```

Then add the `demo_content_address` maintenance line. Replace:

```sql
            demo_hlc_wall  = GREATEST(pc.demo_hlc_wall, NEW.hlc_wall),
```

with:

```sql
            demo_content_address = CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN NEW.content_address ELSE pc.demo_content_address END,
            demo_hlc_wall  = GREATEST(pc.demo_hlc_wall, NEW.hlc_wall),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test overlay_tiebreaker patient_chart_converges_under_hlc_collision -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/002_projection.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "fix(floor): patient_chart overlay converges under HLC collision (#115)"
```

---

### Task 7: Full-suite green, docs, and PR

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`
- No code change (verification + docs only)

- [ ] **Step 1: Whole-workspace gates** — run the exact CI gates:

```bash
cargo fmt --all --check
cargo clippy --workspace --tests -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
```
Expected: fmt clean; clippy 0 warnings; all tests pass (the new `overlay_tiebreaker` file's 6 tests + no regressions in `identity_linkage` / `identity_dispute` / `identity_identify` / `identity_repudiate` / `apply_remote_event` / `demographics*`).

- [ ] **Step 2: Docs build** — confirm the docs still build (no spec/ADR change, but keep the habit):

```bash
uv run --with-requirements docs/requirements.txt -- mkdocs build
```
Expected: builds clean.

- [ ] **Step 3: Update HANDOVER.md** — add a concise "This session" entry at the top summarizing: #115 part 1 + the HLC-overlay bullet of part 2 done; shared `cairn_hlc_overlay_wins()` predicate; `content_address` final tiebreaker on the five Group-A state overlays (db/002/018/023/024/025); demographic overlays (db/010–014) deferred to #69; the convergence-via-remote-apply-door test file; no wire/ADR/spec change. Move the older top entry down / prune to keep the file focused (target ≤ 500 lines).

- [ ] **Step 4: Update ROADMAP.md** — in Phase 2 (in-DB enforcement floor) or the identity/projection area, note the deterministic overlay tiebreaker is now enforced via the shared helper, closing #115 part 1; #115's remaining part-2 items (twin-ladder registry, `cairn_require_uuid`) and #69 (TEXT-collation) stay open.

- [ ] **Step 5: Commit docs**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — HLC-overlay deterministic tiebreaker done (#115)"
```

- [ ] **Step 6: Push + open PR** — link #115, note scope (Group A; #69 + part-2 remainder deferred), and that it is projection-read-side only (no wire/ADR/spec change):

```bash
git push -u origin refactor/hlc-overlay-tiebreaker-115
gh pr create --base main --title "fix(floor): deterministic HLC-overlay tiebreaker via shared helper (#115)" \
  --body "$(cat <<'EOF'
Closes #115 part 1 (+ the HLC-overlay bullet of part 2).

## What
Appends the event `content_address` (BYTEA multihash — canonical, UNIQUE, collation-free) as a deterministic final tiebreaker to the overlay-winner comparison, via one shared pure `cairn_hlc_overlay_wins()` predicate, on the five uniform state overlays: `patient_chart` (db/002), `patient_link` (db/018), `chart_dispute` (db/023), `chart_identity_state` (db/024), `name_repudiation` (db/025).

## Why
When two distinct events share an identical `(hlc_wall, hlc_counter, origin)` triple (a Byzantine/broken signer reusing its own triple), the old strict-`>` guard was false both ways, so the winner was decided by arrival order — two honest nodes could converge to different standing state (silent cross-node divergence in the safety-critical projection layer; clinician-visible for `chart_dispute`). The content_address tiebreaker makes the winner deterministic on every node.

## Scope
- Group A (the five state overlays) only. The demographic overlays (db/010–014) already converge modulo TEXT-collation and are deferred to #69.
- Projection-read-side only: no wire/event-format/floor-gate change, no new event type, no ADR/spec bump. `content_address` already travels on every event.
- #115 part-2 remainder (twin-ladder registry, `cairn_require_uuid`) remains open.

## Tests
New `crates/cairn-node/tests/overlay_tiebreaker.rs`: a pure-predicate unit test + five convergence tests that apply two HLC-colliding events through the remote-apply door in both arrival orders and assert the same deterministic winner. `name_repudiation` (suppressing) is exercised through the attested apply door.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Notes for the implementer

- **Where the helper lives:** `db/002_projection.sql`, right after `BEGIN;`. It is the first projection migration, so every later overlay (db/018/023/024/025) sees the function at load time. Do not rely on deferred name-resolution across files being "fine" — defining it first makes the dependency explicit.
- **`content_address` availability:** the overlay triggers fire `AFTER INSERT ON event_log`, whose row has `content_address BYTEA NOT NULL` (db/001, `\x1220 ‖ sha256(signed_bytes)`), so `NEW.content_address` is always present and correct.
- **Why the remote-apply door in tests:** `submit_event` assigns a fresh monotonic HLC (`node_hlc_tick`), so it cannot produce a colliding triple. `apply_remote_event` takes the wire HLC verbatim — the only way to stage the collision, and the realistic foreign-node case.
- **`reset_between_orders` keeps `actor_event`:** the pre-signed test events are signed by the enrolled key; re-running `setup()` would mint a new key and un-enroll them. Only the event log + projections are truncated between the two arrival orders.
- **Out of scope (do not touch):** db/010–014 demographic overlays and their display views; #69 (TEXT-collation); #115 part-2's twin-ladder registry and `cairn_require_uuid`.
```
