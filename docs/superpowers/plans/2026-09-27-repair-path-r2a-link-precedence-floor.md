# Repair path R2a — a human's link judgement outranks a machine's, and the node can author one — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A human-attested `link`/`unlink` can no longer be displaced by an un-attested one (ADR-0076 decision 5, in the database, at both doors), existing databases re-decide every pair under that order on upgrade, and `cairn-node` gains `link_charts`/`unlink_charts` (library + CLI) — the orchestration R2b's window gesture will call.

**Architecture:** `patient_link` gains an `attested` column holding the ONE definition db/018 already uses (`e.attester_key IS NOT NULL AND cairn_attestation_vouched(e.event_id)`), computed once per applied event. A new pure comparator `cairn_link_overlay_wins` ranks *attested first*, then defers to the existing `cairn_hlc_overlay_wins` — still a total order, so still convergent. A new migration `db/055` bumps `SCHEMA_GENERATION` to 55; the loader's existing generation-change heal (`cairn_reproject('', false, 'loader')`) then replays every link event through the new applier, which is what re-folds winners chosen under the old order (a column backfill alone would not). On the Rust side a new module `chart_link.rs` owns the attested body for both verbs, the in-transaction core that submits it and resolves any open `match_proposal` row, and the two public entry points; `apply_accepted_proposal` becomes a thin wrapper over the core.

**Tech Stack:** PostgreSQL 18 (PL/pgSQL, `db/*.sql` replayed on every connect), Rust (`cairn-node`, `cairn-event`), clap CLI. No GUI change (that is R2b).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (section *R2*) and [ADR-0076](../../spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md) decisions 4 and 5.

## Global Constraints

- **AGPL-3.0**; no new dependency (none is needed).
- **TDD**: every task's test is written and seen to FAIL before the code that passes it.
- **Identity events are CLEAR, never sealed** — db/005 refuses a sealed non-`clinical.%` body. Use `sign` → `sign_attestation` → 3-arg `submit_event`, never `seal_sign_submit`.
- **"Attested" has exactly one definition**: `attester_key IS NOT NULL AND cairn_attestation_vouched(event_id)`. Never write a second spelling.
- **Every db/*.sql change is idempotent** — `connect_and_load_schema` replays every file on every connect. A widened `CREATE TABLE IF NOT EXISTS` needs a paired `ALTER … ADD COLUMN IF NOT EXISTS` (#207) and a `WIDENED` row in `migration_replay_widening.rs`.
- **A SQL edit needs a REBUILD** — migrations are `include_str!`'d; a stale test binary runs the old SQL (repo memory #593).
- **UUIDs cross the tokio-postgres boundary as text** (`$1::text::uuid`, `::text` reads) — this crate has no uuid `ToSql`.
- **A new `pub fn` in `crates/cairn-node/tests/common/mod.rs` is ALSO added to `identity_scaffolding_shared.rs`'s expected list**, or `derivation_finds_the_expected_helpers` fails.
- **Commit messages**: run `python3 scripts/check_closing_keywords.py <msgfile>` BEFORE committing; write issue refs as `fix(#681):`, never "closes #N" unless the close is intended. End each message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Test commands**: DB-gated suites need the three env strings; a bare `cargo test` FAILS unless `CAIRN_ALLOW_DB_SKIP=1`. Per-task runs: `CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test <suite>` with `CAIRN_TEST_PG` set (find the cluster with `scripts/pg-target.sh`). The final gate is `scripts/run-db-gated-tests.sh` (Task 6), never a targeted run (trap 18). **Subagents run FOREGROUND ONLY.**
- **Out of scope** (do not build): the window gesture and panel (R2b); #697's withholding (next, after R2a); the worklist (R5); allergy anything.

## Review Focus

1. **An exact HLC-triple collision between an attested and an un-attested assertion** — a reasonable person expects the attested one to win regardless of which content address sorts higher. Pinned in Task 1 (`an_attested_assertion_wins_even_an_hlc_triple_collision`).
2. **A human `unlink` that is OLDER than a vetoed machine link already standing and flagged** — the unlink must now win, clear the `link_veto_flag` row and split the charts (under the old order it lost and left the charts merged under review). Pinned in Task 1.
3. **An upgraded database whose standing winner is the machine link that already displaced a human `unlink`** — after the upgrade connect, the human's `unlink` must stand and the charts must be two. Pinned in Task 2.
4. **`link-charts` naming a chart this node has never seen** (a typo, a chart outside sync scope) — refused before anything is signed, naming the chart, because the human cannot have looked at it. Pinned in Task 3.
5. **Unlinking a pair whose proposal is already closed** (`applied`, `auto_applied`, `rejected`, `retracted`) — the event is authored, the closed row is left exactly as it was (the invariant `applied_event_id IS NOT NULL ⇔ status IN ('applied','auto_applied')` must never break). Pinned in Task 3.

---

### Task 1: the D5 precedence floor in db/018

**Files:**
- Modify: `db/018_identity_linkage.sql` (the `patient_link` CREATE at :91-104 and its ALTER at :110; a new comparator above `patient_link_apply`; `patient_link_apply` at :300-455)
- Modify: `crates/cairn-node/tests/common/mod.rs` (one new helper), `crates/cairn-node/tests/identity_scaffolding_shared.rs` (its expected list)
- Modify: `crates/cairn-node/tests/migration_replay_widening.rs` (`WIDENED`)
- Create: `crates/cairn-node/tests/link_precedence.rs`

**Interfaces:**
- Produces (SQL): column `patient_link.attested BOOLEAN NOT NULL DEFAULT FALSE`; function `cairn_link_overlay_wins(new_attested boolean, new_wall bigint, new_counter int, new_origin text, new_addr bytea, cur_attested boolean, cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea) RETURNS boolean`.
- Produces (tests/common): `pub fn link_assertion_event(kid: &str, a: Uuid, b: Uuid, link: bool, wall: i64, counter: i32, origin: &str, attested: bool) -> EventBody` — `attested = true` adds the responsibility-bearing contributor `{"actor_id": kid, "role": "attested", "responsibility": {"held_by": kid}}`, else `{"actor_id": kid, "role": "recorded"}`; `schema_version` `identity.link/1` / `identity.unlink/1`; provenance `"test:precedence"`.
- Consumes (tests/common, existing): `apply_remote_raw(c, sk, body)`, `apply_remote_attested(c, sk, body, sk_h, kid_h)`, `submit_attested(c, sk, body, sk_h, kid_h)`, `register_pair(c, sk, kid, low, high)`, `submit_registration`.

- [ ] **Step 1: Add the shared body builder to tests/common**

Append to `crates/cairn-node/tests/common/mod.rs` (the file already imports `LinkAssertion`, `link_assertion_body`, `unlink_assertion_body`, `render_link_twin`, `render_unlink_twin`, `EventBody`, `Hlc` for `submit_link_event`; add any that are missing):

```rust
/// A link/unlink body at a CHOSEN HLC triple, attested or not — the shape every ADR-0076
/// decision-5 test needs, where the whole question is how an attested and an un-attested
/// assertion of the same pair rank against each other.
///
/// `attested = true` gives the body the responsibility-bearing contributor a human vouch
/// carries (db/005/db/020 then DEMAND a verified human token — submit it with
/// [`submit_attested`] / [`apply_remote_attested`]); `false` gives a plain `recorded`
/// contributor (a matcher or agent writer — submit with `submit_event($1)` /
/// [`apply_remote_raw`]). The caller picks `origin` so two events can collide on the
/// full `(wall, counter, origin)` triple when a test needs that.
pub fn link_assertion_event(
    kid: &str,
    a: Uuid,
    b: Uuid,
    link: bool,
    wall: i64,
    counter: i32,
    origin: &str,
    attested: bool,
) -> EventBody {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let la = LinkAssertion {
        subject_a: &a_s,
        subject_b: &b_s,
        provenance: "test:precedence",
        confidence: None,
    };
    let (etype, sver, payload, twin) = if link {
        ("identity.link.asserted", "identity.link/1", link_assertion_body(&la), render_link_twin(&la))
    } else {
        ("identity.unlink.asserted", "identity.unlink/1", unlink_assertion_body(&la), render_unlink_twin(&la))
    };
    let contributors = if attested {
        serde_json::json!([{"actor_id": kid, "role": "attested", "responsibility": {"held_by": kid}}])
    } else {
        serde_json::json!([{"actor_id": kid, "role": "recorded"}])
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
        hlc: Hlc { wall, counter, node_origin: origin.into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors,
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}
```

Add `"fn link_assertion_event(",` to the expected vector in `identity_scaffolding_shared.rs::derivation_finds_the_expected_helpers`, in sorted position (after `"fn body_from_spec(",`), with a one-line comment above the vector's other entries' comments: `// `link_assertion_event` joined in the R2a plan's Task 1 (`link_precedence.rs`, `chart_link.rs`): attested and un-attested link bodies at a chosen HLC triple.`

- [ ] **Step 2: Write the failing precedence tests**

Create `crates/cairn-node/tests/link_precedence.rs`:

```rust
//! ADR-0076 decision 5 — an ATTESTED link assertion outranks an UN-ATTESTED one.
//!
//! `patient_link` used to be latest-HLC-wins, so a matcher's link (ours or a peer's) with a
//! later HLC silently displaced a reviewer's `unlink`, and an un-attested `unlink` (the
//! ADR-0030 agent writer can author one — unlinks are never veto-gated) could split a
//! reviewer's `link`. The winner order is now: attested first, then `(hlc_wall,
//! hlc_counter, origin)`, then `content_address`. Still a TOTAL order, so every node
//! converges — which is why every test here applies its events in BOTH arrival orders.
//!
//! "Attested" is the one definition db/018 already used for the #190 veto check: an
//! attester key is present and `cairn_attestation_vouched` holds.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_attested, apply_remote_raw, link_assertion_event, register_pair};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Empty every identity projection this suite reads, then enroll one agent (the machine
/// writer) and one human (the reviewer). Returns (agent_sk, agent_kid, human_sk, human_kid).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.link_veto_flag') IS NOT NULL THEN TRUNCATE link_veto_flag; END IF; \
           IF to_regclass('public.match_proposal') IS NOT NULL THEN TRUNCATE match_proposal; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"prec-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"P\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Between arrival orders: empty the log and the projections (event_log is append-only —
/// a DELETE is refused, so TRUNCATE as `overlay_tiebreaker.rs::reset_between_orders` does),
/// keep the enrolled actors (`actor_event` is not truncated), and re-register the pair so
/// the second order starts from the same charts as the first.
async fn reset_links(c: &Client, sk_a: &SigningKey, kid_a: &str, a: Uuid, b: Uuid) {
    c.batch_execute(
        "TRUNCATE event_log, patient_chart, patient_link, person_member, link_veto_flag, \
         identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    register_pair(c, sk_a, kid_a, a, b).await;
}

/// The standing (state, attested) for the canonical pair.
async fn standing(c: &Client, a: Uuid, b: Uuid) -> (String, bool) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let r = c
        .query_one(
            "SELECT state, attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&lo.to_string(), &hi.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

/// Are a and b one person now? (Compare representatives; a missing row is "not linked".)
async fn same_person(c: &Client, a: Uuid, b: Uuid) -> bool {
    let r: Option<bool> = c
        .query_one(
            "SELECT (SELECT person_id FROM person_member WHERE patient_id = $1::text::uuid)
                  = (SELECT person_id FROM person_member WHERE patient_id = $2::text::uuid)",
            &[&a.to_string(), &b.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    r.unwrap_or(false)
}

/// One assertion to land: its body, and whether it travels with a human token.
struct Landing {
    body: EventBody,
    attested: bool,
}

/// Land `l` through the REMOTE door (apply_remote_event), attested or not.
async fn land_remote(c: &Client, l: &Landing, sk_a: &SigningKey, sk_h: &SigningKey, kid_h: &str) {
    if l.attested {
        apply_remote_attested(c, sk_h, l.body.clone(), sk_h, kid_h).await.expect("attested lands");
    } else {
        apply_remote_raw(c, sk_a, l.body.clone()).await.expect("un-attested lands");
    }
}

/// Land `l` through the LOCAL door (submit_event), attested or not.
async fn land_local(c: &Client, l: &Landing, sk_a: &SigningKey, sk_h: &SigningKey, kid_h: &str) {
    if l.attested {
        common::submit_attested(c, sk_h, l.body.clone(), sk_h, kid_h).await.expect("attested lands");
    } else {
        let signed = sign(&l.body, sk_a).unwrap();
        c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.expect("un-attested lands");
    }
}

/// Land `first` then `second`, read the standing edge; reset; land them the other way round;
/// read again. Both orders must agree (convergence) — the value returned is that agreement.
#[allow(clippy::too_many_arguments)]
async fn both_orders_remote(
    c: &Client, first: &Landing, second: &Landing, a: Uuid, b: Uuid,
    sk_a: &SigningKey, kid_a: &str, sk_h: &SigningKey, kid_h: &str,
) -> ((String, bool), bool) {
    land_remote(c, first, sk_a, sk_h, kid_h).await;
    land_remote(c, second, sk_a, sk_h, kid_h).await;
    let one = (standing(c, a, b).await, same_person(c, a, b).await);
    reset_links(c, sk_a, kid_a, a, b).await;
    land_remote(c, second, sk_a, sk_h, kid_h).await;
    land_remote(c, first, sk_a, sk_h, kid_h).await;
    let two = (standing(c, a, b).await, same_person(c, a, b).await);
    assert_eq!(one, two, "the two arrival orders must converge on one winner");
    one
}

#[tokio::test]
async fn a_later_machine_link_does_not_displace_a_human_unlink() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_unlink = Landing { body: link_assertion_event(&kid_h, a, b, false, 10, 0, "nodeH", true), attested: true };
    let machine_link = Landing { body: link_assertion_event(&kid_a, a, b, true, 20, 0, "nodeM", false), attested: false };

    let ((state, attested), merged) =
        both_orders_remote(&c, &human_unlink, &machine_link, a, b, &sk_a, &kid_a, &sk_h, &kid_h).await;
    assert_eq!(state, "unlink", "the human's judgement stands against a later machine link");
    assert!(attested, "the standing row records that its winner was attested");
    assert!(!merged, "the two charts stay two people");
}

#[tokio::test]
async fn a_later_machine_unlink_does_not_split_a_human_link() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_link = Landing { body: link_assertion_event(&kid_h, a, b, true, 10, 0, "nodeH", true), attested: true };
    let machine_unlink = Landing { body: link_assertion_event(&kid_a, a, b, false, 20, 0, "nodeM", false), attested: false };

    let ((state, _), merged) =
        both_orders_remote(&c, &human_link, &machine_unlink, a, b, &sk_a, &kid_a, &sk_h, &kid_h).await;
    assert_eq!(state, "link", "an un-attested unlink never splits a human's link");
    assert!(merged);
}

#[tokio::test]
async fn between_two_human_judgements_the_later_wins() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let link = Landing { body: link_assertion_event(&kid_h, a, b, true, 10, 0, "nodeH", true), attested: true };
    let unlink = Landing { body: link_assertion_event(&kid_h, a, b, false, 20, 0, "nodeH", true), attested: true };

    let ((state, attested), merged) =
        both_orders_remote(&c, &link, &unlink, a, b, &sk_a, &kid_a, &sk_h, &kid_h).await;
    assert_eq!(state, "unlink", "a later human judgement reverses an earlier one");
    assert!(attested);
    assert!(!merged);
}

#[tokio::test]
async fn an_attested_assertion_wins_even_an_hlc_triple_collision() {
    // Review Focus 1: identical (wall, counter, origin) — the old order fell through to the
    // content address; the attested assertion must win whichever address sorts higher.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_unlink = Landing { body: link_assertion_event(&kid_h, a, b, false, 30, 4, "same", true), attested: true };
    let machine_link = Landing { body: link_assertion_event(&kid_a, a, b, true, 30, 4, "same", false), attested: false };

    let ((state, _), _) =
        both_orders_remote(&c, &human_unlink, &machine_link, a, b, &sk_a, &kid_a, &sk_h, &kid_h).await;
    assert_eq!(state, "unlink");
}

#[tokio::test]
async fn the_local_door_ranks_the_same_way() {
    // Both doors reach the ONE applier; pin that the local door (submit_event) does not
    // take a different path. Non-vetoed pair, so the local door admits the agent's link.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let human_unlink = Landing { body: link_assertion_event(&kid_h, a, b, false, 10, 0, "nodeH", true), attested: true };
    let machine_link = Landing { body: link_assertion_event(&kid_a, a, b, true, 20, 0, "nodeM", false), attested: false };
    land_local(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;
    land_local(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;

    assert_eq!(standing(&c, a, b).await, ("unlink".to_string(), true));
    assert!(!same_person(&c, a, b).await);
}

#[tokio::test]
async fn an_older_human_unlink_clears_a_standing_vetoed_machine_link() {
    // Review Focus 2. A vetoed machine link lands first (remote door admits it, flags it,
    // both charts read under-review). A human unlink with an EARLIER HLC then arrives —
    // under the old order it lost; now it wins, the flag clears and the charts split.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let machine_link = Landing { body: link_assertion_event(&kid_a, a, b, true, 20, 0, "nodeM", false), attested: false };
    land_remote(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;
    assert_eq!(flag_count(&c).await, 1, "precondition: the vetoed machine link is flagged");

    let human_unlink = Landing { body: link_assertion_event(&kid_h, a, b, false, 10, 0, "nodeH", true), attested: true };
    land_remote(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;

    assert_eq!(standing(&c, a, b).await, ("unlink".to_string(), true));
    assert_eq!(flag_count(&c).await, 0, "the human decision clears the veto worklist row");
    assert!(!same_person(&c, a, b).await);
}

#[tokio::test]
async fn a_vetoed_machine_link_after_a_human_unlink_raises_no_flag() {
    // The mirror: the human unlink is standing; a LATER vetoed machine link arrives. It loses,
    // so the flag lifecycle (derived from the standing winner) must not raise a phantom row.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let human_unlink = Landing { body: link_assertion_event(&kid_h, a, b, false, 10, 0, "nodeH", true), attested: true };
    let machine_link = Landing { body: link_assertion_event(&kid_a, a, b, true, 20, 0, "nodeM", false), attested: false };
    land_remote(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;
    land_remote(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;

    assert_eq!(standing(&c, a, b).await.0, "unlink");
    assert_eq!(flag_count(&c).await, 0);
    assert!(!same_person(&c, a, b).await);
}

#[tokio::test]
async fn every_standing_row_records_its_winners_attestation_truthfully() {
    // The column must never disagree with the one definition, evaluated on the winning
    // event. Land a mixed history over three pairs, then check every row.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    for (i, attested) in [(0, true), (1, false), (2, true)] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, a, b).await;
        let kid = if attested { &kid_h } else { &kid_a };
        let l = Landing { body: link_assertion_event(kid, a, b, true, 10 + i, 0, "n", attested), attested };
        land_remote(&c, &l, &sk_a, &sk_h, &kid_h).await;
    }
    let disagreeing: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_link pl JOIN event_log el ON el.content_address = pl.content_address
              WHERE pl.attested IS DISTINCT FROM
                    (el.attester_key IS NOT NULL AND cairn_attestation_vouched(el.event_id))",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(disagreeing, 0);
    let rows: i64 = c.query_one("SELECT count(*) FROM patient_link", &[]).await.unwrap().get(0);
    assert_eq!(rows, 3, "positive control: the check above ran over three rows");
}

async fn flag_count(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM link_veto_flag", &[]).await.unwrap().get(0)
}

/// Two registered charts whose verified DOBs clash — a hard veto by construction (the same
/// fixture `link_veto_floor.rs` uses, reduced to what this suite needs).
async fn vetoed_pair(c: &Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid) {
    use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(c, sk, kid, a, b).await;
    for (p, wall, value) in [(a, 2, "1980-07-15"), (b, 3, "1975-01-02")] {
        let body = EventBody {
            event_id: Uuid::now_v7().to_string(),
            patient_id: p.to_string(),
            event_type: "demographic.field.asserted".into(),
            schema_version: "demographic.field/1".into(),
            hlc: cairn_event::Hlc { wall, counter: 0, node_origin: "n".into() },
            t_effective: None,
            signer_key_id: kid.into(),
            contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
            payload: dob_assertion_body(value, "day", Some("document"), "document-verified"),
            attachments: vec![],
            plaintext_twin: Some(render_dob_twin(value, "day", "document-verified")),
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
            safety: None,
        };
        let signed = sign(&body, sk).unwrap();
        c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap();
    }
    let vetoed: bool = c
        .query_one("SELECT cairn_has_hard_veto($1::text::uuid, $2::text::uuid)", &[&a.to_string(), &b.to_string()])
        .await
        .unwrap()
        .get(0);
    assert!(vetoed, "precondition: the pair must trip the hard veto");
    (a, b)
}
```

Add `("patient_link", "attested"),` to `WIDENED` in `migration_replay_widening.rs` with the comment `// ADR-0076 decision 5 (R2a): the winner's attestation, ranked before the HLC.`

- [ ] **Step 3: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG=… CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test link_precedence --test migration_replay_widening --test identity_scaffolding_shared`
Expected: `link_precedence` fails to compile or fails at `standing()` (`column "attested" does not exist`); `migration_replay_widening` fails (column absent after replay). `identity_scaffolding_shared` passes.

- [ ] **Step 4: Implement the floor in db/018**

In `db/018_identity_linkage.sql`:

(a) In the `CREATE TABLE IF NOT EXISTS patient_link` body, after `content_address BYTEA NOT NULL,`, add:
```sql
    -- ADR-0076 decision 5: was the WINNING assertion attested? The one definition
    -- (attester_key present AND cairn_attestation_vouched), evaluated when the winner was
    -- applied. Ranked BEFORE the HLC by cairn_link_overlay_wins below.
    attested    BOOLEAN NOT NULL DEFAULT FALSE,
```

(b) After the existing `ALTER TABLE patient_link ADD COLUMN IF NOT EXISTS content_address BYTEA;` add:
```sql
-- ADR-0076 decision 5 widening (#207 discipline: the CREATE above no-ops on an existing
-- table, so the column ALSO ships as an idempotent ALTER). NOT NULL DEFAULT FALSE: every
-- pre-existing row starts "not attested", which is corrected — together with the WINNER,
-- which a column fill alone could never re-decide — by db/055's backfill and the loader's
-- generation-change heal replaying every link event through the applier below. Guarded by
-- migration_replay_widening.rs and link_precedence.rs::an_upgraded_node_refolds_*.
ALTER TABLE patient_link ADD COLUMN IF NOT EXISTS attested BOOLEAN NOT NULL DEFAULT FALSE;
```

(c) Immediately above the `CREATE OR REPLACE FUNCTION patient_link_apply(e event_log)` (after the `DROP FUNCTION IF EXISTS patient_link_apply();` line), add:
```sql
-- ADR-0076 decision 5 — the patient_link winner order: ATTESTED FIRST, then the ordinary
-- HLC overlay order (cairn_hlc_overlay_wins, db/002: wall, counter, origin, content_address).
--
-- Why: latest-HLC-wins let a machine's link (a peer's matcher, carrying a later clock)
-- silently displace a human reviewer's unlink, and an un-attested unlink (the ADR-0030
-- agent writer can author one — unlinks are never veto-gated) split a human's link. This
-- ranks a human judgement above a machine one in both directions; between two human
-- judgements, or two machine ones, the later still wins.
--
-- Convergence: this is a lexicographic order over (attested, wall, counter, origin,
-- address) — still TOTAL, so every node applying the same set of assertions in any order
-- keeps the same winner (principle 1). NULL is read as "not attested" (the column is NOT
-- NULL; the COALESCE only makes the rule legible without that knowledge).
-- Pure and IMMUTABLE, like the comparator it wraps.
CREATE OR REPLACE FUNCTION cairn_link_overlay_wins(
    new_attested boolean, new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_attested boolean, cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN COALESCE(new_attested, FALSE) <> COALESCE(cur_attested, FALSE)
            THEN COALESCE(new_attested, FALSE)
        ELSE cairn_hlc_overlay_wins(new_wall, new_counter, new_origin, new_addr,
                                    cur_wall, cur_counter, cur_origin, cur_addr)
    END;
$$;
```

(d) In `patient_link_apply`:
- In `DECLARE`, add `v_attested boolean;` (NOT initialised in DECLARE — same reason as `a`/`b`: it must not run before the seal guard).
- Right after `hi := GREATEST(a, b);`, add:
```sql
    -- The ONE definition of "attested" (ADR-0076 decision 5; the #190 veto check below and
    -- the stored column share it). An unvouched token — carried by db/020 for a deferred
    -- event, never verified — is NOT a vouch (PR #302 finding F2).
    --
    -- Evaluated ONCE, here, and stored with the winner. That is safe because an event's
    -- vouch never changes after its first projection: a deferred event projects for the
    -- first time in db/043's gate 4, AFTER gate 1 has cleared (or kept) its unvouched
    -- marker; a non-deferred event's attester_key is only ever stored once verified.
    v_attested := e.attester_key IS NOT NULL AND cairn_attestation_vouched(e.event_id);
```
- In the #190 door refusal, replace `AND (e.attester_key IS NULL OR NOT cairn_attestation_vouched(e.event_id))` with `AND NOT v_attested`.
- In the upsert: add `attested` to the column list and `v_attested` to `VALUES` (after `content_address` / `e.content_address`); add `attested = EXCLUDED.attested,` to the `DO UPDATE SET` list; replace the `WHERE cairn_hlc_overlay_wins(...)` clause with:
```sql
    -- ADR-0076 decision 5: attested first, then the HLC overlay order (see
    -- cairn_link_overlay_wins above). Content address remains the final tiebreak (#115).
    WHERE cairn_link_overlay_wins(
        EXCLUDED.attested, EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin,
        EXCLUDED.content_address,
        patient_link.attested, patient_link.hlc_wall, patient_link.hlc_counter,
        patient_link.origin, patient_link.content_address);
```
- Replace the winner read-back (`SELECT pl.state, pl.content_address, el.attester_key IS NOT NULL AND cairn_attestation_vouched(el.event_id) INTO … FROM patient_link pl JOIN event_log el … WHERE …`) with:
```sql
    SELECT pl.state, pl.content_address, pl.attested
      INTO v_win_state, v_win_ca, v_win_attested
      FROM patient_link pl
      WHERE pl.low = lo AND pl.high = hi;
```
  and append to the block comment above it: `-- Since ADR-0076 decision 5 the winner's attestation is the stored column, written from v_attested by the ONE definition when that winner was applied (see v_attested above for why it cannot go stale).`
- Update the file-header comment line 9 (`an HLC-overlay patient_link edge table`) to `an attested-first, then HLC-overlay patient_link edge table (ADR-0076 decision 5)`, and the comment above the upsert block ("Overlay only when the incoming event outranks …") is already replaced by the new one.

- [ ] **Step 5: Run the tests to verify they pass, plus the existing identity suites**

Run: `CAIRN_TEST_PG=… CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test link_precedence --test migration_replay_widening --test identity_scaffolding_shared --test link_veto_floor --test identity_linkage --test overlay_tiebreaker --test apply_proposal --test person_charts --test combined_read`
Expected: all PASS. If a `link_veto_floor` test fails, read it before changing it: a failure there means the flag lifecycle and the new order disagree — fix the SQL, not the test (the one exception would be a test that asserted the OLD order; there is none known — report it if found).

- [ ] **Step 6: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/tests/link_precedence.rs \
  crates/cairn-node/tests/common/mod.rs crates/cairn-node/tests/identity_scaffolding_shared.rs \
  crates/cairn-node/tests/migration_replay_widening.rs
git commit -F <msgfile>   # "feat(R2a): an attested link assertion outranks an un-attested one (ADR-0076 decision 5)"
```

---

### Task 2: the upgrade re-folds winners the old order chose (db/055, generation 55)

**Files:**
- Create: `db/055_link_precedence_refold.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs` (`SCHEMA_GENERATION` 54 → 55 and its doc example line)
- Modify: `crates/cairn-node/src/db.rs` (append the `055_link_precedence_refold` entry after `054_person_charts`, ~:350)
- Test: `crates/cairn-node/tests/link_precedence.rs` (one new test)

**Interfaces:**
- Consumes: Task 1's column and applier; the loader's existing heal (`db.rs` ~:704, `cairn_reproject('', false, 'loader')`, run only when the recorded generation differs from the embedded one).
- Produces: nothing new for later tasks.

**Not in cairn-sync's list** — cairn-sync loads no identity migration (not db/018), so it legitimately lags (#284), exactly as R1's db/054 did.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/link_precedence.rs`:

```rust
#[tokio::test]
async fn an_upgraded_node_refolds_a_winner_the_old_order_chose() {
    // Review Focus 3. Recreate what a generation-54 node holds after the old order ran: a
    // human unlink (HLC 10) displaced by a machine link (HLC 20) — the machine link is the
    // stored winner and the charts are merged. The upgrade connect must leave the human's
    // unlink standing and the charts apart. Filling the new column alone would not: it
    // would mark the machine link "not attested" and keep it as the winner.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let (lo_s, hi_s) = (lo.to_string(), hi.to_string());

    let human_unlink = Landing { body: link_assertion_event(&kid_h, a, b, false, 10, 0, "nodeH", true), attested: true };
    let machine_link = Landing { body: link_assertion_event(&kid_a, a, b, true, 20, 0, "nodeM", false), attested: false };
    land_remote(&c, &human_unlink, &sk_a, &sk_h, &kid_h).await;
    land_remote(&c, &machine_link, &sk_a, &sk_h, &kid_h).await;

    // Forge the OLD order's outcome: the machine link as the stored winner, the pre-D5
    // table shape (no column), the component merged, the generation one behind.
    c.batch_execute(&format!(
        "UPDATE patient_link pl SET state = 'link', hlc_wall = el.hlc_wall,
                hlc_counter = el.hlc_counter, origin = el.node_origin,
                content_address = el.content_address
           FROM event_log el
          WHERE el.event_type = 'identity.link.asserted'
            AND pl.low = '{lo_s}'::uuid AND pl.high = '{hi_s}'::uuid;
         ALTER TABLE patient_link DROP COLUMN attested CASCADE;
         SELECT cairn_recompute_component('{lo_s}'::uuid, NULL);
         UPDATE node_schema SET version = 54;"
    ))
    .await
    .unwrap();
    assert!(same_person(&c, a, b).await, "precondition: the forged old state is merged");
    drop(c);

    // The upgrade: reconnect = replay every migration, then (generation 54 → 55) the heal.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(
        standing(&c, a, b).await,
        ("unlink".to_string(), true),
        "the human's unlink is re-decided as the winner"
    );
    assert!(!same_person(&c, a, b).await, "and the charts are two people again");
    let recorded: i32 = c.query_one("SELECT version FROM node_schema", &[]).await.unwrap().get(0);
    assert_eq!(recorded, cairn_event::schema_generation::SCHEMA_GENERATION);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `CAIRN_TEST_PG=… CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test link_precedence an_upgraded_node`
Expected: FAIL at the first `standing` assertion — the loader sees generation 54 == embedded 54, runs no heal, and the machine link stays the winner with `attested = false`.

- [ ] **Step 3: Create db/055 and bump the generation**

Create `db/055_link_precedence_refold.sql`:

```sql
-- db/055_link_precedence_refold.sql
-- Cairn — ADR-0076 decision 5 on an EXISTING database: re-decide every patient_link winner
-- under the attested-first order (repair path R2a).
--
-- WHAT THIS FILE DOES, AND WHAT ITS EXISTENCE DOES.
--
-- db/018 now ranks an attested link assertion above an un-attested one. That governs every
-- assertion APPLIED from now on. It does not revisit a winner the OLD order already chose:
-- on a database that has run, a human's unlink may already have been displaced by a later
-- machine link, and that machine link is still the stored winner, with the two charts
-- merged. Filling the new `attested` column cannot fix that — a column fill marks the
-- machine link "not attested" and leaves it standing.
--
-- The fix is to re-apply every link assertion through the new applier. Cairn already has
-- that pass: when a node's recorded schema generation differs from its binary's, the loader
-- runs a heal (cairn_reproject, db/039) that replays every replay-eligible event through its
-- heal-safe appliers. patient_link_apply is heal-safe and its order is total, so replaying
-- every link and unlink over the live table leaves exactly the winner the new order picks,
-- whatever the stored row held before; each replay also recomputes both endpoints'
-- component and the #190 flag from the standing winner.
--
-- So THIS FILE'S EXISTENCE IS LOAD-BEARING: it is the newest migration, so it moves
-- SCHEMA_GENERATION to 55, so every existing node heals on its next connect. Do not fold its
-- content back into db/018 "to tidy up" — without a generation change no heal runs.
--
-- The backfill below makes `attested` truthful for every standing row the moment this file
-- loads, before the heal: a reader between the two (none today; R5's worklist and db/054's
-- doubted-link check are candidates) never sees an attested winner reported as un-attested.
-- It is idempotent: once converged, no row matches and the UPDATE writes nothing.
--
-- Node loader only: cairn-sync loads no identity migration (#284).

BEGIN;

UPDATE patient_link pl
   SET attested = TRUE
  FROM event_log el
 WHERE el.content_address = pl.content_address
   AND NOT pl.attested
   AND el.attester_key IS NOT NULL
   AND cairn_attestation_vouched(el.event_id);

COMMIT;
```

In `crates/cairn-event/src/schema_generation.rs`: `pub const SCHEMA_GENERATION: i32 = 55;` and change the doc example to ``(`db/055_link_precedence_refold.sql` → 55).``

In `crates/cairn-node/src/db.rs`, after the `054_person_charts` entry:
```rust
    // db/055 (ADR-0076 decision 5, R2a): backfills patient_link.attested — and, by being
    // the newest file, moves the generation to 55 so every existing node's loader heal
    // re-applies every link assertion under the attested-first order. Node list only.
    (
        "055_link_precedence_refold",
        include_str!("../../../db/055_link_precedence_refold.sql"),
    ),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG=… CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test link_precedence --test migration_replay_widening` and `cargo test -p cairn-event --test schema_generation` and `cargo test -p cairn-node --lib db::` (the loader-list guard).
Expected: all PASS. If a cairn-sync schema-generation test complains, read it: cairn-sync must report the SAME constant but need not load db/055 (R1 precedent) — do not add db/055 to its list.

- [ ] **Step 5: Commit**

```bash
git add db/055_link_precedence_refold.sql crates/cairn-event/src/schema_generation.rs \
  crates/cairn-node/src/db.rs crates/cairn-node/tests/link_precedence.rs
git commit -F <msgfile>   # "feat(R2a): an upgraded node re-decides every link winner (db/055, generation 55)"
```

---

### Task 3: `chart_link` — `link_charts` / `unlink_charts`, resolving an open proposal in the same transaction

**Files:**
- Create: `crates/cairn-node/src/chart_link.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod chart_link;`, alphabetical, after `capture`)
- Create: `crates/cairn-node/tests/chart_link.rs`

**Interfaces:**
- Consumes: `cairn_event::identity::{LinkAssertion, link_assertion_body, unlink_assertion_body, render_link_twin, render_unlink_twin}`; `cairn_event::{sign, sign_attestation, event_address, EventBody, Hlc, SigningKey}`; `crate::db::next_hlc(client, node_origin) -> anyhow::Result<Hlc>`; `crate::identify::attester_is_enrolled_human(client, kid) -> anyhow::Result<bool>`; `crate::patient::person::person_charts(client, patient) -> anyhow::Result<ChartSet>`.
- Produces:
  - `pub enum LinkVerb { Link, Unlink }` (`Debug, Clone, Copy, PartialEq, Eq`), with `pub fn event_type(self) -> &'static str`, `pub fn schema_version(self) -> &'static str`, `pub fn resolved_status(self) -> &'static str` (`"applied"` / `"rejected"`).
  - `pub fn canonical_pair(a: Uuid, b: Uuid) -> (Uuid, Uuid)`.
  - `pub fn compose_review_provenance(verb: LinkVerb, human_kid: &str) -> String` → `"chart-review linked-by:{kid}"` / `"chart-review unlinked-by:{kid}"`.
  - `pub fn build_attested_assertion_body(verb: LinkVerb, event_id: Uuid, low: Uuid, high: Uuid, provenance: &str, confidence: Option<&str>, human_kid: &str, hlc: Hlc) -> EventBody`.
  - `pub struct Reviewer<'a> { pub human_sk: &'a SigningKey, pub human_kid: &'a str }`.
  - `pub struct LinkOutcome { pub event_id: Uuid, pub proposal_resolved: bool, pub charts: cairn_medication_view::ChartSet }` — `charts` is the resulting `cairn_person_charts` set of the FIRST chart named, read after commit.
  - `pub async fn assert_link_in_tx(tx: &tokio_postgres::Transaction<'_>, verb: LinkVerb, low: Uuid, high: Uuid, provenance: &str, confidence: Option<&str>, reviewer: &Reviewer<'_>, hlc: Hlc) -> anyhow::Result<(Uuid, bool)>` — returns (event id, whether an open proposal row moved).
  - `pub async fn link_charts(client: &mut tokio_postgres::Client, a: Uuid, b: Uuid, reviewer: &Reviewer<'_>, node_origin: &str) -> anyhow::Result<LinkOutcome>` and `pub async fn unlink_charts(…same…)`.

(Check the actual import path of `ChartSet` in `crates/cairn-node/src/patient/person.rs` and use the same one.)

- [ ] **Step 1: Write the failing unit tests (pure half)**

At the foot of the new `chart_link.rs`, with only `pub enum LinkVerb` and stub signatures (`todo!()` bodies) above so it compiles:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc {
        Hlc { wall: 7, counter: 0, node_origin: "n".into() }
    }

    fn pair() -> (Uuid, Uuid) {
        let lo = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let hi = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        (lo, hi)
    }

    #[test]
    fn canonical_pair_orders_either_way_round() {
        let (lo, hi) = pair();
        assert_eq!(canonical_pair(hi, lo), (lo, hi));
        assert_eq!(canonical_pair(lo, hi), (lo, hi));
    }

    #[test]
    fn each_verb_names_its_type_schema_and_resolution() {
        assert_eq!(LinkVerb::Link.event_type(), "identity.link.asserted");
        assert_eq!(LinkVerb::Unlink.event_type(), "identity.unlink.asserted");
        assert_eq!(LinkVerb::Link.schema_version(), "identity.link/1");
        assert_eq!(LinkVerb::Unlink.schema_version(), "identity.unlink/1");
        assert_eq!(LinkVerb::Link.resolved_status(), "applied");
        assert_eq!(LinkVerb::Unlink.resolved_status(), "rejected");
    }

    #[test]
    fn review_provenance_names_the_act_and_the_human() {
        let l = compose_review_provenance(LinkVerb::Link, "kidH");
        let u = compose_review_provenance(LinkVerb::Unlink, "kidH");
        assert!(l.contains("linked-by:kidH") && !l.contains("unlinked"));
        assert!(u.contains("unlinked-by:kidH"));
    }

    #[test]
    fn the_body_carries_the_responsibility_that_demands_a_human_token() {
        let (lo, hi) = pair();
        for verb in [LinkVerb::Link, LinkVerb::Unlink] {
            let eid = Uuid::now_v7();
            let b = build_attested_assertion_body(verb, eid, lo, hi, "prov", None, "kidH", hlc());
            assert_eq!(b.event_type, verb.event_type());
            assert_eq!(b.schema_version, verb.schema_version());
            assert_eq!(b.event_id, eid.to_string());
            assert_eq!(b.patient_id, lo.to_string(), "an identity event is about subject_a = low");
            assert_eq!(b.payload["subject_a"], lo.to_string());
            assert_eq!(b.payload["subject_b"], hi.to_string());
            assert_eq!(b.payload["provenance"], "prov");
            assert!(b.payload.get("confidence").is_none(), "absent, never null (principle 4)");
            assert_eq!(b.contributors[0]["responsibility"]["held_by"], "kidH");
            assert!(!b.plaintext_twin.as_deref().unwrap().trim().is_empty());
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --lib chart_link`
Expected: FAIL (`not yet implemented` panics).

- [ ] **Step 3: Implement the pure half**

```rust
//! Link or unlink two charts as a HUMAN judgement (ADR-0076 decisions 4 and 5; repair path
//! R2a, for #681's gesture).
//!
//! Two paper folders the clinician has laid side by side are either clipped together
//! ("same person" → an attested `identity.link.asserted`) or marked as two people ("not
//! the same person" → an attested `identity.unlink.asserted`). Both are events in the
//! closed §5.7 identity algebra; both are reversible by the other; neither erases anything.
//! "Different people" is an unlink on a pair that may never have been linked (decision 4).
//!
//! WHY ATTESTED: db/018 ranks an attested assertion above an un-attested one (decision 5),
//! so a machine's later link can never undo this judgement, and a hard veto (§5.13) — which
//! refuses an un-attested link at the local door — is exactly the human decision it forces.
//!
//! Split, per house rule 4: pure body assembly (unit-tested here), one in-transaction core
//! (`assert_link_in_tx`) that `apply_proposal::apply_accepted_proposal` also uses, and the
//! two public entry points that add the pre-checks and the transaction.
//!
//! Identity events are CLEAR (db/005 refuses a sealed non-clinical body), so this signs and
//! attests directly and submits through the 3-argument `submit_event` door — never the
//! medication seal path.

use cairn_event::identity::{
    link_assertion_body, render_link_twin, render_unlink_twin, unlink_assertion_body,
    LinkAssertion,
};
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use uuid::Uuid;

/// Which judgement the human made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkVerb {
    /// "Same person" — clip the folders together.
    Link,
    /// "Not the same person" — whether or not they were ever linked.
    Unlink,
}

impl LinkVerb {
    /// The registered event type (db/018).
    pub fn event_type(self) -> &'static str {
        match self {
            LinkVerb::Link => "identity.link.asserted",
            LinkVerb::Unlink => "identity.unlink.asserted",
        }
    }

    /// The body's schema version (the convention the C1 tests and `apply_proposal` use).
    pub fn schema_version(self) -> &'static str {
        match self {
            LinkVerb::Link => "identity.link/1",
            LinkVerb::Unlink => "identity.unlink/1",
        }
    }

    /// The `match_proposal.status` an OPEN proposal for the pair moves to when a human
    /// decides it this way.
    pub fn resolved_status(self) -> &'static str {
        match self {
            LinkVerb::Link => "applied",
            LinkVerb::Unlink => "rejected",
        }
    }
}

/// `(least, greatest)` — the order `patient_link` and `match_proposal` store a pair in, so
/// a caller naming the charts either way round finds the same rows.
pub fn canonical_pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// The §4.1 provenance of a judgement made while reviewing a chart. Non-empty by
/// construction (the db/018 floor requires it); names the act and the human.
pub fn compose_review_provenance(verb: LinkVerb, human_kid: &str) -> String {
    match verb {
        LinkVerb::Link => format!("chart-review linked-by:{human_kid}"),
        LinkVerb::Unlink => format!("chart-review unlinked-by:{human_kid}"),
    }
}

/// The attested link or unlink body. Pure: the caller supplies `event_id` and `hlc`.
/// The human is the sole contributor and carries `responsibility` — which is what makes
/// both write doors demand a verified human attestation token for this event.
#[allow(clippy::too_many_arguments)]
pub fn build_attested_assertion_body(
    verb: LinkVerb,
    event_id: Uuid,
    low: Uuid,
    high: Uuid,
    provenance: &str,
    confidence: Option<&str>,
    human_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let low_s = low.to_string();
    let high_s = high.to_string();
    let la = LinkAssertion {
        subject_a: &low_s,
        subject_b: &high_s,
        provenance,
        confidence,
    };
    let (payload, twin) = match verb {
        LinkVerb::Link => (link_assertion_body(&la), render_link_twin(&la)),
        LinkVerb::Unlink => (unlink_assertion_body(&la), render_unlink_twin(&la)),
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: low_s.clone(), // C1 convention: an identity event is "about" subject_a
        event_type: verb.event_type().into(),
        schema_version: verb.schema_version().into(),
        hlc,
        t_effective: None,
        signer_key_id: human_kid.into(),
        // ADR-0051 wire shape: responsibility = {held_by}, held_by = the verified attester
        // (the #195 binding chain).
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "attested",
             "responsibility": {"held_by": human_kid}}
        ]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}
```

(If `link_assertion_body`/`unlink_assertion_body` include `confidence` as JSON null when `None`, the unit test fails — then read `cairn_event::identity` and pass what `apply_proposal` passes; do not special-case here.)

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --lib chart_link`
Expected: PASS.

- [ ] **Step 5: Write the failing DB tests**

Create `crates/cairn-node/tests/chart_link.rs`:

```rust
//! Repair path R2a — `link_charts` / `unlink_charts`: a human's judgement on two charts,
//! authored as an ATTESTED identity event, resolving any open match_proposal for the pair
//! in the same transaction (ADR-0076 decisions 4 and 5; #681's orchestration).
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, SigningKey};
use cairn_node::chart_link::{link_charts, unlink_charts, Reviewer};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_raw, link_assertion_event, register_pair};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "r2a-test-node";

/// Clean identity + proposal state; enroll an agent (to register charts and play the
/// machine) and a human reviewer.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.link_veto_flag') IS NOT NULL THEN TRUNCATE link_veto_flag; END IF; \
           TRUNCATE match_proposal; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"cl-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"CL\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

async fn standing(c: &Client, a: Uuid, b: Uuid) -> Option<(String, bool)> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    c.query_opt(
        "SELECT state, attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
}

async fn seed_proposal(c: &Client, a: Uuid, b: Uuid, status: &str) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    c.execute(
        "INSERT INTO match_proposal \
           (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version, status) \
         VALUES ($1::text::uuid, $2::text::uuid, 0.91, 'review', '[]'::jsonb, '[]'::jsonb, 'cfg@test', $3)",
        &[&lo.to_string(), &hi.to_string(), &status.to_string()],
    )
    .await
    .unwrap();
}

async fn proposal(c: &Client, a: Uuid, b: Uuid) -> (String, Option<String>) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let r = c
        .query_one(
            "SELECT status, applied_event_id::text FROM match_proposal \
             WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid",
            &[&lo.to_string(), &hi.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

#[tokio::test]
async fn a_link_is_attested_and_the_two_charts_read_as_one_set() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let out = link_charts(&mut c, b, a, &who, ORIGIN).await.expect("link");

    assert_eq!(standing(&c, a, b).await, Some(("link".into(), true)));
    assert_eq!(out.charts.members().len(), 2, "the returned set is the combined chart");
    assert!(!out.proposal_resolved, "no proposal existed");
    let attester: Option<Vec<u8>> = c
        .query_one("SELECT attester_key FROM event_log WHERE event_id = $1::text::uuid", &[&out.event_id.to_string()])
        .await
        .unwrap()
        .get(0);
    assert_eq!(attester, Some(sk_h.verifying_key().to_bytes().to_vec()), "the human vouched");
}

#[tokio::test]
async fn different_people_is_an_unlink_on_a_pair_never_linked() {
    // ADR-0076 decision 4.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let out = unlink_charts(&mut c, a, b, &who, ORIGIN).await.expect("unlink");
    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)));
    assert_eq!(out.charts.members(), &[a], "a stays a chart of its own");
}

#[tokio::test]
async fn an_unlink_after_a_link_splits_the_set_again() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    let out = unlink_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    assert_eq!(out.charts.members().len(), 1);
}

#[tokio::test]
async fn a_later_machine_link_from_a_peer_does_not_undo_the_reviewers_unlink() {
    // D5 end to end: the reviewer says "different people"; a peer's matcher then links the
    // pair with a later clock. The charts stay two.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    unlink_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();

    let wall: i64 = c.query_one("SELECT max(hlc_wall) FROM patient_link", &[]).await.unwrap().get(0);
    let later = link_assertion_event(&kid_a, a, b, true, wall + 1_000, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk_a, later).await.expect("the peer's link is admitted (set-union)");

    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)));
}

#[tokio::test]
async fn a_hard_vetoed_pair_can_still_be_linked_by_a_human_and_is_not_flagged() {
    // §5.13: a veto forces a human decision, never an automatic refusal.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    link_charts(&mut c, a, b, &who, ORIGIN).await.expect("a human may link a vetoed pair");
    let flags: i64 = c.query_one("SELECT count(*) FROM link_veto_flag", &[]).await.unwrap().get(0);
    assert_eq!(flags, 0, "an attested link is the human decision the veto forces");
}

#[tokio::test]
async fn a_human_link_resolves_a_doubted_machine_link() {
    // The doubted-link state R1 withholds from sign-off (db/054) is lifted by the human.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;
    let machine = link_assertion_event(&kid_a, a, b, true, 50, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk_a, machine).await.unwrap();
    let doubted = |c: &Client| {
        let ids = vec![a.to_string(), b.to_string()];
        async move {
            let d: bool = c
                .query_one("SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])", &[&ids])
                .await
                .unwrap()
                .get(0);
            d
        }
    };
    assert!(doubted(&c).await, "precondition: the machine link is doubted");

    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    assert!(!doubted(&c).await, "the human's attested link lifts the doubt");
}

#[tokio::test]
async fn an_open_proposal_moves_with_the_judgement_in_the_same_transaction() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };

    for open in ["pending", "accepted", "review"] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, a, b).await;
        seed_proposal(&c, a, b, open).await;
        let out = link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
        assert!(out.proposal_resolved, "{open} is an open proposal");
        assert_eq!(proposal(&c, a, b).await, ("applied".into(), Some(out.event_id.to_string())));

        let (x, y) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, x, y).await;
        seed_proposal(&c, x, y, open).await;
        let out = unlink_charts(&mut c, y, x, &who, ORIGIN).await.unwrap();
        assert!(out.proposal_resolved);
        assert_eq!(
            proposal(&c, x, y).await,
            ("rejected".into(), None),
            "a rejection names no applied event (db/019's invariant)"
        );
    }
}

#[tokio::test]
async fn a_closed_proposal_is_left_exactly_as_it_was() {
    // Review Focus 5.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    for closed in ["applied", "auto_applied", "rejected", "retracted"] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, a, b).await;
        seed_proposal(&c, a, b, closed).await;
        let before = proposal(&c, a, b).await;
        let out = unlink_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
        assert!(!out.proposal_resolved, "{closed} is not an open proposal");
        assert_eq!(proposal(&c, a, b).await, before);
    }
}

#[tokio::test]
async fn a_non_human_key_is_refused_and_nothing_moves() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    seed_proposal(&c, a, b, "pending").await;

    let agent = Reviewer { human_sk: &sk_a, human_kid: &kid_a };
    let err = link_charts(&mut c, a, b, &agent, ORIGIN).await.unwrap_err().to_string();
    assert!(err.contains("not an enrolled human"), "names why: {err}");
    assert_eq!(standing(&c, a, b).await, None, "no event landed");
    assert_eq!(proposal(&c, a, b).await.0, "pending", "the proposal did not move");
}

#[tokio::test]
async fn a_chart_this_node_has_never_seen_is_refused_before_signing() {
    // Review Focus 4.
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let a = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    let stranger = Uuid::now_v7();

    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    for verb_is_link in [true, false] {
        let r = if verb_is_link {
            link_charts(&mut c, a, stranger, &who, ORIGIN).await
        } else {
            unlink_charts(&mut c, stranger, a, &who, ORIGIN).await
        };
        let err = r.unwrap_err().to_string();
        assert!(err.contains(&stranger.to_string()), "names the unknown chart: {err}");
    }
    let n: i64 = c
        .query_one("SELECT count(*) FROM event_log WHERE event_type LIKE 'identity.%link.asserted'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_chart_cannot_be_linked_to_itself() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let a = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let err = link_charts(&mut c, a, a, &who, ORIGIN).await.unwrap_err().to_string();
    assert!(err.contains("same chart"), "{err}");
}

/// Two registered charts with a verified-DOB clash (hard veto), as in link_precedence.rs.
async fn vetoed_pair(c: &Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid) {
    use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
    use cairn_event::{sign, EventBody, Hlc};
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(c, sk, kid, a, b).await;
    for (p, wall, value) in [(a, 2, "1980-07-15"), (b, 3, "1975-01-02")] {
        let body = EventBody {
            event_id: Uuid::now_v7().to_string(),
            patient_id: p.to_string(),
            event_type: "demographic.field.asserted".into(),
            schema_version: "demographic.field/1".into(),
            hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
            t_effective: None,
            signer_key_id: kid.into(),
            contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
            payload: dob_assertion_body(value, "day", Some("document"), "document-verified"),
            attachments: vec![],
            plaintext_twin: Some(render_dob_twin(value, "day", "document-verified")),
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
            safety: None,
        };
        let signed = sign(&body, sk).unwrap();
        c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap();
    }
    (a, b)
}
```

**The two `vetoed_pair` copies** (this suite and `link_precedence.rs`) are identical — before committing, promote ONE copy to `tests/common/mod.rs` as `pub async fn vetoed_pair(c, sk, kid) -> (Uuid, Uuid)` (keeping the precondition assert), register it in `identity_scaffolding_shared.rs`'s expected list (`"async fn vetoed_pair(",`, sorted), and delete both local copies. `link_veto_floor.rs` keeps its own (it asserts the veto message; leave it). Verify `cairn_chart_set_has_doubted_link`'s real signature in `db/054_person_charts.sql` and adjust the test's call to it if it takes something other than `uuid[]`.

- [ ] **Step 6: Run to verify failure**

Run: `CAIRN_TEST_PG=… CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test chart_link`
Expected: FAIL to compile (`link_charts`, `unlink_charts`, `Reviewer` not found).

- [ ] **Step 7: Implement the IO half**

Append to `chart_link.rs` (above the tests module):

```rust
/// The human making the judgement: the unlocked signing key and its key id. A judgement
/// about who a person is belongs to a human (ADR-0053) — there is no node-key fallback.
pub struct Reviewer<'a> {
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
}

/// What a judgement wrote, and what the chart now is.
#[derive(Debug)]
pub struct LinkOutcome {
    /// The attested identity event.
    pub event_id: Uuid,
    /// Whether an OPEN `match_proposal` for the pair moved (`applied` / `rejected`).
    pub proposal_resolved: bool,
    /// The chart set of the first chart named, read after commit — what a window reopens.
    pub charts: ChartSet,
}

/// The proposal statuses a human judgement resolves. Every other status is CLOSED and is
/// left exactly as it is: `applied`/`auto_applied` carry an `applied_event_id` that db/019's
/// invariant ties to them, `rejected` is already decided, `retracted` was withdrawn by the
/// matcher. What stands is `patient_link`'s business, not the proposal row's; the row only
/// records how an open proposal was first answered.
const OPEN_PROPOSAL_STATUSES: [&str; 3] = ["pending", "accepted", "review"];

/// Sign, attest and submit one judgement inside the caller's transaction, then move an
/// OPEN proposal for the pair. Returns (event id, whether a proposal moved).
///
/// Shared with `apply_proposal::apply_accepted_proposal`, so the matcher-proposal path
/// and the chart-review path cannot drift. `low`/`high` must already be canonical.
///
/// Errors roll the caller's transaction back when it drops: nothing is written and the
/// proposal does not move (the db/005 gate refuses a non-human attester, db/018 a
/// self-link or empty provenance).
#[allow(clippy::too_many_arguments)]
pub async fn assert_link_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    verb: LinkVerb,
    low: Uuid,
    high: Uuid,
    provenance: &str,
    confidence: Option<&str>,
    reviewer: &Reviewer<'_>,
    hlc: Hlc,
) -> anyhow::Result<(Uuid, bool)> {
    let event_id = Uuid::now_v7();
    let body = build_attested_assertion_body(
        verb, event_id, low, high, provenance, confidence, reviewer.human_kid, hlc,
    );
    // The human both authors (signs) and vouches (attests) — the token is what makes the
    // event "attested" in db/018's one definition, and so what ranks it (decision 5).
    let signed = sign(&body, reviewer.human_sk)?;
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, reviewer.human_kid, "attested", reviewer.human_sk)?;
    let attester_vk = reviewer.human_sk.verifying_key().to_bytes().to_vec();
    tx.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &attester_vk],
    )
    .await?;

    // Move an OPEN proposal for the pair. A link records its event (db/019: applied ⇔
    // applied_event_id set); a rejection records none.
    let applied_event: Option<String> = match verb {
        LinkVerb::Link => Some(event_id.to_string()),
        LinkVerb::Unlink => None,
    };
    let open: Vec<String> = OPEN_PROPOSAL_STATUSES.iter().map(|s| s.to_string()).collect();
    let moved = tx
        .execute(
            "UPDATE match_proposal \
                SET status = $3, applied_event_id = $4::text::uuid, updated_at = clock_timestamp() \
              WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid \
                AND status = ANY($5)",
            &[
                &low.to_string(),
                &high.to_string(),
                &verb.resolved_status(),
                &applied_event,
                &open,
            ],
        )
        .await?;
    Ok((event_id, moved > 0))
}

/// "Same person": link two charts as the reviewer's attested judgement.
pub async fn link_charts(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    judge(client, LinkVerb::Link, a, b, reviewer, node_origin).await
}

/// "Not the same person": record the reviewer's attested judgement that two charts are two
/// people — on a linked pair it splits them; on a pair never linked it is the record that
/// they were looked at and are different (decision 4), which no machine link then undoes.
pub async fn unlink_charts(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    judge(client, LinkVerb::Unlink, a, b, reviewer, node_origin).await
}

/// The shared body of both entry points: pre-checks (legible refusals before anything is
/// signed), one transaction, and the resulting chart set.
async fn judge(
    client: &mut tokio_postgres::Client,
    verb: LinkVerb,
    a: Uuid,
    b: Uuid,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    if a == b {
        anyhow::bail!("{a} and {b} are the same chart — a chart cannot be linked to itself");
    }
    // A judgement is about two charts the human has LOOKED at, so both must be held here
    // (a `patient_chart` row — see `patient::person::ChartIdentity::held`). The floor admits
    // a link naming a chart that has not synced yet, correctly (offline-first); a human's
    // deliberate act from this node has no such excuse, and a typo would otherwise attach a
    // stranger's future chart to this person.
    for chart in [a, b] {
        let held: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM patient_chart WHERE patient_id = $1::text::uuid)",
                &[&chart.to_string()],
            )
            .await?
            .get(0);
        if !held {
            anyhow::bail!(
                "chart {chart} is not held on this node — only a chart you can open here can be \
                 judged the same person as, or a different person from, another"
            );
        }
    }
    // Legibility only; the db/005 gate is the enforcement (a raw-SQL client skipping this
    // still cannot attest with a non-human key).
    if !crate::identify::attester_is_enrolled_human(client, reviewer.human_kid).await? {
        anyhow::bail!(
            "key {} is not an enrolled human actor — linking or unlinking charts is a human \
             judgement (unlock a clinician's key)",
            reviewer.human_kid
        );
    }

    let (low, high) = canonical_pair(a, b);
    let provenance = compose_review_provenance(verb, reviewer.human_kid);
    // The tick self-commits before the transaction; a rolled-back judgement leaves only a
    // clock gap, which the HLC allows (the identify_patient shape).
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let tx = client.transaction().await?;
    let (event_id, proposal_resolved) =
        assert_link_in_tx(&tx, verb, low, high, &provenance, None, reviewer, hlc).await?;
    tx.commit().await?;

    let charts = crate::patient::person::person_charts(client, a).await?;
    Ok(LinkOutcome { event_id, proposal_resolved, charts })
}
```

Add `use` for `ChartSet` at the top (same path `patient/person.rs` uses). `$5` binds a `Vec<String>` as `text[]` — tokio-postgres supports `Vec<String>: ToSql`. If `ANY($5)` against a text column complains about types, write `status = ANY($5::text[])`.

- [ ] **Step 8: Run to verify they pass**

Run: `CAIRN_TEST_PG=… CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test chart_link --test link_precedence --test identity_scaffolding_shared` and `cargo test -p cairn-node --lib chart_link`
Expected: all PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/cairn-node/src/chart_link.rs crates/cairn-node/src/lib.rs \
  crates/cairn-node/tests/chart_link.rs crates/cairn-node/tests/link_precedence.rs \
  crates/cairn-node/tests/common/mod.rs crates/cairn-node/tests/identity_scaffolding_shared.rs
git commit -F <msgfile>   # "feat(R2a): link_charts / unlink_charts — a human judgement, attested, resolving its proposal (#681)"
```

---

### Task 4: `apply_accepted_proposal` becomes a thin wrapper

**Files:**
- Modify: `crates/cairn-node/src/apply_proposal.rs` (the body builder :33-73 and the IO fn :93-180)
- Test: existing `crates/cairn-node/tests/apply_proposal.rs` (unchanged — it IS the behaviour pin), `crates/cairn-node/tests/identify*` via `identify.rs`'s use of `build_attested_link_body`

**Interfaces:**
- Consumes: `chart_link::{assert_link_in_tx, build_attested_assertion_body, canonical_pair, LinkVerb, Reviewer}`.
- Produces: unchanged public signatures `apply_accepted_proposal(client, low, high, human_sk, human_kid, hlc) -> anyhow::Result<Uuid>` and `build_attested_link_body(event_id, low, high, provenance, confidence, human_kid, hlc) -> EventBody`.

- [ ] **Step 1: Pin the behaviour first (it is already pinned — confirm green before touching it)**

Run: `CAIRN_TEST_PG=… CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --test apply_proposal` and `cargo test -p cairn-node --lib apply_proposal identify`
Expected: PASS on the unmodified code. (This is a refactor under existing tests: the RED phase is Step 3's mutation check, not a new test.)

- [ ] **Step 2: Rewrite**

- `build_attested_link_body` becomes a one-line delegate:
```rust
pub fn build_attested_link_body(
    event_id: Uuid, low: Uuid, high: Uuid, provenance: &str, confidence: Option<&str>,
    human_kid: &str, hlc: Hlc,
) -> EventBody {
    crate::chart_link::build_attested_assertion_body(
        crate::chart_link::LinkVerb::Link, event_id, low, high, provenance, confidence, human_kid, hlc,
    )
}
```
  Keep its doc comment, adding: "Delegates to `chart_link::build_attested_assertion_body` — one body builder for every attested identity judgement." Remove now-unused imports (`link_assertion_body`, `render_link_twin`, `LinkAssertion`, `LINK_SCHEMA_VERSION` if unused).
- In `apply_accepted_proposal`, keep steps 0–1 (transaction, canonicalise via `crate::chart_link::canonical_pair`, the `FOR UPDATE` read and the `status != "accepted"` bail) and step 2's provenance/confidence composition; replace steps 2's body build, 3, 4 and 5 with:
```rust
    // 2–5. Build, sign, attest and submit through the shared core, which also moves this
    //      (open, 'accepted') proposal to 'applied' with its event id — in this transaction.
    let reviewer = crate::chart_link::Reviewer { human_sk, human_kid };
    let (event_id, moved) = crate::chart_link::assert_link_in_tx(
        &tx,
        crate::chart_link::LinkVerb::Link,
        low,
        high,
        &provenance,
        Some(&confidence),
        &reviewer,
        hlc,
    )
    .await?;
    // The row was read FOR UPDATE as 'accepted' above, so the core's open-status move must
    // have hit it; anything else is a logic error, not a state to commit.
    anyhow::ensure!(moved, "match_proposal ({low}, {high}) did not move to 'applied'");
    tx.commit().await?;
    Ok(event_id)
```
  Update the module doc's "Split:" paragraph: the IO function now reads the proposal and delegates the write to `chart_link::assert_link_in_tx`.

- [ ] **Step 3: Verify, including one mutation**

Run the Step 1 commands again → PASS. Then, as the RED check for the delegation, temporarily change `OPEN_PROPOSAL_STATUSES` in `chart_link.rs` to omit `"accepted"`, run `--test apply_proposal` → expect FAIL (`did not move to 'applied'`), and restore it (**undo by re-editing the line, never `git checkout --`** — repo memory). Run again → PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/src/apply_proposal.rs
git commit -F <msgfile>   # "refactor(R2a): apply_accepted_proposal delegates to chart_link's core"
```

---

### Task 5: the CLI — `link-charts` and `unlink-charts`

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (two `Cmd` variants beside `IdentifyPatient` ~:1875; two match arms beside its handler ~:4545; one private helper; one unit test in the `mod tests` at ~:5603)

**Interfaces:**
- Consumes: `chart_link::{link_charts, unlink_charts, Reviewer, LinkOutcome}`; existing private `load_attester_key(path, passphrase)`, `cairn_node::db::connect`, `cairn_node::identity::load_local(&db).await?.node_id_hex`.
- Produces: `cairn-node link-charts <A> <B> --attester-key <PATH> [--attester-passphrase …]` and `cairn-node unlink-charts <A> <B> …`.

- [ ] **Step 1: Write the failing parse test**

In `main.rs`'s `mod tests` (check the top-level clap type's name — `Cli` — and that it derives `Parser`):

```rust
    /// The two judgement verbs parse, and a missing attester key is a parse error rather
    /// than a run-time fallback to the node key: an identity judgement is a human's.
    #[test]
    fn link_and_unlink_charts_parse_and_demand_an_attester_key() {
        use clap::Parser;
        let (a, b) = ("0190a000-0000-7000-8000-000000000001", "0190a000-0000-7000-8000-000000000002");
        for verb in ["link-charts", "unlink-charts"] {
            assert!(
                super::Cli::try_parse_from(["cairn-node", verb, a, b, "--attester-key", "/k"]).is_ok(),
                "{verb} parses"
            );
            assert!(
                super::Cli::try_parse_from(["cairn-node", verb, a, b]).is_err(),
                "{verb} without --attester-key must not parse"
            );
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --bin cairn-node link_and_unlink`
Expected: FAIL (unrecognized subcommand).

- [ ] **Step 3: Implement**

Variants, next to `IdentifyPatient`:
```rust
    /// "Same person": link two charts as a human's attested judgement (ADR-0076 decision
    /// 5 — no machine link or unlink can then undo it). Both charts must be held here.
    /// Resolves an open match_proposal for the pair.
    LinkCharts {
        a: Uuid,
        b: Uuid,
        /// The human signing key that makes the judgement (required — never the node key).
        #[arg(long)]
        attester_key: PathBuf,
        /// Passphrase to unseal --attester-key (else CAIRN_ATTESTER_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_ATTESTER_PASSPHRASE")]
        attester_passphrase: Option<String>,
    },
    /// "Not the same person": record a human's attested judgement that two charts are two
    /// people — splits a linked pair, and on a pair never linked stops any matcher from
    /// joining them (ADR-0076 decision 4). Both charts must be held here.
    UnlinkCharts {
        a: Uuid,
        b: Uuid,
        #[arg(long)]
        attester_key: PathBuf,
        #[arg(long, env = "CAIRN_ATTESTER_PASSPHRASE")]
        attester_passphrase: Option<String>,
    },
```

Arms (beside `Cmd::IdentifyPatient`'s):
```rust
        Cmd::LinkCharts { a, b, attester_key, attester_passphrase } => {
            run_chart_judgement(&cli.conn, cairn_node::chart_link::LinkVerb::Link, a, b,
                                &attester_key, attester_passphrase).await?;
        }
        Cmd::UnlinkCharts { a, b, attester_key, attester_passphrase } => {
            run_chart_judgement(&cli.conn, cairn_node::chart_link::LinkVerb::Unlink, a, b,
                                &attester_key, attester_passphrase).await?;
        }
```

Helper (near `load_attester_key`):
```rust
/// `link-charts` / `unlink-charts`: unseal the human's key, make the judgement, and say
/// what the chart now is. No node key is loaded — the event is the human's, signed and
/// attested by them (ADR-0053); the node contributes only its HLC origin.
async fn run_chart_judgement(
    conn: &str,
    verb: cairn_node::chart_link::LinkVerb,
    a: Uuid,
    b: Uuid,
    attester_key: &std::path::Path,
    attester_passphrase: Option<String>,
) -> anyhow::Result<()> {
    use cairn_node::chart_link::{link_charts, unlink_charts, LinkVerb, Reviewer};
    let sk = load_attester_key(attester_key, attester_passphrase)?;
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let mut db = cairn_node::db::connect(conn).await?;
    let origin = cairn_node::identity::load_local(&db).await?.node_id_hex;
    let reviewer = Reviewer { human_sk: &sk, human_kid: &kid };
    let out = match verb {
        LinkVerb::Link => link_charts(&mut db, a, b, &reviewer, &origin).await?,
        LinkVerb::Unlink => unlink_charts(&mut db, a, b, &reviewer, &origin).await?,
    };
    let what = match verb {
        LinkVerb::Link => "linked (same person)",
        LinkVerb::Unlink => "unlinked (not the same person)",
    };
    println!("{a} and {b} {what}; event {}", out.event_id);
    if out.proposal_resolved {
        println!("the open duplicate proposal for this pair is resolved");
    }
    let members: Vec<String> = out.charts.members().iter().map(Uuid::to_string).collect();
    println!("chart {a} now reads as: {}", members.join(", "));
    Ok(())
}
```

(Check `load_local`'s actual field name for the node origin — `identify-patient` passes `&id.node_id_hex`; use the same. Check whether `cli.conn` is the field name `IdentifyPatient` uses.)

- [ ] **Step 4: Run to verify it passes, and the clippy gate**

Run: `CARGO_TARGET_DIR=/tmp/cairn-r2a cargo test -p cairn-node --bin cairn-node link_and_unlink` then `cargo clippy -p cairn-node --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -F <msgfile>   # "feat(R2a): cairn-node link-charts / unlink-charts (#681)"
```

---

### Task 6: currency, spec prose, and the gate

**Files:** `docs/spec/identity.md` (only if its `patient_link` prose states latest-HLC-wins without ADR-0076's rule), the design page (a dated *As built* note under *R2*), `docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1: Spec prose.** `grep -n "patient_link\|latest-HLC\|latest HLC\|HLC-overlay" docs/spec/identity.md`. Where the link overlay's winner rule is stated, add one sentence citing ADR-0076 decision 5 (attested first, then HLC). The ADR is already accepted at v0.78; this aligns prose, so the spec version stays **v0.78** unless the edit changes meaning — if it does, stop and ask.
- [ ] **Step 2: Design page.** Under *R2*, add a dated `> [!NOTE]` "As built (R2a)": R2 split into R2a/R2b; the re-fold is the generation-55 heal, not a backfill; `link_charts` refuses a chart not held here; closed proposals are left untouched; `apply_accepted_proposal` now has a shared core (still no production caller — R5's worklist will be its first).
- [ ] **Step 3: The gate — after the last code edit.** `scripts/run-db-gated-tests.sh` (background; it takes long — poll its log, do Step 4 meanwhile), then in CI's order: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`. Local Postgres is ICU and CI's is libc (trap 17) — nothing here touches case or collation, so one cluster suffices; say so in the PR.
- [ ] **Step 4: HANDOVER + ROADMAP.** ⇒ NEXT: R2a done → **R2b** (the window: "Same person as…" → search → side-by-side panel → `link_charts`; "Not the same person" on each member line → `unlink_charts`; its §1.2 budget measurement), then #697 (b). Add a durable rule: *`patient_link`'s order is attested-first (`cairn_link_overlay_wins`); db/055's existence is what makes existing nodes re-fold — never fold it into db/018; "attested" is computed once per applied event and stored.* Record any filings.
- [ ] **Step 5: Commit, push, PR** (house rules 8–10): PR body links #681 without a closing keyword (R2b still owes the gesture), states the gate evidence, and ends with the Claude Code attribution line.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk laying two folders side by side and either clipping them together ("same patient") or writing "NOT the same patient — checked, [initials], [date]" on both covers so nobody clips them later.
- **Steps:** paper 3 (fetch the other folder, lay them side by side, clip or annotate) → architecture-forced 3 (find the chart, look, sign — the attested event IS the clip or the annotation; per-write authorship adds no act while the key is unlocked) → UI target 3 (R2b's gesture; 2 from R5's banner, where the find is done). This slice adds no act: the CLI verb is one command given the two charts, and D5 is invisible — it removes the paper failure where a later clerk re-clips folders someone marked "not the same patient". `M ≤ N`.
- **Time + cognitive load:** budget — review-and-link from the banner ≤ 20 s, the side-by-side read being the load (the design's figure). R2a exposes no clinical-surface runnable gesture beyond the CLI, so the measurement is owed by **R2b** (the runbook, a human act). The upgrade heal is a one-time loader cost bounded by the existing generation-change heal; no per-open cost is added.
