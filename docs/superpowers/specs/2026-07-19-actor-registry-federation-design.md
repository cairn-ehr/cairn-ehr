# Actor-registry federation: sync-apply merge, dispute, and adjudication semantics

**Design session 2026-07-19 · resolves issue [#205](https://github.com/cairn-ehr/cairn-ehr/issues/205)
(2026-07-15 review finding C4) · deliverable: ADR-0054 + spec updates (code slices follow later).**

Related: [#172](https://github.com/cairn-ehr/cairn-ehr/issues/172) (door mirror obligation),
[#154](https://github.com/cairn-ehr/cairn-ehr/issues/154) (attestation gate is node-local),
[#94](https://github.com/cairn-ehr/cairn-ehr/issues/94) (human key custody — named follow-on),
[#200](https://github.com/cairn-ehr/cairn-ehr/issues/200) (quarantine-floor sibling ADR).

## 1. Problem

ADR-0011 §4 promises *partition-safe enrolment* ("enroll locally during an outage; the event syncs
upstream like any other"). ADR-0044/0046 made `enroll_actor` fail closed bidirectionally (≤1 key per
`actor_id`, ≤1 `actor_id` per key, whole-history). Two partitioned nodes can each locally-validly
create what is jointly a violation:

- **A-direction:** two people, degenerately identical pinned sets → same `actor_id`, two keys.
- **B-direction:** one key under two `actor_id`s — most commonly *one clinician enrolled at two
  facilities during a partition* whose two nodes minted slightly divergent pinned sets. (Identical
  pinned sets collide into the *same* `actor_id` and dedupe harmlessly; only sloppy divergence
  conflicts.)

On reconnect, set-union delivers both events. A sync-apply door that mirrors the enroll checks as
refusals violates lossless custody ("never reject, never drop", sync.md §6.3/§6.5); accepting both
into today's `actor_current` recreates the silent merge/attribution loss the guards exist to
prevent. No ADR specified the resolution. Meanwhile the attestation/owner gates trust only the local
registry (#154), so a human's vouch is not portable.

**Ground truth discovered in session:** `actor_event` rows are today *not wire events at all* — no
signature, no content address, no HLC, no origin node; `actor_current` orders by node-local
`(recorded_at, seq)`, which cannot converge across nodes even without conflicts. The wire shape is
therefore a day-one, can't-retrofit design obligation of this ADR (the ADR-0042 pattern).

## 2. Settled by the session (with the user)

1. **Scope:** merge/dispute/adjudication rule + wire shape + transport plane + ordering contract.
   Rotate-key door and human key custody / key-loss ceremony (#94) are named follow-ons.
2. **Threat model: benign-dominant.** The common conflict producer is the locum-at-two-sites /
   provisioning-sloppiness case; hostile key-reuse is the rare tail. Mechanism must make the benign
   case cheap to adjudicate and must never auto-resolve either case (ADR-0014 posture: uncertainty
   may only withhold; hard vetoes force a human decision).
3. **Interim clinical semantics:** clinical events signed by an implicated key **flow and apply**
   (availability over consistency), with attribution honestly degraded to the candidate set.
4. **Approach chosen: Admit-and-dispute** (Approach A). Pen-outside-and-adjudicate (B) rejected:
   registries diverge until adjudication, the conflict has no convergent representation, and the pen
   was built for unverifiable bytes, not unwelcome verifiable history. Deterministic tiebreak (C)
   rejected on principle 2 — an automated identity resolution is the forbidden move; it appears in
   the ADR as considered-and-rejected.

## 3. Design

### 3.1 Wire shape (day-one, can't-retrofit)

An actor event becomes a first-class signed wire event:

- **COSE_Sign1** (ADR-0015) under a **new dedicated signing context** for the actor plane
  (ADR-0040 domain separation — registry bytes can never replay as clinical or node events).
- **Signer = the enrolling node** (ceremony authority; the ADR-0053 `{node, recorded}` posture).
  The enrollee's public key + pinned set are content; a human co-signature and the ADR-0044
  person-distinguishing determinant ride in the body. Ceremony strength stays policy (ADR-0011 §4).
- **Content-addressed** (multihash of signed bytes) — dedupe/idempotence key on the wire.
- **HLC-stamped** (`t_recorded`, ADR-0027 graded) + origin `node_id`. Cross-node winner order for
  `actor_current` becomes **`(HLC, content_address)`** (deterministic, collation-independent —
  ADR-0045 discipline; winner-rule change is ADR-gated per sync.md §6.1). Local `seq` survives only
  as intra-node tiebreak for pre-wire local rows.
- **`actor_id` derivation unchanged** (content-address of the pinned set, ADR-0011).
- **Pre-wire rows never sync.** The door refuses unsigned registry history — the ADR-0051/0052
  wipe-dev-rigs posture. Pre-clinical: no migration burden, only stated discipline.
- **Registry trust is node trust** (stated explicitly): a node-signed enrolment is inherited only as
  far as the peer is trusted; a hostile node fabricating enrolments is answered by deny-all
  admission (§3.2), zero-authority-while-disputed (§3.3), and the ADR-0018 revocation cascade.

### 3.2 Transport and the apply door

- Actor events ride the **node-plane infrastructure** (db/007/db/022 machinery) as a distinct
  stream: **deny-all trusted-peer admission** (ADR-0017), **full replication within the trust
  neighborhood** (registry is small trust-plane state; this is what makes a vouch portable),
  per-peer seq cursor, and the **db/022-style pen** (re-offer floor + `acked` human exclusion) for
  unverifiable bytes.
- **Ordering contract — honest, not strict.** Registry and clinical streams have independent
  cursors; no cross-plane ordering is promised. Instead, at the consuming doors:
  1. **Content never waits.** A clinical event citing an unknown key applies normally, attribution
     honestly degraded ("key not yet resolved"), re-derived when the enrolment arrives.
  2. **Permissions always wait.** Operations where the registry *grants* authority (suppression
     owner-gate, attestation authority) are penned (delayed-never-lost, re-offered) until the cited
     registry state arrives and is clean. Fail-safe direction: a held suppression keeps the note
     visible; a held attestation reads un-vouched. **Registry uncertainty may withhold a
     permission, never withhold content.** This closes #154 structurally.
- **`apply_remote_actor_event`** (lands the #172 sync-door obligation): verify signature+context
  (unverifiable → pen) → origin admission (untrusted → skip-and-sweep) → content-address dedupe →
  **INSERT unconditionally** (custody total) → re-evaluate the **derived dispute state** (§3.3's
  live-bindings predicates — deliberately *not* the whole-history door predicates, see §3.4's
  split) and raise the worklist item on a newly-disputed `actor_id`/key. Detection, never refusal.
  The local `enroll_actor` door keeps its fail-closed whole-history stance unchanged (loud at
  creation stays the first line).

### 3.3 The disputed state

- **Derived, never declared** (ADR-0010 discipline): an `actor_id`/key is disputed iff the conflict
  predicates detect a violation among **live bindings** in the local log. No dispute event exists;
  adjudication changes the log the state derives from. Consequence: **log convergence ⇒ state
  convergence** — no dispute flag to race, no "who noticed first" ordering.
- **Projection:** `actor_current` stops picking winners for a disputed `actor_id`; a sibling
  projection (`actor_registry_state`) carries `clean | disputed(candidate_set) | revoked` per
  `actor_id` and per key, enumerating the conflicting rows.
- **Consumers:** db/005 authorship resolution renders a disputed key's events as the **candidate
  set** ("one of Dr X / Dr Y — registry dispute pending"), a first-class §5.10 authorship state
  (NULL-on-ambiguity survives only as fallback for states the candidate set can't express).
  Attestation/owner gates treat disputed as *not clean* → permission withheld → a hostile enrolment
  gains a worklist entry, zero authority. A new dispute raises a **salience-routed worklist item**
  (§5.12) at both implicated nodes — never a modal (paper-parity).
- **Interim semantics:** clinical events keep flowing (settled decision 3); on adjudication every
  projection re-derives retroactively and losslessly; the dispute window stays auditable forever.
- **A-direction sharp edge (named):** per-event attribution is unambiguous by signing key the whole
  time; only the registry identity of each key is disputed, so the candidate set is temporary by
  construction. Genuine permanent ambiguity only in the hostile stolen-key tail → `revoke` +
  contamination cascade (ADR-0011 §6, unchanged).

### 3.4 Adjudication algebra

- **No new verbs.** Adjudication = the existing `supersede` (a link — lineage preserved, never a
  merge) applied by an audited human ceremony:
  - **B-case (join):** two supersede events, each old `actor_id` → one successor (corrected pinned
    set with proper determinants).
  - **A-case (per-key fork):** the degenerate binding superseded per key; each person gets a
    successor `actor_id` with distinguishing determinants binding their key. Attribution re-derives
    exactly, per key.
  - **Hostile:** `revoke` with compromise-time + contamination cascade.
- **The load-bearing predicate split (must be explicit in the ADR):** the **derived disputed state
  computes over live bindings** (non-superseded, non-revoked) so adjudication clears the dispute by
  construction; the **enroll-door predicates stay whole-history** (anti-resurrection, fail-closed).
  Without this split the A-case repair deadlocks: re-enrolling a key under its proper successor
  would trip the whole-history B-check; supersede is the sanctioned repair path the doors recognize.
- **Adjudication ceremony:** audited, mandatory recorded human adjudicator (ADR-0011 §4 backstop);
  *who* may adjudicate is policy (principle 9). Adjudication events sync as ordinary actor events.
  **Conflicting concurrent adjudications** (two nodes resolve the same dispute differently during
  another partition): the live-binding derivation sees conflicting supersedes → state reads
  disputed again, worklist escalates — never auto-picked. Rare, honest, convergent.

## 4. What this closes / names

| Thread | Effect |
|---|---|
| #205 (C4) | Closed by the ADR. |
| #172 | Sync-door half landed by §3.2; rotate-key door mirror obligation restated, unchanged. |
| #154 | Closed structurally by permissions-wait + registry federation. |
| #94 + key-loss ceremony | Named follow-on ADR (constrains ceremony strength, not this algebra). |
| #200 (B5) | This ADR leans on the §6.3 quarantine floor; sibling ADR formalizes it generally. |

## 5. Blast radius and testing

Safety-critical surface throughout (§9: in-DB/Rust, reviewer-legible, TDD). Key tests:

- **Property (the one that matters):** the derived dispute state is a pure function of the log —
  **admission order never changes it** (set-union commutativity + idempotence over arbitrary
  interleavings of enrol/supersede/revoke/adjudication events; P5 proptest style).
- Multi-node convergence via the existing `CAIRN_TEST_PG2`/`PG3` harness (partition → dual enrol →
  reconnect → both nodes converge to disputed → adjudicate on one → both converge to clean).
- Hostile pins: fabricated enrolment yields worklist entry + zero authority; unsigned pre-wire row
  refuses at the door; stolen-key revoke cascades.

## 6. Deliverables (this design's implementation plan)

1. **ADR-0054** — actor-registry federation: admit-and-dispute, wire shape, node-plane transport,
   content-never-waits/permissions-always-wait, live-vs-whole-history predicate split, supersede
   adjudication. Considered-and-rejected: pen-outside, deterministic tiebreak.
2. **Spec updates:** security.md §7.5 (registry federation + dispute state), sync.md (the actor
   stream + ordering contract rows), data-model §3.12 (wire shape + convergent ordering),
   identity/§5.10 (candidate-set authorship state).
3. Code slices follow as ordinary TDD feature work (wire shape, apply door, dispute projection,
   adjudication ceremony) — not part of this session.
