# Design — §5.4 finisher 3: `identify` → optional link

**Date:** 2026-07-11
**Slice:** §5.4 John-Doe subsystem, finisher 3
**Status:** design approved; ready for implementation plan
**Change class:** additive Rust (node authoring path + CLI) only — **no** new event type,
migration, floor change, SCHEMA/ADR/spec version bump.

## Purpose

Give a clinician the front door to **resolve a John-Doe chart**: record *who* the patient is
(flipping the chart's §5.7 trust state from *unconfirmed* → *confirmed*) and, when that person
already has a prior chart in this record, **optionally join the two charts** so their history
flows together.

This is the third and last structural finisher of the §5.4 unidentified-registration subsystem.
`register-john-doe` (slice A) mints the *unconfirmed* chart; this slice closes it out.

Reattribution of the clinical *documentation* itself (§5.5 `reattribute`) is explicitly **not**
part of this slice — it needs a clinical-note surface that does not yet exist. This slice is only:
confirm identity + optionally link to a prior chart.

## What already exists (so this slice does not rebuild it)

- **Event type + floor.** `db/024_identity_identify.sql` already registers
  `identity.identify.asserted` (additive, `targets_other_author=FALSE`), validates it
  (`cairn_check_identity_state_assertion` — subject must be a valid uuid; `method` non-empty),
  HARD-requires an authored twin, and folds it into the `chart_identity_state` overlay
  (`pending` → `identified`), which `chart_trust` reads as *confirmed*.
- **Event builders.** `cairn-event::identity` already provides `IdentifyAssertion`,
  `identify_assertion_body`, and `render_identify_twin`, fully unit-tested. This slice consumes
  them; it adds nothing to `cairn-event`.
- **The attested-link machinery.** `cairn-node::apply_proposal::build_attested_link_body` is
  already pure and proposal-independent — it takes `(event_id, low, high, provenance, confidence,
  human_kid, hlc)` and returns a human-attested `identity.link.asserted` body. The full
  sign → `sign_attestation` → 3-arg `submit_event($1,$2,$3)` shape is proven there and is reused
  verbatim.

So the only missing pieces are (1) a node authoring path that mints the identify event, and
(2) a CLI that drives identify + the optional attested link.

## Accountability model (the load-bearing decision)

**identify is device-additive; the human attests the link only.**

- The `identity.identify.asserted` event is authored with the **node key** and the node's
  `device` registration actor (via the existing `ensure_registration_actor`), exactly like the
  other §5.4 CLIs (`register-john-doe`, `assert-observed-evidence`, `assert-identity-evidence`).
  It is additive, carries no responsibility-bearing contributor, and so trips no attestation gate.
- The optional `identity.link.asserted` **merges a John-Doe chart into a prior identity** — a real
  human attribution. It carries a responsibility-bearing `attested` contributor and is signed +
  attested by a **human** key supplied at the CLI (`--attester-key`). This is the "attestation
  from the CLI" that no prior §5.4 command exercised.

**Human enrollment is a precondition, never auto-created.** The attester key must already resolve
to a `kind='human'` actor (human-ness is resolved from the append-only `actor_event` history, per
ADR-0043/0044). This slice does **not** enroll a human actor — auto-enrolling one would fabricate a
trust anchor without a real person-distinguishing determinant (ADR-0044). A dedicated
human-enrollment ceremony CLI is a separate future slice; until then, tests enroll the human via
raw SQL, mirroring `attestation.rs`.

## CLI surface

New subcommand **`identify-patient`** (named to avoid the one-character typo trap against the
existing `identity` node-identity command):

```
cairn-node identify-patient <PATIENT_UUID> --method <TEXT>
    [--link <PRIOR_UUID> --attester-key <PATH> [--attester-passphrase <PASS>]]
```

- `<PATIENT_UUID>` — the John-Doe chart being identified.
- `--method <TEXT>` — required; §5.7 "method recorded" (e.g. "driver's licence + family
  confirmation"). Non-empty (principle 4; the db/024 floor also enforces this).
- `--link <PRIOR_UUID>` — optional; the prior chart to join to.
- `--attester-key <PATH>` — the human signing key that vouches for the merge. **Required when
  `--link` is given.** Supplying it without `--link` is rejected with a clear message (an attester
  with nothing to attest is a mistake worth surfacing, not silently ignoring).
- `--attester-passphrase` — unseal passphrase for the attester key (env
  `CAIRN_ATTESTER_PASSPHRASE`, else prompt), following the `resolve_passphrase` convention.

Behavior:

1. Author `identity.identify.asserted` (device-additive) → chart flips to *confirmed*.
2. If `--link` given: also author a human-attested `identity.link.asserted` joining
   `<PATIENT_UUID>` ↔ `<PRIOR_UUID>`.
3. Both events are submitted in **one transaction** — atomic. If the link is refused (attester not
   an enrolled human, floor rejection, tamper), the whole operation rolls back: the identify is not
   written either. A caller who asked to link is never left with a half-resolved chart.
4. `--link` without `--attester-key` → legible refusal before any work
   ("linking to a prior chart is a human attribution — supply --attester-key").

## Module design

New module `crates/cairn-node/src/identify.rs` (well under 500 lines; split pure / IO like
`john_doe.rs` and `apply_proposal.rs`):

- **Pure** `build_identify_body(event_id: Uuid, patient: Uuid, method: &str, node_kid: &str,
  hlc: Hlc) -> EventBody` — the device-additive identify event. Contributor role `recorded`, no
  `responsibility` key; `plaintext_twin` from `render_identify_twin`; payload from
  `identify_assertion_body`. Fully unit-testable, no I/O.
- **Pure** `compose_identify_link_provenance(human_kid: &str) -> String` →
  `"john-doe-identify linked-by:<kid>"`. Non-empty by construction (the db/018 floor requires a
  non-empty provenance) and legible (names the vouching human).
- **Async** `identify_patient(client, node_sk, node_kid, node_origin, patient, method,
  link: Option<LinkParams>) -> anyhow::Result<IdentifyOutcome>`:
  - Ticks the HLC once for identify (and once more for the link when present) — the same
    tick-then-submit shape as `register_john_doe`; ticks self-commit and gaps are allowed.
  - Signs the identify body with the node key.
  - If linking: canonicalizes `(patient, prior)` to `(low, high)` (matching the C1 edge overlay's
    canonical key and db/018's ordering), calls `build_attested_link_body` with `confidence: None`
    (a human's direct assertion, not a matcher score) and the composed provenance, then
    `sign` + `sign_attestation` with the human key.
  - Opens one transaction: `submit_event($1)` for the identify, then (if linking)
    `submit_event($1,$2,$3)` for the link; commits.
  - Returns `IdentifyOutcome { identify_event_id: Uuid, link_event_id: Option<Uuid> }`.
- **`LinkParams<'a> { prior: Uuid, human_sk: &'a SigningKey, human_kid: &'a str }`** — the
  optional link inputs, threaded explicitly so the orchestrator stays a pure function of its args.

`main.rs` handler (`Cmd::IdentifyPatient`):
- Load the node signing key (interactive); `ensure_registration_actor(node_kid)`.
- If `--link`: load the attester key from `--attester-key` (its own passphrase), derive its kid,
  and **pre-check** it resolves to a `kind='human'` actor
  (`EXISTS(SELECT 1 FROM actor_event … kind='human' …)` — resolved from history, so a
  rotated/departed human still counts) → bail with a legible message if not.
- Call `identify_patient`; print the outcome (identify event id; "chart now confirmed"; and, when
  linked, "linked to <prior> (event <id>)").

## Guards & non-guards

- **Enforced:** the attester must be an enrolled human (pre-checked in the CLI for legibility; the
  db/005 attestation gate is the real, unbypassable enforcement).
- **Atomic:** identify + link succeed together or not at all.
- **Not pre-checked (by design):** existence of `patient` or `prior`. The offline-first floor does
  no cross-existence check (a pending marker or the prior chart may not have synced yet); the
  db/018 floor rejects only a self-link (`a == b`) and an empty provenance. Adding a CLI existence
  gate would break offline-first and paper-parity for no safety gain.
- **Not changed:** identify remains additive even when a link is present — the two events have
  distinct authorship (device records the identification; the human vouches for the merge),
  exactly the compositional-authorship model (principle 10).

## Testing (TDD — failing test first)

**Pure unit tests** (in `identify.rs`):
- `build_identify_body` produces `event_type = "identity.identify.asserted"`, `payload.subject ==
  patient`, `payload.method == method`, a non-empty authored twin, and a `recorded` contributor
  with **no** `responsibility` key (so no attestation is demanded).
- `compose_identify_link_provenance` is non-empty and contains the human kid.

**DB-gated integration tests** (`crates/cairn-node/tests/identify.rs`, gated on `CAIRN_TEST_PG`,
serialized via `db::test_serial_guard`, following `attestation.rs`/`identity_linkage.rs`):
- **identify alone** flips a pending chart: after `identify_patient` with no link,
  `chart_identity_state.state = 'identified'` for the subject and `chart_trust` yields *confirmed*
  (or no row → coalesced confirmed).
- **identify --link** joins the charts: the link edge is present and `person_of(patient) ==
  person_of(prior)` (same connected component), and the chart reads *confirmed*.
- **atomicity**: a link whose attester is a non-human / unenrolled key is refused by the floor and
  the whole transaction rolls back — **no** identify event is written (the chart stays *pending*).
- **pre-check legibility**: an unenrolled/non-human attester key produces the CLI's clear error
  (exercised at the library boundary or via the pre-check helper).

Full workspace green (cairn-node DB-gated, cairn-event, cairn-sync), `cargo fmt` + `clippy` clean,
mkdocs builds, before commit.

## Out of scope / deferred (recorded)

- `reattribute` of clinical documentation (§5.5) — waits on a clinical-note surface.
- A human-enrollment ceremony CLI (`enroll-human`) — its own slice; ADR-0044 person-distinguishing
  determinant must be got right.
- The "prior history now available" push-alert on link (§5.12) — no notification tier yet.
- The search-before-create registration funnel (§5.3/§5.8) — UI/API tier.
