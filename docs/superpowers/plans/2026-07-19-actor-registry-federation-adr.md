# ADR-0054 Actor-Registry Federation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write ADR-0054 (actor-registry federation: admit-and-dispute) and land its prose in the four canonical spec homes, closing issue #205.

**Architecture:** Docs-only session. The approved design is
`docs/superpowers/specs/2026-07-19-actor-registry-federation-design.md`; this plan turns it into the
immutable ADR plus additive spec updates. Code slices (wire shape, apply door, dispute projection,
adjudication ceremony) are explicitly NOT in this plan — they become ordinary TDD feature work later.

**Tech Stack:** Markdown; mkdocs (pinned via `docs/requirements.txt`); git; `gh` CLI.

## Global Constraints

- Branch: `design/205-actor-registry-federation` (already exists, has the design-doc commit).
- ADRs are immutable once merged; get the text right now. Status `Accepted`, date `2026-07-19`.
- Docs build MUST use the pinned requirements: `uv run --with-requirements docs/requirements.txt -- mkdocs build` from the repo root (never ad-hoc installs — security note in `docs/requirements.txt`).
- Spec files carry NO in-file changelogs; the version lives only in `docs/spec/index.md` (bump 0.55 → 0.56).
- Callouts use GitHub/Obsidian syntax (`> [!NOTE]`).
- Link style: relative markdown links exactly as neighboring prose does (e.g. `[ADR-0044](decisions/0044-enroll-fail-closed-on-actor-id-collision.md)` from a spec file; `[ADR-0044](0044-....md)` from another ADR).
- Every commit message ends with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: Write ADR-0054

**Files:**
- Create: `docs/spec/decisions/0054-actor-registry-federation-admit-and-dispute.md`

**Interfaces:**
- Produces: the ADR file that Tasks 2–5 link to as
  `decisions/0054-actor-registry-federation-admit-and-dispute.md` (from spec files) and
  `0054-actor-registry-federation-admit-and-dispute.md` (from other ADRs, if ever needed).

- [ ] **Step 1: Create the ADR file with exactly this content**

````markdown
# ADR-0054 — Actor-registry federation: admit-and-dispute, the signed actor-event wire shape, and the adjudication algebra

- **Status:** Accepted (refines [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md), [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md), [ADR-0046](0046-enroll-fail-closed-on-key-actor-dual-mapping.md); applies [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)/[ADR-0040](0040-signing-context-domain-separation.md)/[ADR-0045](0045-collation-independent-projection-tiebreaks.md)/[ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)/[ADR-0018](0018-federation-revocation-cascade-and-the-anchor-as-power.md)/[ADR-0027](0027-trusted-time-anchoring.md); the interim-attribution surface extends [ADR-0007](0007-authorship-and-accountability.md)'s consumer side)
- **Date:** 2026-07-19

## Context

[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) §4 promises **partition-safe
enrolment** — "enroll locally during an outage; the event syncs upstream like any other."
[ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md)/[ADR-0046](0046-enroll-fail-closed-on-key-actor-dual-mapping.md)
then made `enroll_actor` **fail closed bidirectionally** (≤ 1 key per `actor_id`, ≤ 1 `actor_id` per
key, whole-history). Two partitioned nodes can each locally-validly create what is *jointly* a
violation:

- **A-direction:** two different people whose pinned sets are degenerately identical → the same
  `actor_id` under two keys (each node saw only one enrolment, so neither refusal fired).
- **B-direction:** one signing key under two `actor_id`s — most commonly **one clinician enrolled at
  two facilities during a partition** whose nodes minted slightly divergent pinned sets. (Identical
  pinned sets compute the *same* `actor_id` and dedupe harmlessly; only divergence conflicts.)

On reconnect, set-union delivers both signed events. A sync-apply door that mirrors the enroll
checks as refusals **refuses signed history** — violating lossless custody
([sync §6.3/§6.5](../sync.md#63-failure-modes-designed-for), "never reject, never drop") — while
accepting both into today's `actor_current` (`DISTINCT ON (actor_id)`, latest wins) **recreates the
silent merge / attribution loss** the guards exist to prevent. No prior ADR specified the
resolution (the 2026-07-15 review, finding C4 / issue #205). Meanwhile the attestation and
suppression owner-gates trust only the **local** registry (issue #154), so a human's vouch is not
portable across nodes at all — and everything accountability-shaped now routes through
`actor_current`.

Two facts sharpen the design. First, the **threat model is benign-dominant**: the common conflict
producer is provisioning sloppiness or the locum-at-two-sites case; hostile key-reuse is the rare
tail. So the mechanism must make the benign case cheap to adjudicate while **never auto-resolving
either case** (the [ADR-0014](0014-locale-pluggable-matcher-comparators.md) posture: uncertainty may
only *withhold*; a hard conflict forces a human decision, never an auto-pick). Second, today's
`actor_event` rows are **not wire events at all** — no signature, no content address, no HLC, no
origin node — and `actor_current` orders by node-local `(recorded_at, seq)`, which cannot converge
across nodes *even without conflicts*. The wire shape is therefore a day-one, can't-retrofit
obligation of this ADR (the [ADR-0042](0042-concrete-attachment-reference-shape.md) pattern).

## Decision

**1. The actor event becomes a first-class signed wire event (day-one shape).**
COSE_Sign1 ([ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)) under a
**new dedicated signing context** for the actor plane
([ADR-0040](0040-signing-context-domain-separation.md) domain separation — registry bytes can never
replay as clinical or node events). **Signer = the enrolling node** (the ceremony authority — the
[ADR-0053](0053-per-write-human-authorship.md) `{node, recorded}` posture); the enrollee's public
key, pinned set, any human co-signature, and the [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md)
person-distinguishing determinant are *content*. **Content-addressed** (multihash of the signed
bytes — the wire dedupe/idempotence key). **HLC-stamped** (`t_recorded`, graded per
[ADR-0027](0027-trusted-time-anchoring.md)) with origin `node_id`. The cross-node winner order for
`actor_current` becomes **`(HLC, content_address)`** — deterministic, clock-collision-proof,
collation-independent ([ADR-0045](0045-collation-independent-projection-tiebreaks.md); this
winner-rule change is itself ADR-gated per [sync §6.1](../sync.md#61-mechanism), and this is that
ADR). Local `seq` survives only as the intra-node tiebreak for pre-wire local rows. The `actor_id`
derivation (content-address of the pinned set alone) is **unchanged**. **Pre-wire unsigned rows
never sync** — the apply door refuses unsigned registry history; dev/PoC rigs are wiped, never
synced through (the [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)/[ADR-0052](0052-born-sealed-clinical-bodies.md)
posture; we are pre-clinical, so this is stated discipline, not migration).

**2. Transport: the actor stream rides the node plane.**
Deny-all, trusted-peer admission ([ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)):
**registry trust is node trust** — a node-signed enrolment is inherited exactly as far as the peer
is trusted, and a peer that turns out bad is answered by the
[ADR-0018](0018-federation-revocation-cascade-and-the-anchor-as-power.md) revocation cascade. The
registry is small trust-plane state, so it replicates **fully within the trust neighborhood** —
which is what makes a human's vouch portable. Per-peer seq cursor and the existing node-plane
quarantine pen (re-offer floor, `acked` human exclusion — `db/022`'s contract) are reused verbatim
for unverifiable bytes.

**3. The apply door admits, then detects — never refuses verifiable history.**
`apply_remote_actor_event`: verify signature + signing context (unverifiable → pen) → origin
admission (untrusted → skip-and-sweep) → content-address dedupe (idempotent) → **INSERT
unconditionally** (custody is total) → re-evaluate the **derived dispute state** (point 4) and
raise a salience-routed worklist item ([identity §5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor))
on a newly-disputed `actor_id`/key — at *both* implicated nodes, never a modal (principle 3). The
local `enroll_actor` door keeps its fail-closed whole-history stance **unchanged**; loud-at-creation
remains the first line. Admission ≠ endorsement: safety lives in the projection.

**4. Disputed is derived, never declared — and computed over live bindings.**
An `actor_id`/key is *disputed* iff the conflict predicates detect a violation among **live**
(non-superseded, non-revoked) bindings in the local log: ≥ 2 live keys under one `actor_id`, or
≥ 2 live `actor_id`s under one key. No event creates or clears the dispute; adjudication changes
the *log* the state derives from ([ADR-0010](0010-additive-vs-suppressing-classification.md)
derived-not-declared). Consequence: **log convergence ⇒ state convergence** — no dispute flag to
race, no "who noticed first." Projection surface: `actor_current` stops picking winners for a
disputed `actor_id`; a sibling projection (`actor_registry_state`) carries
`clean | disputed(candidate_set) | revoked` per `actor_id` and per key, enumerating the conflicting
rows. **The predicate split is deliberate and load-bearing:** the *derived state* uses live
bindings so adjudication clears it by construction; the *enroll-door* predicates stay whole-history
(anti-resurrection, fail-closed). Without the split, the A-case repair deadlocks — re-enrolling a
key under its proper successor would trip the whole-history B-check; `supersede` is the sanctioned
repair path the doors recognize.

**5. Content never waits; permissions always wait.**
No strict cross-plane ordering is promised (registry and clinical streams have independent
cursors; promising more would lie at the first partial sync). Instead, at the consuming doors:
*(a)* a clinical event citing a key the local registry doesn't know yet, or one in dispute,
**applies normally** — availability over consistency — with attribution honestly degraded: unknown
key → "key not yet resolved," disputed key → the **candidate set** (*"one of Dr X / Dr Y — registry
dispute pending"*), a first-class [identity §5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)
authorship state (db/005's `NULL`-on-ambiguity survives only as the fallback the candidate set
cannot express). *(b)* An operation where the registry **grants authority** — a suppression's
owner-gate, an attestation's authority check — is **penned** (delayed-never-lost, re-offered) until
the cited registry state arrives and is clean; disputed = not clean = withheld. Fail-safe by
direction: a held suppression keeps the note *visible*; a held attestation reads *un-vouched* —
never the reverse. A hostile fabricated enrolment therefore buys a worklist entry and **zero
authority**. This rule — *registry uncertainty may withhold a permission, never withhold content* —
is principle 4 operationalized, and it closes issue #154 structurally. Attribution re-derives
retroactively and losslessly the moment the registry catches up or the dispute resolves; the
dispute window stays auditable in the log forever. (In the A-direction, per-event attribution is
unambiguous *by signing key* the whole time — only the registry identity of each key is disputed —
so the candidate set is temporary by construction; genuine permanent ambiguity exists only in the
stolen-key tail, resolved by `revoke` + the [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md)
§6 contamination cascade, unchanged.)

**6. Adjudication is `supersede` by audited human ceremony — no new verbs.**
- **B-case (join):** two `supersede` events, each old `actor_id` → one successor (corrected pinned
  set, proper determinants). A join expressed as links — never a merge; both lineages survive.
- **A-case (per-key fork):** the degenerate binding is superseded per key; each person gets a
  successor `actor_id` (distinguishing determinants added) binding their key. Attribution then
  re-derives exactly, per signing key.
- **Hostile tail:** `revoke` with compromise-time + contamination cascade.

The ceremony is audited with a **mandatory recorded human adjudicator** (the
[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) §4 backstop); *who* may
adjudicate is policy (principle 9). Adjudication events sync as ordinary actor events. If two nodes
adjudicate the same dispute **differently** during another partition, the live-binding derivation
sees conflicting supersedes and the state honestly reads **disputed again**, worklist escalated —
never auto-picked. Rare, honest, convergent.

**7. Considered and rejected.**
- **Pen-outside + adjudicated admission** (mirror the checks as refusals into a quarantine pen):
  registries *diverge* until adjudication — each node holds its own enrolment and pens the other's —
  so the conflict has no convergent representation; the adjudication event must cite bytes outside
  the log; candidate-set attribution cannot be computed legibly from a pen row; and the pen was
  built for *unverifiable* bytes, not unwelcome verifiable history — stretching it there is exactly
  the "reject signed history" the sync contract forbids.
- **Deterministic tiebreak + auto-supersede** (HLC/content-address picks a winner): an **automated
  identity resolution** — the move principle 2 and [ADR-0014](0014-locale-pluggable-matcher-comparators.md)
  forbid. A tiebreak that silently hands Dr X's authorship to Dr Y because of a clock value is the
  failure mode this project exists to refuse.

**Canonical homes:** [security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)
(registry federation + dispute + adjudication), [sync §6.9](../sync.md#69-the-actor-registry-stream)
(the actor stream + ordering contract), [data-model §3.12](../data-model.md#312-actor-identity-in-the-registry)
(wire shape + convergent ordering), [identity §5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)
(candidate-set authorship state).

## Consequences

- **Easier:** the C4 contradiction dissolves — custody is total (both signed events, every trusted
  node, byte-for-byte), "never reject" holds, and the silent merge is unreachable because no
  projection picks a winner under dispute. A human's enrolment/vouch becomes portable (closes #154
  structurally). The sync-door half of the #172 mirror obligation is discharged *by specification*
  (the door runs detection, not refusal — deliberately). Log convergence ⇒ dispute-state
  convergence, for free.
- **Harder:** `actor_current` and every accountability-shaped consumer must learn the disputed
  state — a real projection blast radius. The "≤ 1 key per `actor_id`" invariant becomes a
  steady-state *projection* claim, not a table invariant. A second predicate family (live-bindings)
  joins the whole-history one; the split must stay legible or a future door will call the wrong one.
- **Operational:** pre-wire unsigned `actor_event` rows never sync — wipe dev/PoC rigs before
  federating them. The `rotate-key`/`supersede` *local* door obligation (#172's other half) is
  restated unchanged: it must mirror **both whole-history checks**. The human key-loss recovery
  ceremony remains an open gap needing its own ADR before any pilot enrolls real humans
  ([security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody),
  issue #94); nothing here forecloses it.
- **Blast radius (§9):** the wire shape, apply door, dispute derivation, and adjudication ceremony
  are **safety-critical** (in-DB/Rust, reviewer-legible, TDD). The dispute derivation is a pure
  function of the log — property-test that **admission order never changes it** (set-union
  commutativity + idempotence over arbitrary interleavings), plus multi-node convergence
  (partition → dual enrol → reconnect → both disputed → adjudicate once → both clean).
- **The bet:** that a derived, live-bindings disputed state plus supersede-only repair absorbs
  every registry conflict a partition can mint — with no new verbs and no auto-resolution. We would
  know it is wrong if a real conflict class cannot be expressed as live-binding supersedes, if the
  dispute state flaps pathologically under normal provisioning, or if withheld-while-disputed
  permissions block a clinically-urgent workflow (which would indicate the permission, not the
  dispute, is mis-scoped).
- **No new founding principle.** This is principles 1, 2, and 4 applied to the actor registry:
  append-only custody, never-merge/always-link, and acknowledged uncertainty — now at the trust
  anchor itself.
````

- [ ] **Step 2: Verify the file renders and links resolve**

Run: `cd /Users/hherb/src/cairn-ehr && uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -5`
Expected: build completes; no new warnings referencing `0054-` (mkdocs warns on broken internal links).

- [ ] **Step 3: Commit**

```bash
git add docs/spec/decisions/0054-actor-registry-federation-admit-and-dispute.md
git commit -m "docs(#205): ADR-0054 — actor-registry federation: admit-and-dispute

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: security.md §7.5 — federation prose + mirror-obligation correction

**Files:**
- Modify: `docs/spec/security.md` (§7.5, lines ~77–83)

**Interfaces:**
- Consumes: ADR-0054 file from Task 1 (link target `decisions/0054-actor-registry-federation-admit-and-dispute.md`).

- [ ] **Step 1: Correct the mirror-obligation sentence in the §7.5 second bullet**

In the bullet beginning `**Immutable, version-pinned identity over a closed actor-event algebra**`,
replace exactly:

> Both future doors that bind a key to an actor (`rotate-key`/`supersede`, actor-event sync apply) must mirror both checks.

with:

> The future `rotate-key`/`supersede` door must mirror both whole-history checks; the **actor-event sync-apply door deliberately does not refuse** — it admits signed history and renders a conflict as a first-class **disputed** state instead ([ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md), federation bullet below).

- [ ] **Step 2: Insert a new federation bullet after the "Skill-epoch and served-model digest" bullet (line ~82), before the "Honest gap" bullet**

```markdown
- **Registry federation is admit-and-dispute** ([ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md)). On the wire an actor event is a first-class signed event ([data-model §3.12](data-model.md#312-actor-identity-in-the-registry)) riding the node plane under deny-all peer admission ([sync §6.9](sync.md#69-the-actor-registry-stream)) — **registry trust is node trust**, recoverable by the [ADR-0018](decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md) cascade. The sync-apply door **admits unconditionally and detects**: a conflict two partitioned nodes minted locally-validly (one `actor_id`/two keys, or one key/two `actor_id`s) becomes a **derived, first-class disputed state** — computed over **live** (non-superseded, non-revoked) bindings, so log convergence ⇒ state convergence — never a refusal, never an auto-picked winner. While disputed: `actor_current` picks no winner, implicated events attribute to the honest **candidate set** ([identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side)), and **registry uncertainty withholds permissions, never content** — attestation/suppression authority waits; the med list keeps working. Adjudication is the existing `supersede` by audited human ceremony (join for the same-person case, per-key fork for the two-person case, `revoke` + contamination cascade for the hostile tail) — a mandatory recorded human adjudicator, *who* may adjudicate is policy. Conflicting concurrent adjudications honestly re-derive as disputed, never auto-picked. The enroll door's fail-closed whole-history stance is unchanged — loud-at-creation stays the first line.
```

- [ ] **Step 3: Build + grep verification**

Run: `grep -c "0054-actor-registry-federation" docs/spec/security.md`
Expected: `2`
Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/security.md
git commit -m "docs(#205): security §7.5 — registry federation admit-and-dispute prose (ADR-0054)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: data-model.md §3.12 — the wire shape and convergent ordering

**Files:**
- Modify: `docs/spec/data-model.md` (§3.12, after the final bullet at line ~281)

- [ ] **Step 1: Append this bullet to §3.12 (after the "Signing publics are immortal; DEKs are destroyable" bullet, before the `## 3.13` heading)**

```markdown
- **On the wire, an actor event is a first-class signed event** ([ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md)): COSE_Sign1 under a **dedicated actor-plane signing context** ([ADR-0040](decisions/0040-signing-context-domain-separation.md) — registry bytes never replay as clinical or node events), **signed by the enrolling node** (ceremony authority; the enrollee's key, pinned set, and person-distinguishing determinant are content), **content-addressed** (the wire dedupe key), and **HLC-stamped with origin node**. The cross-node registry winner order is **`(HLC, content_address)`** — deterministic and collation-independent ([ADR-0045](decisions/0045-collation-independent-projection-tiebreaks.md)); the `actor_id` derivation (content-address of the pinned set alone) is unchanged. Pre-wire unsigned registry rows **never sync**; concurrent-conflict semantics are admit-and-dispute ([security §7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)).
```

- [ ] **Step 2: Build + grep verification**

Run: `grep -c "0054-actor-registry-federation" docs/spec/data-model.md`
Expected: `1`
Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/data-model.md
git commit -m "docs(#205): data-model §3.12 — signed actor-event wire shape + convergent ordering (ADR-0054)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: sync.md — §6.3 failure-mode row + new §6.9 actor-registry stream

**Files:**
- Modify: `docs/spec/sync.md` (§6.3 table, line ~35; new §6.9 after §6.8)

- [ ] **Step 1: Add a row to the §6.3 failure-mode table, after the "Hostile-but-credentialed peer" row**

```markdown
| Two partitioned nodes each locally-validly enrolled conflicting actor-registry state (one `actor_id`/two keys, or one key/two `actor_id`s) | **Admit-and-dispute.** The actor-stream apply door admits both signed events (custody total, never a refusal of verifiable history) and the conflict surfaces as a **derived disputed state** — no winner picked, permissions withheld, content still flowing with candidate-set attribution — until a human adjudication ceremony (`supersede`) resolves it. Never auto-resolved. See [§6.9](#69-the-actor-registry-stream), [ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md) |
```

- [ ] **Step 2: Append the new §6.9 section at the end of the file (after §6.8)**

```markdown
## 6.9 The actor-registry stream
> Resolves the 2026-07-15 review finding C4 (issue #205) — see [ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md). Registry design: [security §7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody); wire shape: [data-model §3.12](data-model.md#312-actor-identity-in-the-registry).

Actor-registry events (enroll / supersede / revoke — signed, content-addressed, HLC-stamped,
[data-model §3.12](data-model.md#312-actor-identity-in-the-registry)) travel as a **distinct stream
on the node plane**: deny-all trusted-peer admission ([security §7.7](security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract)
— **registry trust is node trust**), **full replication within the trust neighborhood** (the
registry is small trust-plane state; full replication is what makes an enrolment or vouch portable),
per-peer cursor, and the node plane's quarantine pen + re-offer floor ([§6.3](#63-failure-modes-designed-for))
for unverifiable bytes. Pre-wire unsigned registry rows never sync.

**The ordering contract is honest, not strict.** The registry and clinical streams have independent
cursors; no cross-plane ordering is promised. Instead, two rules at the consuming doors
([ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md)):

- **Content never waits.** A clinical event citing a key the local registry does not know yet (or
  one in dispute) applies normally — availability over consistency — with attribution honestly
  degraded ("key not yet resolved" / the candidate set, [identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side));
  it re-derives losslessly when the registry catches up.
- **Permissions always wait.** An operation where the registry *grants* authority (a suppression's
  owner-gate, an attestation's authority check) is penned — delayed, never lost, re-offered — until
  the cited registry state arrives and is **clean**; disputed is not clean. Fail-safe by direction:
  a held suppression keeps the note visible; a held attestation reads un-vouched — never the
  reverse. **Registry uncertainty may withhold a permission, never withhold content.**
```

- [ ] **Step 3: Build + grep verification**

Run: `grep -c "0054-actor-registry-federation" docs/spec/sync.md`
Expected: `4`
Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/sync.md
git commit -m "docs(#205): sync §6.3/§6.9 — the actor-registry stream + honest ordering contract (ADR-0054)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: identity.md §5.10 — the candidate-set authorship state

**Files:**
- Modify: `docs/spec/identity.md` (§5.10, layer 2 "Projected trust signal", line ~131–137)

- [ ] **Step 1: Extend layer 2**

In item 2 (**Projected trust signal**), after the sentence ending
`— overlaid, never erased.` (the recall-marker sentence), append to the same paragraph:

```markdown
The same projection carries the **registry-dispute state** ([ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md)): an event signed by a key implicated in an unresolved actor-registry conflict attributes to the honest **candidate set** (*"one of Dr X / Dr Y — registry dispute pending"*) — acknowledged uncertainty, distinct from *unknown* — and re-derives to the exact author when the dispute is adjudicated; while disputed, registry-granted permissions are withheld but content keeps flowing ([sync §6.9](sync.md#69-the-actor-registry-stream)).
```

- [ ] **Step 2: Build + grep verification**

Run: `grep -c "0054-actor-registry-federation" docs/spec/identity.md`
Expected: `1`
Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/identity.md
git commit -m "docs(#205): identity §5.10 — candidate-set authorship state under registry dispute (ADR-0054)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Spec version bump + full docs verification

**Files:**
- Modify: `docs/spec/index.md` (line 9)

- [ ] **Step 1: Bump the spec version**

Replace exactly:
> **Spec version:** 0.55

with:
> **Spec version:** 0.56

- [ ] **Step 2: Full clean docs build**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -10`
Expected: `Documentation built` with no warnings mentioning any file touched by Tasks 1–5.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/index.md
git commit -m "docs: spec v0.56 — ADR-0054 actor-registry federation

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: Session close — HANDOVER/ROADMAP, push, PR

**Files:**
- Modify: `docs/HANDOVER.md` (NEXT header, session paragraph, ADR index table)
- Modify: `docs/ROADMAP.md` (new Slice 46 under Phase 4's slice list tail, after Slice 45)

- [ ] **Step 1: HANDOVER updates**

1. ADR index table: append after the 0053 row:
```markdown
| [0054](spec/decisions/0054-actor-registry-federation-admit-and-dispute.md) | Actor-registry federation is admit-and-dispute: signed actor-event wire shape on the node plane; derived live-bindings disputed state; content never waits, permissions always wait; adjudication = supersede by human ceremony, never auto-resolved | §7.5/§6.9/§3.12/§5.10 (refines 0011/0044/0046) |
```
2. NEXT header (line 3): change the trailing directive from "Next: a Priority-6 design session
   (start with #205) — or the feature work below, now unblocked" to "Next: continue the Priority-6
   design queue (#205 ✅ done → ADR-0054; #206/#200/#208/#216/#217 remain) — or the feature work
   below, now unblocked".
3. In the "Priority 6 — design sessions" list, mark the #205 entry done:
   `- **#205 (C4) ✅ 2026-07-19** — resolved by [ADR-0054](spec/decisions/0054-actor-registry-federation-admit-and-dispute.md) (admit-and-dispute; spec v0.56); code slices are future feature work.`
4. Prepend a new session paragraph (above the current "Session (2026-07-19, latest)" one, which
   becomes a condensed prior entry) summarizing: the #205 design session → ADR-0054 + spec v0.56
   (four spec homes), the design/plan docs under docs/superpowers/, HANDOVER P5-merge currency fix.
   Update the "Session date" line to name this as the latest session. Keep the file under ~410 lines
   by condensing where needed.
5. Update the "Spec/ADRs:" status line from `v0.55 (through ADR-0053...)` to `v0.56 (through
   ADR-0054)`.

- [ ] **Step 2: ROADMAP Slice 46**

After the Slice 45 block (ends "...next is the Priority-6 design queue (#205 first) or the
now-unblocked feature work.**"), append:

```markdown
**Slice 46 — the #205 design session: ADR-0054 actor-registry federation (2026-07-19; P6 design
queue, first item; spec v0.56; docs-only — code slices are future feature work).** The C4
contradiction (fail-closed enrolment vs never-reject set-union custody) is resolved as
**admit-and-dispute**: the actor event becomes a first-class signed wire event (node-signed COSE
under a dedicated context, content-addressed, HLC+origin; `(HLC, content_address)` winner order;
pre-wire rows never sync) riding the **node plane** (deny-all peers, full replication in the trust
neighborhood, db/022 pen); the apply door **admits unconditionally and detects** — the conflict
becomes a **derived disputed state over live bindings** (log convergence ⇒ state convergence, no
dispute events), under which `actor_current` picks no winner, implicated events attribute to the
honest **candidate set**, and **registry uncertainty withholds permissions, never content** (closes
#154 structurally; the #172 sync-door half discharged by specification). Adjudication = the
existing `supersede` by audited human ceremony (join / per-key fork / revoke+cascade), with
conflicting adjudications honestly re-deriving as disputed. Considered-and-rejected: pen-outside
(registries diverge), deterministic tiebreak (automated identity resolution — principle 2's
forbidden move). Spec homes: security §7.5, sync §6.3+§6.9, data-model §3.12, identity §5.10.
Design/plan docs under `docs/superpowers/`.
```

- [ ] **Step 3: Final verification, commit, push, PR**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — Slice 46, ADR-0054 session close

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push -u origin design/205-actor-registry-federation
gh pr create --title "ADR-0054: actor-registry federation is admit-and-dispute (closes #205)" --body "$(cat <<'EOF'
## Summary
Design-session output for issue #205 (2026-07-15 review finding C4): **ADR-0054** resolves the
contradiction between fail-closed enrolment (ADR-0044/0046) and never-reject set-union custody as
**admit-and-dispute** — the sync-apply door admits conflicting signed enrolments and renders them
as a derived, live-bindings **disputed state** (never refused, never auto-resolved), with a signed
actor-event wire shape on the node plane, an honest ordering contract (*content never waits,
permissions always wait* — closes #154 structurally), and adjudication via the existing
`supersede` by audited human ceremony. Spec v0.55 → **v0.56**; prose landed in security §7.5,
sync §6.3+§6.9, data-model §3.12, identity §5.10.

Also: HANDOVER currency (P5 merged as #253+#255), ROADMAP Slice 46, and the session's
design/plan docs under `docs/superpowers/`.

Closes #205.

## Review notes
- ADRs are immutable post-merge — review the ADR text itself hardest.
- Docs-only: no code, no schema, no wire change. The code slices (wire shape, apply door, dispute
  projection, adjudication ceremony) are future feature work.
- `mkdocs build` clean via the pinned `docs/requirements.txt`.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review (completed at write time)

1. **Spec coverage:** design §3.1 wire shape → Task 1 (Decision 1) + Task 3; §3.2 transport/door →
   Task 1 (Decisions 2–3) + Task 4; §3.3 disputed state → Task 1 (Decisions 4–5) + Tasks 2/5;
   §3.4 adjudication → Task 1 (Decision 6) + Task 2; design §2 settled decisions → ADR Context;
   design §4 closes/names table → ADR Consequences + PR body; design §5 testing → ADR blast-radius
   bullet (code tests are future work by design); design §6 deliverables → Tasks 1–6. No gaps.
2. **Placeholder scan:** none — every step carries its full text. (The one flagged typo-guard in
   Task 1 Step 1 is an instruction, not a placeholder.)
3. **Type consistency:** link slugs `0054-actor-registry-federation-admit-and-dispute.md` and the
   §6.9 anchor `#69-the-actor-registry-stream` are used identically across Tasks 1–5; grep counts
   in each task verify them mechanically.
