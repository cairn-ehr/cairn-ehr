# ADR-0005 — Erasure as key-custody redistribution: crypto-shredding and a policy-neutral severity ladder

- **Status:** Accepted
- **Date:** 2026-06-14
- **Supersedes:** —

## Context

Former open question [§11.5](../open-questions.md): legal deletion (GDPR "right to be forgotten",
retention ceilings) in a system whose first principle is **append-only + immutable** ([data-model
§3.1](../data-model.md#31-append-only-clinical-event-log-source-of-truth)) and whose second is **never
erase, always overlay** ([identity §5.7](../identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)).
Naïvely this reads as a contradiction: deletion mutates the log, breaks the signature and hash chain,
and — fatally — **set-union sync resurrects the deleted row** from any sibling, parent, backup, or WORM
archive that still holds it. Case-mining dissolved the contradiction by reframing what erasure *is*.

Three observations did it.

- **Erasure is narrow and enumerated, not "delete the patient".** Most legal regimes exempt health
  records under a retention obligation from a general right to erasure during their statutory life — so
  the clinical content is largely *un-erasable by law* exactly when append-only would resist erasing it.
  The EU GDPR is one concrete example (cited as illustration, not as a standard Cairn adopts — Cairn is
  jurisdiction-agnostic, [§7](../security.md)): its right to erasure (Art. 17(1)) is disapplied by
  Art. 17(3)(b) where processing is required by a legal retention obligation, and by Art. 17(3)(c)
  (referring to Art. 9(2)(h)–(i)) for the provision of health/social care and public-health interest;
  Art. 17(3)(e) further preserves data for the establishment or defence of legal claims — the
  clinician's medico-legal cover. *(Article references verified against the GDPR text, June 2026.)*
  What remains erasable is a small set of triggers: subject requests for data with *no* retention basis;
  the retention **ceiling** (you must not keep *beyond* the period — a scheduled, clock-driven
  destruction); and surplus / illegitimate copies (the
  [ADR-0004](0004-dynamic-sync-scope-prefetch-not-authority.md) garbage-collection follow-on, an
  unwarranted break-the-glass copy, a node that left the mesh).

- **The patient's real adversary is discoverable, enforceable existence — not byte-presence.** The
  motivating clinical reality (EM physician, multiple health systems): the overwhelming majority of
  record-disclosure subpoenas are legally baseless fishing expeditions (family-court/divorce or
  insurers reinterpreting records to discredit a patient or deny a payout), rubber-stamped by
  overloaded courts. They can be contested and defeated, but that is substantial unpaid work most
  clinicians and hospital administrators skip — so records are handed over. *You cannot be compelled to
  produce what there is no record of.* The defence the patient wants is the paper one: shred the chart
  so there is nothing to subpoena, while the patient keeps their own copy to hand to a clinician they
  trust.

- **The clinician-vs-patient conflict is not zero-sum.** Clinicians want retention for their own
  medico-legal protection; patients sometimes want erasure (fear of disclosure, or stigma — a syphilis
  or mental-illness diagnosis). These look opposed only while we think in terms of *deleting data*.
  Reframed as *who holds a key*, both are satisfiable at once.

Two further constraints shape the answer. **Cairn is jurisdiction- and health-system-agnostic** ([§7
compliance-is-configuration](../security.md)): it cannot bake in any one retention rule, so it must
build *mechanism* spanning the worst-case extremes (indefinite retention ↔ complete erasure) and let
policy/UI select. And **deletion can never be *guaranteed*** — even paper leaves stray copies — so the
strongest honest claim is bounded, a direct corollary of acknowledged uncertainty ([§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)).

## Decision

**Erasure is the redistribution of key-custody, not the deletion of data. Cairn builds a
policy-neutral severity ladder of erasure mechanisms; which rungs are reachable in a deployment is
configuration, never a stance the system takes.**

### 1. Crypto-shredding is the deletion primitive

Clinical bodies are stored as **ciphertext under a per-unit data-encryption key (DEK)**; the envelope
([§3.5](../data-model.md#35-event-storage-model-hybrid-envelope)) stays plaintext (UUID, HLC, signature,
scope keys — identity/sync/matching must bind on these). To *erase* is to **destroy the DEK**, not the
row. The event row remains immutable, signature-over-ciphertext still verifies, the hash chain is
intact, and set-union sync still works — the row is simply permanent, keyless noise. This is the **only
deletion model compatible with append-only + WORM archival**: ciphertext may sit on write-once medico-legal
media forever while the key dies on erasable media elsewhere. It also makes **mesh-resurrection
harmless** — if sync later re-delivers an opaque row from an unreached node, there is no key and no
projection that references it.

### 2. Per-record encryption with a key-holder hierarchy — reserved from day one, off by default

The §3.5 envelope **reserves the shape now** (it cannot be retrofitted onto an append-only log without
re-encrypting history): a clinical body is *either* plaintext JSONB *or* ciphertext under a DEK
**wrapped for a set of key-holders** — `{node}` by default, optionally `{patient}` and/or named
`{clinicians}`. Default deployments still use whole-storage encryption ([§7](../security.md)); per-record
encryption and a patient-held key are an **opt-in capability reserved for special cases** (stigma-sensitive
episodes, the deniable-deletion rung below). Patient-as-key-holder is opt-in precisely because it trades
availability for confidentiality (a lost patient key = oblivion), which the default must not do.

### 3. The policy-neutral severity ladder

Cairn builds every rung; policy/UI gates which are offered, to whom, and with what authorization.

| Rung | Mechanism | Content | Reversible | Trace |
|---|---|---|---|---|
| 0 **Hide** | repudiation / reattribution overlay ([§5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows)) | retained, projection-filtered | yes | full audit |
| 1 **Sequester** | per-record encryption; key-holders = node (+ optional patient/clinicians); *policy-defined* safety-relevant metadata (e.g. interaction/allergy class) may remain; break-glass audited | sealed | yes (key-holder) | audited |
| 2 **Deniable deletion** | destroy institution's index + node DEK; **sealed copies escrowed to patient + chosen clinician(s)**; **no discoverable institutional record of existence** | unrecoverable *by the institution*; recoverable by a key-holder's consent | by consent only | **none** |
| 3 **Audited crypto-shred** | destroy all keys; immutable shred event records *existed → destroyed, basis Z*; opaque ciphertext remains | unrecoverable | no | proof-of-destruction |
| 4 **Best-effort oblivion** | shred keys *and* all known custodian copies | unrecoverable | no | declared best-effort |

Rung 0 (indefinite retention) and rung 4 (complete erasure) are the worst-case extremes the agnostic
design must span; the clinically-real cases live in the middle.

### 4. Rungs 2 and 3 pull opposite ways on the same log — and that is deliberate

- **Audited shred (3)** wants an **explicit immutable tombstone** — *"event X erased, basis Z"* —
  because that proves existence + lawful destruction, which is the clinician's medico-legal cover. For
  jurisdictions that *require* an auditable deletion trail.
- **Deniable deletion (2)** must leave **no tombstone at all** — a tombstone saying "event X was
  erased" itself proves event X existed, which is exactly what the patient needs gone. The institution
  must hold **nothing**, so it can honestly answer a subpoena "no record". The clinician's medico-legal
  cover **migrates** from an institutional audit trail (deliberately absent) to **their own retained
  sealed copy**, producible later by the patient's consent. Where a jurisdiction will not accept that
  contingency, rung 2 is simply not offered — rung 3 is used instead. This is policy selecting a rung,
  not the system taking a side.

### 5. The honest-erasure ceiling (normative)

The strongest claim Cairn ever makes is:

> **"To our knowledge, we have erased all copies in our existence."**

Both hedges are load-bearing and both are corollaries of acknowledged uncertainty
([§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)): *"to our
knowledge"* (offline nodes, old backups, and WORM cannot be confirmed) and *"in our existence"* (scoped
to this institution's node-set; sealed copies a patient or trusted clinician holds are intentionally
outside that boundary and unknown to the institution). The system never claims a guarantee it cannot
keep — an honest *"copies may persist"* beats a precise untruth *"permanently deleted"*.

## Consequences

**Easier / gained:**

- The append-only/immutability contradiction disappears: nothing in the clinical log is ever mutated;
  only keys die, on a tiny separate surface (the keystore). The whole erasure capability is confined to
  key-custody, keeping the safety-critical mutable surface minimal.
- The clinician-vs-patient conflict dissolves: redistributing key-custody satisfies retention *and*
  confidentiality without forcing either party to lose data.
- Crypto-shredding is the one deletion model that works on immutable/WORM archives and degrades
  gracefully across a partitioned mesh.
- Per-node DEK custody answers the long-standing "erase one node's copy, keep it elsewhere" granularity
  for free, absorbing the [ADR-0004](0004-dynamic-sync-scope-prefetch-not-authority.md) surplus-copy-GC
  follow-on.

**Harder / the bet:**

- A **keystore** becomes a safety-critical component with irreversible operations: an *accidental*
  shred is catastrophic data loss (founding principle 1's anti-data-loss). Key destruction needs the
  same gravity, authorization, and audit (for the audited rungs) as the erasure it effects, and keys
  must not be silently reconstructable from ordinary DB backups after destruction. Key granularity
  (per-event vs. per-episode key hierarchy) trades erasure precision against keystore overhead on
  **Pi-class** nodes ([ADR-0001](0001-fat-postgres-thin-daemon.md) latency budget) — to be measured in
  the same benchmark spike.
- Deniable deletion (rung 2) is only as strong as the mesh's reach at the time it is invoked, and
  shifts the clinician's protection onto a self-held copy + future patient cooperation. The honesty of
  the ceiling claim depends on faithfully surfacing what could not be confirmed.
- Rung 1's "safety-relevant metadata may remain while content is sealed" is *infrastructure*: Cairn
  provides the capability to seal a body while exposing a configurable metadata projection; **what
  metadata remains is a policy decision**, not a Cairn stance. It interacts with visibility-scope ↔
  sync-scope ([§11.8](../open-questions.md)) and pseudonymous/sanctioned care
  ([§5.6](../identity.md#56-pseudonymous-sanctioned-care)); the mechanism for *configuring* that
  projection is deferred there.

**How we'd know it's wrong:** if real deployments find the ladder's rungs don't map cleanly onto the
legal regimes they must satisfy — i.e. a jurisdiction needs an erasure semantics no rung expresses, or
the rungs force a policy choice the system was supposed to stay neutral on — then the *ladder*, not the
key-custody principle, needs another rung.
