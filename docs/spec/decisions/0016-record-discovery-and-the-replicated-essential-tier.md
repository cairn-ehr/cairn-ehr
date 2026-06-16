# ADR-0016 — National-scale record discovery: the replicated essential-state tier and federation admission

- **Status:** Accepted
- **Date:** 2026-06-16

## Context

Cairn is meant to scale to a nation (≥10⁸ people) or larger, where **no node holds the whole
population's records** and the smallest autonomous node is a Pi-class clinic
([§2](../topology.md)). A patient who has *never been seen in this region* presents at a small,
under-resourced clinic. The clinic does *search-before-create* ([§5.3](../identity.md#53-registration-classes))
and finds nothing locally; its parent hub has never seen the patient either. **How does the clinic
learn that a record exists elsewhere, and how does it request it?**

The existing primitives carry everything *after* "a candidate was found" but not the first contact:

- **Identity-as-claim** ([§5.1](../identity.md#51-linkage-layer-never-merge-always-link)): the clinic
  correctly mints a **new local UUID** (it cannot know the prior one); discovery's job is never to
  adopt a chart but to surface a candidate so the matcher *proposes* a **`link`** — feeding the
  existing closed algebra ([§5.7](../identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)).
  No new identity primitive is needed.
- **Acquisition is a prefetch hint, not an authority** ([§6.4](../sync.md#64-scope-is-a-prefetch-hint-not-an-authority),
  [ADR-0004](0004-dynamic-sync-scope-prefetch-not-authority.md)): once a link is confirmed, fetching
  the record is already specified — sibling / patient-carried / parent, INSERT-only set-union,
  audited break-glass. It only needs a *source*.
- **Matching follows topology** ([§5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)):
  *"cross-facility at the lowest tier that sees both registrations (typically the hub)."*

The crack is precisely there: §5.2 assumes **a tier that has seen *both* registrations.** For a
first-contact patient with no regional footprint, the lowest common ancestor is *the nation* — which
cannot be a fat index a clinic queries, and the clinic cannot hold a national index. Three forces
shape the decision:

- **The conventional answer is the capture surface the mission forbids.** A national **Master Patient
  Index** (a single authority that knows who-and-where for the whole population) is the richest
  surveillance and lock-in target imaginable — exactly what [principle 7](../index.md#founding-principles-the-lens-for-every-decision)
  exists to refuse. Discovery must work with **no central master index and no real-time dependency on
  one.**
- **Patient-carried identity fails in practice.** Cards are forgotten, app logins fail at the point of
  care, the unconscious patient carries nothing. A *memorable* national identifier (Norway's
  *personnummer*) is best-of-breed but still not universal (children, dementia, non-residents, the
  uncommunicative). Discovery **must not depend on the patient carrying or recalling anything.**
- **The existence-disclosure surface is unavoidable** (the clinician working the case ruled this
  directly): to find the just-arrived patient's allergy list you must learn *that a record exists
  somewhere*, which is itself a disclosure. The resolution is not to eliminate it but to **bound it to
  accountable custodians** — see federation admission below.

A real failure mode sharpened the design (the clinician's *Sildenafil* case): a sporadically-taken,
privacy-sensitive medication the patient **will not disclose**, yet giving nitrates for chest pain on
top of it can kill them. The essential safety dataset therefore cannot be a fixed schema list, and it
must reach the ED *without* depending on disclosure at the point of care or on revealing the drug.

## Decision

The question **dissolves into existing primitives composed with one new replication tier**, plus a
hard dependency on a new governance spec. **No new founding principle.** Canonical homes:
[sync §6.7](../sync.md#67-record-discovery-and-the-replicated-essential-state-tier) (the tier and the
discovery mechanism), [identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)
(discovery feeds the matcher; national ID as accelerator), with the confidential-essential case tying
into [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope).

1. **Two phases of opposite mathematical character.**
   - **Phase 1 — fuzzy *identity* discovery** (*"does a record probably exist anywhere for a person
     like this?"*) is **irreducibly the matcher's problem** — fuzzy demographics, locale comparators
     ([§5.13](../identity.md#513-locale-pluggable-comparators-the-matcher-extension-point)), the
     false-merge ≫ false-split asymmetry. **You cannot content-address a human**, which is *why* the
     matcher exists and why [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)'s clean
     content-addressing does **not** solve identity discovery.
   - **Phase 2 — exact *part/locator* discovery** (*"given a UUID, which nodes hold its events?"*) **is**
     content-addressable — a tracker/DHT keyed by the immortal patient UUID, the same self-verifying
     multi-source swarm-fetch shape as the [§6.6](../sync.md#66-attachments-the-lazy-byte-tier) byte
     tier. Easy once Phase 1 has produced a confirmed UUID.

2. **A replicated *essential-state* tier — current state, not history.** Cairn defines a third
   replication volume between *"sync everything"* and *"scope it"*: a deliberately tiny,
   replicate-to-all-federated projection of each person's **essential safety set** (key demographics +
   national/local identifiers, active allergy list, active medication list, problem list / PMH, a
   code-status/advance-directive flag, current-care pointer). This is
   [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)'s **reference-eager, byte-lazy**
   pattern generalized from attachments to **patient existence**: the cheap *"someone like this may
   exist over there"* hint and the essential snapshot replicate widely; the **full longitudinal record
   and attachments stay scoped and lazy** ([§6.4](../sync.md#64-scope-is-a-prefetch-hint-not-an-authority)),
   fetched on legitimate need *after* a match.
   - **The load-bearing boundary (the easy-to-get-wrong footgun): the essential tier carries
     current-*state* changes, never transaction history.** Real-system data shows ~77% of dispensed
     prescription items are *repeats* that do not change the current list (England NHSBSA: ~21
     items/person/yr, of which only ~3.9 are non-repeat "acute" — Petty 2014). Dispensing history,
     observations, vitals, notes, and labs belong to the scoped/lazy full record; the essential tier
     replicates only **start/stop/change of an essential item.** This is the line that keeps the tier
     affordable.

3. **Discovery becomes a *local* matcher query, not a network operation.** Because every federated node
   holds the essential snapshot + summary, first-contact discovery runs the
   [§5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)
   matcher **locally and offline** against data already present — partition-proof, no online round-trip,
   and crucially **no broadcast of *who* is being looked for** (the clinic tests candidates against the
   summary it holds). A hit is an ordinary middle-band candidate → human confirmation → `link` → the
   [§6.4](../sync.md#64-scope-is-a-prefetch-hint-not-an-authority) acquisition pulls the full record from
   the holders found in Phase 2. No summary or a missing comparator degrades honestly to *"no history
   available,"* the [§5.4](../identity.md#54-unidentified-registration-john-doe-baked-into-the-root)
   unconfirmed trust state — which is exactly paper-parity (a paper clinic has nothing on the interstate
   patient either).

4. **"Essential" is a graded, multi-source, append-only flag — not a fixed list.** Predefining the
   safety minimum is impossible (the *Sildenafil* case). So Cairn ships the **mechanism** (an
   *essential / safety-relevant* dimension on an item), policy ships a **default pre-label pack** (e.g.
   PDE5 inhibitors flagged for the nitrate interaction, anticoagulants, insulin), and **any accountable
   contributor may tag an item essential** — prescriber, dispensing pharmacy, downstream clinician, or
   patient. By [principle 4](../index.md#founding-principles-the-lens-for-every-decision), **when unsure,
   err toward essential** (over-inclusion is the safe side, mirroring false-split ≫ false-merge). The
   flag is the same shape as the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
   sensitivity stream — graded, multi-source, append-only, effective value a projection.

5. **The confidential-essential case composes with the safety projection — two sub-tiers.** An item can
   be both **essential** *and* **confidential** (the *Sildenafil* case: safety-critical, privacy-
   sensitive, undisclosed). It resolves on the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
   mechanism with no new construct:
   - The **de-identified safety projection** (interaction class + severity, *naming nothing*) replicates
     broadly in the essential tier — *"⚠ nitrate-contraindicated agent present — break glass."* That
     projection **is** the clinically actionable fact (don't give nitrates); the clinician needs no name
     to make the right call. Over-inclusion here is cheap and safe.
   - The **identified essential item** (the drug, dose, dates) replicates but stays **sealed** under
     normal key-custody ([§3.8](../data-model.md#38-erasure-and-key-custody)); the name requires
     legitimate-need / audited break-glass. So the patient who would rather not disclose is *still* kept
     safe, without being outed in the chart, and **without depending on disclosure at the point of
     care** because the item was flagged upstream.

6. **A national/memorable identifier is a deterministic accelerator, never a dependency.** Where one
   exists (Norway *personnummer*, a Medicare number) it is a strong [§5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)
   deterministic-tier key and the best blocking key in the summary → instant high-confidence links.
   Where it does not (children, dementia, non-residents, the uncommunicative), the fuzzy matcher over
   the local essential mirror still works. Cairn **uses** such an ID as a high-value key but never
   **mandates** one (mandating it would re-create the capture surface). A patient-carried token is a
   welcome accelerator, explicitly **not relied upon.**

7. **Federation admission bounds the disclosure surface (new dependency, separate spec).** Replicating a
   nation's essential set among nodes is lawful and safe only because **every node holding it is a
   contracted, accountable custodian**: to join the mesh a node must present **proof of health-system
   participation + an enforceable privacy contract**; absent that, it may run fully (offline-first,
   single-node) but **the federation will not exchange data with it.** This is admission control for the
   sync mesh, sibling to [GOVERNANCE](../../principles/GOVERNANCE.md) and
   [Stewardship of the Name](../../principles/STEWARDSHIP-OF-THE-NAME.md), and is the mechanism that makes
   the unavoidable existence-disclosure tolerable (disclosure is to vetted custodians under enforceable
   law, not the open internet; granularity is **region**, never a named clinic). It is specified
   separately as **Custodian & Federation Admission** ([§11](../open-questions.md)) and is a hard
   prerequisite for this tier in any cross-entity deployment.

8. **Sizing (validated against real-system data, not assumed).** The essential set is a few tens of
   signed events per person at the spike-measured **~494 B/event** (+ ~100 B plaintext twin ≈ ~600 B):
   | Tier | Per person | × 100M | Verdict |
   |---|---|---|---|
   | Discovery summary (blocking keys + region pointer) | ~4–16 B | **~0.4–1.6 GB** | trivial on a Pi |
   | Essential set (central) | ~25 KB | **~2.5 TB** | a commodity 4 TB SSD; range 1.2–5 TB |
   | Full longitudinal + attachments | MB–GB | petabytes | scoped + lazy, never broadcast |

   At an essential-set **state-change churn of ~5–10/person/yr** (central; new/changed meds ~3.9
   acute/yr + ~0.1–0.3 new diagnoses/yr + rare allergy/demographic edits), a worst-case full-mirror node
   ingests **~300–600 GB/yr ≈ ~75–150 kbit/s sustained** — **~1 % of a mediocre Starlink link**, with
   1–2 orders of magnitude of headroom even at a sicker population's ~25/yr. The binding constraint is
   **post-partition catch-up bursts**, already governed by the preemptible own-thread byte-tier
   discipline proven on the real Cape York ↔ Dorrigo link ([ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)).
   Underlying distributions (heavily right-skewed): ~40 % of the population on zero regular medications,
   ~60 % with zero documented drug allergies, multimorbidity in most over-65s, a small elderly cohort
   carrying the load — corroborated across England (NHSBSA, CPRD/CFAS), Scotland & Sweden (polypharmacy
   registries), the Barnett 2012 multimorbidity study, Zhou 2016 (allergy, n≈1.77M), and US MEPS/NHANES.

9. **Blast radius ([§9](../language-substrate.md)).** **Fit-for-purpose** (Python, advisory): the
   discovery summary build, the local matcher query, and the candidate ranking — a defect yields a bad
   *proposal* a human reviews. **Safety-critical** (in-DB / Rust): the essential-tier replication
   predicate and the **current-state projection seam** (the one path that decides what enters the
   replicate-to-all tier — get it wrong and either a safety fact is withheld or transaction history
   floods the mesh), the **essential-flag → safety-projection seam** (the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
   seal-time seam reused), and **federation-admission credential verification** (security plane). The
   recurring seam motif.

## Consequences

- **Easier:** first-contact discovery works **offline, with no central index and no patient token** —
  the unconscious cardless interstate patient is matched against a local mirror; the ED gets the
  allergy/med/problem snapshot and the de-identified nitrate warning instantly; the full record follows
  lazily on confirmation; a national ID, where present, makes it near-instant.
- **Harder / new surface:** a new replication tier with a **current-state-vs-history boundary** that must
  be enforced precisely; a signed regional **existence-summary** artifact (build + gossip + staleness
  handling); the **essential flag** as new graded provenance plus a tagging affordance and a default
  pre-label pack; and a hard dependency on the **Custodian & Federation Admission** spec, which does not
  yet exist.
- **The bet:** that the essential set is small enough to replicate nation-wide on commodity hardware and
  a mediocre satellite link (the sizing says yes, with large margin), that the current-state boundary
  holds churn to single digits, and that bounding disclosure to contracted custodians at region
  granularity is an acceptable, mission-aligned price for finding the patient who carries nothing. We
  would know it is wrong if essential-tier churn or volume runs an order of magnitude over these
  estimates in a real deployment, or if the federation-admission contract proves unenforceable in a
  jurisdiction Cairn must serve.
- **Policy-neutral ([principle 9](../index.md#founding-principles-the-lens-for-every-decision)):** Cairn
  ships the tier mechanism, the summary, the matcher, the essential-flag dimension, and a default
  pre-label pack; *which* items a deployment pre-labels essential, whether tagging is silent / clinician-
  confirmed / manual, whether a node holds the full essential set or summary-only, and the federation-
  admission criteria, are policy.
