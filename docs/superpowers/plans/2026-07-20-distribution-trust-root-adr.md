# ADR-0055 Distribution-Plane Trust-Root Governance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write ADR-0055 (distribution-plane trust-root governance: channels, the chained root
document, the root/release role split) and land its prose in the spec homes, closing issue #206.

**Architecture:** Docs-only session. The approved design is
`docs/superpowers/specs/2026-07-20-distribution-trust-root-design.md`; this plan turns it into the
immutable ADR plus additive spec updates and files the named follow-on issues. Code slices (root-doc
verifier, load gate, bundle format, log role) are explicitly NOT in this plan — no §7.6 code exists
today; they become ordinary TDD feature work when the distribution plane is built.

**Tech Stack:** Markdown; mkdocs (pinned via `docs/requirements.txt`); git; `gh` CLI.

## Global Constraints

- Branch: `design/206-distribution-trust-root` (already exists, has the design-doc commit).
- ADRs are immutable once merged; get the text right now. Status `Accepted`, date `2026-07-20`.
- Docs build MUST use the pinned requirements: `uv run --with-requirements docs/requirements.txt -- mkdocs build` from the repo root (never ad-hoc installs — security note in `docs/requirements.txt`).
- Spec files carry NO in-file changelogs; the version lives only in `docs/spec/index.md` (bump 0.56 → 0.57).
- Callouts use GitHub/Obsidian syntax (`> [!WARNING]`).
- Link style: relative markdown links exactly as neighboring prose does (`[ADR-0017](decisions/0017-….md)` from a spec file; `[ADR-0017](0017-….md)` from another ADR).
- mkdocs nav gets the ADR entry in the SAME task as the ADR file (the #205 session's plan gap — nav was caught late by the build; don't repeat it).
- Every commit message ends with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: Write ADR-0055 + mkdocs nav entry

**Files:**
- Create: `docs/spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md`
- Modify: `mkdocs.yml:161` (append nav line after the ADR-0054 entry)

**Interfaces:**
- Produces: the ADR file that Tasks 2–4 link to as
  `decisions/0055-distribution-trust-root-governance-chained-root-document.md` (from spec files).

- [ ] **Step 1: Create the ADR file with exactly this content**

````markdown
# ADR-0055 — Distribution-plane trust-root governance: channels, the chained root document, and the root/release role split

- **Status:** Accepted (refines [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) §4's distribution plane and [ADR-0024](0024-hard-policy-expression-the-policy-assertion-stream.md)'s authority bootstrap; applies [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)/[ADR-0018](0018-federation-revocation-cascade-and-the-anchor-as-power.md) anchor doctrine to the steward key; reuses [ADR-0027](0027-trusted-time-anchoring.md)'s transparency-log shape and [ADR-0026](0026-node-durability-and-disaster-recovery.md)'s escrow rungs; completes the plane contrast opened by [ADR-0054](0054-actor-registry-federation-admit-and-dispute.md))
- **Date:** 2026-07-20

## Context

[ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) §4 signs code — the
**highest-blast-radius artifact in the system**: native extensions loaded into the database trusted
base — against **a single steward key**, with no threshold signing, no rotation, no compromise
recovery, and no succession story. Meanwhile mere timestamps got threshold multi-anchor treatment
([ADR-0027](0027-trusted-time-anchoring.md)) and federation credentials got explicit no-single-root
doctrine ([ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)/[ADR-0018](0018-federation-revocation-cascade-and-the-anchor-as-power.md):
*"never mandate a single anchor — that builds the kill-switch"*; *"a trust anchor is a position of
power"*). [Security §7.9](../security.md#79-hard-policy-expression-projection-and-enforcement)
notes the policy-authority root bootstraps "the same as the steward key," inheriting the gap.

A steward-key compromise is **signed remote-code-execution into every upgrading node**; steward
capture is the mission's own named adversary. The corpus applies anti-capture everywhere except the
most powerful key it defines. No ADR owned this. (The 2026-07-15 review, finding C5 / issue #206.)

## Decision

**1. No privileged root on the distribution plane: channels, with the steward as default anchor.**
"The steward key" is not a system-wide root; it is the **default configured anchor of the official
release channel**. A **channel** is the unit of distribution trust: `{trust-root chain,
transparency log, release stream}`. The distribution trust root is **node provisioning
configuration** on the [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
anchor spectrum: official builds ship the steward set as the default; a national deployment runs
its own channel (rebuilding from source, or re-signing official bundles after its own review); a
solo practice uses the default and never sees any of this. The channel operator is a **node role**
— fractal topology applied to distribution; *no Cairn-owned root* now holds on this plane too.
This completes the plane contrast [ADR-0054](0054-actor-registry-federation-admit-and-dispute.md)
opened: the content plane **admits-and-disputes** (content never waits); the code plane
**verifies-or-refuses** (code always waits). A refused bundle is a legible, logged refusal that
never touches running code — refusal to upgrade can never brick a clinic (the §7.6 blue-green
line, unchanged).

**2. The chained trust-root document.** Per channel, a content-addressed, self-contained document:

```
trust_root = {
  channel_id,
  version,                 -- monotonic per channel
  root_signers:    [{kid, pubkey, custody_note}], root_threshold M_root,
  release_signers: [{kid, pubkey}],               release_threshold M_rel,   -- default 1
  prev_root_hash           -- content address of version N-1 (absent on genesis)
}
```

Verifiable with **zero database and zero network** — a bare installer or a phone can verify it
(the registry-as-DB-state alternative fails exactly there; see point 10). Version 1 is the
self-signed genesis distributed at provisioning; every version N+1 must carry **≥ M_root
signatures from version N's `root_signers`** (chain-of-custody rotation, the TUF root-role shape,
as explicit multi-signature — which signers signed is public evidence, and **N=1/M=1 is
first-class**: a solo practice and the pre-entity steward are valid single-signer roots; the
threshold is an honest ratchet, never fake multi-party ceremony). A node pins
`(channel, highest verified version)` and refuses regression — the schema-generation guard pattern
applied to trust metadata. **Roots do not expire** — a stated divergence from TUF: an expired root
bricking an offline clinic violates the availability floor. Freshness is a
[§7.9](../security.md#79-hard-policy-expression-projection-and-enforcement) policy rung (a
deployment MAY gate *upgrades* on log-checkpoint age); staleness is surfaced honestly; the cost of
the divergence is real and named in Consequences (immortal retired keys).

**3. The root/release role split.** The **root role** signs only root-document versions — rare,
high-ceremony, offline keys: the constitution. The **release role** signs day-to-day release
manifests — the daily pen, default `M_rel = 1`, cheaply revoked or rotated by publishing root
version N+1 *without touching root-key custody*. Without the split, M humans with air-gapped keys
sign every 2am security patch — and security that is expensive gets bypassed (batched releases,
delayed patches, a threshold quietly kept at 1 "for practicality"). The split benefits the 1-of-1
era most: even a solo steward keeps the constitution key in the safe and the daily key on the
bench, so a release-key compromise is recoverable by one signed rotation instead of a fleet
re-provisioning.

**4. Every lifecycle event is "publish version N+1"; the catastrophic case is declared, not
solved.** Routine rotation, signer add/remove, threshold change, emergency exclusion of a
compromised key — all the same operation, recorded in the transparency log (the log entry *is* the
evidence). **Succession is a rotation like any other**: founder → stewarding foundation is
entity-neutral mechanism, so the parked legal-entity question stays parked; the governance doc
binds whoever the holders are. **Retirement includes key destruction**: because roots never
expire, a retired root key remains forever able to sign a fork at its historical version;
retirement is therefore a ceremony that destroys the key material, and the log/gossip cross-check
(point 7) is the standing fork detector. If ≥ M_root keys are compromised (an attacker can rotate)
or fewer than M_root survive (the holders cannot), the chain is dead — recovery is the
out-of-band audited **re-provisioning ceremony** (the §7.6/§7.7 ceremony vocabulary), establishing
a new genesis, with the discontinuity recorded in the log. This is the
[ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) posture — best-effort and declared,
never guaranteed — and at fleet scale it is a product recall, which is exactly why point 8's
ratchet is a commitment with a trigger, not an aspiration.

**5. Fork conflicts are surfaced, never resolved by arrival order.** A node advances its pin only
when **exactly one verifiable successor** exists. If it ever observes two distinct verifiable
successors of the same version — at upgrade time, or later via log/gossip cross-check after a
restore-from-backup ([ADR-0026](0026-node-durability-and-disaster-recovery.md)) — it **freezes the
pin**, surfaces a **security incident** on the honest-assembly status surface, and resolves only
via the audited ceremony. Freezing blocks *upgrades* on that channel; running code, local reads,
and writes are untouched. Without this rule, regression refusal is first-write-wins — and works
*for* an attacker who reaches a stale node first after an emergency rotation.

**6. The load gate verifies-or-refuses; refusals are legible.** The release bundle is
self-contained — `{manifest (channel_id, monotonic release_version, artifact digests per
architecture × Postgres-major + DDL + rebuild recipe, root_version_ref), ≥ M_rel release
signatures, the root-chain segment the receiver may be missing, inclusion proof}` — verifiable
with zero network: the sneakernet path is first-class, not degraded. Gate order (the one
safety-critical seam, [§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius)
Rust/in-DB, reviewer-legible): verify root chain from the pin → verify release signatures → verify
every artifact digest → only then load. Release signatures verify against **the release set of the
newest verified root version** — `root_version_ref` is diagnostic, never authoritative: a manifest
can never elect an older root's release set, so a rotated-out release key is dead the moment the
node sees the rotation. Never partial-load; release versions monotonic per channel. **Refusals
speak practice-manager language** — *what is pinned, what this bundle needs, what is missing, what
to do next* — never "signature verification failed." Provisioning config may require **K
additional named co-signers** before load (a hospital security team, a national authority, an
independent rebuilder); [§7.9](../security.md#79-hard-policy-expression-projection-and-enforcement)
policy may ratchet *stricter*, never weaker (soft-policy guidance: co-signing trades patch latency
for review — define the emergency path *in advance*, because the real-world alternative is "skip
the check just this once"). `status` reports channel, pinned root version, release version, log
freshness, and any frozen-pin incident — beside sync freshness and backup health.

**7. Transparency is the evidence plane; its limits are stated.** A channel MAY — **the official
channel MUST** — log every release manifest and every root-document version in an append-only,
content-addressed, Merkle-checkpointed **transparency log**: the
[ADR-0027](0027-trusted-time-anchoring.md) shape reused, a **self-hostable node role**, mirrorable
and sneakernet-distributable. Bundles carry inclusion proofs; verification is offline-capable;
peers **gossip log checkpoints and root pins** on the existing plane — which is what makes a
*targeted* poisoned build (served to one victim clinic) and a forked root chain **detectable
evidence** rather than silent success. **Rebuilder attestations slot in for free**: an independent
reproducible-build attestation is just another signature on the manifest — the co-signer mechanism
reused. Named limits, honestly: **genesis TOFU stays procedural** (the chain protects every step
*after* trust establishment; a poisoned installer carries a poisoned verifier that validates its
own poisoned genesis — initial provisioning integrity is ceremony + second-channel fingerprint,
not cryptography); **a log without independent monitors is weak** (the early fleet has one log and
one monitor; adversarial value matures with witness diversity — until then reproducible builds
carry the near-term weight); **the log operator is a small power position** (refusing to log is a
publish-DoS; mitigated by mirrorability, multi-log additive later); **reproducible builds are
load-bearing and genuinely hard** (pgrx × architectures × Postgres-majors × SDK drift — until a CI
check exists and independent rebuilders byte-match, "verify by rebuilding" is aspirational, a
named follow-on).

**8. Honest current posture, and the ratchet tripwire.** The official channel today is **N=1,
M_root=1** (the founder), **declared in the root document itself** —
[principle 4](../index.md#founding-principles-the-lens-for-every-decision) applied to governance:
declare 1-of-1 honestly rather than perform fake multi-party ceremony. Until the signer set grows,
the operative protections are the role split, transparency, and reproducible builds — stated
plainly. **Governance commitment with a named trigger:** *before the first production deployment
operated outside the steward's own control, the official channel's root role MUST be ≥ 2-of-3.*
Custody floor for the 1-of-1 era: the
[ADR-0026](0026-node-durability-and-disaster-recovery.md) escrow rungs — offline root key plus an
escrowed recovery path (Shamir M-of-N / QR-in-the-safe); the same rung is the recommended default
for practice issuing keys ("print the recovery QR, put it in the drug safe").

**9. One root shape, three roots.** The root-document mechanism, ceremony vocabulary, and fork
rule serve **all three provisioning-time roots**: the
[§7.6](../security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)
distribution channel (full shape, both roles); the
[§7.9](../security.md#79-hard-policy-expression-projection-and-enforcement) policy-authority
bootstrap (root role only, naming the initial policy-authority set); the
[§7.7](../security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract)
practice issuing key (1-of-1 default with the escrow rung, upgradeable in place as the practice
grows). One verifier for all three concentrates blast radius — stated plainly, and accepted: one
well-reviewed, reviewer-legible verifier beats three ad-hoc ones (the §9.1 small-trusted-surface
argument working as intended).

**10. Considered and rejected.**
- **FROST aggregate signature for releases** — the verifier stays single-sig, but participation is
  invisible (anti-audit for a governance key), the signing ceremony is interactive, and share
  rotation is opaque to the fleet. FROST stays earmarked for the high-volume notary plane
  ([ADR-0027](0027-trusted-time-anchoring.md)) it was named for.
- **Roots as [§7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)
  actors** — maximal corpus reuse, but bootstrap circularity: the actor registry is database state
  installed *by* the artifact being verified; the load gate must verify with zero DB. (Steward
  identities MAY additionally be mirrored as actors for in-record attribution.)
- **Local-config-only key set (no root document)** — no self-describing rotation; every rotation
  is a fleet-wide manual ceremony; fails the finding's rotation/succession requirement.
- **Steward floor with additive-only co-signers** — mandates a single anchor: the named
  kill-switch footgun; makes steward capture unrecoverable except by forking.
- **Pure policy, no shipped default** — hostile to the solo-practice bootstrap; the default path
  must just work (paper-parity).
- **Full TUF freshness roles (snapshot/timestamp)** — expiring metadata bricks offline clinics
  (availability floor); freshness becomes a policy rung instead, and the local downgrade guard
  (the schema-generation pattern) already refuses regression.

**Canonical homes:** [security §7.6](../security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)
(channels, root document, load gate, transparency), [security §7.9](../security.md#79-hard-policy-expression-projection-and-enforcement)
(authority bootstrap), [security §7.7](../security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract)
(practice issuing key), [sync §6.5](../sync.md#65-schema-evolution-two-planes-and-lossless-forwarding)
(distribution-plane pointer).

## Consequences

- **Easier:** finding C5 closes by **reuse** — the [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
  anchor spectrum, the [ADR-0027](0027-trusted-time-anchoring.md) log shape, the
  [ADR-0026](0026-node-durability-and-disaster-recovery.md) escrow rungs, the ceremony vocabulary,
  the monotonic-guard pattern. The steward stops being a structural exception to the corpus's own
  doctrine: capture is answered by threshold + evidence + a fleet that can walk away.
- **Harder / new trusted surface:** the root-chain + multi-signature verifier and the load gate
  (small, reviewer-legible, [§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius));
  the signing ceremonies (procedural discipline); the log role (fit-for-purpose server software).
- **Honest weaknesses carried:** no-expiry ⇒ immortal retired keys (compensated by the destruction
  ceremony + standing fork detection); genesis TOFU (procedural); early-fleet log-monitor scarcity
  (reproducible builds carry the weight); N=1 today (declared, ratcheted, escrowed).
- **Operational:** no §7.6 code exists yet — the verifier, load gate, bundle format, and log role
  are future TDD feature work; the root-document and bundle shapes above are the day-one,
  can't-retrofit commitments (the [ADR-0042](0042-concrete-attachment-reference-shape.md) pattern).
- **The bet:** that a chained, non-expiring, threshold-capable root document plus transparency
  gives every deployment tier — solo practice to nation — a rotation, succession, and
  compromise-recovery story without a privileged root or a mandatory online authority. We would
  know it is wrong if real deployments cannot operate the ceremonies (rotations skipped, keys
  unescrowed), if the no-expiry fork window is exploited faster than log/gossip detection catches
  it, or if channel pluralism fragments the fleet into incompatible release lines (which would
  indicate the *format*, not the trust root, needs governance).
- **No new founding principle.** This is principles 4, 7, and 9 applied to the steward itself:
  acknowledged uncertainty (declared 1-of-1, honest staleness), anti-capture (no privileged root,
  walk-away pluggability), and policy-neutral mechanism (thresholds, co-signers, and freshness are
  rungs; Cairn ships the mechanism).
````

- [ ] **Step 2: Add the mkdocs nav entry**

In `mkdocs.yml`, after line 161
(`      - ADR-0054 · Actor-registry federation — admit-and-dispute: spec/decisions/0054-actor-registry-federation-admit-and-dispute.md`),
insert:

```yaml
      - ADR-0055 · Distribution trust root — channels & chained root document: spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md
```

- [ ] **Step 3: Verify the file renders and links resolve**

Run: `cd /Users/hherb/src/cairn-ehr && uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -5`
Expected: build completes; no warnings referencing `0055-`.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md mkdocs.yml
git commit -m "docs(#206): ADR-0055 — distribution-plane trust-root governance

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: security.md §7.6 rewrite

**Files:**
- Modify: `docs/spec/security.md:89-100` (§7.6 body)

**Interfaces:**
- Consumes: ADR-0055 file from Task 1 (link target
  `decisions/0055-distribution-trust-root-governance-chained-root-document.md`).

- [ ] **Step 1: Replace the §7.6 body**

Keep the `## 7.6` heading and the existing `> Resolves part of former open question §11.4 …`
pointer line, but extend that pointer line by appending before its final period:
`; trust-root governance is [ADR-0055](decisions/0055-distribution-trust-root-governance-chained-root-document.md)`.

Then replace the section body — the intro paragraph and the four bullets (lines 92–97), keeping
the existing `> [!WARNING]` callout (lines 99–100) — with:

```markdown
Clinical events sync over the mesh; **code, DDL, and extensions travel a separate plane with a different trust model.** Much of Cairn's safety-critical logic runs as Postgres extensions ([ADR-0002](decisions/0002-in-database-rust-pgrx-escape-hatch.md)), and an extension is *native, architecture-specific* code running inside the database — so the sync plane must **never** carry it (a synced, loaded `.so` is a remote-code-execution channel into every node, violating [principle 8](index.md#founding-principles-the-lens-for-every-decision)). The two planes carry **opposite postures** ([ADR-0055](decisions/0055-distribution-trust-root-governance-chained-root-document.md), completing [ADR-0054](decisions/0054-actor-registry-federation-admit-and-dispute.md)'s contrast): the content plane admits-and-disputes — content never waits; the code plane **verifies-or-refuses** — code always waits, and a refusal is legible, logged, and never touches running code.

- **No privileged root: channels, with the steward as default anchor** ([ADR-0055](decisions/0055-distribution-trust-root-governance-chained-root-document.md)). A **channel** is the unit of distribution trust — `{trust-root chain, transparency log, release stream}` — and the trust root a node honors is **provisioning configuration** on the [§7.7](#77-federation-admission-peering-trust-anchors-and-the-custodian-contract) anchor spectrum: official builds default to the steward set; a national deployment or self-building fork repoints without forking code; the channel operator is a node role ([principle 6](index.md#founding-principles-the-lens-for-every-decision)). A solo practice on defaults never sees any of this — the friction budget lands on channel *operators*, never consumers (paper-parity).
- **The chained trust-root document.** Per channel: `{channel_id, version, root signers + threshold, release signers + threshold, prev-root hash}` — content-addressed, self-contained, verifiable with zero database and zero network. Version N+1 carries ≥ M_root signatures from version N's root set (explicit multi-signature — *which* signers signed is public evidence; **N=1/M=1 is first-class**: the solo practice and the pre-entity steward are honest single-signer roots). Nodes pin the highest verified version and refuse regression; **roots do not expire** (an expired root bricking an offline clinic violates the availability floor — freshness is a [§7.9](#79-hard-policy-expression-projection-and-enforcement) policy rung; the retired-key fork window this opens is compensated by retirement key-destruction + the log/gossip fork detector). **The root role signs root versions** (rare, offline — the constitution); **the release role signs releases** (the daily pen, cheaply rotated by one root version without fleet re-provisioning). **A fork — two verifiable successors of one version — is never resolved by arrival order:** the node freezes the pin, surfaces a security incident, and only the audited ceremony resolves it; upgrades pause, running code and local reads/writes are untouched.
- **A migration is one signed atomic bundle:** `{ DDL (architecture-independent text) + extension binary per architecture × Postgres-major + projection-rebuild recipe }` — plus its release signatures, the root-chain segment the receiver may be missing, and the transparency inclusion proof: **self-contained, so the sneakernet path is first-class** ([§6.1](sync.md#61-mechanism)), installable through the same audited ceremony as node provisioning. The load gate verifies root chain → release signatures (against the **newest verified root's** release set — a manifest can never elect an older root's keys) → every artifact digest, then loads; release versions are monotonic per channel. **Refusals speak practice-manager language** — what is pinned, what this bundle needs, what to do next — never "signature verification failed." *Which* releases a deployment installs, on what schedule, through which channel, who may authorize an upgrade, and whether **additional named co-signers** are required before load (a hospital security team, a national authority, an independent rebuilder — policy may ratchet stricter, never weaker) are policy ([principle 9](index.md#founding-principles-the-lens-for-every-decision)).
- **Transparency is the evidence plane** ([ADR-0055](decisions/0055-distribution-trust-root-governance-chained-root-document.md); the [ADR-0027](decisions/0027-trusted-time-anchoring.md) log shape reused as a self-hostable node role). The official channel logs every release manifest and root version; bundles carry inclusion proofs (offline-verifiable); peers gossip checkpoints and root pins — a *targeted* poisoned build becomes detectable evidence. Independent reproducible-build attestations are just additional manifest signatures. Limits stated honestly: genesis trust is procedural (ceremony + second-channel fingerprint); a young log has few monitors (reproducible builds carry the near-term weight); the official channel's root is **1-of-1 today, declared in the root document itself**, escrowed per [ADR-0026](decisions/0026-node-durability-and-disaster-recovery.md), and committed to reach ≥ 2-of-3 before the first production deployment outside the steward's control.
- **Fail-safe upgrades — availability beats upgrade** ([principle 5](index.md#founding-principles-the-lens-for-every-decision)). An unattended Pi must never brick: the prior extension is retained until the new one is verified healthy (blue-green at the extension level); additive DDL ([data-model §3.13](data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)) means rollback loses nothing; and writes during a half-applied upgrade are just more append-only events, re-projected once it settles. `status` reports channel, pinned root version, release version, log freshness, and any frozen-pin incident — beside sync freshness and backup health (honest assembly applied to trust).
- **Difficulty tracks native-code surface.** PL/pgSQL/SQL migrations are architecture-independent text that ride a trivial channel; only pgrx forces the per-architecture binary plane. The [ADR-0001](decisions/0001-fat-postgres-thin-daemon.md)/[ADR-0002](decisions/0002-in-database-rust-pgrx-escape-hatch.md) discipline of keeping the native surface small therefore earns a second payoff — it minimizes migration blast radius.
```

- [ ] **Step 2: Extend the §7.6 WARNING callout**

In the `> [!WARNING]` block at the end of §7.6, replace exactly:

> The distribution-plane **signature verification and extension load** are **safety-critical**

with:

> The distribution-plane **root-chain + multi-signature verification and extension load** are **safety-critical**

- [ ] **Step 3: Build + grep verification**

Run: `grep -c "0055-distribution-trust-root" docs/spec/security.md`
Expected: `4` (pointer line, first bullet, second-bullet is link-free, transparency bullet, intro paragraph — count the actual link occurrences; if the number differs, verify each intended link exists rather than chasing the count)
Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/security.md
git commit -m "docs(#206): security §7.6 — channels, chained trust root, role split, transparency (ADR-0055)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: security.md §7.7 + §7.9 — root unification deltas

**Files:**
- Modify: `docs/spec/security.md` (§7.7 practice-key bullet, line ~111; §7.9 bootstrap bullet, line ~158)

- [ ] **Step 1: §7.7 practice issuing key pointer**

In the §7.7 bullet beginning `- **the practice's own issuing key**`, replace exactly:

> — the practice *is* its authority, issuing credentials to its nodes and **setting its own join rules**: a self-sovereign network;

with:

> — the practice *is* its authority, issuing credentials to its nodes and **setting its own join rules**: a self-sovereign network. Its key is a **1-of-1 [ADR-0055](decisions/0055-distribution-trust-root-governance-chained-root-document.md) trust-root document** — escrowed by default ([ADR-0026](decisions/0026-node-durability-and-disaster-recovery.md): print the recovery QR, put it in the drug safe), upgradeable in place to a threshold as the practice grows;

- [ ] **Step 2: §7.9 bootstrap bullet**

In the §7.9 bullet beginning `- **Authority-gated authoring, bootstrapped at provisioning.**`,
replace exactly:

> *who* holds it bottoms out at the **root authority set at node provisioning** ([§7.6](#76-the-software-distribution-plane-signed-releases-and-extension-load)) — the same bootstrap as the steward key and the [§7.7](#77-federation-admission-peering-trust-anchors-and-the-custodian-contract) self-issued practice key.

with:

> *who* holds it bottoms out at the **root authority set at node provisioning** — an [ADR-0055](decisions/0055-distribution-trust-root-governance-chained-root-document.md) **trust-root document** (root role only: threshold-capable, chain-rotated, ceremony-recovered), the same root shape as the [§7.6](#76-the-software-distribution-plane-signed-releases-and-extension-load) distribution channel and the [§7.7](#77-federation-admission-peering-trust-anchors-and-the-custodian-contract) practice issuing key.

- [ ] **Step 3: Build + grep verification**

Run: `grep -c "0055-distribution-trust-root" docs/spec/security.md`
Expected: Task 2's count + 2.
Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/security.md
git commit -m "docs(#206): security §7.7/§7.9 — one root shape for all three provisioning-time roots (ADR-0055)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: sync.md §6.5 — distribution-plane pointer update

**Files:**
- Modify: `docs/spec/sync.md:60`

- [ ] **Step 1: Update the distribution-plane bullet**

In §6.5, replace exactly:

> - **The distribution plane** ([security §7.6](security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)) carries code/DDL/extensions — per-node, per-architecture, signed against a steward key and verified before install, delivered online or by **sneakernet** ([§6.1](#61-mechanism)).

with:

> - **The distribution plane** ([security §7.6](security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)) carries code/DDL/extensions — per-node, per-architecture, **threshold-signed under a channel's chained trust root and verified before install** ([ADR-0055](decisions/0055-distribution-trust-root-governance-chained-root-document.md)), delivered online or by **sneakernet** ([§6.1](#61-mechanism)).

- [ ] **Step 2: Build + grep verification**

Run: `grep -c "0055-distribution-trust-root" docs/spec/sync.md`
Expected: `1`
Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/sync.md
git commit -m "docs(#206): sync §6.5 — distribution plane rides the ADR-0055 chained trust root

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Spec version bump + full docs verification

**Files:**
- Modify: `docs/spec/index.md:9`

- [ ] **Step 1: Bump the spec version**

On line 9, replace exactly `**Spec version:** 0.56` with `**Spec version:** 0.57`.

- [ ] **Step 2: Full clean docs build**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -10`
Expected: `Documentation built` with no warnings mentioning any file touched by Tasks 1–4.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/index.md
git commit -m "docs: spec v0.57 — ADR-0055 distribution-plane trust-root governance

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: File the follow-on issues

**Files:** none (GitHub only). Capture each created issue number for Task 7's HANDOVER paragraph.

- [ ] **Step 1: Create the five follow-on issues**

```bash
gh issue create --title "distribution plane: implement the ADR-0055 root-chain verifier + load gate + bundle format" --body "Code slices for ADR-0055 (no §7.6 code exists today): the chained trust-root verifier (chain from pin, threshold multi-sig, monotonic pin, fork-freeze incident), the release bundle format, the load-gate order (root chain → release sigs against newest root → artifact digests → load), legible practice-manager refusals, and the \`status\` trust surface (channel / pinned root / release / log freshness / frozen-pin). Safety-critical seam (§9.1): Rust/in-DB, TDD, reviewer-legible. Design: docs/superpowers/specs/2026-07-20-distribution-trust-root-design.md. Blocked on nothing, but sequenced with the distribution-plane build itself."

gh issue create --title "distribution plane: transparency-log node role (ADR-0055)" --body "Implement the release/root transparency log as a self-hostable node role (ADR-0027 shape reused): append-only, content-addressed, Merkle-checkpointed; inclusion proofs in bundles; checkpoint + root-pin gossip on the existing plane. Official channel MUST log; fit-for-purpose tier (server software), verification stays in the trusted gate. Design: docs/superpowers/specs/2026-07-20-distribution-trust-root-design.md §3.6."

gh issue create --title "build: CI reproducible-build check + rebuilder-attestation wiring (ADR-0055)" --body "ADR-0055 names reproducible builds as the near-term evidence carrier while the transparency log matures — currently aspirational. Add a CI job that rebuilds the release artifacts and diffs digests (pgrx × arch × PG-major; expect SDK-drift pain — see the cairn_pgx Xcode memory), and wire independent rebuilder attestations as additional manifest signatures (the co-signer mechanism reused)."

gh issue create --title "policy: distribution-freshness rung (anti-freeze) on the ADR-0055 log checkpoint" --body "ADR-0055 roots/releases never expire (availability floor). The anti-freeze compensation is a §7.9 policy rung: a deployment MAY gate *upgrades* (never running code, never local reads) on transparency-log-checkpoint age. Spec the rung + its honest-staleness surfacing when it lands with the policy-stream code plane."

gh issue create --title "design session: sync-authorization onboarding UX (invite-token pairing, LAN discovery, practice-delegation flow)" --body "Raised during the #206 session: the least-friction node-authorizes-node story. Direct pairwise pairing exists (ADR-0017, built); the wanted rungs are (a) single-use invite-token pairing (mint/redeem, short expiry, fingerprint-bound), (b) LAN discovery + code-confirm as a UX layer over any trust rung, (c) the practice-as-authority delegation flow (one enrolment ceremony per node, auto-peer within the practice) whose issuing key is now an ADR-0055 1-of-1 root document. Integrity is event-signature-layer regardless of pairing UX — the ceremony can be low-friction. Design session → likely ADR + spec §7.7/§5.11 deltas."
```

Expected: five issue URLs printed; note the numbers.

---

### Task 7: Session close — HANDOVER/ROADMAP, push, PR

**Files:**
- Modify: `docs/HANDOVER.md` (NEXT header line 3, P6 list, session paragraph, status line ~59, ADR index table after the 0054 row)
- Modify: `docs/ROADMAP.md` (new Slice 47 after the Slice 46 block at line ~1088)

- [ ] **Step 1: HANDOVER updates**

1. ADR index table — append after the 0054 row:
```markdown
| [0055](spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md) | Distribution trust root: no privileged root — channels with the steward as default anchor; chained threshold-capable root document (N=1 first-class, no expiry); root/release role split; fork-freeze never-silently-pick; transparency log by ADR-0027 reuse; one root shape for §7.6/§7.9/§7.7 | §7.6/§7.9/§7.7/§6.5 (refines 0012/0024; applies 0017/0018) |
```
2. NEXT header (line 3): change `(#205 ✅ done → ADR-0054; #206/#200/#208/#216/#217 remain)` to
   `(#205 ✅ → ADR-0054; #206 ✅ → ADR-0055; #200/#208/#216/#217 remain)`.
3. In the "Priority 6 — design sessions" list, replace the `- **#206 (C5)** — …` entry with:
   `- **#206 (C5) ✅ 2026-07-20** — resolved by [ADR-0055](spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md) (chained trust-root document; spec v0.57); code slices + log role + reproducibility CI + freshness rung filed as follow-ons (issue numbers from Task 6).`
4. Prepend a new session paragraph above the "Session (2026-07-19, latest)" one (retitle that one
   to drop ", latest"): the #206 design session → ADR-0055 + spec v0.57 (security §7.6 rewrite +
   §7.7/§7.9 unification + sync §6.5), the chained-root-document design (role split, fork rule,
   N=1 ratchet tripwire, transparency by ADR-0027 reuse, three-root unification), the five filed
   follow-ons (insert real numbers), and the parked sync-authorization-UX design session. Update
   the "Session date" line and the "Spec/ADRs:" status line to `v0.57 (through ADR-0055)`.
   Keep the file under ~430 lines by condensing older session paragraphs if needed.

- [ ] **Step 2: ROADMAP Slice 47**

After the Slice 46 block, append:

```markdown
**Slice 47 — the #206 design session: ADR-0055 distribution-plane trust-root governance
(2026-07-20; P6 design queue, second item; spec v0.57; docs-only — code slices filed as
follow-ons).** Review finding C5 (single steward key signing the highest-blast-radius artifact)
resolves by applying the corpus's own anchor doctrine to the steward: **no privileged root on the
distribution plane** — a **channel** `{trust-root chain, transparency log, release stream}` is the
trust unit, the root is provisioning config on the ADR-0017 spectrum, the steward is only the
official channel's default anchor. Mechanism: a **chained, content-addressed trust-root document**
(version N+1 signed by ≥ M_root of version N; explicit TUF-shape multi-sig, N=1/M=1 first-class;
monotonic pin; **no expiry** — availability floor, compensated by retirement key-destruction +
log/gossip fork detection), a **root/release role split** (constitution key vs daily pen — a
release-key compromise is one rotation, not a fleet re-provisioning), a **fork-freeze rule** (two
verifiable successors ⇒ security incident + ceremony, never arrival-order), the **verify-or-refuse
load gate** (newest-root rule: `root_version_ref` is diagnostic, never authoritative; legible
refusals; co-signer floor), the **transparency log** (ADR-0027 shape as a self-hostable node role;
rebuilder attestations = co-signatures; limits stated: genesis TOFU procedural, young-log monitor
scarcity, reproducible builds carry near-term weight), an **honest N=1 posture with a ratchet
tripwire** (≥ 2-of-3 before the first production deployment outside the steward's control;
ADR-0026 escrow custody floor), and **one root shape for all three provisioning-time roots**
(§7.6/§7.9/§7.7). Code plane vs content plane now carry opposite postures (verifies-or-refuses vs
admits-and-disputes — the ADR-0054 contrast completed). Design/plan docs under
`docs/superpowers/`.
```

- [ ] **Step 3: Final verification, commit, push, PR**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -3`
Expected: clean build.

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — Slice 47, ADR-0055 session close

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push -u origin design/206-distribution-trust-root
gh pr create --title "ADR-0055: distribution-plane trust-root governance (closes #206)" --body "$(cat <<'EOF'
## Summary
Design-session output for issue #206 (2026-07-15 review finding C5): **ADR-0055** replaces the
single steward key — the highest-blast-radius trust root in the system — with the corpus's own
anchor doctrine applied to the steward itself: **no privileged root on the distribution plane**.
A **channel** `{trust-root chain, transparency log, release stream}` is the trust unit; the root
is a **chained, content-addressed, threshold-capable root document** (TUF-shape explicit
multi-sig, N=1 first-class, no expiry — availability floor), with a **root/release role split**,
a **fork-freeze never-silently-pick rule**, a **verify-or-refuse load gate** with
practice-manager-legible refusals, **transparency by ADR-0027 reuse** (rebuilder attestations =
co-signatures), an **honest 1-of-1 posture with a ratcheted ≥2-of-3 tripwire** + ADR-0026 escrow
floor, and **one root shape for all three provisioning-time roots** (§7.6 distribution, §7.9
policy authority, §7.7 practice issuing key). Spec v0.56 → **v0.57**; prose landed in security
§7.6 (rewrite) + §7.7/§7.9, sync §6.5.

Follow-ons filed: verifier/load-gate code slices, transparency-log node role, CI reproducibility
check, freshness policy rung, and the sync-authorization onboarding-UX design session.

Closes #206.

## Review notes
- ADRs are immutable post-merge — review the ADR text itself hardest.
- Docs-only: no code, no schema, no wire change. No §7.6 code exists today; the root-document and
  bundle shapes are the day-one can't-retrofit commitments.
- `mkdocs build` clean via the pinned `docs/requirements.txt`.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review (completed at write time)

1. **Spec coverage:** design §3.1 doctrine/channels → ADR point 1 + §7.6 intro/first bullet;
   §3.2 root document → ADR point 2 + §7.6 second bullet; §3.3 lifecycle/fork/catastrophic → ADR
   points 4–5; §3.4 posture/tripwire → ADR point 8; §3.5 bundle/gate/co-signers/status → ADR
   point 6 + §7.6 third/fifth bullets; §3.6 transparency + limits → ADR point 7 + §7.6 fourth
   bullet; §3.7 unification → ADR point 9 + Task 3; §4 UX requirements → ADR points 1/6 +
   §7.6 prose; §5 consequences → ADR Consequences; §6 deliverables → Tasks 1–5 + 7; §7 follow-ons
   → Task 6; §8 rejected → ADR point 10. No gaps.
2. **Placeholder scan:** none — every step carries full text. Task 6→7 issue numbers are runtime
   data captured at execution, explicitly instructed, not placeholders.
3. **Type consistency:** the ADR slug
   `0055-distribution-trust-root-governance-chained-root-document.md` is identical across Tasks
   1–5 and the HANDOVER row; field names (`M_root`, `M_rel`, `root_version_ref`, `prev_root_hash`,
   `channel_id`) match the design doc §3.2/§3.5 exactly; grep counts verify links mechanically.
