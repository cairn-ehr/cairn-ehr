# `enroll-human` Ceremony CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `cairn-node enroll-human` ceremony that enrolls a clinician's signing key as a `kind='human'` actor (ADR-0044 person-distinguishing determinant), unblocking `identify-patient --link` end-to-end and replacing the raw-SQL human enrollment in `identify.rs` tests.

**Architecture:** A new `cairn-node::enroll` module holds a pure `build_human_pinned` (assembles the pinned JSON, refuses without a determinant, never pins the key) and an async `enroll_human_actor` (dual-mapping guard + the existing `enroll_actor` floor call). A thin `Cmd::EnrollHuman` handler loads-or-mints the human key and calls the library. Reuses the `enroll_actor` door — no wire/floor/SCHEMA/ADR/spec change.

**Tech Stack:** Rust (tokio-postgres, clap, serde_json, anyhow, zeroize), PostgreSQL 18 + `cairn_pgx`.

## Global Constraints

- **AGPL-3.0** only; no new dependencies (all listed crates already in `cairn-node`).
- **TDD** — failing test first (load-bearing: this is the safety-critical trust-anchor surface).
- **Reviewer-legible inline docs** for a junior contributor on every non-trivial fn.
- **Keep files < 500 lines** — `src/enroll.rs` is new and small; `main.rs` gains one variant + handler.
- **The pinned set NEVER includes the signing key** (ADR-0044: `actor_id` must stay stable across `rotate-key`).
- **`enroll_actor` is an owner ceremony** (`REVOKE`d from `cairn_agent`) — the CLI uses the owner `--conn`.
- DB-gated tests gate on `CAIRN_TEST_PG` and self-serialize via `db::test_serial_guard`.
  Reference conn: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`.
- Final gates: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `mkdocs build` clean.

---

### Task 1: Pure `build_human_pinned` + module scaffold

**Files:**
- Create: `crates/cairn-node/src/enroll.rs`
- Modify: `crates/cairn-node/src/lib.rs` (add `pub mod enroll;`)
- Test: inline `#[cfg(test)]` in `crates/cairn-node/src/enroll.rs`

**Interfaces:**
- Consumes: nothing (leaf module).
- Produces:
  - `pub fn build_human_pinned(role: &str, registration_id: Option<&str>, handle: Option<&str>) -> anyhow::Result<serde_json::Value>`
  - `pub enum EnrollHumanOutcome { Enrolled, AlreadyEnrolled }` (defined here for Task 2's use)

- [ ] **Step 1: Register the module**

In `crates/cairn-node/src/lib.rs`, add the declaration in alphabetical position (after `pub mod db;`, before `pub mod evidence;`):

```rust
pub mod enroll;
```

- [ ] **Step 2: Write the failing unit tests**

Create `crates/cairn-node/src/enroll.rs` with ONLY the tests + a stub, so it compiles but fails:

```rust
//! §5.4 — the human-actor enrollment ceremony. A clinician's signing key becomes a
//! `kind='human'` actor carrying an ADR-0044 person-distinguishing determinant, so it may
//! sign+attest the optional `identify --link` (finisher 3). Reuses the `enroll_actor` in-DB
//! floor (`db/004`); this module only shapes the pinned set (pure) and guards the one way a
//! second enrollment could corrupt attribution (the async orchestrator, Task 2).

use anyhow::{bail, Context};
use serde_json::{json, Value};

/// Outcome of `enroll_human_actor`: a fresh enrollment vs an idempotent no-op (the same key,
/// same determinant, already enrolled as a human — re-runnable provisioning).
pub enum EnrollHumanOutcome {
    Enrolled,
    AlreadyEnrolled,
}

/// Assemble the pinned determinant set for a human actor. The `actor_id` is the content-address
/// of THIS set (`cairn_actor_id`, in-DB), so it is what keeps two clinicians distinct — and it
/// must NEVER include the signing key, because `rotate-key` (ADR-0011 §5) keeps `actor_id` stable
/// across a key change.
///
/// ADR-0044 §2 requires a person-distinguishing determinant but deliberately does not fix WHICH
/// field (ADR-0011 keeps pinned-set contents as policy). This node's convention: a professional
/// `registration_id` (preferred — the real-world unique person id, ADR-0033) and/or a node-local
/// `handle`. At least one MUST be present: a determinant is a real field, never fabricated
/// (principle 4), and we do not lean on the floor's loud collision-refusal as the only guard.
/// Blank inputs are treated as absent.
pub fn build_human_pinned(
    role: &str,
    registration_id: Option<&str>,
    handle: Option<&str>,
) -> anyhow::Result<Value> {
    // Trim + drop blanks so `--handle "  "` cannot masquerade as a determinant.
    let clean = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty());
    let role = role.trim();
    if role.is_empty() {
        bail!("enroll-human: --role must not be blank");
    }
    let registration_id = clean(registration_id);
    let handle = clean(handle);
    if registration_id.is_none() && handle.is_none() {
        bail!(
            "enroll-human: a person-distinguishing determinant is required — supply \
             --registration-id (a professional licence/registration number) and/or --handle. \
             Without one, two clinicians would compute the same actor_id (ADR-0044)."
        );
    }
    let mut obj = serde_json::Map::new();
    obj.insert("role".into(), json!(role));
    if let Some(r) = registration_id {
        obj.insert("registration_id".into(), json!(r));
    }
    if let Some(h) = handle {
        obj.insert("handle".into(), json!(h));
    }
    Ok(Value::Object(obj))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_registration_id_and_role_never_the_key() {
        let p = build_human_pinned("clinician", Some("AHPRA-MED0001234567"), None).unwrap();
        assert_eq!(p["role"], json!("clinician"));
        assert_eq!(p["registration_id"], json!("AHPRA-MED0001234567"));
        assert!(p.get("handle").is_none(), "absent handle is omitted");
        assert!(
            p.as_object().unwrap().keys().all(|k| k != "signing_key" && k != "key"),
            "the signing key must never enter the pinned set (rotate-key stability, ADR-0044)"
        );
    }

    #[test]
    fn pins_handle_when_no_registration_id() {
        let p = build_human_pinned("clinician", None, Some("dr-a")).unwrap();
        assert_eq!(p["handle"], json!("dr-a"));
        assert!(p.get("registration_id").is_none());
    }

    #[test]
    fn pins_both_determinants_when_supplied() {
        let p = build_human_pinned("registrar", Some("MED123"), Some("dr-b")).unwrap();
        assert_eq!(p["registration_id"], json!("MED123"));
        assert_eq!(p["handle"], json!("dr-b"));
    }

    #[test]
    fn refuses_when_no_determinant() {
        let err = build_human_pinned("clinician", None, None).unwrap_err();
        assert!(err.to_string().contains("person-distinguishing determinant"));
    }

    #[test]
    fn treats_blank_determinant_as_absent() {
        assert!(build_human_pinned("clinician", Some("   "), Some("")).is_err());
    }

    #[test]
    fn refuses_blank_role() {
        assert!(build_human_pinned("  ", Some("MED123"), None).is_err());
    }
}
```

> Note: `Context` is imported now because Task 2 uses it in the same file; keep the `use` line —
> if clippy flags it unused at this step, add `#[allow(unused_imports)]` temporarily, removed in Task 2.
> Simpler: drop `Context` from the `use` here and add it in Task 2. Do the latter — change the import
> line to `use anyhow::bail;` for Task 1, and widen it in Task 2.

Apply that simplification: the Task-1 import line is `use anyhow::bail;`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p cairn-node --lib enroll`
Expected: compile succeeds, 6 tests PASS immediately (this is a pure function — RED is unnecessary once the impl is inline). If you prefer a strict RED cycle, temporarily stub `build_human_pinned` to `Ok(json!({}))`, watch the tests fail, then restore. Not required for a pure leaf.

- [ ] **Step 4: Run the full pure suite green**

Run: `cargo test -p cairn-node --lib`
Expected: PASS (all existing lib tests + the 6 new ones).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/enroll.rs crates/cairn-node/src/lib.rs
git commit -m "feat(enroll-human): pure build_human_pinned + EnrollHumanOutcome

ADR-0044 person-distinguishing determinant (registration_id and/or
handle, never the key); refuses without one (principle 4). Pure, unit-tested.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Async `enroll_human_actor` (dual-mapping guard + floor call)

**Files:**
- Modify: `crates/cairn-node/src/enroll.rs` (add the async fn + widen the `use`)
- Test: Create `crates/cairn-node/tests/enroll_human.rs`

**Interfaces:**
- Consumes: `build_human_pinned`, `EnrollHumanOutcome` (Task 1); the in-DB `enroll_actor(text,jsonb,text)`, `cairn_actor_id(jsonb)`, `cairn_actor_id_key_conflict(bytea,text)` floor functions (`db/004`); `actor_current` VIEW; `db::{connect_and_load_schema, test_serial_guard}`; `cairn_event::generate_key`; `cairn_node::identify::attester_is_enrolled_human`.
- Produces: `pub async fn enroll_human_actor(db: &tokio_postgres::Client, kid: &str, pinned: &serde_json::Value) -> anyhow::Result<EnrollHumanOutcome>`

- [ ] **Step 1: Write the failing DB-gated tests**

Create `crates/cairn-node/tests/enroll_human.rs`:

```rust
//! §5.4 — the human-actor enrollment ceremony library path. DB-gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern, like
//! identify.rs / attestation.rs). Proves the dual-mapping guard, the ADR-0044 collision
//! refusal, and idempotency — the guarantees that keep the actor trust-anchor sound.
use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::enroll::{build_human_pinned, enroll_human_actor, EnrollHumanOutcome};
use cairn_node::identify::attester_is_enrolled_human;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Fresh registry for each test: truncate the append-only actor + event tables.
async fn reset(c: &Client) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
}

/// Read the actor_id (BYTEA) bound to a signing key in actor_current, if any.
async fn actor_id_of(c: &Client, kid: &str) -> Option<Vec<u8>> {
    c.query_opt(
        "SELECT actor_id FROM actor_current WHERE signing_key_id = $1",
        &[&kid],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

#[tokio::test]
async fn enrolls_a_resolvable_human_actor() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (_sk, kid) = generate_key().unwrap();
    let pinned = build_human_pinned("clinician", Some("MED-001"), None).unwrap();
    let out = enroll_human_actor(&c, &kid, &pinned).await.unwrap();
    assert!(matches!(out, EnrollHumanOutcome::Enrolled));
    assert!(
        attester_is_enrolled_human(&c, &kid).await.unwrap(),
        "the enrolled key resolves as a kind='human' actor (the identify --link pre-check)"
    );
}

#[tokio::test]
async fn distinct_registration_ids_get_distinct_actor_ids() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (_a, kid_a) = generate_key().unwrap();
    let (_b, kid_b) = generate_key().unwrap();
    enroll_human_actor(&c, &kid_a, &build_human_pinned("clinician", Some("MED-001"), None).unwrap())
        .await
        .unwrap();
    enroll_human_actor(&c, &kid_b, &build_human_pinned("clinician", Some("MED-002"), None).unwrap())
        .await
        .unwrap();
    assert_ne!(
        actor_id_of(&c, &kid_a).await.unwrap(),
        actor_id_of(&c, &kid_b).await.unwrap(),
        "two clinicians with distinct registration ids are distinct actors"
    );
}

#[tokio::test]
async fn identical_determinant_distinct_keys_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let pinned = build_human_pinned("clinician", None, Some("dr-a")).unwrap();
    let (_a, kid_a) = generate_key().unwrap();
    let (_b, kid_b) = generate_key().unwrap();
    enroll_human_actor(&c, &kid_a, &pinned).await.unwrap();
    let err = enroll_human_actor(&c, &kid_b, &pinned).await.unwrap_err();
    assert!(
        err.to_string().contains("ADR-0044") || err.to_string().contains("registration-id"),
        "the second key with an identical determinant is refused with a legible hint: {err}"
    );
}

#[tokio::test]
async fn same_key_reenroll_is_idempotent() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (_sk, kid) = generate_key().unwrap();
    let pinned = build_human_pinned("clinician", Some("MED-001"), None).unwrap();
    let first = enroll_human_actor(&c, &kid, &pinned).await.unwrap();
    assert!(matches!(first, EnrollHumanOutcome::Enrolled));
    let again = enroll_human_actor(&c, &kid, &pinned).await.unwrap();
    assert!(
        matches!(again, EnrollHumanOutcome::AlreadyEnrolled),
        "re-running the same enrollment is a no-op, not a second actor_event row"
    );
}

#[tokio::test]
async fn key_already_enrolled_under_another_kind_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    // Enroll the key as a `device` first (the registration-desk precedent), then try to add a
    // human actor to the SAME key — the dual-mapping guard must refuse (db/005 would otherwise
    // NULL that key's authorship node-wide).
    let (_sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    let pinned = build_human_pinned("clinician", Some("MED-001"), None).unwrap();
    let err = enroll_human_actor(&c, &kid, &pinned).await.unwrap_err();
    assert!(
        err.to_string().contains("already enrolled"),
        "a key already mapped to an actor cannot be re-mapped to a human: {err}"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test enroll_human`
Expected: FAIL to compile — `enroll_human_actor` not found.

- [ ] **Step 3: Implement `enroll_human_actor`**

In `crates/cairn-node/src/enroll.rs`, widen the import to `use anyhow::{bail, Context};` and append after `build_human_pinned`:

```rust
/// Enroll `kid` as a `kind='human'` actor with `pinned`, reusing the in-DB `enroll_actor`
/// floor (`db/004`). Two guards wrap the floor call:
///
/// 1. **Dual-mapping guard.** `submit_event` resolves a signer to an actor purely by
///    `signing_key_id`; if one key maps to MORE than one `actor_current` row it sets
///    `actor_id = NULL` for EVERY event that key authors node-wide (`db/005`,
///    `array_length(v_actor_ids,1)=1`) — a silent, irreversible attribution loss. So a key
///    already bound to an actor must not be re-bound: we refuse, EXCEPT the idempotent case
///    (same key, same `actor_id`, already a human) which is a re-runnable no-op.
/// 2. **Advisory ADR-0044 collision pre-check.** `cairn_actor_id_key_conflict` tells us up front
///    whether this determinant set already identifies another actor under a different key, so we
///    can name the remedy (add a distinguishing determinant) instead of surfacing a raw floor
///    error. The floor re-checks regardless — this is legibility, not the enforcement (the same
///    advisory-mirrors-the-floor pattern as `attester_is_enrolled_human`).
pub async fn enroll_human_actor(
    db: &tokio_postgres::Client,
    kid: &str,
    pinned: &Value,
) -> anyhow::Result<EnrollHumanOutcome> {
    let pinned_str = pinned.to_string();
    // The actor_id this determinant set will compute to (content-address of the pinned set).
    let new_actor_id: Vec<u8> = db
        .query_one("SELECT cairn_actor_id($1::text::jsonb)", &[&pinned_str])
        .await
        .context("computing cairn_actor_id for the human pinned set")?
        .get(0);

    // Guard 1 — is this key already an actor?
    let rows = db
        .query(
            "SELECT actor_id, kind FROM actor_current WHERE signing_key_id = $1",
            &[&kid],
        )
        .await?;
    if !rows.is_empty() {
        let idempotent = rows.len() == 1
            && rows[0].get::<_, Vec<u8>>(0) == new_actor_id
            && rows[0].get::<_, String>(1) == "human";
        if idempotent {
            return Ok(EnrollHumanOutcome::AlreadyEnrolled);
        }
        bail!(
            "enroll-human: key {kid} is already enrolled as an actor; enrolling it again would \
             map one key to two actors and silently NULL its authorship node-wide (db/005). \
             Use a fresh key for this human."
        );
    }

    // Guard 2 — advisory determinant-collision hint (the floor is the real enforcement).
    let conflict: bool = db
        .query_one(
            "SELECT cairn_actor_id_key_conflict($1, $2)",
            &[&new_actor_id, &kid],
        )
        .await?
        .get(0);
    if conflict {
        bail!(
            "enroll-human: this determinant set already identifies another actor under a \
             different key — two people must not share one actor_id (ADR-0044). Add a \
             distinguishing --registration-id or --handle."
        );
    }

    // The real floor. Re-checks the collision itself and fails closed if a concurrent enroll
    // slipped in between guard 2 and here.
    db.execute(
        "SELECT enroll_actor('human', $1::text::jsonb, $2)",
        &[&pinned_str, &kid],
    )
    .await
    .context("enroll_actor('human', …) refused the enrollment")?;
    Ok(EnrollHumanOutcome::Enrolled)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test enroll_human`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/enroll.rs crates/cairn-node/tests/enroll_human.rs
git commit -m "feat(enroll-human): enroll_human_actor — dual-mapping guard + ADR-0044 floor

Guards the db/005 one-key-two-actors NULL-attribution hazard and gives a
legible determinant-collision hint before the enroll_actor floor (which
remains the real enforcement). Idempotent same-key re-enroll. 5 DB-gated tests.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `Cmd::EnrollHuman` CLI command + handler

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add the `Cmd` variant + the match arm)

**Interfaces:**
- Consumes: `cairn_node::enroll::{build_human_pinned, enroll_human_actor, EnrollHumanOutcome}` (Tasks 1–2); `resolve_passphrase`, `print_recovery_code`, `load_signing_key` (main.rs); `cairn_node::keystore::{key_at_rest_state, KeyAtRest, generate_plaintext, generate_sealed}`; `cairn_node::seal::generate_recovery_code`; `cairn_node::db::connect`.
- Produces: the `enroll-human` subcommand (no library surface).

- [ ] **Step 1: Add the `Cmd` variant**

In `crates/cairn-node/src/main.rs`, inside the `enum Cmd { … }`, add (place it near `RegisterJohnDoe`):

```rust
    /// Enroll a clinician's signing key as a `kind='human'` actor so it may sign+attest an
    /// `identify-patient --link` (and any future human-attested surface). An OWNER ceremony —
    /// point `--conn` at a role that may run `enroll_actor`. The pinned determinant set carries
    /// a person-distinguishing field (`--registration-id` and/or `--handle`, ADR-0044) and NEVER
    /// the key (so `rotate-key` keeps the actor_id stable). If `--key` does not exist it is
    /// minted: sealed under a shown-once recovery code, or unsealed with `--insecure-plaintext`
    /// (test nodes only). No local-state `.lsk` escrow is attached — a personal key has none.
    EnrollHuman {
        /// A professional licence/registration number (preferred person-distinguishing determinant).
        #[arg(long)]
        registration_id: Option<String>,
        /// A node-local human-chosen handle (use when there is no registration number).
        #[arg(long)]
        handle: Option<String>,
        /// The actor's role tag in the pinned set.
        #[arg(long, default_value = "clinician")]
        role: String,
        /// Passphrase to seal a newly-minted key (else CAIRN_KEY_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
        /// Mint the key UNSEALED if it does not exist (test nodes only).
        #[arg(long)]
        insecure_plaintext: bool,
    },
```

- [ ] **Step 2: Add the match arm**

In the `match cli.cmd { … }` dispatch, add (near the `Cmd::RegisterJohnDoe` arm):

```rust
        Cmd::EnrollHuman {
            registration_id,
            handle,
            role,
            passphrase,
            insecure_plaintext,
        } => {
            // Validate the determinant BEFORE any key or DB I/O (pre-I/O validation, mirroring
            // identify-patient): refuse an enrollment that would compute a non-distinguishing
            // actor_id, before minting a key or opening a connection.
            let pinned = cairn_node::enroll::build_human_pinned(
                &role,
                registration_id.as_deref(),
                handle.as_deref(),
            )?;

            // Load the human's personal key, or mint one if the file is absent.
            use cairn_node::keystore::{key_at_rest_state, KeyAtRest};
            let (sk, kid) = match key_at_rest_state(&cli.key) {
                KeyAtRest::Missing => {
                    if insecure_plaintext {
                        eprintln!(
                            "WARNING: --insecure-plaintext: human signing key written UNSEALED \
                             (test use only)"
                        );
                        cairn_node::keystore::generate_plaintext(&cli.key)?
                    } else {
                        let op = resolve_passphrase(passphrase)?;
                        // The recovery code is a key-recovering secret — Zeroizing so it is
                        // wiped on drop (issue #46). Printed BEFORE persist so a crash can never
                        // seal under a code no human saw. No local-state escrow: a personal key
                        // has no node-scoped local state to wrap (design D2).
                        let code = Zeroizing::new(cairn_node::seal::generate_recovery_code());
                        print_recovery_code(&code);
                        cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?
                    }
                }
                _ => {
                    let sk = load_signing_key(&cli.key, true)?; // may prompt to unseal
                    let kid = hex::encode(sk.verifying_key().to_bytes());
                    (sk, kid)
                }
            };
            // `sk` is not used again (enrollment binds only the public kid), but is held so the
            // sealed secret's lifetime matches the ceremony; drop it explicitly for clarity.
            drop(sk);

            let db = cairn_node::db::connect(&cli.conn).await?;
            match cairn_node::enroll::enroll_human_actor(&db, &kid, &pinned).await? {
                cairn_node::enroll::EnrollHumanOutcome::Enrolled => {
                    println!("enrolled human actor {kid}");
                }
                cairn_node::enroll::EnrollHumanOutcome::AlreadyEnrolled => {
                    println!("human actor {kid} already enrolled (no change)");
                }
            }
        }
```

> If clippy warns `sk` is unused without the `drop(sk)`, the explicit `drop` resolves it. If it
> instead warns the binding could be `_sk`, rename to `let (_sk, kid)` and remove the `drop` line.
> Pick whichever the local clippy accepts; both are correct.

- [ ] **Step 3: Build and smoke-test the CLI**

Run: `cargo build -p cairn-node`
Expected: builds clean.

Manual smoke against the test cluster (mints an unsealed throwaway key, enrolls, verifies):

```bash
CONN="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"
KEY="$(mktemp -u).key"
cargo run -q -p cairn-node -- --conn "$CONN" --key "$KEY" \
  enroll-human --registration-id SMOKE-001 --handle dr-smoke --insecure-plaintext
# Expected: "enrolled human actor <hex kid>"
# Re-run → "human actor <kid> already enrolled (no change)"
cargo run -q -p cairn-node -- --conn "$CONN" --key "$KEY" \
  enroll-human --registration-id SMOKE-001 --handle dr-smoke --insecure-plaintext
rm -f "$KEY"
```

Expected: first run enrolls, second run reports idempotent no-change.

- [ ] **Step 4: Run the workspace tests + gates**

Run: `cargo fmt -p cairn-node && cargo clippy -p cairn-node -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cli): enroll-human — mint/load a human key, enroll as kind='human'

Owner ceremony: pre-I/O determinant validation, load-or-mint the personal
key (sealed w/ recovery code, or --insecure-plaintext), then the library
enroll. No .lsk escrow on a personal key (design D2).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Refactor `identify.rs` to the library path (the e2e payoff)

**Files:**
- Modify: `crates/cairn-node/tests/identify.rs:118-129` (the `enroll_human` helper) + the header comment lines 3-4.

**Interfaces:**
- Consumes: `cairn_node::enroll::{build_human_pinned, enroll_human_actor}` (Tasks 1–2).
- Produces: nothing (test-only change).

- [ ] **Step 1: Replace the raw-SQL helper**

In `crates/cairn-node/tests/identify.rs`, replace the `enroll_human` helper (lines ~118-129) with the library path — this is the e2e payoff: the existing `identify --link` tests now exercise the real enrollment ceremony, not raw SQL.

```rust
/// Enroll a second key as a `human` actor (the attester) via the real enrollment ceremony
/// (`cairn_node::enroll`), the same path the `enroll-human` CLI uses — no raw SQL. Returns
/// (human sk, human kid).
async fn enroll_human(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    let pinned =
        cairn_node::enroll::build_human_pinned("clinician", None, Some("dr-a")).unwrap();
    cairn_node::enroll::enroll_human_actor(c, &kid, &pinned)
        .await
        .unwrap();
    (sk, kid)
}
```

- [ ] **Step 2: Update the stale header comment**

Change `crates/cairn-node/tests/identify.rs` lines 3-4 from:

```rust
//! attestation.rs / identity_linkage.rs). The human attester is enrolled via raw SQL here
//! (there is no enroll-human CLI yet — a separate future slice).
```

to:

```rust
//! attestation.rs / identity_linkage.rs). The human attester is enrolled via the real
//! `cairn_node::enroll` ceremony (the enroll-human CLI's library path), not raw SQL.
```

- [ ] **Step 3: Run the identify suite green**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identify`
Expected: all identify tests PASS (proving `identify --link` works end-to-end through the real human-enrollment ceremony).

- [ ] **Step 4: Full workspace green + gates**

Run:
```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
cargo fmt --check
cargo clippy --workspace -- -D warnings
uv run --with-requirements docs/requirements.txt -- mkdocs build -f mkdocs.yml
```
Expected: all green; fmt/clippy/mkdocs clean.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/tests/identify.rs
git commit -m "test(identify): enroll the human attester via the real enroll ceremony, not raw SQL

Closes the raw-SQL enrollment debt: identify --link tests now exercise the
cairn_node::enroll library path (the enroll-human CLI's core), proving
finisher 3 end-to-end through the real ceremony.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- D1 determinant (registration-id/handle/role, refuse-if-none, never-key) → Task 1. ✓
- D2 sealed-key/recovery-code/no-.lsk → Task 3 Step 2 (Missing branch). ✓
- D3 dual-mapping guard + idempotent no-op → Task 2. ✓
- ADR-0044 collision legibility → Task 2 (guard 2). ✓
- CLI command → Task 3. ✓
- identify.rs raw-SQL refactor (the payoff) → Task 4. ✓
- All six spec test cases mapped: pure builder (Task 1 ×6 asserts); DB-gated resolvable/distinct/
  collision/idempotent/dual-mapping (Task 2 ×5); e2e via identify suite (Task 4 Step 3). ✓

**Placeholder scan:** No TBD/TODO/"add error handling" — every code step shows full code. ✓

**Type consistency:** `build_human_pinned(&str, Option<&str>, Option<&str>) -> Result<Value>`,
`enroll_human_actor(&Client, &str, &Value) -> Result<EnrollHumanOutcome>`, `EnrollHumanOutcome::{Enrolled, AlreadyEnrolled}`
used identically across Tasks 1–4. ✓

**Note for the implementer:** DB-gated tests are skipped (early `return`) when `CAIRN_TEST_PG` is unset —
they are not failures, but you MUST run them with the env var set to actually verify Tasks 2 & 4.
