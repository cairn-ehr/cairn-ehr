# Identity C2 — the match_proposal → apply seam (design)

**Date:** 2026-07-02 · **Piece:** matcher/identity C2 · **Status:** design approved, pre-plan
**Implements (settled, no new ADR/spec):** §5.1/§5.7 identity algebra (C1) · ADR-0030 advisory-actor
attestation contract · ADR-0014 matcher provenance. **No floor change, no new event type, no spec bump.**

## Goal

Turn a **human-accepted** advisory `match_proposal` row (matcher piece B2 output, `db/017`) into a real,
human-**attested** `identity.link.asserted` event through the C1 door (`db/018`), so the link projects
into `patient_link` / `person_member` like any other link. This closes the matcher → identity loop for the
review-gated case.

Scope is deliberately the **first, smallest safe** slice: **human-accepted only** (no auto-apply), and the
accepting human is a **responsibility-bearing (attested) contributor** — a human vouching for a patient
merge should bear responsibility for it.

## The key insight — C2 needs no floor change

`identity.link.asserted` is registered *additive* (C1, `db/018`). But the seam deliberately places a
**responsibility-bearing contributor** (the accepting human) in the event's `contributors`. That trips the
**existing** attestation gate in `db/005` (`submit_event`), which — for any asserted responsibility —
already requires a valid attestation token, bound to *this* event, minted by an **enrolled human** actor.

So C2 composes three already-settled floors verbatim:

- **db/018** — the C1 identity structural floor (`cairn_check_link_assertion`) + the HARD authored-twin
  requirement in `cairn_event_twin`.
- **db/005** — the ADR-0030 attestation gate (responsibility ⇒ valid human token bound to the event).
- **db/018** — the `patient_link_apply` AFTER trigger → edge overlay + component projection.

There is **no `submit_event` change, no new event type, no new door, no spec/ADR change.**

## Approaches considered

- **A — Rust seam in `cairn-node`, reading `match_proposal` directly (chosen).** Selection, event
  assembly, signing, token minting, `submit_event`, and mark-applied all live in `cairn-node`. Follows the
  §9 defect-blast-radius rule (event construction + signing is safety-critical → Rust / reviewer-legible),
  reuses `cairn-event::identity` + `sign` + `sign_attestation` with **zero serialization drift**, and
  mirrors how C1 shipped (db migration + cairn-node logic + integration tests, no new floor).
- **B — Python matcher selects, shells to a Rust minter.** Rejected: adds an IPC boundary and splits the
  seam across two languages for no benefit — the selection is a trivial `WHERE status='accepted'`, not
  matcher logic. Signing in Python would also duplicate COSE serialization (drift risk, wrong §9 tier).
- **C — a new dedicated in-DB apply door.** Rejected as redundant: `submit_event(3-arg)` + the attestation
  gate + the C1 twin floor already do everything; a new door would duplicate the floor.

## Components

### 1. `db/019_apply_proposal.sql` (additive)

- `ALTER TABLE match_proposal ADD COLUMN applied_event_id UUID` (nullable).
- A comment documenting the invariant `status='applied' ⇔ applied_event_id IS NOT NULL`.
- Wire into `cairn-node` `db.rs` `SCHEMA` array (length **17 → 18**). **No SCHEMA-floor version bump** —
  additive DDL only. No new event type; `submit_event` untouched; `match_proposal`'s existing
  `GRANT … UPDATE … TO cairn_agent` already permits the mark-applied write.

### 2. `cairn-node/src/apply_proposal.rs` (new module, target < 500 lines)

**Pure** (unit-testable, no DB):

```
build_attested_link_body(
    low: Uuid, high: Uuid, provenance: &str, confidence: Option<&str>,
    human_kid: &str, hlc: Hlc,
) -> EventBody
```

Assembles the link body: `event_type = "identity.link.asserted"`,
`schema_version = "identity.link/1"`, `patient_id = low` (C1 "about subject_a" convention, with
`subject_a = low`, `subject_b = high` in canonical order), `contributors =
[{"actor_id": human_kid, "role": "attested", "responsibility": "attested"}]`, authored twin via
`cairn_event::identity::render_link_twin`, payload via `link_assertion_body`. Confidence omitted when
`None` (principle 4 omit-when-absent, inherited from the C1 builder).

**IO** (the only DB-touching function):

```
apply_accepted_proposal(
    conn, low: Uuid, high: Uuid,
    human_sk: &SigningKey, human_kid: &str, hlc: Hlc,
) -> Result<Uuid /* applied event_id */>
```

In **one transaction**:
1. `SELECT score_total, matcher_version, status FROM match_proposal WHERE patient_low=low AND patient_high=high`.
   Refuse if not found or `status <> 'accepted'` (a legible error; only accepted proposals apply).
2. Compose `provenance = "matcher:{matcher_version} accepted-by:{human_kid}"` and
   `confidence = format!("{:.3}", score_total)`.
3. `build_attested_link_body(...)` → `sign(body, human_sk)` → `signed_bytes`.
4. `sign_attestation(event_address(&signed_bytes), human_kid, "attested", human_sk)` → token.
5. `SELECT submit_event($signed, $token, $human_vk)` (the 3-arg attested door).
6. `UPDATE match_proposal SET status='applied', applied_event_id=$eid, updated_at=clock_timestamp()
   WHERE patient_low=low AND patient_high=high`.
7. `COMMIT`.

**Atomicity gives idempotency.** If any step fails, the whole transaction rolls back: no link event, and
`status` stays `'accepted'` so the next run retries. On success both the event and the `'applied'`
transition commit together; a re-run selects only `'accepted'` rows and skips this one — no duplicate link
event.

Optional convenience `apply_all_accepted(conn, human_sk, human_kid, hlc_source)` iterating every
`status='accepted'` pair may be added if trivial; not required for the slice.

### 3. Provenance / confidence / identity mapping

- `provenance` — `"matcher:{matcher_version} accepted-by:{human_kid}"`. Non-empty (floor requirement),
  carries the ADR-0014 matcher config digest **and** records the human vouch.
- `confidence` — `format!("{:.3}", score_total)`: preserves the matcher's acknowledged-uncertainty trail
  (principle 4). The **human's acceptance** is the authority; the score is retained as honest context.
- `subject_a / subject_b` — the canonical `(patient_low, patient_high)` pair, already `low < high`.
- **Signer = attester = the accepting human.** The honest reading of "a human created and vouched for this
  link": the human authors the authoritative link event (signs it) *and* is its responsibility-bearing
  attester. The matcher is not a signer.

## Data flow

```
match_proposal (status='accepted')
  └─ read (low, high, score_total, matcher_version)
      └─ build_attested_link_body  (pure)
          └─ sign (human key)  →  sign_attestation (human key)
              └─ submit_event($signed, $token, $human_vk)     [one txn]
                  ├─ db/018 cairn_event_twin  → cairn_check_link_assertion + authored-twin  ✓
                  ├─ db/005 attestation gate  → valid human token bound to event            ✓
                  └─ db/018 patient_link_apply (AFTER trigger)
                      └─ patient_link edge overlay → cairn_recompute_component → person_member
              └─ UPDATE match_proposal → status='applied', applied_event_id
          └─ COMMIT
```

## Testing (TDD)

**Pure unit tests** (in `apply_proposal.rs`, no DB):
- body carries a responsibility-bearing contributor (`role="attested"`, `responsibility` present);
- provenance non-empty and contains the matcher_version;
- authored twin present and starts with `link: `;
- subjects in canonical `(low, high)` order; `event_type = identity.link.asserted`.

**DB-gated integration tests** (`crates/cairn-node/tests/apply_proposal.rs`, gated on `$CAIRN_TEST_PG`,
serialized via `db::test_serial_guard`; enroll one **human** actor + seed a `match_proposal` row directly):
1. **Happy path** — accepted proposal → link event appended to `event_log`; `patient_link` edge exists
   (`state='link'`); both patients project to the min-UUID `person_id` in `person_member`; proposal
   `status='applied'` with `applied_event_id` set to the link event id.
2. **Idempotency** — running apply twice appends exactly one link event (second run finds no
   `'accepted'` row).
3. **Forgery refused** — applying with an *agent* (non-human) key is rejected by the floor
   (`not an enrolled human actor`); nothing appended; proposal stays `'accepted'`.
4. **Non-accepted skipped** — a `'pending'` proposal is refused / no-op (only `'accepted'` applies);
   nothing appended.

**Commands:** `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb
dbname=cairn_test" cargo test --test apply_proposal` (PG18 + `cairn_pgx`); `cargo clippy --tests` clean;
full `cairn-node` suite green before commit.

## Scope boundaries (deferred, recorded — not lost)

- **Auto-apply of the `auto_candidate` band** (C2b) — matcher-authored, un-attested, recallable link for
  above-threshold pairs (§5.2 "auto above threshold"). This slice is human-accepted only.
- **Matcher as a compositional contributor** (principle 10) — depends on the not-yet-built §7.5
  matcher-actor registration; the matcher lives in the provenance string for now.
- **CLI subcommand** + **where production sources the accepting human's signing key** (review-backend key
  custody, ADR-0011) — the seam takes a provided human `SigningKey`, exactly as C1 / `attestation.rs`
  already do; production key custody is an open thread those pieces already carry.
- **Proposal retraction / unlink-from-a-rejected-proposal** — rejecting a *proposal* means "never linked",
  so no unlink is involved; out of scope.
- **Sourcing the HLC from the node clock** — the function takes an `Hlc`; wiring it to a live clock tick is
  a caller/CLI concern, consistent with existing clinical-event authoring.
```
