# Distribution-plane trust-root governance: chained root document, role split, transparency

**Design session 2026-07-20 · resolves issue [#206](https://github.com/cairn-ehr/cairn-ehr/issues/206)
(2026-07-15 review finding C5) · deliverable: ADR-0055 + spec updates (code slices follow later).**

Related: [ADR-0012](../../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)
(the two planes; the distribution plane this ADR governs),
[ADR-0017](../../spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)/
[ADR-0018](../../spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md)
(the no-single-anchor doctrine this ADR finally applies to the steward key),
[ADR-0027](../../spec/decisions/0027-trusted-time-anchoring.md) (the transparency-log shape reused here),
[ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) (escrow rungs = the
custody floor for 1-of-1 roots), [ADR-0024](../../spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)
(§7.9 policy-authority root — inherits this bootstrap).

## 1. Problem

`security.md` §7.6 / `sync.md` §6.5 / ADR-0012 §4 sign code — the **highest-blast-radius artifact in
the system** (native extensions loaded into the DB trusted base) — against **a single steward key**,
with no threshold signing, no rotation, no compromise recovery, and no succession story. Meanwhile
mere timestamps got FROST/threshold multi-anchor treatment (ADR-0027) and federation credentials got
explicit no-single-root doctrine (ADR-0017/0018: *"never mandate a single anchor — that builds the
kill-switch"*; *"a trust anchor is a position of power"*). §7.9 notes the policy-authority root
bootstraps "the same as the steward key," inheriting the same gap.

A steward-key compromise is **signed remote-code-execution into every upgrading node**; steward
capture is the mission's own named adversary. Anti-capture is the tie-breaker, and the corpus
applies it everywhere except the most powerful key it defines. No ADR owns this. (Finding C5,
severity Important.)

## 2. Settled by the session (with the user)

1. **Root posture: pluggable, steward = default.** The distribution trust root is node provisioning
   configuration on the ADR-0017 anchor spectrum. Official builds ship the steward set as the
   default anchor; a deployment can repoint without forking code. *No Cairn-owned root* applied to
   the distribution plane itself.
2. **Threshold shape: explicit multi-signature (the TUF root-metadata shape), with N=1/M=1
   first-class.** Reviewer-legible verify loop; which signers signed is public evidence; signers
   hold heterogeneous custody. A solo practice (and the pre-entity steward today) is a valid
   single-signer root; the threshold is an honest ratchet, never fake multi-party ceremony.
   FROST stays earmarked for the high-volume notary plane (ADR-0027), not release signing.
3. **Transparency: core piece, by reuse** of the ADR-0027 transparency-log shape (self-hostable
   node role; inclusion proofs travel with bundles; offline never fails closed).
4. **Root unification: one root-metadata mechanism for all three provisioning-time roots**
   (§7.6 distribution, §7.9 policy authority, §7.7 practice issuing key).
5. **Approach: chained root document** (each version signed by a threshold of the previous
   version's signers). Roots-as-§7.5-actors rejected (bootstrap circularity: the registry is DB
   state installed *by* the code plane); local-config-only rejected (no rotation story at fleet
   scale). See §8.
6. **Pitfall-session amendments folded in:** the root/release role split, the fork-conflict
   never-silently-pick rule, and the N=1 ratchet tripwire + escrow custody floor (§3.3–§3.5).

## 3. Design

### 3.1 Doctrine: channels, and no privileged root on the distribution plane

- **"The steward key" is not a system-wide root; it is the default configured anchor of the
  official release channel.** A **channel** is the unit of distribution trust:
  `{trust-root chain, transparency log, release stream}`. The official channel ships
  steward-rooted as the default; a national deployment runs its own channel (rebuilding from
  source, or re-signing official bundles after its own review); a solo practice uses the default
  and never sees any of this. The channel operator is a node role — fractal topology applied to
  distribution.
- **The two planes get opposite postures**, completing the contrast ADR-0054 opened: the content
  plane **admits-and-disputes** (content never waits); the code plane **verifies-or-refuses**
  (code always waits). A refused bundle is a legible, logged refusal that never affects running
  code — refusal to upgrade can never brick a clinic (the §7.6 blue-green line, unchanged).
- **Steward capture is answered structurally:** threshold makes silent single-key compromise
  insufficient; transparency makes a targeted poisoned build *evidence*; pluggability means a
  captured steward faces a fleet that can walk away. ADR-0018's "anchor is a position of power"
  turned on the steward itself.

### 3.2 The chained trust-root document

```
trust_root = {
  channel_id,
  version,                 -- monotonic per channel
  root_signers:    [{kid, pubkey, custody_note}], root_threshold M_root,
  release_signers: [{kid, pubkey}],               release_threshold M_rel,   -- default 1
  prev_root_hash           -- content address of version N-1 (absent on genesis)
}
```

- Content-addressed, self-contained, verifiable with **zero DB and zero network** — a bare
  installer or a phone can verify it. Version 1 is the self-signed genesis distributed at
  provisioning; every version N+1 must carry **≥ M_root signatures from version N's
  `root_signers`** (chain-of-custody rotation).
- **Role split (amendment):** the **root role** signs only root-document versions — rare,
  high-ceremony, offline keys: the constitution. The **release role** signs day-to-day release
  manifests — the daily pen, cheaply revoked or rotated by publishing root version N+1 *without
  touching root-key custody*. The split benefits the 1-of-1 era most: even a solo steward keeps
  the constitution key in the safe and the daily key on the bench, so a release-key compromise is
  recoverable by one signed rotation instead of a fleet re-provisioning.
- **Monotonic pinning:** a node pins `(channel, highest verified version)` and refuses regression —
  the `SCHEMA_GENERATION` guard pattern (PR #251) applied to trust metadata.
- **No expiry, deliberately** — a stated divergence from TUF: an expired root bricking an offline
  clinic violates the availability floor. Freshness is a §7.9 policy rung (a deployment MAY gate
  *upgrades* on log-checkpoint age); staleness is surfaced honestly; running code is never
  touched. The cost of this divergence is real and named in §5 (immortal retired keys).

### 3.3 Lifecycle: every change is "publish version N+1"

- **Routine rotation, signer add/remove, threshold change, emergency exclusion of a compromised
  key** — all the same operation: a new root version signed by ≥ M_root of the previous set,
  recorded in the transparency log (the log entry *is* the evidence).
- **Succession is a rotation like any other.** Founder → stewarding foundation is entity-neutral
  mechanism; the parked legal-entity question stays parked. The governance doc binds whoever the
  holders are.
- **Retirement includes key destruction.** Because roots never expire, a retired root key remains
  forever able to sign a fork at its historical version; retirement is therefore a ceremony that
  destroys the key material, and the log/gossip cross-check (§3.6) is the standing fork detector.
- **Fork-conflict rule (amendment — never silently pick):** a node advances its pin only when
  exactly one verifiable successor exists. If it ever observes **two distinct verifiable
  successors of the same version** — at upgrade time, or later via log/gossip cross-check after a
  restore-from-backup — it **freezes the pin**, surfaces a **security incident** on the
  honest-assembly status surface, and resolves only via the audited ceremony. Freezing blocks
  *upgrades* on that channel; running code, local reads, and writes are untouched. Without this
  rule, regression refusal is first-write-wins and works *for* an attacker who reaches a stale
  node first after an emergency rotation.
- **The catastrophic case is declared, not solved** (the ADR-0005 posture: best-effort and
  declared, never guaranteed). If ≥ M_root keys are compromised (an attacker can rotate) or fewer
  than M_root survive (the holders cannot), the chain is dead — recovery is the out-of-band
  audited **re-provisioning ceremony** (the same ceremony vocabulary as §7.6/§7.7), establishing a
  new genesis, with the discontinuity recorded in the transparency log. At fleet scale this is a
  product recall — weeks of clinical-IT cost — which is exactly why the ratchet below is a
  commitment with a trigger, not an aspiration.

### 3.4 Honest current posture, and the ratchet tripwire (amendment)

- The official channel today is **N=1, M_root=1** (the founder), **declared in the root document
  itself** — principle 4 applied to governance: declare 1-of-1 honestly rather than perform fake
  multi-party ceremony. Until the signer set grows, the operative protections are the role split,
  transparency, and reproducible builds — stated plainly.
- **Governance commitment with a named trigger:** *before the first production deployment operated
  outside the steward's own control, the official channel's root role MUST be ≥ 2-of-3.*
- **Custody floor for the 1-of-1 era:** the ADR-0026 escrow rungs — offline root key plus an
  escrowed recovery path (Shamir M-of-N / QR-in-the-safe). The same rung is the recommended
  default for practice issuing keys (§3.7): "print the recovery QR, put it in the drug safe."

### 3.5 The release bundle and the load gate

- **Self-contained bundle:**

```
release_bundle = {
  manifest: {
    channel_id, release_version,          -- monotonic per channel
    artifact_digests,                     -- per (architecture × Postgres-major) binary,
                                          --   DDL text, projection-rebuild recipe
    root_version_ref                      -- which trust_root version governs this release
  },
  release_signatures[],                   -- ≥ M_rel from the current release_signers
  root_chain_segment[],                   -- trust_root versions the receiver may be missing
  inclusion_proof                         -- when the channel runs a log (official: always)
}
```

  Verifiable with zero network — the sneakernet path is first-class, not degraded.
- **Load-gate order (the one safety-critical seam, §9.1 Rust/in-DB, reviewer-legible):**
  verify root chain from the pin → verify release signatures → verify every artifact digest →
  only then load. Release signatures verify against **the release set of the newest verified root
  version** — `root_version_ref` is diagnostic, never authoritative: a manifest can never elect an
  older root's release set, so a rotated-out release key is dead the moment the node sees the
  rotation. Never partial-load; blue-green retention unchanged; release versions monotonic per
  channel.
- **Refusal legibility is a requirement, not a nicety.** Refusals speak practice-manager language —
  *what is pinned, what this bundle needs, what is missing, what to do next* ("your node last
  accepted trust version 4; this update needs 7; versions 5–6 are missing — fetch them or contact
  support") — never "signature verification failed."
- **Co-signer floor:** node provisioning config may require **K additional named signers** on any
  release before load (a hospital security team, a national authority, an independent rebuilder).
  §7.9 policy may ratchet *stricter* (add signers), never weaker than the provisioning floor.
  **Soft-policy guidance:** co-signing trades patch latency for review — a deployment that
  requires it must define its emergency path *in advance*, because the real-world alternative is
  "skip the check just this once."
- **Honest assembly:** `status` reports channel, pinned root version, current release version, log
  freshness, and any frozen-pin incident — beside sync freshness and backup health. Pluggable
  roots add a trust-topology dimension to support ("why won't node X take the update"); the
  status surface is what keeps it diagnosable.

### 3.6 Transparency: the evidence plane, with its limits stated

- A channel MAY — **the official channel MUST** — log every release manifest and every root-doc
  version in an append-only, content-addressed, Merkle-checkpointed **transparency log**: the
  ADR-0027 shape reused, a **self-hostable node role**, mirrorable and sneakernet-distributable.
- Bundles carry inclusion proofs; verification is offline-capable; a node cross-checks log
  consistency when connectivity allows; peers **gossip log checkpoints and root pins** on the
  existing plane. This is what makes a *targeted* poisoned build (served to one victim clinic) and
  a forked root chain **detectable evidence** rather than silent success.
- **Rebuilder attestations slot in for free:** an independent reproducible-build attestation is
  just another signature on the manifest — the co-signer mechanism reused, no new machinery.
- **Named limits (written into the ADR, not hidden):**
  - **Genesis TOFU stays procedural.** The chain protects every step *after* trust establishment;
    the first root arrives with the installer or the provisioning medium, and a poisoned installer
    carries a poisoned verifier that validates its own poisoned genesis. Initial provisioning
    integrity is ceremony + second-channel fingerprint, not cryptography.
  - **A log without independent monitors is weak.** In the early fleet the steward runs the only
    log and the only monitor; the log's adversarial value matures with witness diversity, and
    until then **reproducible builds carry the near-term weight**.
  - **The log operator is itself a small power position** (refusing to log is a publish-DoS) —
    mitigated by mirrorability; multi-log (CT-style dual inclusion proofs) is additive later.
  - **Reproducible builds are load-bearing and genuinely hard** (pgrx × architectures ×
    PG-majors × SDK drift). Until a CI reproducibility check exists and independent rebuilders
    byte-match, "verify by rebuilding" is aspirational — a named follow-on, not an assumed fact.

### 3.7 One root shape, three roots

The root-document mechanism, ceremony vocabulary, and fork rule serve **all three
provisioning-time roots**:

| Root | Shape | Notes |
|---|---|---|
| §7.6 distribution channel | full (root + release roles) | this ADR's main subject |
| §7.9 policy-authority bootstrap | root role only | names the initial policy-authority set; §7.9's "same bootstrap as the steward key" now points here |
| §7.7 practice issuing key | 1-of-1 default, escrow rung | upgradeable in place as the practice grows |

One verifier for all three concentrates blast radius — stated plainly, and accepted: one
well-reviewed, reviewer-legible verifier beats three ad-hoc ones (the §9.1 small-trusted-surface
argument working as intended).

## 4. UX requirements (paper-parity applied to distribution)

- **Solo-practice invisibility is mandatory.** With defaults, a solo GP never sees channels, roots,
  or thresholds — updates just verify. The friction budget lands entirely on channel *operators*,
  never channel *consumers*. Any default-path fingerprint ceremony or root-version dialog is a
  paper-parity failure.
- **Refusal language** per §3.5 — the diagnostic quality *is* the UX.
- **Practice-root custody default** per §3.4 — the escrow rung is the recommended default, or the
  sovereignty story becomes a foot-gun story.
- **Co-signer policy warning** per §3.5 — patch latency is a measured trade, not a free win.

## 5. Consequences (for the ADR's honesty section)

- **Easier:** closes C5 by reuse — the ADR-0017 anchor spectrum, the ADR-0027 log shape, the
  ADR-0026 escrow rungs, the §7.5 ceremony vocabulary, the PR-#251 monotonic-guard pattern. The
  steward stops being a structural exception to the corpus's own doctrine.
- **Harder / new trusted surface:** the root-chain + multi-sig verifier and the load gate
  (small, reviewer-legible, §9.1); the signing ceremonies (procedural discipline); the log role
  (fit-for-purpose server software).
- **Honest weaknesses carried:** no-expiry ⇒ immortal retired keys (compensated by destruction
  ceremony + fork detection, §3.3/§3.6); genesis TOFU (procedural); early-fleet log-monitor
  scarcity (reproducible builds carry the weight); N=1 today (declared, ratcheted, escrowed).

## 6. Deliverables (this session)

1. **ADR-0055** — distribution-plane trust-root governance: chained root document, root/release
   role split, pluggable channel roots, transparency log, root unification.
2. **Spec deltas:** `security.md` §7.6 (main rewrite: channel, root document, load gate, co-signer
   floor, honest-assembly status), §7.9 (bootstrap bullet → root document), §7.7 (practice issuing
   key = 1-of-1 root doc + escrow rung pointer); `sync.md` §6.5 (pointer line); `index.md` spec
   version → v0.57; mkdocs nav entry for ADR-0055.
3. **HANDOVER/ROADMAP** close-out.

## 7. Named follow-ons (file as issues)

- **Code slices** (when the distribution plane gets built — no §7.6 code exists today): the
  root-doc verifier + load gate; bundle format; `status` trust surface.
- **Transparency-log node role** implementation.
- **CI reproducible-build check** (+ rebuilder-attestation wiring).
- **Freshness policy rung** (anti-freeze: gate *upgrades* on log-checkpoint age).
- **Sync-authorization onboarding UX** (invite-token pairing + LAN discovery + practice-delegation
  flow — separate design session; raised in this session, out of scope here).

## 8. Considered and rejected

- **FROST aggregate signature for releases** — verifier stays single-sig, but participation is
  invisible (anti-audit for a governance key), the signing ceremony is interactive, and share
  rotation is opaque. Kept for the high-volume notary plane (ADR-0027) it was earmarked for.
- **Roots as §7.5 actors** — maximal corpus reuse, but bootstrap circularity: the actor registry
  is DB state installed *by* the artifact being verified; the load gate must verify with zero DB.
  (Steward identities MAY additionally be mirrored as actors for in-record attribution.)
- **Local-config-only key set (no root document)** — no self-describing rotation; every rotation
  is a fleet-wide manual ceremony; fails the finding's rotation/succession requirement.
- **Steward floor with additive-only co-signers** — mandates a single anchor: the named
  kill-switch footgun; makes steward capture unrecoverable except by forking.
- **Pure policy, no shipped default** — hostile to the solo-practice bootstrap; the default path
  must just work.
- **Full TUF freshness roles (snapshot/timestamp)** — expiring metadata bricks offline clinics
  (availability floor); freshness becomes a policy rung instead; the local downgrade guard
  (PR #251) already covers regression.
