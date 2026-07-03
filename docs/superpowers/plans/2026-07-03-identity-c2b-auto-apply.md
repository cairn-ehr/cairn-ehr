# C2b — Auto-Apply of the Matcher's `auto_candidate` Band — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-apply a matcher `auto_candidate` proposal as a matcher-authored, un-attested, recallable `identity.link.asserted` event through the existing `submit_event` door — no human in the loop.

**Architecture:** All in `cairn-node` (Rust). A per-epoch matcher `agent` actor (one fresh signing key per `matcher_version`, auto-enrolled) authors an un-attested link (contributor role `suggested`, no `responsibility`). A driver selects `pending` auto_candidate proposals, re-checks the db/016 veto (kicking a since-vetoed pair to human `review`), signs, submits, and marks the proposal `auto_applied`. No `db/` migration, no floor change, no SCHEMA bump.

**Tech Stack:** Rust, `tokio-postgres`, `cairn-event` (COSE/Ed25519 signing + identity builders), `cairn_pgx` in-DB verify, PostgreSQL 18.

## Global Constraints

- **License:** AGPL-3.0; no new dependency without an AGPL-compatible license check. (This plan adds **no** new crate.)
- **TDD:** failing test first, then minimal code. No production code without a driving test.
- **Docs:** every non-trivial fn/module carries junior-legible inline comments (why + how it fits).
- **File size:** keep files < 500 lines; `auto_apply.rs` and `matcher_actor.rs` are new and small.
- **No floor change:** `submit_event` (db/005), db/018, db/016, db/004 are reused **unmodified**. No file under `db/` is edited. No `SCHEMA` array change in `db.rs`.
- **Purity split:** pure (no-DB) functions are unit-tested under plain `cargo test`; IO functions are DB-gated on `CAIRN_TEST_PG` and serialized cluster-wide via `db::test_serial_guard`.
- **UUID binding convention:** this crate's `tokio-postgres` has no uuid `ToSql`; pass UUIDs as `.to_string()` and cast in SQL with `$N::text::uuid` (see `apply_proposal.rs`).
- **kid derivation:** a key-id is `hex::encode(sk.verifying_key().to_bytes())` (matches `cairn_event::generate_key`).
- **Contributor invariant:** the matcher contributor is `{"actor_id": kid, "role": "suggested"}` with **no** `responsibility` key → `submit_event` requires no attestation.
- **Test DB command:** `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test auto_apply` (PG18 + `cairn_pgx`).

---

## File Structure

- **Create** `crates/cairn-node/src/matcher_actor.rs` — per-epoch matcher actor: pure determinants + key filename, and the IO `resolve_matcher_actor` (load-or-generate key, idempotent enroll).
- **Create** `crates/cairn-node/src/auto_apply.rs` — pure link-body/provenance builders + the IO `apply_auto_candidate` (one proposal) and `apply_auto_candidates` (batch driver).
- **Modify** `crates/cairn-node/src/lib.rs` — add `pub mod matcher_actor;` and `pub mod auto_apply;`.
- **Modify** `crates/cairn-node/src/main.rs` — add the `ApplyAutoCandidates` CLI subcommand.
- **Create** `crates/cairn-node/tests/auto_apply.rs` — DB-gated integration tests.

Pure unit tests live inside `matcher_actor.rs` / `auto_apply.rs` `#[cfg(test)]` modules (project convention, see `apply_proposal.rs`).

---

## Task 1: Pure matcher-actor determinants

**Files:**
- Create: `crates/cairn-node/src/matcher_actor.rs`
- Modify: `crates/cairn-node/src/lib.rs` (add `pub mod matcher_actor;`)

**Interfaces:**
- Produces: `matcher_pinned(matcher_version: &str) -> serde_json::Value`; `matcher_key_filename(matcher_version: &str) -> String`.

- [ ] **Step 1: Add the module declaration**

In `crates/cairn-node/src/lib.rs`, add after `pub mod localstate;` (keep the list alphabetical-ish, matching existing order):

```rust
pub mod matcher_actor;
```

- [ ] **Step 2: Write the failing pure tests**

Create `crates/cairn-node/src/matcher_actor.rs` with ONLY the test module and stub signatures:

```rust
//! The per-epoch matcher actor (§7.5 / ADR-0029). Each distinct `matcher_version`
//! (already `"{pkg}+{weights-digest}"`, ADR-0014's config-pin) is its OWN `agent`
//! actor with its OWN signing key. A fresh key per epoch gives UNIQUE key->actor
//! attribution, so `submit_event` stamps `event_log.actor_id` precisely and a
//! contamination-cascade recall (db/006) selects exactly one config's auto-links.
//!
//! Pure determinants here; the IO `resolve_matcher_actor` (load-or-generate key +
//! idempotent enroll) is added in a later task.

use serde_json::{json, Value};

/// The ADR-0029 pinned determinant set for a matcher epoch. `matcher_version` IS the
/// epoch; the actor_id is its content-address (`cairn_actor_id`, in-DB). Deterministic:
/// same version -> byte-identical pinned set -> same actor_id on every node.
pub fn matcher_pinned(matcher_version: &str) -> Value {
    json!({ "kind": "agent", "actor": "cairn-matcher", "matcher_version": matcher_version })
}

/// A filesystem-safe, collision-free filename for a per-epoch key. `matcher_version`
/// contains `.` and `+` (e.g. `0.3.0+ab12cd34ef56`); we keep alphanumerics and map every
/// other byte to `_`, then append `.key`. Injective enough for our version strings (the
/// digest suffix disambiguates), and never escapes the keystore dir (no `/`, no `..`).
pub fn matcher_key_filename(matcher_version: &str) -> String {
    let safe: String = matcher_version
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    format!("matcher_{safe}.key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_is_deterministic_agent_with_version() {
        let p = matcher_pinned("0.3.0+abc123");
        assert_eq!(p["kind"], "agent");
        assert_eq!(p["actor"], "cairn-matcher");
        assert_eq!(p["matcher_version"], "0.3.0+abc123");
        // Deterministic: same input -> identical JSON value.
        assert_eq!(p, matcher_pinned("0.3.0+abc123"));
    }

    #[test]
    fn distinct_versions_give_distinct_pinned_sets() {
        assert_ne!(matcher_pinned("0.3.0+aaa"), matcher_pinned("0.3.0+bbb"));
    }

    #[test]
    fn key_filename_is_safe_and_distinct() {
        let f = matcher_key_filename("0.3.0+abc123");
        assert!(f.starts_with("matcher_"));
        assert!(f.ends_with(".key"));
        assert!(!f.contains('/') && !f.contains("..") && !f.contains('+') && !f.contains('.') || f.ends_with(".key"));
        // Distinct epochs -> distinct filenames.
        assert_ne!(matcher_key_filename("0.3.0+aaa"), matcher_key_filename("0.3.0+bbb"));
    }
}
```

- [ ] **Step 3: Run the pure tests to verify they pass**

Run: `cd crates/cairn-node && cargo test matcher_actor::tests -- --nocapture`
Expected: 3 passing (the functions are already implemented above — this task is small enough that test + impl land together; verify green).

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/src/matcher_actor.rs crates/cairn-node/src/lib.rs
git commit -m "feat(identity): C2b pure matcher-actor determinants (pinned set + key filename)"
```

---

## Task 2: Pure link-body + provenance builders

**Files:**
- Create: `crates/cairn-node/src/auto_apply.rs`
- Modify: `crates/cairn-node/src/lib.rs` (add `pub mod auto_apply;`)

**Interfaces:**
- Consumes: `cairn_event::identity::{LinkAssertion, link_assertion_body, render_link_twin}`, `cairn_event::{EventBody, Hlc}`, `uuid::Uuid`.
- Produces:
  - `build_suggested_link_body(event_id: Uuid, low: Uuid, high: Uuid, provenance: &str, confidence: Option<&str>, matcher_kid: &str, hlc: Hlc) -> EventBody`
  - `compose_auto_provenance(matcher_version: &str) -> String`

- [ ] **Step 1: Add the module declaration**

In `crates/cairn-node/src/lib.rs`, add (keep near `apply_proposal`):

```rust
pub mod auto_apply;
```

- [ ] **Step 2: Write the failing pure tests + builders**

Create `crates/cairn-node/src/auto_apply.rs`:

```rust
//! §5.2/§5.7 C2b — auto-apply of the matcher's `auto_candidate` band. Sibling of
//! `apply_proposal.rs` (the human-accepted C2 seam). Here the MATCHER authors the link
//! un-attested (contributor role `suggested`, no `responsibility`), so `submit_event`
//! requires NO attestation token (db/018: an identity link is additive +
//! targets_other_author=FALSE). Recallability comes for free: the matcher is a real
//! per-epoch `agent` actor (see matcher_actor.rs), so the db/006 recall surface can
//! recall a bad config's auto-links precisely.
//!
//! Split: pure body/provenance assembly (unit-tested, no DB) + IO functions (a single
//! proposal and a batch driver) added in later tasks.

use cairn_event::identity::{link_assertion_body, render_link_twin, LinkAssertion};
use cairn_event::{EventBody, Hlc};
use uuid::Uuid;

/// schema_version for a link event (mirrors the C1/C2 convention).
const LINK_SCHEMA_VERSION: &str = "identity.link/1";

/// Compose the §4.1 provenance for a matcher-AUTO-applied link. Distinct from C2's
/// `matcher:{v} accepted-by:{kid}`: there is NO human, so it reads `matcher:{v} auto`.
/// Legible that the link was applied by the matcher alone (no human vouched).
pub fn compose_auto_provenance(matcher_version: &str) -> String {
    format!("matcher:{matcher_version} auto")
}

/// Assemble the un-attested `identity.link.asserted` EventBody the matcher will sign.
/// Pure: `event_id` is supplied by the caller (deterministic/testable). `low`/`high`
/// are the canonical pair (low < high); subject_a := low. The SOLE contributor is the
/// matcher with role `suggested` (ADR-0028 contributory, non-bearing) and NO
/// `responsibility` key — this is what keeps the event off the db/005 attestation gate.
pub fn build_suggested_link_body(
    event_id: Uuid,
    low: Uuid,
    high: Uuid,
    provenance: &str,
    confidence: Option<&str>,
    matcher_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let low_s = low.to_string();
    let high_s = high.to_string();
    let la = LinkAssertion { subject_a: &low_s, subject_b: &high_s, provenance, confidence };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: low_s.clone(), // C1 convention: an identity event is "about" subject_a
        event_type: "identity.link.asserted".into(),
        schema_version: LINK_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: matcher_kid.into(),
        // Authorship present (the matcher suggested the link), accountability ABSENT
        // (no `responsibility`) — principle 10 on the auto path. No responsibility ->
        // submit_event demands no attestation.
        contributors: serde_json::json!([
            {"actor_id": matcher_kid, "role": "suggested"}
        ]),
        payload: link_assertion_body(&la),
        attachments: vec![],
        plaintext_twin: Some(render_link_twin(&la)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid, Uuid) {
        let a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let b = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let eid = Uuid::parse_str("22222222-0000-0000-0000-000000000000").unwrap();
        (eid, a, b)
    }

    #[test]
    fn provenance_names_version_and_auto_not_human() {
        let p = compose_auto_provenance("0.3.0+abc");
        assert!(p.contains("0.3.0+abc"));
        assert!(p.contains("auto"));
        assert!(!p.contains("accepted-by"), "the auto path has no human voucher");
    }

    #[test]
    fn body_contributor_is_suggested_with_no_responsibility() {
        let (eid, a, b) = ids();
        let body = build_suggested_link_body(
            eid, a, b, "matcher:x auto", None, "mkid",
            Hlc { wall: 5, counter: 0, node_origin: "n".into() });
        let c = &body.contributors[0];
        assert_eq!(c["actor_id"], "mkid");
        assert_eq!(c["role"], "suggested");
        assert!(c.get("responsibility").is_none(),
            "the matcher bears NO responsibility -> no attestation required");
    }

    #[test]
    fn body_is_a_link_event_with_authored_twin_and_canonical_subjects() {
        let (eid, a, b) = ids();
        let body = build_suggested_link_body(
            eid, a, b, "matcher:x auto", Some("0.950"), "mkid",
            Hlc { wall: 5, counter: 0, node_origin: "n".into() });
        assert_eq!(body.event_type, "identity.link.asserted");
        assert_eq!(body.payload["subject_a"], a.to_string());
        assert_eq!(body.payload["subject_b"], b.to_string());
        assert_eq!(body.payload["confidence"], "0.950");
        assert!(body.plaintext_twin.as_deref().unwrap().starts_with("link: "),
            "authored twin required by the db/018 floor");
    }
}
```

- [ ] **Step 3: Run the pure tests**

Run: `cd crates/cairn-node && cargo test auto_apply::tests -- --nocapture`
Expected: 3 passing.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/src/auto_apply.rs crates/cairn-node/src/lib.rs
git commit -m "feat(identity): C2b pure un-attested link builder + auto provenance"
```

---

## Task 3: `resolve_matcher_actor` — load-or-generate key + idempotent enroll

**Files:**
- Modify: `crates/cairn-node/src/matcher_actor.rs` (add the IO fn + a DB-gated test at the bottom, or in tests file)
- Test: `crates/cairn-node/tests/auto_apply.rs` (create; holds all DB-gated tests for C2b)

**Interfaces:**
- Consumes: `cairn_event::{generate_key, SigningKey}`, `crate::keystore`, `crate::db`.
- Produces: `async fn resolve_matcher_actor(client: &tokio_postgres::Client, keystore_dir: &std::path::Path, secret: Option<&str>, matcher_version: &str) -> anyhow::Result<(SigningKey, String)>` — returns `(signing_key, kid_hex)`; ensures the per-epoch key file exists (sealed if `secret` is `Some`, else plaintext) and the `agent` actor is enrolled.

- [ ] **Step 1: Write the IO function**

Append to `crates/cairn-node/src/matcher_actor.rs` (above the `#[cfg(test)]` module), adding imports at the top:

```rust
use crate::keystore;
use cairn_event::{generate_key, SigningKey};
use std::path::Path;
use tokio_postgres::Client;
```

```rust
/// Resolve (load-or-create) the per-epoch matcher signing key AND ensure its `agent`
/// actor is enrolled. Idempotent and owner-privileged (the caller connects as a role
/// that may run `enroll_actor` — the runtime `cairn_agent` role deliberately cannot,
/// per the db/004 trust-anchor floor).
///
/// Key at rest: sealed under `secret` when present (we pass it as BOTH seal recipients,
/// so a single operational passphrase both seals and unseals — a matcher key needs no
/// separate paper recovery escrow because it is regenerable: losing it only retires the
/// epoch). When `secret` is None (throwaway/test nodes) the key is written plaintext 0600.
pub async fn resolve_matcher_actor(
    client: &Client,
    keystore_dir: &Path,
    secret: Option<&str>,
    matcher_version: &str,
) -> anyhow::Result<(SigningKey, String)> {
    std::fs::create_dir_all(keystore_dir)?;
    let path = keystore_dir.join(matcher_key_filename(matcher_version));

    // 1. Load the key if the epoch already has one; else mint + persist a fresh key.
    let (sk, kid) = if path.exists() {
        let sk = keystore::load(&path, secret)?;
        let kid = hex::encode(sk.verifying_key().to_bytes());
        (sk, kid)
    } else if let Some(s) = secret {
        // Seal under the op passphrase (both recipients = the same secret).
        keystore::generate_sealed(&path, s, s)?
    } else {
        keystore::generate_plaintext(&path)?
    };

    // 2. Ensure the actor is enrolled EXACTLY once (idempotent). actor_current holds
    //    only non-revoked current identities; enroll only when this key has none.
    let already: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_current WHERE signing_key_id = $1 AND kind = 'agent')",
            &[&kid],
        )
        .await?
        .get(0);
    if !already {
        let pinned = matcher_pinned(matcher_version);
        client
            .execute("SELECT enroll_actor('agent', $1::jsonb, $2)", &[&pinned, &kid])
            .await?;
    }

    Ok((sk, kid))
}
```

Note: `hex` is already a workspace dependency (used across `cairn-event`); confirm `hex` is in `cairn-node`'s `Cargo.toml` — if not, add `hex = "0.4"` (it is transitively present; a direct dep entry may be needed).

- [ ] **Step 2: Write the failing DB-gated test**

Create `crates/cairn-node/tests/auto_apply.rs`:

```rust
//! Integration coverage for the §5.2/§5.7 C2b auto-apply seam. Real Postgres, gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. No submit_event
//! change is exercised: C2b composes the db/018 identity floor + db/016 veto + db/004
//! actor registry, all reused unmodified.
use cairn_event::{generate_key, Hlc, SigningKey};
use cairn_node::auto_apply::{apply_auto_candidate, apply_auto_candidates, AutoOutcome};
use cairn_node::db;
use cairn_node::matcher_actor::resolve_matcher_actor;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Truncate the tables this seam touches. patient_link/person_member guarded by
/// to_regclass so this stays correct as db/018 grows.
async fn reset(c: &Client) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart, match_proposal CASCADE")
        .await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.patient_link')  IS NOT NULL THEN TRUNCATE patient_link;  END IF; \
           IF to_regclass('public.person_member') IS NOT NULL THEN TRUNCATE person_member; END IF; \
         END $$;").await.unwrap();
}

/// Seed one match_proposal for the canonical (low, high) pair with the given band/status.
async fn seed_proposal(c: &Client, low: Uuid, high: Uuid, band: &str, status: &str, version: &str) {
    let (low_s, high_s) = (low.to_string(), high.to_string());
    c.execute(
        "INSERT INTO match_proposal \
           (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version, status) \
         VALUES ($1::text::uuid,$2::text::uuid, 9.10, $3, '[]'::jsonb, '[]'::jsonb, $4, $5)",
        &[&low_s, &high_s, &band.to_string(), &version.to_string(), &status.to_string()],
    ).await.unwrap();
}

fn canonical(a: Uuid, b: Uuid) -> (Uuid, Uuid) { if a < b { (a, b) } else { (b, a) } }

#[tokio::test]
async fn resolve_enrolls_agent_once_and_reuses_it() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    let (_sk1, kid1) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa").await.unwrap();
    // First sight enrolled exactly one agent actor for this key.
    let n1: i64 = c.query_one(
        "SELECT count(*) FROM actor_event WHERE signing_key_id=$1 AND kind='agent' AND op='enroll'",
        &[&kid1]).await.unwrap().get(0);
    assert_eq!(n1, 1, "matcher agent enrolled on first sight");

    // Second call, same epoch -> same key, no duplicate enroll row.
    let (_sk2, kid2) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa").await.unwrap();
    assert_eq!(kid1, kid2, "same epoch reuses the same key");
    let n2: i64 = c.query_one(
        "SELECT count(*) FROM actor_event WHERE signing_key_id=$1 AND kind='agent' AND op='enroll'",
        &[&kid1]).await.unwrap().get(0);
    assert_eq!(n2, 1, "no duplicate enroll on reuse");
}

#[tokio::test]
async fn distinct_epochs_get_distinct_actors_and_keys() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    let (_a, kid_a) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa").await.unwrap();
    let (_b, kid_b) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+bbb").await.unwrap();
    assert_ne!(kid_a, kid_b, "a fresh key per epoch");
    // Two distinct agent actor_ids exist.
    let n: i64 = c.query_one(
        "SELECT count(DISTINCT actor_id) FROM actor_event WHERE kind='agent' AND op='enroll'", &[])
        .await.unwrap().get(0);
    assert_eq!(n, 2);
}
```

Add `tempfile` to `crates/cairn-node/Cargo.toml` `[dev-dependencies]` if absent (check first — it may already be there for other tests).

This test references `apply_auto_candidate`, `apply_auto_candidates`, `AutoOutcome` which don't exist yet — it will fail to COMPILE until Task 4/5. To keep this task independently green, **temporarily** comment the two `use cairn_node::auto_apply::...` driver imports and the tests that use them (they are added in Task 4/5). Simplest: in this task, include ONLY the two `resolve_*` tests and the imports they need; add the driver imports/tests in Task 4/5.

For this task, the top of the test file is:

```rust
use cairn_event::{generate_key, Hlc, SigningKey};
use cairn_node::db;
use cairn_node::matcher_actor::resolve_matcher_actor;
use tokio_postgres::Client;
use uuid::Uuid;
```

(Add `apply_auto_candidate`, `apply_auto_candidates`, `AutoOutcome`, and the `auto_apply` builder imports in Task 4/5. `generate_key`/`Hlc`/`SigningKey` are used by later tasks — allow the `unused_imports` warning for now, or add them in Task 4.)

- [ ] **Step 3: Run to verify it fails, then passes**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test auto_apply resolve_ -- --nocapture`
Expected first: compile error / FAIL (fn missing). After Step 1 is in place: 2 passing (skips silently if `CAIRN_TEST_PG` unset).

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/src/matcher_actor.rs crates/cairn-node/tests/auto_apply.rs crates/cairn-node/Cargo.toml
git commit -m "feat(identity): C2b resolve_matcher_actor (per-epoch key + idempotent agent enroll)"
```

---

## Task 4: `apply_auto_candidate` — one proposal, one transaction

**Files:**
- Modify: `crates/cairn-node/src/auto_apply.rs` (add `AutoOutcome`, `next_hlc`, `apply_auto_candidate`)
- Test: `crates/cairn-node/tests/auto_apply.rs` (add happy-path, veto→review, skip tests)

**Interfaces:**
- Consumes: `resolve_matcher_actor` (Task 3), `build_suggested_link_body` + `compose_auto_provenance` (Task 2), `cairn_event::{event_address, sign}`, `tokio_postgres::Client`.
- Produces:
  - `pub enum AutoOutcome { Applied(Uuid), VetoedToReview, Skipped(String) }`
  - `pub async fn apply_auto_candidate(client: &mut Client, low: Uuid, high: Uuid, matcher_sk: &SigningKey, matcher_kid: &str, hlc: Hlc) -> anyhow::Result<AutoOutcome>`

- [ ] **Step 1: Write the failing DB tests**

Add to `crates/cairn-node/tests/auto_apply.rs` (and add the imports `use cairn_node::auto_apply::{apply_auto_candidate, AutoOutcome};`):

```rust
/// Assert a hard veto between the pair by writing clashing VERIFIED sex-at-birth
/// demographic assertions (db/016 cairn_match_veto reads the projection). Helper mirrors
/// the veto tests' setup: two provenance='fact-proven' sex-at-birth values that differ.
async fn assert_verified_sex_clash(c: &Client, a: Uuid, b: Uuid) {
    // Uses the demographic projection tables db/013 populates. If the exact seeding API
    // differs, reuse the helper from tests that already exercise cairn_match_veto (grep
    // `cairn_match_veto` under crates/cairn-node/tests). The REQUIREMENT: after this call,
    // `SELECT EXISTS(SELECT 1 FROM cairn_match_veto(a,b))` is TRUE.
    // (See db/016 tests for the canonical seeding; do NOT invent a new path.)
    let _ = (c, a, b);
    unimplemented!("wire to the existing cairn_match_veto seeding helper");
}

#[tokio::test]
async fn auto_candidate_becomes_unattested_link_and_projects_person() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa").await.unwrap();
    let out = apply_auto_candidate(&mut c, low, high, &sk, &kid,
        Hlc { wall: 100, counter: 0, node_origin: "testnode".into() }).await.unwrap();
    assert!(matches!(out, AutoOutcome::Applied(_)));

    // Exactly one link event, appended with NO attestation (attestation column NULL).
    let n_ev: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted' AND attestation IS NULL",
        &[]).await.unwrap().get(0);
    assert_eq!(n_ev, 1, "one un-attested link event appended");

    // Standing edge + person projection (both patients -> min-UUID person).
    let (low_s, high_s) = (low.to_string(), high.to_string());
    let n_edge: i64 = c.query_one(
        "SELECT count(*) FROM patient_link WHERE low=$1::text::uuid AND high=$2::text::uuid AND state='link'",
        &[&low_s, &high_s]).await.unwrap().get(0);
    assert_eq!(n_edge, 1);
    let person: Uuid = c.query_one(
        "SELECT person_id FROM person_member WHERE patient_id=$1::text::uuid", &[&low_s])
        .await.unwrap().get(0);
    assert_eq!(person, low.min(high), "person = min-UUID of the component");

    // Proposal marked auto_applied with an applied_event_id.
    let (status, applied): (String, Option<Uuid>) = {
        let r = c.query_one(
            "SELECT status, applied_event_id FROM match_proposal WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low_s, &high_s]).await.unwrap();
        (r.get(0), r.get(1))
    };
    assert_eq!(status, "auto_applied");
    assert!(applied.is_some());
}

#[tokio::test]
async fn veto_appeared_since_propose_kicks_to_review_no_event() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;
    assert_verified_sex_clash(&c, low, high).await; // a hard veto now exists

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa").await.unwrap();
    let out = apply_auto_candidate(&mut c, low, high, &sk, &kid,
        Hlc { wall: 100, counter: 0, node_origin: "testnode".into() }).await.unwrap();
    assert!(matches!(out, AutoOutcome::VetoedToReview));

    let n_ev: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'", &[])
        .await.unwrap().get(0);
    assert_eq!(n_ev, 0, "no link event when a veto appeared");
    let status: String = c.query_one(
        "SELECT status FROM match_proposal WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
        &[&low.to_string(), &high.to_string()]).await.unwrap().get(0);
    assert_eq!(status, "review", "since-vetoed proposal kicked to a human");
}

#[tokio::test]
async fn non_pending_or_non_auto_candidate_is_skipped() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    // A human already REJECTED this auto_candidate -> must NOT be auto-applied.
    seed_proposal(&c, low, high, "auto_candidate", "rejected", "0.3.0+aaa").await;

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa").await.unwrap();
    let out = apply_auto_candidate(&mut c, low, high, &sk, &kid,
        Hlc { wall: 100, counter: 0, node_origin: "testnode".into() }).await.unwrap();
    assert!(matches!(out, AutoOutcome::Skipped(_)));
    let n_ev: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'", &[])
        .await.unwrap().get(0);
    assert_eq!(n_ev, 0);
    let status: String = c.query_one(
        "SELECT status FROM match_proposal WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
        &[&low.to_string(), &high.to_string()]).await.unwrap().get(0);
    assert_eq!(status, "rejected", "a human's disposition is untouched");
}
```

**Before writing the veto helper**, grep the existing tests for the canonical `cairn_match_veto` seeding and reuse it verbatim:
Run: `grep -rn "cairn_match_veto\|sex-at-birth\|fact-proven" crates/cairn-node/tests/*.rs db/tests/016*.sql`
Wire `assert_verified_sex_clash` to that path (do not invent a new demographic-seeding API).

- [ ] **Step 2: Run to verify the tests fail (compile error: `apply_auto_candidate` missing)**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="…" cargo test --test auto_apply auto_candidate -- --nocapture`
Expected: FAIL (unresolved import / fn not found).

- [ ] **Step 3: Implement `apply_auto_candidate`**

Add to `crates/cairn-node/src/auto_apply.rs` (add imports `use cairn_event::{event_address, sign, SigningKey};` and `use tokio_postgres::Client;`):

```rust
/// The result of attempting to auto-apply one proposal.
pub enum AutoOutcome {
    /// A link event was appended; carries its event_id.
    Applied(Uuid),
    /// A veto appeared since propose; the proposal was kicked to human `review`.
    VetoedToReview,
    /// Not eligible (not auto_candidate, not pending, or absent); nothing changed.
    Skipped(String),
}

/// Apply ONE proposal: read it `FOR UPDATE`, require band='auto_candidate' AND
/// status='pending', RE-CHECK the db/016 veto (any severity) — a veto that appeared since
/// propose kicks the pair to human `review` instead of auto-linking — else build + sign an
/// un-attested link with the matcher's key, submit through the 1-arg submit_event door, and
/// mark the proposal 'auto_applied'. All in ONE transaction: any rejection rolls back, so
/// no event is written and the proposal stays 'pending' to retry (atomicity = idempotency).
///
/// The pair may be passed in either order; it is canonicalized to (least, greatest) to
/// match match_proposal's `CHECK (patient_low < patient_high)`.
pub async fn apply_auto_candidate(
    client: &mut Client,
    low: Uuid,
    high: Uuid,
    matcher_sk: &SigningKey,
    matcher_kid: &str,
    hlc: Hlc,
) -> anyhow::Result<AutoOutcome> {
    let (low, high) = if low <= high { (low, high) } else { (high, low) };
    let (low_s, high_s) = (low.to_string(), high.to_string());
    let tx = client.transaction().await?;

    // 1. Lock the row; require it is an auto_candidate still awaiting disposition.
    let row = tx.query_opt(
        "SELECT band, status, score_total, matcher_version FROM match_proposal \
         WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid FOR UPDATE",
        &[&low_s, &high_s]).await?;
    let Some(row) = row else {
        return Ok(AutoOutcome::Skipped(format!("no proposal for ({low}, {high})")));
    };
    let band: String = row.get(0);
    let status: String = row.get(1);
    let score: f64 = row.get(2);
    let matcher_version: String = row.get(3);
    if band != "auto_candidate" || status != "pending" {
        return Ok(AutoOutcome::Skipped(format!("band='{band}' status='{status}' — not an actionable auto_candidate")));
    }

    // 2. Re-check the veto floor (no human backstop on this path). ANY veto (hard_veto or
    //    degrade_hold) forbids an auto-link — mirrors banding.py. A since-vetoed pair is
    //    kicked to a human, never auto-linked over.
    let vetoed: bool = tx.query_one(
        "SELECT EXISTS(SELECT 1 FROM cairn_match_veto($1::text::uuid, $2::text::uuid))",
        &[&low_s, &high_s]).await?.get(0);
    if vetoed {
        tx.execute(
            "UPDATE match_proposal SET status='review', updated_at=clock_timestamp() \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low_s, &high_s]).await?;
        tx.commit().await?;
        return Ok(AutoOutcome::VetoedToReview);
    }

    // 3. Build + sign the un-attested matcher link.
    let provenance = compose_auto_provenance(&matcher_version);
    let confidence = format!("{score:.3}");
    let event_id = Uuid::now_v7();
    let body = build_suggested_link_body(event_id, low, high, &provenance, Some(&confidence), matcher_kid, hlc);
    let signed = sign(&body, matcher_sk)?;
    let _ca = event_address(&signed.signed_bytes); // (bind order parity with C2; not needed un-attested)

    // 4. Submit through the 1-arg (un-attested) door. db/018 identity floor +
    //    patient_link_apply trigger run here.
    tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;

    // 5. Mark the proposal auto_applied (distinct from C2's human 'applied').
    let event_id_s = event_id.to_string();
    tx.execute(
        "UPDATE match_proposal SET status='auto_applied', applied_event_id=$3::text::uuid, updated_at=clock_timestamp() \
         WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
        &[&low_s, &high_s, &event_id_s]).await?;

    tx.commit().await?;
    Ok(AutoOutcome::Applied(event_id))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="…" cargo test --test auto_apply auto_candidate veto_appeared non_pending -- --nocapture`
Expected: 3 passing.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/auto_apply.rs crates/cairn-node/tests/auto_apply.rs
git commit -m "feat(identity): C2b apply_auto_candidate (veto re-check + un-attested link + auto_applied)"
```

---

## Task 5: `apply_auto_candidates` batch driver + idempotency + recall precision

**Files:**
- Modify: `crates/cairn-node/src/auto_apply.rs` (add `AutoSummary`, `next_hlc`, `apply_auto_candidates`)
- Test: `crates/cairn-node/tests/auto_apply.rs` (batch, idempotency, recall-precision tests)

**Interfaces:**
- Consumes: `apply_auto_candidate` (Task 4), `resolve_matcher_actor` (Task 3), `crate::db` HLC door `node_hlc_tick()`.
- Produces:
  - `pub struct AutoSummary { pub applied: usize, pub vetoed_to_review: usize, pub skipped: usize }`
  - `pub async fn apply_auto_candidates(client: &mut Client, keystore_dir: &std::path::Path, secret: Option<&str>, node_origin: &str) -> anyhow::Result<AutoSummary>`

- [ ] **Step 1: Write the failing DB tests**

Add to `crates/cairn-node/tests/auto_apply.rs` (imports: `use cairn_node::auto_apply::{apply_auto_candidates, AutoSummary};`):

```rust
#[tokio::test]
async fn batch_applies_all_pending_auto_candidates_across_epochs_and_is_idempotent() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    // Two pairs, two different matcher epochs.
    let (l1, h1) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (l2, h2) = canonical(Uuid::now_v7(), Uuid::now_v7());
    seed_proposal(&c, l1, h1, "auto_candidate", "pending", "0.3.0+aaa").await;
    seed_proposal(&c, l2, h2, "auto_candidate", "pending", "0.3.0+bbb").await;
    // A review-band pair must be ignored by the driver.
    let (l3, h3) = canonical(Uuid::now_v7(), Uuid::now_v7());
    seed_proposal(&c, l3, h3, "review", "pending", "0.3.0+aaa").await;

    let s: AutoSummary = apply_auto_candidates(&mut c, dir.path(), None, "testnode").await.unwrap();
    assert_eq!(s.applied, 2);
    let n_ev: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'", &[])
        .await.unwrap().get(0);
    assert_eq!(n_ev, 2, "both auto_candidate pairs linked; the review pair ignored");

    // Idempotent: a second run applies nothing new.
    let s2 = apply_auto_candidates(&mut c, dir.path(), None, "testnode").await.unwrap();
    assert_eq!(s2.applied, 0);
    let n_ev2: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'", &[])
        .await.unwrap().get(0);
    assert_eq!(n_ev2, 2, "no new events on re-run");
}

#[tokio::test]
async fn recall_over_the_matcher_epoch_selects_its_autolinks_precisely() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    let (l1, h1) = canonical(Uuid::now_v7(), Uuid::now_v7());
    seed_proposal(&c, l1, h1, "auto_candidate", "pending", "0.3.0+aaa").await;
    apply_auto_candidates(&mut c, dir.path(), None, "testnode").await.unwrap();

    // The matcher epoch's actor_id (content-address of its pinned set).
    let actor_a: Vec<u8> = c.query_one(
        "SELECT actor_id FROM actor_event WHERE op='enroll' AND kind='agent' \
         AND pinned->>'matcher_version'='0.3.0+aaa'", &[]).await.unwrap().get(0);
    let epoch_a: i64 = c.query_one(
        "SELECT skill_epoch FROM actor_current_epoch_of($1)", &[&actor_a]).await
        .map(|r| r.get(0)).unwrap_or(0); // if no such helper, see NOTE below

    // A recall over THIS epoch selects the auto-link event; a bogus/other epoch does not.
    let hit: i64 = c.query_one(
        "SELECT count(*) FROM events_by_actor_epoch($1, $2)", &[&actor_a, &epoch_a])
        .await.unwrap().get(0);
    assert!(hit >= 1, "contamination-cascade recall selects the epoch's auto-link");
}
```

**NOTE (recall test wiring):** the exact `events_by_actor_epoch` signature is in `db/006_recall.sql`. Before writing this test, run
`grep -n "events_by_actor_epoch\|skill_epoch\|CREATE.*FUNCTION" db/006_recall.sql`
and adapt the call to the real parameters (it takes a `(key, epoch)` or `(actor_id, epoch)` — use whichever db/006 defines, and derive the epoch the same way db/006's own SQL tests do). Assert only the load-bearing property: **a recall keyed on this matcher epoch returns ≥1 row (the auto-link), and a recall keyed on a different epoch returns 0.** Do not invent a helper that db/006 doesn't export.

- [ ] **Step 2: Run to verify failure**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="…" cargo test --test auto_apply batch_applies recall_over -- --nocapture`
Expected: FAIL (fn/type missing).

- [ ] **Step 3: Implement the batch driver + HLC helper**

Add to `crates/cairn-node/src/auto_apply.rs`:

```rust
use std::collections::HashMap;
use std::path::Path;
use crate::matcher_actor::resolve_matcher_actor;

/// Batch outcome counts for the operator's summary line.
pub struct AutoSummary { pub applied: usize, pub vetoed_to_review: usize, pub skipped: usize }

/// Tick the node HLC once (the same door node authoring uses) and stamp `node_origin`.
/// Authoring is single-threaded on a node, so tick->sign->submit per event is safe here.
async fn next_hlc(tx_client: &Client, node_origin: &str) -> anyhow::Result<Hlc> {
    let row = tx_client.query_one("SELECT wall, counter FROM node_hlc_tick()", &[]).await?;
    Ok(Hlc { wall: row.get(0), counter: row.get(1), node_origin: node_origin.into() })
}

/// Auto-apply EVERY pending auto_candidate proposal. Resolves the matcher actor once per
/// distinct epoch (cached), then applies each pair in its own transaction so one bad pair
/// never rolls back the batch (skip-and-report, mirroring pipeline/sweep.py). Owner-run
/// (resolve_matcher_actor enrolls actors, which the runtime role may not).
pub async fn apply_auto_candidates(
    client: &mut Client,
    keystore_dir: &Path,
    secret: Option<&str>,
    node_origin: &str,
) -> anyhow::Result<AutoSummary> {
    // Snapshot the worklist first (a read), then act — so we never hold a cursor across
    // the per-pair transactions.
    let rows = client.query(
        "SELECT patient_low::text, patient_high::text, matcher_version \
         FROM match_proposal WHERE band='auto_candidate' AND status='pending' \
         ORDER BY patient_low, patient_high", &[]).await?;

    let mut keys: HashMap<String, (SigningKey, String)> = HashMap::new();
    let mut summary = AutoSummary { applied: 0, vetoed_to_review: 0, skipped: 0 };

    for r in rows {
        let low: Uuid = r.get::<_, String>(0).parse()?;
        let high: Uuid = r.get::<_, String>(1).parse()?;
        let version: String = r.get(2);

        // Resolve (and cache) the matcher key/actor for this epoch.
        if !keys.contains_key(&version) {
            let (sk, kid) = resolve_matcher_actor(client, keystore_dir, secret, &version).await?;
            keys.insert(version.clone(), (sk, kid));
        }
        let (sk, kid) = keys.get(&version).unwrap();
        let kid = kid.clone();

        // One HLC per event.
        let hlc = next_hlc(client, node_origin).await?;
        // clone the key out of the cache map to satisfy the &mut borrow of `client`.
        let sk = sk.clone();
        match apply_auto_candidate(client, low, high, &sk, &kid, hlc).await {
            Ok(AutoOutcome::Applied(_)) => summary.applied += 1,
            Ok(AutoOutcome::VetoedToReview) => summary.vetoed_to_review += 1,
            Ok(AutoOutcome::Skipped(_)) => summary.skipped += 1,
            Err(e) => { eprintln!("auto-apply ({low},{high}): {e}"); summary.skipped += 1; }
        }
    }
    Ok(summary)
}
```

Note the borrow dance: `resolve_matcher_actor` and `next_hlc` take `&Client`, `apply_auto_candidate` takes `&mut Client`. Because `client` is `&mut`, the `&Client` calls reborrow fine as long as no `&Client` borrow is held across the `&mut` call — the code above resolves/ticks, then clones the key, then calls the `&mut` fn, so no overlap. If the borrow checker complains, split: collect the resolved `(version -> (sk,kid))` map in a first pass (all `&Client`), then a second pass of `&mut` applies.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="…" cargo test --test auto_apply batch_applies recall_over -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Full-suite + clippy gate**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="…" cargo test && cargo clippy --tests -- -D warnings`
Expected: entire cairn-node suite green, clippy clean.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/auto_apply.rs crates/cairn-node/tests/auto_apply.rs
git commit -m "feat(identity): C2b batch driver apply_auto_candidates + idempotency & recall-precision tests"
```

---

## Task 6: CLI `apply-auto-candidates` subcommand

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add the `Cmd::ApplyAutoCandidates` variant + arm)

**Interfaces:**
- Consumes: `cairn_node::auto_apply::apply_auto_candidates`, `cairn_node::db::connect`, the existing `--conn`/`--key`/`CAIRN_KEY_PASSPHRASE` plumbing.

- [ ] **Step 1: Add the subcommand variant**

In `crates/cairn-node/src/main.rs`, in the `Commands`/`Cmd` enum, add near `Quarantine`:

```rust
    /// Auto-apply every pending `auto_candidate` match proposal as a matcher-authored,
    /// un-attested, recallable identity link. OWNER ceremony: point `--conn` at a role
    /// that may run enroll_actor (the per-epoch matcher actor is enrolled on first sight),
    /// NOT the unprivileged runtime role. Re-checks the db/016 veto per pair.
    ApplyAutoCandidates,
```

- [ ] **Step 2: Add the match arm**

In the `match cli.command` block, add:

```rust
        Cmd::ApplyAutoCandidates => {
            // Owner connection (needs enroll_actor). Fail fast with a legible hint if the
            // DB predates db/018 (the identity floor is required).
            let db = cairn_node::db::connect(&cli.conn).await?;
            if cairn_node::db::to_regclass(&db, "public.patient_link").await?.is_none() {
                anyhow::bail!("this database predates db/018 (no patient_link) — run `cairn-node init` to load the identity floor");
            }
            // The matcher keystore lives beside the node key.
            let keystore_dir = cli.key.parent().unwrap_or(std::path::Path::new(".")).join("matcher-keys");
            // Seal matcher keys under the same operational passphrase when the node key is
            // sealed; a plaintext node key -> plaintext matcher keys.
            let secret = std::env::var("CAIRN_KEY_PASSPHRASE").ok().filter(|s| !s.is_empty());
            let node_origin = cairn_node::identity::load(&db).await?.node_id_hex;
            let mut db = db;
            let s = cairn_node::auto_apply::apply_auto_candidates(
                &mut db, &keystore_dir, secret.as_deref(), &node_origin).await?;
            println!("auto-apply: applied {}  vetoed->review {}  skipped {}",
                s.applied, s.vetoed_to_review, s.skipped);
        }
```

**Adapt to reality:** confirm the exact names before writing — `grep -n "pub async fn load\|node_id_hex\|pub fn to_regclass\|to_regclass" crates/cairn-node/src/identity.rs crates/cairn-node/src/db.rs`. If `db::to_regclass` doesn't exist, inline the check: `db.query_one("SELECT to_regclass('public.patient_link') IS NOT NULL", &[]).await?.get::<_,bool>(0)`. If `identity::load` returns a different field for the node id hex, use that. Do not invent helpers.

- [ ] **Step 3: Build + clippy**

Run: `cd crates/cairn-node && cargo build && cargo clippy -- -D warnings`
Expected: builds clean; the new subcommand appears in `cargo run -- --help`.

- [ ] **Step 4: Manual smoke (optional, if a rig DB is up)**

Run: `cargo run -- --conn "$CAIRN_TEST_PG" --key /tmp/c2b-smoke.key apply-auto-candidates`
Expected: prints an `auto-apply: applied N …` line (0/0/0 on an empty worklist), no panic.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(identity): C2b CLI apply-auto-candidates (owner ceremony)"
```

---

## Task 7: Workspace gate + docs currency

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Full workspace test + clippy**

Run: `cd /Users/hherb/src/cairn-ehr && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace && cargo clippy --workspace --tests -- -D warnings`
Expected: all suites green, clippy clean.

- [ ] **Step 2: Update HANDOVER + ROADMAP**

Add a concise C2b "done this session" note to `docs/HANDOVER.md` top block and mark C2b done in the identity-slices line + the ROADMAP identity slice. Note the deferred items (no scheduler; key sealing without recovery escrow; ADR-0028 role enum still not DB-enforced #96) and that #111 (last session) is merged. Keep both files < 500 lines (prune older detail per the house rule).

- [ ] **Step 3: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: C2b done — HANDOVER/ROADMAP currency"
```

---

## Self-Review

**Spec coverage:**
- §2.1 per-epoch actor/fresh key/auto-enroll → Task 1 (pinned) + Task 3 (`resolve_matcher_actor`).
- §2.2 apply-time veto re-check → Task 4 step 3 (`cairn_match_veto` EXISTS → `review`).
- §2.3 `suggested`/no-responsibility → Task 2 (`build_suggested_link_body`) + test.
- §2.4 status transitions & idempotency → Task 4 (`auto_applied`) + Task 5 (idempotent re-run).
- §2.5 key at rest sealed, no recovery escrow → Task 3 (`generate_sealed(path,s,s)` / `generate_plaintext`).
- §3.3 CLI → Task 6.
- §5 tests 1–10 → distributed across Tasks 1–5.
- §6 honest ceilings → recorded in the design doc + HANDOVER note (Task 7).
- §7 no ADR → no `db/`/spec/ADR file touched in any task.

**Placeholder scan:** two spots deliberately defer to *existing* code and MUST be wired to it, not invented: the `cairn_match_veto` seeding helper (Task 4) and the `events_by_actor_epoch` recall call (Task 5). Both include the exact grep to find the canonical path. This is reuse-not-invent, not a placeholder.

**Type consistency:** `AutoOutcome`/`AutoSummary`, `apply_auto_candidate`/`apply_auto_candidates`, `resolve_matcher_actor`, `matcher_pinned`/`matcher_key_filename`, `build_suggested_link_body`/`compose_auto_provenance` are used with identical names/signatures across tasks and tests. kid is always `hex::encode(sk.verifying_key().to_bytes())`. UUIDs always `$N::text::uuid`.
