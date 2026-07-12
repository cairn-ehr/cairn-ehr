# Design — `enroll-human` ceremony CLI

**Date:** 2026-07-11 · **Branch:** `feat/enroll-human-ceremony-cli` · **Tier:** safety-critical
(trust-anchor enrollment) → Rust + the existing in-DB floor. **No** wire / event-format / floor /
SCHEMA / ADR / spec change — additive Rust that reuses the `enroll_actor` door.

## Why

The §5.4 John-Doe subsystem's structural finishers 1–3 are built, but **finisher 3**
(`identify-patient --link`) needs an enrolled `kind='human'` actor to sign+attest the optional link,
and there is **no `enroll-human` CLI yet** — `crates/cairn-node/tests/identify.rs` enrolls the human
attester via **raw SQL** (`enroll_actor('human', '{"role":"clinician","handle":"dr-a"}', $1)`) with a
`// there is no enroll-human CLI yet` comment. This slice builds that ceremony so `identify --link`
works end-to-end and the tests exercise the real authoring path (no raw-SQL workaround = removed
technical debt, house rule 5).

It is the natural closer of the §5.4 line and is purely desk-doable (no external dependency).

## Background (what already exists — do not rebuild)

- **`enroll_actor(kind, pinned, key)`** (`db/004_actors.sql`) — the one enrollment door. Derives
  `actor_id = cairn_actor_id(pinned)` (the content-address of the **pinned set only**, never the key —
  so `rotate-key` keeps `actor_id` stable, ADR-0011 §5). Per **ADR-0044** it **fails closed** if that
  `actor_id` already binds a *different* signing key (whole-history, incl. revoked) — the anti-silent-merge
  floor (principle 2). Idempotent same-key re-enroll passes while the actor is live. `REVOKE`d from
  `cairn_agent` — an **owner** ceremony, so the CLI needs the owner `--conn`.
- **`attester_is_enrolled_human(db, kid)`** (`src/identify.rs`) — already reads
  `actor_current WHERE signing_key_id=$1 AND kind='human'`; the `identify --link` pre-check.
- **`ensure_registration_actor`** (`main.rs:1234`) — the device-key precedent. Its comment (main.rs:1240)
  documents the **dual-mapping hazard**: `submit_event` resolves a signer to an actor purely by
  `signing_key_id`, and if one key maps to MORE than one `actor_current` row it sets `actor_id = NULL`
  for **every** event that key authors node-wide (db/005 `array_length(v_actor_ids,1)=1`), silently
  degrading attribution. So a key must map to **at most one** actor.
- Key primitives (`src/keystore.rs`): `generate_plaintext`, `generate_sealed`, `load`,
  `key_at_rest_state`; `print_recovery_code` / `resolve_passphrase` (`main.rs`). `init` mints a **sealed**
  key + shown-once recovery code + a node-scoped local-state `.lsk` escrow (ADR-0026 slice D).

## Decisions

### D1 — Person-distinguishing determinant: registration-id primary, handle fallback

ADR-0044 §2 requires a human's pinned set to carry a person-distinguishing determinant but deliberately
does **not** hard-code *which* field (ADR-0011 keeps pinned-set contents as policy). This CLI adopts:

- `--registration-id <str>` — a professional licensure/registration number (the real-world unique person
  id; ADR-0033 fixes licensure IDs in the §7.5 actor registry). Preferred when it exists.
- `--handle <str>` — a node-local, human-chosen label (covers clerks/students/visitors with no licence
  number, and the existing test fixture `dr-a`).
- `--role <str>` — defaults to `clinician`.

The pinned set is `{"role": role, "registration_id"?: …, "handle"?: …}` — **omitting** any absent optional
field, and **never** the signing key. **At least one** of `registration_id`/`handle` MUST be supplied; the
builder refuses otherwise (principle 4 — the determinant is a real field, never fabricated; do not lean on
the floor's loud refusal as the only guard). Cairn does not guarantee global uniqueness of the determinant
(that is registry federation, issue #154); the floor's ADR-0044 refusal is the backstop for an actual
`actor_id` collision, surfaced legibly by the CLI.

### D2 — New sealed key: recovery code, no `.lsk` sidecar

When the key file is absent and not `--insecure-plaintext`, mint a **sealed** key (`generate_sealed`) +
a shown-once recovery code (`print_recovery_code`), but **skip** `establish_local_state_escrow`: the
`.lsk` sidecar wraps **node-scoped** local state (DEKs/drafts/config) that a clinician's **personal** key
does not have. This keeps the ceremony lean and avoids attaching node semantics to a personal key. Honest
limit: lose both passphrase and recovery code and the key is unrecoverable (recover via `rotate-key` once
it exists) — the same posture as any sealed key, documented in the command help.

### D3 — Dual-mapping guard in the orchestrator

Before calling `enroll_actor`, refuse if `kid` already maps to an `actor_current` row (any kind), because a
second actor for one key triggers the db/005 `actor_id=NULL` node-wide degradation (D2 background). An
**idempotent** same-key re-enroll of the **same** `actor_id` is allowed (re-runnable provisioning) — detect
by comparing the existing row's `actor_id` to `cairn_actor_id(pinned)`; equal ⇒ no-op success, different ⇒
refuse loudly. This mirrors `ensure_registration_actor`'s kind-agnostic existence logic.

## Components (all in `crates/cairn-node`)

1. **`src/enroll.rs`** (new, pure + async, kept well under 500 lines):
   - `pub fn build_human_pinned(role: &str, registration_id: Option<&str>, handle: Option<&str>)
     -> anyhow::Result<serde_json::Value>` — assembles the pinned JSON; trims + rejects blank inputs;
     refuses when both determinants are absent; never includes the key. Pure, unit-testable.
   - `pub async fn enroll_human_actor(db, kid: &str, pinned: &serde_json::Value)
     -> anyhow::Result<EnrollHumanOutcome>` — the dual-mapping guard (D3) + the `enroll_actor('human', …)`
     call, mapping the ADR-0044 refusal to a legible "add a distinguishing `--registration-id`/`--handle`"
     error. Returns whether it enrolled or was an idempotent no-op.

2. **`src/main.rs`**:
   - `Cmd::EnrollHuman { registration_id: Option<String>, handle: Option<String>, role: String
     (default "clinician"), passphrase: Option<String>, insecure_plaintext: bool }` on the shared `--key`
     / `--conn` globals.
   - Handler: resolve/create the key (D2), derive `kid`, `build_human_pinned`, `enroll_human_actor`, print
     `enrolled human actor <kid>` (+ the determinant echoed). Cross-flag validation done before any I/O
     (mirrors the `identify-patient` pre-I/O validator style).

3. **`crates/cairn-node/tests/identify.rs`** — the `enroll_human` helper now calls the library
   `enroll_human_actor` (+ `build_human_pinned`) instead of raw SQL (real-path reuse / no-drift).

## Testing (TDD — failing test first)

**Pure (`src/enroll.rs` unit tests):**
- `build_human_pinned` includes `registration_id`/`handle` when present, omits when absent, always carries
  `role`, and **never** a key field.
- refuses when both determinants are `None` (or blank).

**DB-gated (`crates/cairn-node/tests/enroll_human.rs`, `#[ignore]`-free, gated on `CAIRN_TEST_PG`):**
- enrolls a `kind='human'` actor resolvable by `attester_is_enrolled_human`.
- two humans with DISTINCT `registration_id`s get DISTINCT `actor_id`s (no collision).
- two humans with IDENTICAL pinned sets + distinct keys → the floor refuses the second, surfaced with the
  legible determinant hint (ADR-0044).
- idempotent same-key re-enroll (same pinned) passes as a no-op.
- dual-mapping guard: enrolling a key already enrolled (e.g. as `device`) as human is refused before the
  door (D3).
- **e2e payoff:** enroll a human via the library path, then `identify_patient` with `--link` succeeds
  (the raw-SQL replacement, proving finisher 3 works end-to-end through the real enrollment ceremony).

## Out of scope (named, not built)

- `rotate-key` for a human (recovery path for a lost personal key) — deferred, ADR-0011 §5, no door yet.
- Global uniqueness / federation of the determinant (issue #154 — registry does not replicate).
- The actor-event **sync apply door** mirroring the ADR-0044 check (ADR-0044 §3; no such door exists yet).
- The other remaining §5.4 threads: the "prior history now available" push-alert (§5.12, no notification
  tier); the search-before-create funnel (§5.3/§5.8, UI/API tier).

## Verification

- `cargo test -p cairn-node` (pure) + the DB-gated suite against PG18 + `cairn_pgx`
  (`CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`).
- `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `mkdocs build` clean.
- Whole-branch review before PR.
