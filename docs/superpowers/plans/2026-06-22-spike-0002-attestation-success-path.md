# Spike 0002 Attestation Success-Path — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drive `submit_event`'s attestation **accept** branch — and the valid-token-but-bad-binding rejections — end-to-end, closing the one honest gap carried into ADR-0030.

**Architecture:** Add a `cairn-sync attest-stdin` CLI helper (a dumb signer mirroring `sign-stdin`, so the Python stand-in mints tokens without a second crypto impl). Then add a durable Rust integration test (real Postgres, `submit_event` accept + N-rejections) and extend the Python harness for the full external-actor end-to-end. No production logic in `submit_event`/`cairn_pgx`/`cairn-event` changes — the accept branch already exists; this is the coverage that was missing.

**Tech Stack:** Rust (`cairn-sync` bin, `cairn-node` integration tests, `cairn-event` lib), PL/pgSQL (`db/005_submit.sql`, unchanged), Python 3 + `psycopg` (`poc/walking-skeleton/harness`), Postgres ≥ 16 with the `cairn_pgx` extension installed.

## Global Constraints

- **License:** all code AGPL-3.0; any new dependency must be AGPL-3.0-compatible. (No new dependencies are needed by this plan.)
- **TDD:** the new CLI code (Task 1) is written test-first. Tasks 2–3 are coverage of pre-existing in-DB logic — write the test, run it; it is **expected to pass**, and a failure is a real defect to report, not a code change to make.
- **Inline docs for a junior dev:** every new function carries a doc-comment explaining flow + purpose.
- **Pure, reusable functions over cleverness:** the CLI's token encoding is factored into a pure, unit-testable helper.
- **No spec/ADR change.** ADR-0030 is immutable; the closure is recorded in the spike doc + HANDOVER (Task 4).
- **Dumb-signer rule:** `attest-stdin`, like `sign-stdin`, must attest whatever content-address it is given. The in-DB floor (`cairn_attestation_ok`) is the only binding gate. Never add address validation to the CLI.
- **DB-gated tests** skip cleanly when `CAIRN_TEST_PG` is unset and require a local Postgres with `cairn_pgx` installed (`cargo pgrx install` against PG16; see `crates/cairn_pgx`/spike toolchain notes).

## File Structure

- `crates/cairn-sync/src/main.rs` — **modify**: add `attestation_token_hex()` (pure helper), `cmd_attest_stdin()`, a dispatch arm, a usage line, two imports, and a `#[cfg(test)] mod tests`.
- `crates/cairn-node/tests/attestation.rs` — **create**: the durable real-Postgres integration test (P1, P2, N1–N3).
- `poc/walking-skeleton/harness/agent_standin.py` — **modify**: add an `attest()` helper.
- `poc/walking-skeleton/harness/spike_0002.py` — **modify**: add `_content_address_hex()` + the success/rejection block to `selftest`.
- `docs/spikes/0002-advisory-actor-write-contract.md`, `docs/HANDOVER.md` — **modify**: record the gap as closed.

---

### Task 1: `attest-stdin` CLI helper (test-first)

**Files:**
- Modify: `crates/cairn-sync/src/main.rs` (import line `:25`; new fns near `cmd_sign_stdin` `:233`; usage line `:1170`; dispatch arm `:1264`; new test module at end of file)

**Interfaces:**
- Consumes: `cairn_event::{sign_attestation, AttestationBody, SigningKey}`, `load_or_create_key()` (existing, `main.rs:140`).
- Produces: `fn attestation_token_hex(input: &str, sk: &SigningKey) -> R<String>` (pure: JSON `AttestationBody` → hex COSE_Sign1); `fn cmd_attest_stdin(key_path: &str) -> R<()>`; CLI subcommand `attest-stdin --key PATH`.

- [ ] **Step 1: Add the two imports**

In `crates/cairn-sync/src/main.rs:25`, extend the `cairn_event` use line to add `sign_attestation` and `AttestationBody`:

```rust
use cairn_event::{blob_address, plaintext_twin, sign, sign_attestation, verify_self_described, AttestationBody, EventBody, Hlc, SigningKey};
```

- [ ] **Step 2: Write the failing unit test**

Append to the end of `crates/cairn-sync/src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::{event_address, generate_key, verify_attestation};

    #[test]
    fn attest_token_hex_is_verifiable_and_address_bound() {
        // The CLI core must produce a token the verifier accepts for the right
        // key+address and rejects for a different address (the binding guarantee).
        let (sk, kid) = generate_key().unwrap();
        let vk = sk.verifying_key();
        let ca = event_address(b"some signed event bytes");
        let input = format!(
            r#"{{"content_address_hex":"{}","attester_key_id":"{}","role":"attested"}}"#,
            hex::encode(&ca), kid
        );

        let token_hex = attestation_token_hex(&input, &sk).unwrap();
        let token = hex::decode(&token_hex).unwrap();

        assert!(verify_attestation(&token, &ca, &vk), "token verifies for right key + address");
        let other = event_address(b"a different event");
        assert!(!verify_attestation(&token, &other, &vk), "token is bound to its content-address");
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p cairn-sync attest_token_hex_is_verifiable`
Expected: FAIL to **compile** — `cannot find function attestation_token_hex in this scope`.

- [ ] **Step 4: Write the pure helper + the command**

In `crates/cairn-sync/src/main.rs`, immediately after `cmd_sign_stdin` (ends at `:246`), add:

```rust
/// Build a hex COSE_Sign1 attestation token from a JSON `AttestationBody` string,
/// signed by `sk`. Pure (no I/O) so it is unit-testable; `cmd_attest_stdin` wraps it
/// with key-load + stdin-read + stdout-print. Mirrors the sign-stdin split so Rust
/// owns the one canonical attestation encoding (no second crypto impl in Python).
fn attestation_token_hex(input: &str, sk: &SigningKey) -> R<String> {
    let body: AttestationBody = serde_json::from_str(input)?;
    let content_address = hex::decode(&body.content_address_hex)?;
    let token = sign_attestation(&content_address, &body.attester_key_id, &body.role, sk)?;
    Ok(hex::encode(&token))
}

/// Sign an `AttestationBody` supplied as JSON on stdin and emit a hex COSE_Sign1
/// attestation token on stdout. Like `sign-stdin`, this is a DUMB signer: it attests
/// whatever `content_address_hex` it is handed, including one bound to no real event.
/// That is deliberate — it is how the wrong-address adversarial test is constructed —
/// and the in-DB floor (`cairn_attestation_ok`) is what rejects a mis-bound token,
/// never this CLI. Do NOT "harden" it to validate the address: that would break the
/// adversarial tests and move a floor check out of the database (ADR-0021/0030).
fn cmd_attest_stdin(key_path: &str) -> R<()> {
    let (sk, _kid) = load_or_create_key(key_path)?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let token_hex = attestation_token_hex(&input, &sk)?;
    println!("{token_hex}");
    Ok(())
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p cairn-sync attest_token_hex_is_verifiable`
Expected: PASS (`test result: ok. 1 passed`).

- [ ] **Step 6: Wire the subcommand into the CLI**

In `crates/cairn-sync/src/main.rs`, add a usage line after the `sign-stdin` usage line (`:1170`):

```
  attest-stdin --key PATH    (read JSON AttestationBody on stdin, write hex COSE_Sign1 token on stdout)
```

And add a dispatch arm after the `sign-stdin` arm (`:1264-1266`):

```rust
        "attest-stdin" => cmd_attest_stdin(
            &flag(&args, "--key").unwrap_or_else(|| "human.key".into()),
        )?,
```

- [ ] **Step 7: Verify the whole crate builds clean and the CLI works**

Run:
```bash
cargo build -p cairn-sync && cargo clippy -p cairn-sync -- -D warnings
echo '{"content_address_hex":"1220'$(printf '11%.0s' {1..32})'","attester_key_id":"deadbeef","role":"attested"}' \
  | ./target/debug/cairn-sync attest-stdin --key /tmp/plan-human.key
```
Expected: build + clippy clean; the command prints a hex string (the COSE token) and a one-line "generated new signing key" notice on stderr.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-sync/src/main.rs
git commit -m "$(cat <<'EOF'
feat(cairn-sync): add attest-stdin CLI helper (mirror of sign-stdin)

A dumb signer that turns a JSON AttestationBody on stdin into a hex COSE_Sign1
attestation token, so the Python agent stand-in can mint tokens while Rust owns
the canonical encoding. Pure helper attestation_token_hex() is unit-tested.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Durable Rust integration test — `submit_event` accept + rejections

**Files:**
- Create: `crates/cairn-node/tests/attestation.rs`

**Interfaces:**
- Consumes: `cairn_node::db::{connect_and_load_schema, test_serial_guard}`; `cairn_event::{sign, sign_attestation, event_address, generate_key, EventBody, Hlc, SigningKey}`; SQL `enroll_actor(text, jsonb, text)`, `submit_event(bytea, bytea, bytea)`.
- Produces: four `#[tokio::test]`s exercising P1, P2, and N1–N3. (Nothing consumes this; it is leaf coverage.)

> **Why these tests may pass on the first run:** the accept branch (`db/005_submit.sql:79-92`) already exists. These tests are the coverage that was missing, not new behaviour. If any fails, that is a genuine defect — report it, do not "fix" it by weakening the assertion.

- [ ] **Step 1: Write the test file**

Create `crates/cairn-node/tests/attestation.rs`:

```rust
//! Integration coverage for submit_event's attestation ACCEPT branch and the
//! valid-token-but-bad-binding rejections (the half Spike 0002 never exercised;
//! the honest gap carried into ADR-0030). Real Postgres, gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern,
//! identical to admission.rs). Tokens are minted directly via cairn_event here; the
//! CLI path is covered separately by the Python harness (Task 3).
use cairn_event::{event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

const SUBMIT3: &str = "SELECT submit_event($1,$2,$3)";

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Truncate the advisory-write tables and enroll one human attester + one agent
/// signer (distinct keys). Returns (agent sk, agent kid, human sk, human kid).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await.unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    let agent_pinned =
        String::from(r#"{"model":"triage-stub","version":"1","skill_epoch":"epoch-a"}"#);
    let human_pinned = String::from(r#"{"role":"clinician"}"#);
    c.execute("SELECT enroll_actor('agent', $1::jsonb, $2)", &[&agent_pinned, &kid_a])
        .await.unwrap();
    c.execute("SELECT enroll_actor('human', $1::jsonb, $2)", &[&human_pinned, &kid_h])
        .await.unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Build an agent-authored EventBody. `with_responsibility` adds a contributor
/// carrying a `responsibility` key (the v_bears attestation trigger on an additive
/// event). `target` (if Some) is written as payload.target_event_id (suppress target).
fn body(
    event_type: &str, patient: Uuid, kid_a: &str,
    with_responsibility: bool, target: Option<&str>,
) -> EventBody {
    let contrib = if with_responsibility {
        serde_json::json!([{"actor_id": kid_a, "role": "attested", "responsibility": "attested"}])
    } else {
        serde_json::json!([{"actor_id": kid_a, "role": "triaged"}])
    };
    let payload = match target {
        Some(t) => serde_json::json!({ "target_event_id": t }),
        None => serde_json::json!({ "text": "seen, stable" }),
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: "advisory/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "agent".into() },
        t_effective: None,
        signer_key_id: kid_a.into(),
        contributors: contrib,
        payload,
        attachments: vec![],
    }
}

#[tokio::test]
async fn accepts_responsibility_bearing_additive_event_with_valid_human_token() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // P1: a note.added carrying `responsibility` triggers the attestation gate on an
    // additive event (no target/provenance machinery) — isolates the accept.
    let b = body("note.added", patient, &kid_a, true, None);
    let signed = sign(&b, &sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();

    let r = c.execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk_h]).await;
    assert!(r.is_ok(), "valid human attestation must be accepted: {r:?}");
    let n: i64 = c
        .query_one("SELECT count(*) FROM event_log WHERE event_type='note.added'", &[])
        .await.unwrap().get(0);
    assert_eq!(n, 1, "the attested event is appended");
}

#[tokio::test]
async fn accepts_suppressing_event_with_valid_human_token() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Baseline additive note (no token) to be the suppress target — step-5 needs it.
    let baseline = body("note.added", patient, &kid_a, false, None);
    let baseline_signed = sign(&baseline, &sk_a).unwrap();
    c.execute("SELECT submit_event($1)", &[&baseline_signed.signed_bytes]).await.unwrap();

    // P2: salience.downgrade (suppressing) targeting the baseline, human-attested.
    let supp = body("salience.downgrade", patient, &kid_a, false, Some(&baseline.event_id));
    let supp_signed = sign(&supp, &sk_a).unwrap();
    let ca = event_address(&supp_signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();

    let r = c.execute(SUBMIT3, &[&supp_signed.signed_bytes, &token, &vk_h]).await;
    assert!(r.is_ok(), "valid human-attested suppress must be accepted: {r:?}");
}

#[tokio::test]
async fn rejects_bad_attestations_and_keeps_the_floor() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // One baseline target + one suppress event reused across all rejections (none
    // append, so there is no idempotency interaction).
    let baseline = body("note.added", patient, &kid_a, false, None);
    let baseline_signed = sign(&baseline, &sk_a).unwrap();
    c.execute("SELECT submit_event($1)", &[&baseline_signed.signed_bytes]).await.unwrap();
    let supp = body("salience.downgrade", patient, &kid_a, false, Some(&baseline.event_id));
    let supp_signed = sign(&supp, &sk_a).unwrap();
    let ca = event_address(&supp_signed.signed_bytes);
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    let vk_a = sk_a.verifying_key().to_bytes().to_vec();

    // N1: a valid human token bound to a DIFFERENT event's address.
    let wrong = sign_attestation(&event_address(b"a different event"), &kid_h, "attested", &sk_h).unwrap();
    let r = c.execute(SUBMIT3, &[&supp_signed.signed_bytes, &wrong, &vk_h]).await;
    let e = format!("{:?}", r.unwrap_err());
    assert!(e.contains("not bound to this event"), "N1 wrong-address: {e}");

    // N2: a valid token with one byte flipped (signature no longer verifies).
    let good = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let mut tampered = good.clone();
    let m = tampered.len() / 2;
    tampered[m] ^= 0x01;
    let r = c.execute(SUBMIT3, &[&supp_signed.signed_bytes, &tampered, &vk_h]).await;
    let e = format!("{:?}", r.unwrap_err());
    assert!(e.contains("not bound to this event"), "N2 tampered: {e}");

    // N3: a VALID token, correctly bound, but the attester is an enrolled AGENT,
    // not a human (gate check #3, db/005:88-91).
    let agent_tok = sign_attestation(&ca, &kid_a, "attested", &sk_a).unwrap();
    let r = c.execute(SUBMIT3, &[&supp_signed.signed_bytes, &agent_tok, &vk_a]).await;
    let e = format!("{:?}", r.unwrap_err());
    assert!(e.contains("not an enrolled human actor"), "N3 non-human attester: {e}");

    // The floor held: not one suppressing event was appended.
    let n: i64 = c
        .query_one("SELECT count(*) FROM event_log WHERE event_type='salience.downgrade'", &[])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "no rejected suppress leaked into the log");
}
```

- [ ] **Step 2: Run the test with NO database to confirm it skips cleanly**

Run: `cargo test -p cairn-node --test attestation`
Expected: builds; all three tests run and print `skipped: set CAIRN_TEST_PG`; `test result: ok`.

- [ ] **Step 3: Run the test against a real Postgres with `cairn_pgx` installed**

Run:
```bash
CAIRN_TEST_PG="postgresql://localhost/cairn_test" cargo test -p cairn-node --test attestation -- --nocapture
```
(Substitute your local connstring; the DB must have the `cairn_pgx` extension available — `cargo pgrx install` against PG16. If you cannot run pgrx locally, mark this step blocked and note it; do not weaken the test.)
Expected: PASS — `3 passed`. The accept cases succeed; N1/N2/N3 reject with the asserted strings.

- [ ] **Step 4: Confirm clippy is clean**

Run: `cargo clippy -p cairn-node --tests -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/tests/attestation.rs
git commit -m "$(cat <<'EOF'
test(cairn-node): cover submit_event attestation accept + bad-binding rejects

Durable real-Postgres integration test driving the accept branch (P1 responsibility-
bearing additive, P2 human-attested suppress) and the rejections no prior test
reached: N1 wrong-address token, N2 tampered token, N3 valid token but non-human
attester. Closes the end-to-end half of the ADR-0030 advisory-actor contract.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Python harness end-to-end (external actor via the CLI)

**Files:**
- Modify: `poc/walking-skeleton/harness/agent_standin.py` (add `attest()`)
- Modify: `poc/walking-skeleton/harness/spike_0002.py` (add `_content_address_hex()` + the success/reject block)

**Interfaces:**
- Consumes: the new `attest-stdin` CLI (Task 1); existing `agent.key_id`, `agent._sign`, `_agent_body`, `expect_raises`, `_enroll`.
- Produces: `agent_standin.attest(bin_path, key_path, content_address_hex, role="attested") -> str`; one new `results[...]` row in `selftest`.

> Coverage test (Task 2 caveat applies): the success block exercises pre-existing accept logic through the CLI + Python stand-in.

- [ ] **Step 1: Add the `attest()` helper to the stand-in**

In `poc/walking-skeleton/harness/agent_standin.py`, after `_sign` (ends `:37`), add:

```python
def attest(bin_path, key_path, content_address_hex, role="attested"):
    """Mint a hex COSE_Sign1 attestation token bound to content_address_hex, signed
    by key_path's key, via `cairn-sync attest-stdin` (Rust owns the canonical encoding).

    Like _sign, this is a dumb signer: it attests whatever address it is handed, so a
    test can build a wrong-address token. The in-DB floor is what rejects a mis-binding.
    """
    kid = key_id(bin_path, key_path)
    body = {"content_address_hex": content_address_hex, "attester_key_id": kid, "role": role}
    p = subprocess.run([bin_path, "attest-stdin", "--key", key_path],
                       input=json.dumps(body).encode(), capture_output=True)
    if p.returncode != 0:
        raise RuntimeError(f"attest-stdin failed: {p.stderr.decode()}")
    return p.stdout.decode().strip()
```

- [ ] **Step 2: Add the content-address helper + `import hashlib` to the harness**

In `poc/walking-skeleton/harness/spike_0002.py`, add `import hashlib` to the imports (after `import argparse`/`json`, `:8-12`), and add this helper next to `expect_raises` (after `:33`):

```python
def _content_address_hex(signed_hex):
    """The event's content address = 0x1220 (sha2-256 multihash prefix) + sha256 of
    the signed wire bytes — identical to event_address() in cairn-event and v_ca in
    db/005. Attestation tokens bind to THIS value.
    """
    return "1220" + hashlib.sha256(bytes.fromhex(signed_hex)).hexdigest()
```

- [ ] **Step 3: Add the success/reject block to `selftest`**

In `poc/walking-skeleton/harness/spike_0002.py`, immediately after the C5 results line (`results["C5 floor holds against hostile agent"] = ...`, `:138`) and still inside the `with psycopg.connect(...) as db:` block, add:

```python
        # ---- Attestation SUCCESS path (the C2 complement; closes the ADR-0030 gap).
        # A human attests the agent's suppressing event end-to-end -> accepted.
        supp2 = _agent_body("salience.downgrade", pid, {"target_event_id": str(eid)}, [], agent_key)
        supp2_signed = agent._sign(args.bin, "/tmp/agent.key", supp2)
        ca2 = _content_address_hex(supp2_signed)
        token2 = agent.attest(args.bin, "/tmp/human.key", ca2)
        row = db.execute(
            "SELECT submit_event(decode(%s,'hex'),decode(%s,'hex'),decode(%s,'hex'))",
            (supp2_signed, token2, human_key)).fetchone()
        accept_ok = row is not None and row[0] is not None
        print("    P  suppress + valid human token accepted:", accept_ok)

        # A fresh suppress event with a WRONG-address token -> rejected.
        supp3 = _agent_body("salience.downgrade", pid, {"target_event_id": str(eid)}, [], agent_key)
        supp3_signed = agent._sign(args.bin, "/tmp/agent.key", supp3)
        wrong_token = agent.attest(args.bin, "/tmp/human.key", "1220" + "22" * 32)
        n1, d1 = expect_raises(
            db, "SELECT submit_event(decode(%s,'hex'),decode(%s,'hex'),decode(%s,'hex'))",
            (supp3_signed, wrong_token, human_key),
            "not bound to this event", "N1 wrong-address token rejected")
        print("   ", d1)

        # A correctly-bound token with one nibble flipped -> rejected.
        ca3 = _content_address_hex(supp3_signed)
        good3 = agent.attest(args.bin, "/tmp/human.key", ca3)
        flip = "0" if good3[len(good3) // 2] != "0" else "1"
        tampered = good3[:len(good3) // 2] + flip + good3[len(good3) // 2 + 1:]
        n2, d2 = expect_raises(
            db, "SELECT submit_event(decode(%s,'hex'),decode(%s,'hex'),decode(%s,'hex'))",
            (supp3_signed, tampered, human_key),
            "not bound to this event", "N2 tampered token rejected")
        print("   ", d2)

        results["Attestation success-path: accept + wrong-address/tamper rejected"] = (
            accept_ok and n1 and n2)
```

- [ ] **Step 4: Build the CLI and run the full selftest**

Run (from repo root):
```bash
cargo build -p cairn-sync
cd poc/walking-skeleton/harness
python spike_0002.py selftest \
  --conn "postgresql://localhost/cairn_test" \
  --bin "$(git rev-parse --show-toplevel)/target/debug/cairn-sync" --force
```
(Note: pass `--bin` explicitly — the harness default `../target/debug/cairn-sync` is stale since the crates moved to the root workspace. The DB needs `cairn_pgx` installed.)
Expected: the existing C1–C5 rows still PASS, the new row
`[PASS] Attestation success-path: accept + wrong-address/tamper rejected` prints, and the process exits 0.

- [ ] **Step 5: Commit**

```bash
cd "$(git rev-parse --show-toplevel)"
git add poc/walking-skeleton/harness/agent_standin.py poc/walking-skeleton/harness/spike_0002.py
git commit -m "$(cat <<'EOF'
test(spike-0002): exercise the attestation accept path end-to-end via the CLI

Adds agent_standin.attest() (drives the new attest-stdin CLI) and a selftest block:
a human attests the agent's suppressing event (accepted), plus wrong-address and
tampered-token rejections. The external-actor end-to-end complement to C2.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Documentation — record the gap as closed

**Files:**
- Modify: `docs/spikes/0002-advisory-actor-write-contract.md` (the honest-gap / status note)
- Modify: `docs/HANDOVER.md` (the Spike 0002 honest-gap bullet)

**Interfaces:** none (documentation).

- [ ] **Step 1: Update the spike doc**

In `docs/spikes/0002-advisory-actor-write-contract.md`, find the note recording that the attestation **success** path is never exercised, and append that it is now closed. Add a sentence such as:

> **Update 2026-06-22:** the attestation success path is now exercised end-to-end — `cairn-sync attest-stdin` (the token minter), a durable `cairn-node` integration test (`tests/attestation.rs`: accept for responsibility-bearing + suppressing events; reject for wrong-address, tampered, and non-human-attester), and the `spike_0002.py` selftest external-actor accept + wrong-address/tamper cases. No `submit_event` logic changed — the accept branch already existed; this is the coverage that was missing.

- [ ] **Step 2: Update HANDOVER.md**

In `docs/HANDOVER.md`, edit the Spike 0002 honest-gap bullet ("the attestation **success** path … is **never exercised E2E**") to mark it done — e.g. prepend `**(closed 2026-06-22)**` and note the `attest-stdin` CLI + `tests/attestation.rs` + harness coverage. Leave the smaller deferred items (`events_by_actor_epoch`, `actor_current` tiebreaker, FK, skeletal twin) as-is.

- [ ] **Step 3: Verify the docs site builds**

Run: `DISABLE_MKDOCS_2_WARNING=true uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build --strict`
Expected: builds; no new warnings beyond the pre-existing `../../LICENSE` INFO note.

- [ ] **Step 4: Commit**

```bash
git add docs/spikes/0002-advisory-actor-write-contract.md docs/HANDOVER.md
git commit -m "$(cat <<'EOF'
docs(spike-0002): record the attestation success-path gap as closed

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage:** §4.1 attest-stdin → Task 1. §4.2 Rust integration P1/P2/N1/N2/N3 → Task 2. §4.3 harness `attest()` + P2/N1/N2 → Task 3. §4.4 docs (spike + HANDOVER; ADR untouched) → Task 4. §5 TDD (unit-test-first for the CLI; coverage tests for pre-existing logic) → Task 1 Steps 2-5 + the caveats on Tasks 2-3. §6 risks: `connect_and_load_schema` loads db/004-006 (verified — Task 2 uses it directly); two-key flow built into every case; no owner-gate tested. All covered.

**2. Placeholder scan:** no TBD/TODO/"handle errors"/"similar to". Every code step shows complete code; every run step shows the command + expected output. The only ellipsis-like text is the `$(printf …)` shell expansion in Task 1 Step 7, which is literal runnable shell.

**3. Type consistency:** `attestation_token_hex(&str, &SigningKey) -> R<String>` defined in Task 1 and called by its own test + `cmd_attest_stdin`. `AttestationBody` / `sign_attestation` / `event_address` / `verify_attestation` are real `cairn_event` exports (confirmed). `submit_event(bytea,bytea,bytea)` and `enroll_actor(text,jsonb,text)` match `db/005`/`db/004`. `body()` returns the exact `EventBody` field set used in `admission.rs`. `attest()` returns a hex `str`; the harness passes it to `decode(%s,'hex')`. Consistent.
