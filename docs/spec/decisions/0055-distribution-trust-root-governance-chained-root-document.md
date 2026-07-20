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
*"never mandate a single anchor"* — the deployment that does has *built* the kill-switch; *"a
trust anchor is a position of power"*). [Security §7.9](../security.md#79-hard-policy-expression-projection-and-enforcement)
notes the policy-authority root shares "the same bootstrap as the steward key," inheriting the gap.

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
(the registry-as-DB-state alternative fails exactly there; see point 10). Root signatures are
**detached** — they travel beside the document, as release signatures do beside the manifest —
and the **content address covers the payload alone**, so a document's identity never varies with
which signature subset accompanies it: point 5's "two distinct verifiable successors" means two
distinct *payloads* each carrying ≥ M_root valid signatures, never one payload seen with
different signature sets. Version 1 is the
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
node sees the rotation — and, stated honestly, live until then: a stale node that has not yet
seen root version N+1 still accepts new releases signed by a key N+1 rotated out (an attacker
simply withholds the newer chain segment). This revocation-latency window is the concrete price
of dropping TUF's expiring freshness roles (point 2), named in Consequences; point 7's
root-pin/checkpoint gossip and the §7.9 freshness rung bound it. Never partial-load; release
versions monotonic per channel. **Refusals
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
  ceremony + standing fork detection) **and a release-key revocation window** — a stale node
  accepts new releases from a rotated-out release key until the rotation reaches it, and an
  unlogged poisoned bundle is caught retrospectively by gossip/monitors, never blocked by the
  load gate (compensated by root-pin/checkpoint gossip + the §7.9 freshness rung); genesis TOFU
  (procedural); early-fleet log-monitor scarcity
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
