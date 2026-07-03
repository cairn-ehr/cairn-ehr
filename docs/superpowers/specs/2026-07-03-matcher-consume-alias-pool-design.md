# Design — the §5.2 matcher consumes `patient_alias_pool` (known-alias evidence)

**Date:** 2026-07-03 · **Status:** design (brainstorm→spec→plan→TDD) · **Tier:** advisory matcher
(Python, fit-for-purpose §9) · **Schema/floor/ADR change:** none.

## Context — the seam C5 built but left inert

Identity C5 (`db/025_identity_repudiate.sql`) added the *suppressing* `repudiate` event, a value-grained
`name_repudiation` overlay, and a reason-free **`patient_alias_pool`** VIEW `(patient_id, value, hlc_*,
origin, updated_at)` granted to `cairn_agent`. Its stated purpose (§5.5(a)): a name established false leaves
the display header but stays a **known alias** "so that if the same persona returns the staff recognise it."
db/025 itself notes *"The matcher wiring that runs this query is deferred to a later slice."* This slice is
that wiring.

## The finding that shapes the slice: the returning-persona pair *already* gets proposed

C5 deliberately left db/012's `patient_name` retained set **physically untouched** — a struck name is
excluded only from the display winner (`patient_name_current`), not from `patient_name`. The matcher reads
the **retained set** in both places:

- **Blocking** (`pipeline/db.py::_GROUPS_SQL`) tokenizes `patient_name` → a struck alias is still a token.
- **Scoring** (`pipeline/db.py::load_candidate` → `SELECT … FROM patient_name`) → a struck alias is still
  scored as one of the chart's names.

So when a fabricated persona returns under the reused alias, chart A (struck `"John Fakename"` still in its
retained set) and the new chart B (`"John Fakename"`) **already** block together and score high → a proposal
already fires. **Consuming the alias pool does not enable a match that currently fails.** Its genuine value
is different, and naming that correctly is the whole design.

## What the alias pool is actually *for* here — flag, never suppress

Two candidate purposes, one a trap:

1. **Explainability / paper-parity (the real win).** The proposal's `evidence` today says only
   `name: EXACT`. It cannot tell a reviewer the agreement is on a name chart A has **repudiated as
   known-false** — which is *exactly* the "recognised returning persona" signal §5.5(a) wants surfaced. On
   paper the registrar's "known alias" flag sits right in the registry; in the worklist it is invisible. A
   `known_alias` evidence entry restores that flag.

2. **Suppress false-name pollution (a trap — explicitly NOT done).** One might argue A's known-false name
   should stop matching, so it no longer proposes A against a *real, different* person genuinely named
   "John Fakename". But **the matcher cannot distinguish** "b is the returning fabricated persona" from
   "b is a real bearer of that false name" — from the name alone they are identical. Suppressing the alias
   match would suppress the very returning-persona recognition §5.5(a) mandates. Only a human can
   disambiguate. §5.7 marks identity adjudication **Human**. Therefore the correct move is to **flag the
   known-alias fact for a human, never auto-suppress and never auto-link on it.**

This slice implements (1) and deliberately refuses (2).

## What this slice delivers

- **`pipeline/db.py::load_aliases(conn, patient_id) -> frozenset[str]`** — reads the raw repudiated `value`s
  for one chart from `patient_alias_pool`. The only new DB touch; thin, like `load_candidate`.
- **`pipeline/alias.py` (pure, no psycopg)** — `known_alias_evidence(id_a, names_a, aliases_a, id_b,
  names_b, aliases_b) -> tuple[dict, ...]`. For each raw alias `v` of one chart, it is *corroborated* iff its
  normalized name-bag appears in the **other** chart's names **or** the other chart's aliases; each
  corroborated alias emits `{"kind": "known_alias", "value": v, "alias_of": <patient_id>}`. Normalization
  reuses the adapter's `_name_bag` (NFC + casefold + sorted token bag) — the **same** notion of "same name"
  the scorer uses, so there is no second, drifting definition of a name match. Match is
  **normalized-exact on the full name** (deterministic, culture-neutral); fuzzy/edit-distance alias
  recognition is a documented follow-up, not this cut.
- **`pipeline/banding.py::band(..., *, has_known_alias=False)`** — when a known-alias match is present the
  band is **REVIEW**: never `None` (the signal is never silently dropped below threshold) and never
  `AUTO_CANDIDATE` (never auto-link two charts on the strength of a name one of them declared false —
  §5.7 "Human"). This mirrors the existing shared-identifier veto-rescue (a scoped strong signal forcing
  REVIEW), and like it can never auto-*reject* (it only ever *surfaces* the pair).
- **`pipeline/banding.py::build_payload(..., alias_evidence=())`** — appends the `known_alias` entries to the
  proposal's `evidence` tuple so the reviewer sees *why*.
- **`pipeline/runner.py::propose`** — loads both charts' aliases, computes the evidence, threads
  `has_known_alias` into `band` and `alias_evidence` into `build_payload`.
- **`tests/conftest.py`** — extend `_SCHEMA_FILES` to db/018–025 (matching `db.rs`) so `patient_alias_pool`
  exists for the DB-gated test, and truncate `name_repudiation` between tests.

## Load-bearing design calls

1. **Flag, never suppress; REVIEW, never AUTO/None (above).** The core clinical-semantics call. A
   known-alias match always reaches a human, is never auto-actioned, is never dropped.
2. **Normalized-exact on the full name, reusing `_name_bag`.** The suppression *floor* (db/025) is
   exact-string on the opaque value — it must be precise or it strikes the wrong name. The *matcher* is
   advisory, so it recognizes aliases in normalized space (NFC + casefold, token-bag) — this is what db/025's
   design explicitly assigns to "the advisory matcher's job." Reusing `_name_bag` means the alias notion of
   "same name" is byte-identical to the scorer's, so a corroborated alias is one the scorer would also grade
   agreeing — no drift, one definition.
3. **Evidence references `patient_id`, not `low/high`.** The proposal row is stored canonically, but the
   `alias_of` uuid names the actual chart whose alias it is, so the entry reads correctly regardless of which
   side sorted low. The value carried is the name string itself (the alias pool is reason-free — no `reason`,
   no cross-patient forensic leak; ADR-0006 confidentiality split preserved).
4. **No blocking pass added — deliberately.** A candidate `alias` blocking pass would add **zero recall
   today**: struck names remain in `patient_name`, so the existing name-token pass already generates the
   identical pair, and a full-value alias key can never group a pair the token pass does not. Building it now
   is gold-plating (house rule #4). Recorded as a deferred future-proofing item (it only earns its keep if a
   later slice stops keeping struck names in the scored retained set).

## Explicitly deferred (recorded, not lost)

- **Fuzzy/edit-distance alias recognition** (a returning persona giving a *slightly* different spelling of
  the alias). This cut is normalized-exact; the scorer's fuzzy name comparison still runs independently on
  the retained-set names, so a near-miss alias is not lost, just not *tagged* as a known-alias hit.
- **The `alias` blocking pass** (future-proofing only; zero recall today — see call 4).
- **Scoring-weight treatment of known-false names** (#2 above) — declined by design; a genuine re-weighting
  would need weight-learning data (B3) and a spec decision, and risks suppressing true returning-persona
  recognition.
- **Consuming the alias pool in the C2b auto-apply path.** Auto-apply acts on the `auto_candidate` band; a
  known-alias match forces REVIEW, so it never reaches auto-apply — correct by construction, nothing to wire.

## Principles check

- **§5.5(a) / paper-parity (3).** Restores the registrar's "known alias" flag to the worklist; a returning
  persona is recognized *and explained*, matching the paper registry affordance. No confirmation dialog — an
  evidence tag on an advisory proposal a human already reviews.
- **Identity is a claim (2).** Nothing is merged or erased; a known-alias match only *annotates* an advisory
  proposal a human adjudicates into a C1/C2 link.
- **Acknowledged uncertainty (4).** The matcher cannot tell returning-persona from real-bearer and does not
  pretend to — it surfaces the fact and defers to a human, never fabricating a decision.
- **§9 blast radius.** Advisory tier: a defect feeds the human worklist an extra (or mis-tagged) proposal,
  never an auto-link and never a floor bypass. No `db/` floor, no SCHEMA bump, no ADR.
