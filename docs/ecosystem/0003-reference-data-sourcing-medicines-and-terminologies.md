# Ecosystem evaluation — external reference-data sourcing (medicines, and disease/injury terminologies)

**Date:** 2026-06-19
**Status:** Evaluation. Spec unchanged; no ADR minted. Captures sourcing research for the **reference-data
service tier** — a *separable* service consumed by a Cairn node, not part of the wire core. Medicines sourcing
(§1–§7) and disease/injury concept identifiers (§8) are both written up; a short list of human-verify items
(licence clauses behind 403 walls) remains in §7 and §8.4. To be revisited before any of this is committed to.
**Subjects:** open/government drug-reference feeds (WHO INN, RxNorm, DailyMed/openFDA, TGA ARTG, PBS) and
disease/injury classifications (ICD-10/11, ICPC, alternatives) — evaluated for **license compatibility with a
freely-redistributable AGPL-3.0 node**.

> [!NOTE]
> This is an **ecosystem evaluation** — not architecture and not a decision. It records *what reference-data
> sources exist, on what licence, and what we concluded about fit*. The reference-data service is an **optional
> external tier**: a bare Cairn node records medication as `INN-anchored substance UUID + dose + amount + units +
> formulation enum` and disease as a stable concept code, and is a complete EHR **without** any bundled
> commercial drug or terminology database. Nothing here sits on the inter-node path, so a licence-encumbered
> source can never contaminate interoperability or the safety floor — it simply doesn't attach. That posture is
> exactly what founding principle 12 (*uniform core, plural edges*) and the language-substrate rule
> ([§9](../spec/language-substrate.md)) require of an advisory, fit-for-purpose tier.

---

## 1. Framing — why the architecture makes this tractable

The decisive design choice (the user's, ratified in conversation 2026-06-19) is that **the EHR does not
foreign-key into an integrated proprietary product database.** A medication is recorded as:

- a **substance identity** anchored on the **WHO INN** (International Nonproprietary Name) as the stable concept
  UUID — *not* a free-text brand name that is spelled differently across sources and drifts over time;
- plus **dose, amount, units**, and a **formulation enum** (tablet / ointment / solution / …) stored explicitly.

This matters because **the proprietary parts of the drug-data world are precisely the integrated, value-added
product databases** (First Databank, Micromedex, Multum, AMH, DrugBank-commercial). By never depending on one
for core clinical recording, Cairn needs only *reference and decision-support layers*, and those can be
assembled from public-domain government feeds. The reference-data service is therefore a **two-tier** problem:

- **Tier 1 — generic / substance-level**, jurisdiction-independent (the INN-anchored substance dictionary, plus
  classification and decision-support reference).
- **Tier 2 — packaging / product / country-specific**, starting with Australia.

The single hardest gap — unchanged in 20 years since the user's drugref.org work — is **drug–drug interactions**
(§4). The mission tie-breaker throughout: **license-and-redistribution terms win over convenience.** A source
that is free-to-*use* but `NonCommercial` or `NoDerivatives` is **not** freely redistributable and cannot be
bundled into the AGPL artifact; at best it is an operator-supplied, node-local, separately-licensed plug-in.

> [!IMPORTANT]
> **The recurring trap is the licence, not the price.** Four flavours of "free" are *not* AGPL-bundleable:
> `NonCommercial` (CC BY-NC — DrugBank, WHO EML), `NoDerivatives`, member/affiliate-gated (SNOMED CT and all its
> national extensions incl. AMT), and registration-walled-but-otherwise-open (UK dm+d via TRUD). Each is flagged
> 🚩 below.

---

## 2. Tier 1 — generic / substance-level (jurisdiction-independent)

| Source | Body | Licence | AGPL-bundleable? | Verdict |
|---|---|---|---|---|
| **WHO INN** | WHO INN Programme | **Public domain** ("placed in the public domain … used without any restriction whatsoever") | ✅ Yes | **✅ Anchor.** Names are free; only friction is *access* — no clean bulk file (registration-gated INN Hub API, or parse the biannual public-domain PDF lists). An engineering problem, not a legal one. Biannual updates. |
| **RxNorm — Current Prescribable Content** | US NLM | **Public domain** (only `SAB=RXNORM` + `SAB=MTHSPL`) | ✅ Yes | **✅ Primary normalization layer.** Downloadable **without** a UMLS login. Ingredient / clinical-drug / dose-form / strength. |
| RxNorm — *full* release / RxNav-in-a-Box | US NLM | Public-domain core **+ UMLS-restricted SABs** (First DataBank, Micromedex, Multum, VA) | 🚩 **No** | ⚠️ **Avoid for redistribution.** The offline-friendly RxNav-in-a-Box is UMLS-gated. Build your own offline store from the prescribable subset instead. |
| **DailyMed / SPL** | US NLM / FDA | **Public domain** | ✅ Yes | **✅** Active ingredient, strength, form, route, NDC; LOINC-coded label sections. Also the DDI-mining substrate (§4). |
| **openFDA** | US FDA | **CC0 1.0** | ✅ Yes | **✅** NDC directory, drug labels, FAERS. Narrow caveat: a few privately-submitted copyrightable items are marked. |
| **Drugs@FDA** | US FDA | Public domain | ✅ Yes | ✅ Approved products, ingredients, strengths. |
| **PubChem** | NCBI | Public domain | ✅ Yes | ✅ Chemistry layer (structures, InChI, identifiers). Per-depositor caveat for sub-collections. |
| **ChEMBL** | EMBL-EBI | **CC BY-SA 3.0** | ⚠️ **Caveat** | Usable, but **ShareAlike is copyleft** — keep it an isolated, attributed data layer; don't mix into a combined work. Bioactivity-oriented; low priority for prescribing. |
| **WHO ATC/DDD** | WHO CC, Oslo (atcddd.fhi.no) | NC **+ no-derivatives** + attribution | 🚩 **No** | 🚩 **€200 fee + forbids commercial redistribution + forbids modification.** Operator-supplied / separately-licensed only. Community scraper `fabkury/atcd` outputs CC BY-NC-SA, confirming it is not free. |
| **WHO Essential Medicines List** | WHO | **CC BY-NC-SA 3.0 IGO** | 🚩 **No** (NC) | Free to *access* (eEML export at list.essentialmeds.org), but NC blocks bundling. Factual "on-list y/n" flag *may* be reconstructable from non-copyrightable facts — needs legal review. |
| **DrugBank (full)** | OMx / U. Alberta | **CC BY-NC 4.0** / paid commercial | 🚩 **No** | 🚩 **NonCommercial — excluded.** *Only* the tiny **DrugBank Open Data** ID-mapping subset (CC0) is usable. |
| **KEGG DRUG** | Kanehisa Labs | Academic-only / paid commercial | 🚩 **No** | 🚩 Excluded. |

---

## 3. Tier 2 — packaging / product (Australia first)

| Source | Body | Licence | AGPL-bundleable? | Role |
|---|---|---|---|---|
| **PBS API / CSV** | Dept. of Health | **CC BY 3.0 AU** | ✅ Yes | **✅ Primary packaging backbone** — item code, form & strength, pack size/quantity, manner of administration, restrictions, ATC linkage, pricing. See §3.1 for the 2026 format cutover. |
| **TGA ARTG** | TGA | **CC BY** (3.0 AU → 4.0 on newer releases) | ✅ Yes | **✅ Primary product registry** — sponsor, trade name, ARTG ID, active ingredients, dosage form, status. |
| **AMT (in SNOMED CT-AU)** | ADHA / NCTS | SNOMED National/Affiliate licence | 🚩 **No** | ⚠️ **Free to use *inside* Australia (free registration), NOT internationally redistributable.** SNOMED affiliate licensing charges fees in non-member territories and forbids sub-licensee redistribution. Treat as a **per-node, per-jurisdiction-licensed plug-in** fetched over the distribution plane — never bundled. Maps cleanly onto the [ADR-0014](../spec/decisions/0014-locale-pluggable-matcher-comparators.md) content-addressed-component posture. |
| NPS MedicineWise | — | — | — | ☠️ **Defunct** (ceased 31 Dec 2022). Ignore. |
| AMH; Therapeutic Guidelines (eTG) | proprietary | All rights reserved | 🚩 **No** | 🚩 Paid/proprietary subscription products. Exclude. |

### 3.1 PBS feed — viable, but the format just shifted (time-sensitive)

The user fed drugref from PBS data in EasyGP; the old pain was a PBS schedule format that "kept shifting like
quicksand," eventually settling on a mostly-standards-compliant XML that nearly always had easy-to-fix parsing
errors. **As of 2026 that avenue is still viable but the XML is now retired** — the format shifted one final time:

- **PBS XML and PBS Text files: discontinued 1 May 2026.** Gone. Any ingest pointed at the XML broke this May.
- **PBS Offline: discontinued 1 March 2026.** Vendor Schedule distribution ceased October 2024.
- **Replacement: PBS API v3 + monthly "PBS API CSV files"** (every API endpoint/table exported as CSV,
  published monthly). The CSV bundle is the **direct modern equivalent of the old bulk XML dump**, and the
  structured CSV/JSON should largely end the parsing-error pain.

Practical specifics for rebuilding the ingest:

- **Licence: CC BY 3.0 Australia** (confirmed on the data.gov.au PBS datasets — Item Report, ATC Report,
  Patient Category Report). Redistributable, commercial OK, attribution only — **AGPL-compatible.** ✅
- **Public API is free, no login** (default subscription key) but **brutally rate-limited: 1 request / 20 s,
  shared across *all* users globally.** Per-item API enumeration of the full schedule is therefore a non-starter
  — **use the monthly CSV bundle** for bulk load; the dept's own developer docs recommend weekly snapshots.
- **Free registration** via the PBS Data API Portal (`data-api-portal.health.gov.au`) yields a subscription
  key, higher limits, and — important for a dispensing system — **embargo access to *future* schedules**, so
  next month's data can be staged before it goes live. The public API exposes only current + trailing 12 months.
- **~14 tables**, base URL `https://data-api.health.gov.au/pbs/api/v3`: `items`, `item-overview`, `prescribers`,
  `schedules`, `atc-codes`, `organisations`, `restrictions`, `criteria`, `copayments`, `fees`, `programs`,
  `summary-of-changes`, … The **`summary-of-changes`** endpoint is new vs. the EasyGP era — it gives the monthly
  delta directly, so full-dump diffing is no longer required.

**Conclusion:** PBS + TGA ARTG (both CC BY) give the entire Australian product/packaging layer with no licence
fees and full redistribution rights — the EasyGP→drugref pattern, minus the brittle XML parser. Because the
format *just* shifted again, the ingest should treat the PBS schema as a **versioned external dependency**
(founding principle 11, *legibility across time*, applied to an upstream feed), not as a fixed column set.

---

## 4. The drug–drug interaction gap (still the hard part)

**No comprehensive, clinically-validated, openly-redistributable, AGPL-compatible DDI database exists.** Every
clinical-grade source (Stockley's, Lexicomp, Micromedex, First Databank, DrugBank) is paywalled. The defensible
open composite, in descending licence-safety:

1. **ONC high-priority DDI list** — expert-consensus set (Phansalkar *et al.*, *JAMIA* 2012; + CredibleMeds QT
   drugs) in open-access literature. **Clean and authoritative but minimal — a high-severity safety floor, not a
   full checker.** Note: NLM's RxNav Interaction API that served it was **permanently retired 2 Jan 2024**;
   re-encode from the papers.
2. **Mine DailyMed SPL interaction sections** (public domain) with NLP. **ONSIDES** (Tatonetti lab) is
   **MIT-licensed** and proves the pattern for adverse events — reuse as precedent. Engineering- and
   clinical-validation-heavy, but the strongest from-scratch open foundation.
3. **DDInter 2.0** (Central South University) — ~302k DDI records *with severity + mechanism + management*. The
   single best candidate for structured open DDI — **but its exact download-page licence needs a human (browser)
   confirm** (aggregators tag it CC0; that may describe metadata only; the site 403s automated fetch).
4. **TWOSIDES / OFFSIDES, Hetionet** — research-only FAERS signals; ambiguous/likely-NC; **never primary
   alerting.**

The honest conclusion: **the curation is the moat, and it is the part no one gives away.** This is exactly where
Cairn could revive the spirit of the hand-curated drugref DDI set — but as an institutionally-owned,
**append-only overlay** (principle 1), not a volunteer wiki (see §5).

---

## 5. Sustainability — why drugref.org soft-died, and the lesson

drugref.org (the user's ~20-year-old wiki, seeded by the Mercy Ships foundation, with a hand-curated DDI
database) soft-died from **structural volunteer attrition + grant-cycle mismatch** (funders pay for novelty,
never maintenance). **But it survived in Canada because OSCAR's `drugref2` replaced volunteer data-authoring
with an automated feed off Health Canada's open Drug Product Database (DPD).** That is the template:

> **Don't crowd-source the clinical facts. Aggregate them from institutionally-funded government feeds, and
> reserve scarce human curation for the thin, high-value layer machines can't supply (DDI severity, clinical
> judgment).**

Sustainable funding models, most-durable first: **(1) government public-good feeds** (RxNorm, openFDA, DPD,
dm+d, PBS, TGA — standing budgets, not grants); **(2) consortium/member dues** (SNOMED — durable funding,
*conditional* openness); **(3) foundation/nonprofit subscription** (TAIR → Phoenix Bioinformatics became
self-sustaining in a year); **(4) commercial-services-around-open-core.** Avoid CIEL-style single-maintainer
key-person dependency.

---

## 6. Recommended sourcing stack

- **Substance anchor (Tier 1, global):** **WHO INN** (public domain) as the UUID, **+ RxNorm Current
  Prescribable Content** (public domain) for normalization/clinical-drug/dose-form, **+ DailyMed/SPL + openFDA**
  (public domain) for ingredients/labels. All four bundle freely under AGPL.
- **Australian product/packaging (Tier 2):** **PBS API/CSV + TGA ARTG** (both CC BY) as the freely-shippable
  layer. **AMT/SNOMED CT-AU** as a per-node NCTS-licensed plug-in, never bundled.
- **Other jurisdictions (same shape):** open regulatory registry bundled (US FDA/NDC, Canada DPD, UK dm+d via
  free TRUD account); national SNOMED extension as a licensed plug-in.
- **DDI:** ONC high-priority floor (re-encoded) → SPL-mined layer (ONSIDES-style, MIT) → DDInter *if licence
  confirms* → Cairn's own curated append-only overlay as the durable value-add.
- **Hard excludes:** WHO ATC/DDD, WHO EML (NC), DrugBank-full, KEGG, AMH, eTG.

---

## 7. Verify-before-relying (could not auto-fetch; all 403'd)

1. **DDInter** exact download-page licence (the linchpin for structured open DDI).
2. **Canada DPD** and **EU Article 57** per-dataset licence *tags* (expected OGL-Canada / CC-BY; confirm).
3. Literal **CC BY version** on current TGA/PBS downloads and the **SNOMED CT-AU National Licence**
   redistribution clause text from NCTS.

---

## 8. Disease & injury concept identifiers

Medication is as central as disease, and the requirement is identical: **the decision-making pathway must key on
solid, stable concept identifiers — never free-text names that are spelled differently across sources and drift
over time.** This is the same discipline as the INN anchor in §1, applied to the morbidity/injury axis. SNOMED
CT is *clinically* the richest option but is excluded on the mission: it is member/affiliate-gated, charges fees
in non-member territories, forbids sub-licensee redistribution — a money-spinner behind a paywall, the same
defect that put AMT (§3) out. The realistic field is the WHO ICD family, the WONCA ICPC family, and a handful of
genuinely-open biomedical ontologies.

> [!IMPORTANT]
> **The key nuance: NoDerivatives is *not* fatal for the identifier use-case.** Cairn needs to use classification
> codes **verbatim as stable concept anchors** — it does not need to *modify* the classification. WHO's
> CC BY-ND licence permits exactly that: copy, redistribute, and commercial use of the codes with attribution.
> The ND clause only bites if you ship a *modified* codelist, a *translation*, or your *own* crosswalk
> (ICD↔SNOMED, ICD-11↔ICD-10) — each of which needs a **separate WHO agreement**. So ICD is usable as the
> identifier substrate; the boundary to document is "verbatim codes yes, derived maps/translations no."

### 8.1 The candidates

| Classification | Body | Licence | Bundle verbatim codes? | Modify / own crosswalks? | Stable IDs | Fit |
|---|---|---|---|---|---|---|
| **ICD-11** | WHO | **CC BY-ND 3.0 IGO**; API/software royalty-free (no standalone resale) | ✅ Yes (attribution, commercial OK) | 🚩 No (ND → separate WHO agreement) | **Persistent URIs** `id.who.int/icd/entity/{id}` + stem codes | **★ Best technical fit** — see §8.2 |
| **ICD-10** | WHO | **CC BY-ND 3.0 IGO** (historically licence-application-gated) | ✅ Yes (verbatim) | 🚩 No (ND) | Alphanumeric codes (e.g. `J18.9`) | Legacy/bridging where ICD-11 not yet adopted; effectively frozen (~2019) |
| **ICD-10-CM** | US NCHS/CDC | **Public domain** (US gov) | ✅ Yes | ✅ Yes | Annual codes + addenda | 🚩 **US-specific, code-incompatible** with WHO ICD-10/AM — licence-cleanest but least portable |
| **ICD-10-AM** | IHACPA (Sydney/NCCH origin) | Paid licence; free tier = **NonCommercial, AU-internal, no-redistribution** | 🚩 No | 🚩 No | Per-edition codes | 🚩 **Exclude / site-provided plug-in only** — categorically AGPL-incompatible |
| **ICPC-3** | WONCA / WICC | **"Openly available under a Creative Commons licence"** — **exact variant UNCONFIRMED** | ⚠️ **Depends on variant** | ⚠️ Only if CC BY/CC0/CC BY-SA | Concept codes | **GP-aligned, the natural primary-care coder** — *gated on §8.4 verify #1* |
| ICPC-2 / 2e | WONCA / WICC | WONCA copyright, licence-gated (all rights reserved) | 🚩 No | 🚩 No | Rubric codes, mapped to ICD-10 | 🚩 Encumbered — exclude |
| **ICPC-2 PLUS** | Univ. Sydney FMRC → NCCH | **Paid annual licence** (~AUD 120–420/site, renewing as of Feb 2023) | 🚩 No | 🚩 No | Interface terms → ICPC-2 | 🚩 **Exclude** — the copyright-encumbered Australian derivative; Cairn ships only the *capability* to load it |
| **SNOMED CT (full) / national extensions (incl. AMT)** | SNOMED International / NRCs | Member/affiliate-gated; fee in non-member territories | 🚩 No | 🚩 No | SCTIDs | 🚩 **Excluded by mission** — node-local licensed plug-in only |

### 8.2 Why ICD-11 is the best technical fit for an offline-first, stable-ID record

- **Persistent URIs against name-drift.** Every concept has a durable identifier rooted at
  `https://id.who.int/icd/entity/{entityId}`, plus codeable MMS **stem codes**. A URI + entity ID is exactly the
  "stable identifier, not a free-text name" the decision pathway needs — and it composes with principle 11
  (*legibility across time*): the coded event carries the stable anchor, while its [plaintext legibility twin](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
  records the human label *as asserted at the time*, so the event stays readable even as the classification moves.
- **Genuinely offline.** WHO ships an **official Docker container** (`whoicd/icd-api`, ARM-supported) that runs
  the full Coding Tool + browser + API **with no internet connection**, mirroring the canonical URIs locally
  (`id.who.int/icd/entity` → `yourserver/icd/entity`). This is a clean fit for the fractal-topology node
  ([ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md)) and the availability floor — no cloud
  dependency on the decision pathway. The API is free (cloud needs free registration; local needs
  `acceptLicense=true`).
- **The licence boundary to honour:** ship codes/URIs verbatim with WHO attribution; do **not** redistribute a
  *modified* ICD, a translation, or a Cairn-built ICD↔SNOMED / cross-version map without a separate signed WHO
  agreement. Treat any such map as a separately-licensed artifact, never folded into the AGPL corpus.

### 8.3 Genuinely-open ontologies (clean AGPL-compatible enrichment substrate)

Not primary-care morbidity coders, but useful as a **free, stable-ID semantic layer** that cross-maps to the
encumbered ones — and unlike ICD/ICPC they are fully modifiable, so Cairn *can* derive from them:

- **Mondo Disease Ontology** — **CC BY 4.0**, OWL/OBO, stable `MONDO:` IDs, integrates/maps across ICD, SNOMED,
  Orphanet, OMIM. The broadest clean disease ontology. ✅
- **ORDO (Orphanet Rare Disease Ontology)** — **CC BY 4.0**, stable ORPHAcodes; rare-disease-focused. ✅
- **HPO (Human Phenotype Ontology)** — phenotype/sign-symptom layer, stable `HP:` IDs, but a **bespoke
  licence** (not plain CC) — usable-pending-check (§8.4 verify #3).
- **SNOMED CT Global Patient Set (GPS)** — the *only* free SNOMED content for non-members, stable SCTIDs; a 2026
  source suggests the licence may have shifted to **CC BY-ND** (was CC BY 4.0) — verify (§8.4 #2). ND would still
  permit verbatim-code use, same posture as ICD.
- **MedDRA** — 🚩 subscription/paywalled, exclude.

### 8.4 Recommendation and open verifies

> [!NOTE]
> **Ratified.** The ICD-11 decision below graduated from this evaluation to
> [ADR-0025](../spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) (canonical home
> [data-model §3.16](../spec/data-model.md#316-clinical-concept-coding-the-icd-11-interlingua-and-the-local-terminology-overlay)),
> which fixes ICD-11 as the canonical classification interlingua with a local-terminology overlay (map-once,
> offered-not-forced, open mappings deferrable to a professional coder). Spec → v0.27.

**Recommended disease/injury identifier stack:**
- **Primary concept anchor: ICD-11** (CC BY-ND 3.0 IGO) — verbatim entity-URI/stem-code identifiers, free offline
  Docker container, commercial OK with attribution. The ND boundary documented in §8.2. **(Ratified — ADR-0025.)**
- **Bridging: ICD-10** (CC BY-ND) where ICD-11 isn't yet the local standard — same verbatim posture.
- **Primary-care layer: ICPC-3 — *conditionally*.** If its open licence confirms as **CC BY** (not NC/ND), it
  becomes the natural GP-aligned coder and should be adopted for the primary-care reason-for-encounter axis.
  Until the variant is confirmed, treat as pending; **do not assume usable.**
- **Free semantic substrate: Mondo + ORDO** (CC BY 4.0) for derivable, modifiable cross-mapping; HPO for
  phenotype pending its licence check.
- **Exclude / site-plug-in only:** SNOMED CT full + AMT, ICD-10-AM, ICPC-2/2e/2-PLUS — each a node-local,
  separately-licensed dependency the deploying site supplies under its own licence; Cairn ships the load
  capability, never the data.

**Open verifies (all sites 403'd automated fetch — need a human browser read):**
1. **ICPC-3 exact CC variant** (CC BY vs CC BY-NC vs CC BY-ND) — `icpc-3.info` licence page + the WONCA
   "ICPC-3 to Become Openly Licensed" announcement. **Highest priority — it decides whether the GP coder is in.**
2. **SNOMED GPS current licence** — CC BY 4.0 vs CC BY-ND 4.0.
3. **HPO custom licence** full text (`hpo.jax.org/app/license`).
4. **WHO crosswalk/translation separate-agreement terms** — needed before Cairn ships *any* ICD-derived map.

---

## Sources

**Tier 1:** WHO INN (who.int/teams/health-product-and-policy-standards/inn) · RxNorm Current Prescribable
Content (nlm.nih.gov/research/umls/rxnorm/docs/prescribe.html) + Terms of Service + UMLS License Agreement ·
DailyMed SPL Resources · openFDA License (open.fda.gov/license) · ChEMBL licensing (chembl.github.io) · WHO
ATC/DDD copyright (atcddd.fhi.no/copyright_disclaimer/) · WHO EML (list.essentialmeds.org) · DrugBank Terms of
Use (go.drugbank.com/legal/terms_of_use) · KEGG Legal (kegg.jp/kegg/legal.html).
**Tier 2 / PBS:** PBS New API & API CSV files news (pbs.gov.au/info/news/2024/12/...) · PBS Download
(pbs.gov.au/info/browse/download) · data.pbs.gov.au documents 91327/91602/90834 · Accessing PBS embargo data
(hpp.health.gov.au) · data.gov.au PBS Item Report (CC BY 3.0 AU) · TGA datasets (tga.gov.au/resources/datasets)
· SNOMED licensing (snomed.org/get-snomed, snomed.org/licensing).
**DDI:** Phansalkar 2012 (PMC3422823) · NLM RxNav Interaction API retirement · ONSIDES (github.com/tatonetti-lab/onsides,
MIT) · DDInter 2.0 (NAR 2025) · nsides.io.
**Sustainability:** OSCAR drugref2 (oscaremr.atlassian.net) · Health Canada DPD (open.canada.ca) · TAIR/Phoenix
(PMC4795935) · OpenMRS CIEL.
**Disease/injury terminology (§8):** WHO FAQ Licensing ICD-10 (cdn.who.int/.../who-faq-licensing-icd-10.pdf) ·
WHO Copyright policy (who.int/about/policies/publishing/copyright) · ICD-11 License (icd.who.int/en/docs/icd11-license.pdf)
· ICD-API License + Docker container + Local Deployment (icd.who.int/icdapi/docs2/...) · ICD-10 CDN
(icdcdn.who.int/icd10) · CDC NCHS ICD-10-CM Files (cdc.gov/nchs/icd/icd-10-cm/files.html) · IHACPA products &
licenses (ihacpa.gov.au/health-care/products-and-licenses) · Lane Print Electronic Code Lists
(ar-drg.laneprint.com.au) · WONCA "ICPC-3 to Become Openly Licensed" (globalfamilydoctor.com/News/ICPC-3OpenLicense.aspx)
· icpc-3.info · WICC (wicc.one/icpc-classification) · ICPC-2 PLUS (en.wikipedia.org/wiki/ICPC-2_PLUS,
sydney.edu.au NCCH) · Mondo (mondo.monarchinitiative.org) · ORDO (sciences.orphadata.com/ordo) · HPO
(hpo.jax.org/app/license) · SNOMED GPS (snomed.org/gps).
