# HANDOVER — Cairn

**Session date:** 2026-06-14 (spec bumped to **v0.9**)
**Status of this file:** Working scaffolding, not a source of truth. Disposable — regenerate
at the end of each working session. If this file ever disagrees with the canonical documents,
the canonical documents win.

---

## Read these first (the durable state)

The real project state lives in these documents. This handover points at them; it does not
restate them. Repository layout:

- **`docs/spec/`** — the canonical architecture spec, **one file per aspect**. Start at
  **`docs/spec/index.md`** (mission prose + document map), then read aspect files / jump via the map.
  - **`docs/spec/decisions/`** — the **ADR log**: the *why* behind settled decisions. Numbered,
    dated, **immutable** (reversal = a new superseding ADR). **Read the relevant ADR before
    reopening a settled question.** Pre-ADR history (v0.1→v0.6 changelogs) preserved in
    `decisions/0000-pre-adr-changelog-v0.1-v0.6.md`.
  - No filename version suffixes / in-file changelogs; git is the line history; spec version in
    `index.md`. HTML is generated, not committed:
    `uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build` (config `mkdocs.yml`).
- **`docs/principles/`** — statements of project principle / governance.
  - `STEWARDSHIP-OF-THE-NAME.md` — the "name belongs to the mission" governance commitment.
- Root **`README.md`** — mission, founding principles, eventual goal, project identity (GitHub
  shopfront; the same mission prose also lives canonically in `docs/spec/index.md`).

Everything below is the stuff that lives *between* those documents and would otherwise be lost.

---

## Resolved 2026-06-14 — §11.8 visibility-scope ↔ sync-scope (now written into spec v0.9)

Case-mined **§11.8** (does a sequestered episode replicate to a node at all?) plus the **rung-1 metadata
follow-on left open by ADR-0005**. It dissolved into existing primitives + two explicit constructs; no
new architecture, no new founding principle.

- **§11.8 RESOLVED → [ADR-0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md),
  [identity §5.9](spec/identity.md) (canonical home), with pointers from [sync §6.4](spec/sync.md),
  [security §7](spec/security.md), [data-model §3.5](spec/data-model.md), [index principle 9](spec/index.md).**
- **The core ruling (the user's): replication is *never* the confidentiality boundary.** Because there is
  almost always a patient's-best-interest case for the treating clinician to break glass with consent
  (the clincher: a sealed pregnancy termination still implies **Rh-sensitization** a future antenatal
  clinician must act on), a safety-relevant sensitive episode **replicates unconditionally**.
  Confidentiality lives entirely in **key-custody + body-visibility + envelope-abstraction**, never in
  withholding the row. This *confirms* ADR-0004 from the other side (sync scope was never an access control).
- **The word "scope" was hiding four dials**: replication (always on), decryptability (gated),
  body-visibility (sealed), and a newly-sharp fourth — **envelope-metadata exposure** (the plaintext
  envelope's scope key `department = sexual-health` is itself the disclosure; ADR-0005 only seals the body).
- **Two new explicit constructs:** (1) a **safety projection** — a de-identified, severity-graded signal
  (*"⚠ Grade X interaction with confidential content — break glass"*) **mechanically projected from the
  body's coded fields**, replicated in the clear like an allergy, naming nothing; makes the §5.6 promise
  concrete; partition-safe. (2) **Sensitivity as a graded, multi-source, append-only assertion stream**
  (effective grade = projection). **Safety-floor invariant:** the grade controls the signal's *coarseness,
  never its existence* — secrecy blurs the safety signal, never extinguishes it.
- **Infrastructure, not policy (principle 9):** Cairn ships exactly three pieces — a **category blacklist**
  (coded-category → default grade; whitelist is impossibly wide), the **confidentiality grading system**,
  and **human editability** of tag/grade (patient request / clinician judgment). *Whether a blacklist
  auto-tag applies silently, needs clinician acceptance, or is manual-only is a UI-layer policy decision*
  Cairn makes expressible but never enforces.
- **Two findings worth carrying:** the **semantic scope key is abstractable to an opaque "confidential-
  episode" token** — and doing so *forces* safe behavior (the sync prefetch predicate can no longer
  select, so it falls back to replicate-everything-for-this-patient). And the **policy-neutral
  severity-ladder pattern recurs** (erasure ladder → now a disclosure-coarsening ladder) — a structural
  motif, not yet elevated to anything.
- **Break-glass** is audited key-*use* (distinct from key-*destruction*/erasure), mirroring the ADR-0004
  acquisition trichotomy, partition-honest (*"sealed content exists here; the key is not present"*).

**Open follow-on:** the seal-time projection seam (the one code path that reads the coded body en route to
ciphertext) is safety/confidentiality-critical → a §9 blast-radius concern when implementation begins; and
projection quality tracks coding quality (uncoded body → weaker class, still better than paper's nothing).

---

## Resolved 2026-06-14 — §11.5 erasure/GDPR (now written into spec v0.8)

Case-mined **§11.5 (tombstones / retention / GDPR erasure)** — the sharpest standalone open problem —
from the user's real subpoena experience (an EM physician who contested *every* disclosure subpoena and
had each waived or restricted; most clinicians don't, so records leak). It dissolved, and added a ninth
founding principle along the way.

- **§11.5 RESOLVED → [ADR-0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md),
  [data-model §3.8](spec/data-model.md), [security §7.1](spec/security.md).** **Erasure is the
  redistribution of key-custody, not the deletion of data.** The clinical log is never mutated; the
  deletion primitive is **crypto-shredding** — destroy a body's DEK and the immutable, signature-valid,
  sync-safe row becomes keyless noise (the only deletion model compatible with append-only + WORM;
  mesh-resurrection of an opaque row is harmless). Exposed as a **policy-neutral severity ladder**:
  *hide → sequester → deniable sealed-escrow deletion → audited crypto-shred → best-effort oblivion*.
  Cairn builds the rungs; **which are offered is policy/UI configuration** — it facilitates conflicting
  legal/health-system requirements without taking sides.
- **The clinician-vs-patient conflict became positive-sum.** Clinicians want retention (medico-legal
  cover); patients sometimes want erasure (subpoena fishing-expeditions; stigma). Reframed as *who holds
  a key*, both are satisfied: the deniable rung destroys the institution's discoverable index + node key
  and escrows sealed copies to the patient + chosen clinician(s), so **the institution holds nothing**
  and can honestly answer a subpoena "no record" — the clinician's cover migrates to their own retained
  sealed copy, producible by consent.
- **Per-record encryption with a key-holder hierarchy including the patient** is reserved in the §3.5
  envelope **from day one** (can't retrofit onto an append-only log) but is **off by default** (a
  patient-held key trades availability for confidentiality).
- **Two principle-level additions:** a **9th founding principle — "policy-neutral infrastructure"**
  (Cairn provides mechanism, never policy; written into [index §principles](spec/index.md), [vision
  §1.8](spec/vision.md), `CLAUDE.md`); and a **corollary of the 4th** — *deletion is best-effort and
  declared, never guaranteed*. The honest ceiling, in the user's words: **"to our knowledge, we have
  erased all copies in our existence."**

**GDPR was used only as an illustrative example** (article references Art. 17(1), 17(3)(b)/(c)/(e),
9(2)(h)–(i) were **verified by web search**, June 2026, not asserted from training) — Cairn stays
jurisdiction-agnostic.

**Open follow-ons explicitly deferred:** the concrete *policy-defined* safety-relevant metadata that may
remain in rung-1 sequestration (→ §11.8); key granularity (per-event vs per-episode hierarchy) and
keystore Pi-cost (→ the Pi-benchmark spike); the deniable rung's interaction with mesh reach.

---

## Resolved 2026-06-14 — §11.3 dynamic sync scopes (spec v0.7, here for the trail)

Brainstormed **§11.3** from a real ED→ICU transfer case. It dissolved, and spun off the fourth governing
principle.

- **§11.3 RESOLVED → [ADR-0004](spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md),
  [sync §6.4](spec/sync.md).** **Scope is an administrative *prefetch hint*, not an authority.** Nobody
  owns the record — it's the sum of autonomous signed parts, assembled when it can be. A transfer
  triggers *acquisition* (sibling-on-LAN / carried-with-patient / from-parent-on-reconnect), not
  reassignment; the parent ratifies+audits, never gates. Surviving requirement: **honest assembly-state
  disclosure**. (The surplus-copy GC follow-on it spun off is now absorbed by §11.5 / ADR-0005.)
- **4th governing principle "Acknowledged uncertainty"** + **bitemporal time** →
  [ADR-0003](spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md),
  [data-model §3.6/§3.7](spec/data-model.md). `t_recorded` (HLC, objective, the **ceiling**) vs.
  `t_effective` (author-asserted, freely backdatable). Clash detection **flags, never resolves**.

---

## Resolved 2026-06-13 (spec v0.6 — here for the trail)

The **"Postgres-intelligence" cluster** (§11.1 / §11.2 / §11.11) is **resolved** as **"Fat Postgres,
thin Rust daemon"** — full rationale in **[ADR-0001](spec/decisions/0001-fat-postgres-thin-daemon.md)**
(written into `spec/topology.md` §2, `data-model.md` §3.5, `sync.md` §6.1, `language-substrate.md` §9.4):

- **§11.2 storage (→ §3.5):** hybrid event envelope — typed/normalized columns where invariants,
  identity, sync, and matching bind; **Cairn-native JSONB** for clinical bodies; **FHIR is a façade
  only**, never the storage model. *(As of v0.8, the JSONB body slot is also encryption-capable — §3.8.)*
- **§11.11 merge boundary (→ §9.4):** structural invariants + identity event algebra + **all projections
  in Postgres** (trigger-maintained, `AFTER INSERT`); the Rust daemon ships/applies but **carries no
  merge logic**; the probabilistic matcher stays **Python and advisory**. Per-projection pgrx escape
  hatch on measured Pi-performance need ([ADR-0002](spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)).
- **§11.1 sync backbone (→ §6.1):** **build** a thin custom Rust service on Postgres logical decoding;
  **borrow** pgactive/SymmetricDS patterns, **do not depend** on them.

**The load-bearing bet to validate first:** that trigger-maintained in-DB projections + the identity
algebra stay cheap enough on **Pi-class hardware** to keep chart reads local and fast (the §1.2
paper-parity floor). The designed first spike is a **Raspberry-Pi-5 benchmark harness** (rural-clinic
profile, low concurrency; measure single-op projection-maintenance and chart-read latency; threshold =
beat "grab the paper chart"). Mitigation ladder if slow: PL/pgSQL → **pgrx (in-DB Rust)** → external
Rust. *This spike is the go/no-go on the approach.* **(v0.8 adds a second thing the spike should measure:
keystore cost / key granularity for crypto-shredding — see ADR-0005.)**

---

## Decided in conversation, NOT yet written into the documents

1. **Governance / CONTRIBUTING document** is identified as the next principles document to write, but
   does not exist yet. STEWARDSHIP-OF-THE-NAME.md is intended for inclusion in it.
2. **Status line:** README and spec both say "specification / architecture phase." Still accurate today
   — flip when implementation begins.

*(Reference — already written: name is **Cairn** / repo **cairn-ehr**; domains `cairn-ehr.org` canonical
+ `cairn-ehr.com` defensive redirect, both registered, reflected in STEWARDSHIP-OF-THE-NAME.md.)*

---

## Time-sensitive (do soon, before squatters do)

- **Package-registry namespaces — DONE (2026-06-14).** `cairn-ehr` reserved-name placeholders
  (v0.0.0, AGPL-3.0-only) **published** to PyPI, crates.io, and npm (`@cairn-ehr` scope). The bare name
  `cairn` was already taken on all three, so `cairn-ehr` is the canonical package name. Stub sources in
  `packaging/`. Domains held. **GitHub `cairn-ehr` org created, both repos transferred** in
  (`cairn-ehr/cairn-ehr`, `cairn-ehr/cairn`); personal `hherb/…` URLs redirect. Moving the org under a
  stewarding legal entity is the parked governance question.

---

## Open questions / where we'd pick up

Spec §11 lists the remaining open questions (1, 2, 3, **5**, **8**, and 11 now struck-through/resolved).
With §11.8 gone, **§11.9 (armed write-context)** is now the sharpest single problem — and it pairs
naturally with §11.12 (authentication vs. paper-parity), both being point-of-care possession/identity
problems.

**The recurring menu** when resuming (pick one):
- **§11.9 armed write-context** — concrete possession-semantics design ([identity §5.8](spec/identity.md))
  passing paper-parity at ED pace: "picking up a chart" must cost ≤ its paper equivalent (~seconds, zero
  cognitive overhead) without degrading into reflexive click-through. The sharpest remaining problem.
- **Write the GOVERNANCE / CONTRIBUTING document** (folding in STEWARDSHIP-OF-THE-NAME.md).
- **Define the Pi-benchmark spike** in enough detail to be the first implementation task (now validates
  both the ADR-0001 projection cost *and* the ADR-0005 keystore/crypto-shred cost).
- **Polish a non-developer landing page** for the generated site (frontend-design work; draft plans
  already exist under `docs/superpowers/`).
- More clinical **case-mining** — the most productive mode so far: the event-overlay + key-custody
  primitives have absorbed every case raised without new architecture; continuing to stress-test is
  high-value.
- Other still-open §11 items: schema migrations across offline nodes (§11.4), attachment strategy
  (§11.6), locale-pluggable matcher comparators (§11.7), notification economy (§11.10), authentication
  vs. paper-parity (§11.12).

---

## Parked (deliberately not decided yet — don't re-litigate without reason)

- **Stewarding legal entity & jurisdiction.** Options floated: German Stiftung / gemeinnütziger Verein,
  US 501(c)(3), or an umbrella (e.g. Software Freedom Conservancy or a health-specific foundation).
  Deferred until the project has momentum and funding/adoption geography is clearer.
- **Formal trademark / wordmark registration.** Principle recorded now (stewardship doc); the legal
  instrument deferred until there is enough substance to be worth capturing.

---

## Working context for whoever resumes

- The user is a senior physician with an interest in ML / AI / health IT; codes mostly in Python. Brings
  real ED and hospital experience from several nations and health systems — case-mining sessions are
  unusually productive. (Founder of GNUmed, an early FOSS Postgres EHR; instincts are high-signal.)
- The project's founding motivation is explicitly **anti-capture / anti-vendor-lock-in**, rooted in the
  user's experience of government EHR committees being sabotaged by commercial interests. Decisions
  consistently favour the mission over convenience; treat that as the tie-breaker.
- **Nine founding principles** now run through everything ([index.md](spec/index.md)); the **first four**
  are the lens checked before any new design choice: **(1)** append-only + causal ordering; **(2)**
  identity is a claim, never a fact (never merge/erase, always link/overlay); **(3)** paper-parity;
  **(4)** acknowledged uncertainty (incl. the new corollary *deletion is best-effort and declared*). The
  rest: availability-over-consistency, fractal topology, vendor independence, safety-critical-logic-in-
  Rust/DB, and the new **(9) policy-neutral infrastructure** (mechanism, never policy).
