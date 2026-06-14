# ADR-0006 — Visibility-scope vs. sync-scope: replication is not the confidentiality boundary; the safety projection and graded sensitivity

- **Status:** Accepted
- **Date:** 2026-06-14
- **Supersedes:** —

## Context

Former open question [§11.8](../open-questions.md): how do visibility-scoped link events
([identity §5.6](../identity.md#56-pseudonymous-sanctioned-care)) interact with sync scopes — *does a
sequestered/sensitive episode replicate to a node at all?* — together with the rung-1 follow-on left
open by [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md): *what policy-defined
safety-relevant metadata may remain visible while a body is sealed?*

The difficulty is that the single word **"scope"** has been carrying three independent decisions, each
resolved separately, and §11.8 is their collision point. Pulling them apart gives **four dials** that
deployed EHRs (and the open question) conflate:

1. **Replication** — does the (possibly sealed) row land on node N's disk? Owned by *sync scope*,
   which [ADR-0004](0004-dynamic-sync-scope-prefetch-not-authority.md) already ruled a **prefetch
   hint, not an authority** (a node may acquire anything it has legitimate need for).
2. **Decryptability** — does N hold a key-holder credential to open the body? Owned by *key custody*
   ([ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)).
3. **Body visibility** — does the body, or a signal derived from it, surface in a chart view or
   decision-support on N? Owned by *visibility scope* ([§5.6](../identity.md#56-pseudonymous-sanctioned-care)).
4. **Envelope-metadata exposure** — a newly sharp fourth dial. Crypto-shredding and sequestration seal
   only the **body**; the envelope ([§3.5](../data-model.md#35-event-storage-model-hybrid-envelope)) —
   `patient_uuid`, `event_type`, **scope keys (facility/department/encounter)**, author, `t_effective`
   — stays **plaintext by construction**, because identity, sync, and matching must bind on it. For a
   stigma-sensitive episode *the metadata is the sensitive fact*: "seen in the sexual-health clinic" is
   the whole disclosure; the sealed body adds nothing.

Two forces then collide. **ADR-0005 protects the body, but the plaintext envelope is the leak.** And
the obvious confidentiality reflex — *don't replicate the sensitive episode to that node* — runs head-on
into **availability/paper-parity** ([principle 5](../index.md#founding-principles-the-lens-for-every-decision),
[§1.2](../vision.md#12-the-paper-parity-test-normative)) and into ADR-0004: the very principle that the
chart must always assemble is what would pull a sensitive episode onto the wrong node, and withholding
the row destroys safety. The motivating clinical realities (EM physician): a patient's sealed
antiretroviral or psychiatric medication interacts with a drug a rural clinician is about to prescribe;
and a pregnancy termination the patient wants hidden still carries **Rhesus-sensitization** implications
a future antenatal clinician must act on. In both, *the safety signal must reach a node that cannot be
allowed to see the underlying fact*.

Cairn is jurisdiction-, culture-, and health-system-agnostic ([§7](../security.md),
[principle 9](../index.md#founding-principles-the-lens-for-every-decision)): **what counts as
confidential is itself contingent** — STI screening and HIV/hepatitis therapy are sensitive in most
cultures; pregnancy termination only in some; family-dispute and mental-health sensitivity is often a
subjective judgment of the patient or the clinician. So Cairn cannot bake in a confidentiality list any
more than it can bake in a retention rule.

## Decision

### 1. Replication is never the confidentiality boundary

A sensitive episode that carries any safety relevance **replicates unconditionally** (set-union,
mandatory); confidentiality is enforced **entirely** at the decryptability (dial 2), body-visibility
(dial 3), and envelope-abstraction (dial 4) dials, **never** by withholding the row. You cannot break
glass on, nor fire a safety warning about, content that never arrived; and the Rh-after-termination case
proves a maximally-sealed episode still owes a future clinician a signal. This *confirms* ADR-0004 from
the other side: **sync scope was never permitted to be a confidentiality mechanism, and §11.8 is the
case that proves why it must not be.** The answer to "does a sequestered episode replicate to a node at
all?" is **yes**.

### 2. The safety projection — a de-identified signal that survives the seal

When a body is sealed, the authoring node emits a **separate, de-identified safety projection** beside
the ciphertext: coarse safety **classes** (interaction class, allergy class, *Rh-sensitizing event*,
contraindication flags), a **severity grade**, and a pointer to the sealed event — but **not** the
specific agent, diagnosis, or sensitive scope keys. The safety **classes** are a mechanical projection of the body's **coded clinical fields** (a coded
drug's interaction class is a property of the code, not a judgment about the patient) — the same coded
fields ordinary decision-support already reads — so deriving them is a normal pre-seal projection step,
not a confidentiality inference over free text. The §3 sensitivity **grade** sets only *how coarse* the
emitted classes are. It replicates **in the clear** under broad (node-default) key custody, exactly like
an allergy, because it *is* the safety floor. Local decision-support on any node matches new
orders/context against these classes and fires a **severity-graded warning that names nothing**: *"⚠ Grade X interaction with confidential content —
break glass to view / discuss with the patient / document the decision."* This is the concrete
realization of the promise already written into [§5.6](../identity.md#56-pseudonymous-sanctioned-care):
*a sequestered episode joins the connected component, enabling interaction checking, without its
contents flooding every chart view.* It is partition-safe by construction — the warning travels with
the patient and is computed locally, no key required.

The projection's granularity is itself a **policy-configured disclosure-coarsening ladder** (precise
class → generic *"confidential medication, severity X"* → bare *"confidential content, break glass"*),
because a too-specific class re-identifies (*"interacts with antiretrovirals"* → HIV+). This ladder is
structurally the mirror of the ADR-0005 erasure ladder — a recurring **policy-neutral severity-ladder
pattern** ([principle 9](../index.md#founding-principles-the-lens-for-every-decision)).

**Safety-floor invariant:** the sensitivity grade controls the projection's *coarseness, never its
existence*. Even at maximum sensitivity the graded warning still fires (just maximally blurred). Secrecy
blurs the safety signal; it never extinguishes it.

### 3. Sensitivity is a graded, multi-source, append-only assertion stream

Sensitivity is not a boolean on the body; it is an **append-only stream of sensitivity-grade
assertions**, each with provenance, and the **effective grade is a projection** — *never merge, always
overlay*, the same shape as the identity link graph ([§5.1](../identity.md#51-linkage-layer-never-merge-always-link))
and the demographic per-field projection ([§4.2](../demographics.md#42-per-field-projection-policy)).
Because confidentiality is contingent, Cairn provides **three infrastructure pieces and nothing more** —
none alone suffices, and how they are combined is policy:

| Infrastructure piece | What Cairn provides | Why it is necessary |
|---|---|---|
| **Category blacklist** (automatic source) | a coded-category → default-grade map a deployment populates (STI screen, termination, HIV/HBV/HCV therapy, …); an event whose coded category hits the map *can* be auto-tagged | whitelisting is impossibly wide; a blacklist is the only tractable automatic path. The *map* is deployment configuration — Cairn ships the lookup mechanism, never the list |
| **Confidentiality grading system** | the graded sensitivity scale and its append-only assertion/projection machinery | confidentiality is not binary; a grade lets sealing rung (ADR-0005) and projection coarseness (§2) scale together |
| **Human editability** of tag/grade | the affordance for a patient or clinician to assert, raise, or (with authority) lower a grade, with provenance | subjective/cultural sensitivities the blacklist cannot know — divorce, family dispute (patient); domestic violence, mental health (clinician) |

**The workflow that combines them is policy, implemented at the UI layer**
([principle 9](../index.md#founding-principles-the-lens-for-every-decision)): whether an auto-tag from
the blacklist applies silently, requires clinician acceptance before it takes effect, or whether a
deployment runs manual-tagging-only, is a policy decision — Cairn enforces none of these, it only makes
all of them expressible. The grade drives **(i)** whether/how the body is sealed (which ADR-0005 rung,
which key custody) and **(ii)** how coarse the safety projection of §2 is. The effective grade is the
**highest standing assertion** (monotone by overlay); **declassification** — lowering a grade — is a new
*authorized* overlay (patient consent or policy authority), **never an erasure**, mirroring the
reattribution tiers ([§5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows)).

### 4. The envelope is not automatically safe — scope keys are abstractable

For a sensitive episode the plaintext **scope keys** can be the whole disclosure, so the *semantic*
scope key (which department/encounter) is **abstractable to an opaque "confidential-episode" routing
token**, with the real routing held under key. This has a self-reinforcing safety property: opacifying
the scope key **forces** the correct behavior — the sync prefetch predicate can no longer *select* on
it, so it degrades to "replicate every confidential token for this patient," which is exactly the
mandatory replication of §1. Identity/sync still bind on `patient_uuid` (immortal) and HLC ordering;
only the human-meaningful routing label is generalized. This is the §5.6 intersection made concrete and
is where the ADR-0005 rung-1 "policy-defined safety metadata may remain" follow-on is answered: *what
remains plaintext is the safety projection of §2; what is abstracted is the re-identifying envelope
metadata; both are driven by the §3 grade.*

### 5. Break-glass is audited key-*use*, and partition-honest

Break-glass — the escalation when the abstracted warning is not enough and the clinician needs the
specifics — is an **audited key-acquisition/use event**, structurally the mirror of the ADR-0004
acquisition trichotomy and **distinct from key-*destruction*** (erasure, ADR-0005):

- a **key-holder present** (named-key-holder clinician, or the patient present with their key) → local
  unseal;
- **carried with the patient** (the patient is a key-holder; their token escrows the DEK and travels
  with them — paper-parity-exact: the sealed section of the folder travels with the patient);
- from a **sibling/parent on reconnect**;
- **none reachable → honest disclosure**: *"sealed content exists here; the key is not present on this
  node."* The warning already fired (floor intact); only the specifics are unavailable here-and-now —
  acknowledged uncertainty + honest-assembly-state ([§6.2](../sync.md#62-consistency-model)), no new
  principle required.

Break-glass is itself an append-only audited access event (the [§7](../security.md) "break-glass with
mandatory retrospective audit" primitive, reused), syncing upstream at high priority and recording
who/when/basis/patient-consent-status. **The architecture always provides break-glass; whether the UI
offers it, and what authorization it demands, is policy**
([principle 9](../index.md#founding-principles-the-lens-for-every-decision)).

## Consequences

**Easier / gained:**

- §11.8 dissolves into **existing primitives plus two explicit constructs** — the *safety projection*
  (§2) and the *sensitivity-grade assertion stream* (§3) — with no new architecture. Replication,
  key-custody, visibility, break-glass, and append-only-overlay projections all pre-exist.
- The §5.6 promise (interaction-checking without contents flooding the chart) acquires a concrete
  mechanism instead of remaining an assertion.
- The clinician-vs-patient tension is again **positive-sum**, as in ADR-0005: the patient gets a sealed
  body and an abstracted public signal; the clinician gets the safety warning and a consented
  break-glass path; nobody loses data.
- ADR-0004 is confirmed and strengthened: sync scope is definitively *not* a confidentiality control,
  removing a class of "just don't replicate it" mis-designs.

**Harder / the bet:**

- **The seal-time projection seam.** The safety projection (§2) is computed from the body's coded
  fields just before sealing — mechanical, but still the one code path that reads the body en route to
  ciphertext, so it is safety- and confidentiality-critical and must obey the §9 blast-radius rule. Its
  quality is also only as good as the body's coding: an uncoded or free-text-only sensitive body yields
  a weaker safety class (a known limit, not a regression — paper yields nothing at all).
- **The abstraction can itself re-identify.** The disclosure-coarsening ladder is the mitigation, but
  the *default* coarseness a deployment ships is a real policy responsibility, not a Cairn default.
- **Specifics need a node that can decrypt.** The universal signal is the *abstract*; obtaining the
  concrete interacting agent requires break-glass where the DEK is reachable. During a total partition
  with no key-holder present, the clinician gets an honest "sealed, key absent" — strictly better than
  paper (which gives silence) but not omniscient.
- **The sensitivity-grade projection is one more safety-critical in-DB projection** on the Pi latency
  budget ([ADR-0001](0001-fat-postgres-thin-daemon.md)), and key-holder reach now also bounds how often
  break-glass can actually deliver specifics — both to be exercised in the benchmark spike.

**How we'd know it's wrong:** if real deployments find that an abstracted warning is operationally
unusable (clinicians reflexively break glass on every "confidential content" banner, collapsing the
seal into theater), or that no coarsening rung both fires the right safety signal *and* withholds the
re-identifying fact for some real category — then the *safety-projection abstraction*, not the
replicate-everything decision, is what needs rework. The replication-is-not-the-boundary rule is
load-bearing and shared with ADR-0004; the projection mechanism is the replaceable part.
