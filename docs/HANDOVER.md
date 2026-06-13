# HANDOVER — Cairn

**Session date:** 2026-06-13
**Status of this file:** Working scaffolding, not a source of truth. Disposable — regenerate
at the end of each working session. If this file ever disagrees with the canonical documents,
the canonical documents win.

---

## Read these first (the durable state)

The real project state lives in these documents. This handover points at them; it does not
restate them. Repository layout (restructured this session — see "Docs restructure" below):

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

## Resolved this session (now written into spec v0.6 — here for the trail)

The **"Postgres-intelligence" cluster** (§11.1 / §11.2 / §11.11) — the one entangled decision
flagged in prior handovers — is **resolved** as a single architecture: **"Fat Postgres, thin Rust
daemon."** Full rationale in **[ADR-0001](spec/decisions/0001-fat-postgres-thin-daemon.md)**;
written into `spec/topology.md` (§2), `spec/data-model.md` (§3.5), `spec/sync.md` (§6.1),
`spec/language-substrate.md` (§9.4). In brief:

- **§11.2 storage (→ §3.5):** hybrid event envelope — typed/normalized columns where invariants,
  identity, sync, and matching bind; **Cairn-native JSONB** for clinical bodies; **FHIR is a façade
  only**, never the storage model.
- **§11.11 merge boundary (→ §9.4):** structural invariants + the identity event algebra + **all
  projections live in Postgres** (trigger-maintained incremental tables, `AFTER INSERT` only); the
  Rust daemon ships/applies but **carries no merge logic**; the probabilistic matcher stays
  **Python and advisory**. Per-projection Rust **escape hatch** on measured Pi-performance need.
- **§11.1 sync backbone (→ §6.1):** **build** a thin custom Rust service on Postgres logical
  decoding; **borrow** pgactive/SymmetricDS patterns, **do not depend** on them (their row-conflict
  machinery solves a problem Cairn designed away and can violate §4 anti-data-loss policies).

**Two enabling facts established in the session (also written into the spec):**
1. **Tablets are thin clients**, not autonomous edge nodes → the smallest autonomous node is a
   **Pi-class full Postgres ≥18**; in-database logic runs on every computing node. (Revised §2.)
2. **FHIR is a skin, not a skeleton** — useful at the boundary, never the internal model; Cairn's
   internal model is canonical (national-scale ambition). (Reflected in §3.4 / §3.5.)

**The load-bearing bet to validate first when implementation begins:** that trigger-maintained
in-DB projections + the identity algebra stay cheap enough on **Pi-class hardware** to keep chart
reads local and fast (the §1.2 paper-parity floor). A Pi serves only a handful of workstations with
little concurrency, so the risk is **single-operation latency** (weak CPU + SD/USB I/O), not
throughput. The designed first spike is a **Raspberry-Pi-5 benchmark harness** (rural-clinic
profile, low concurrency; measure single-op projection-maintenance and chart-read latency;
threshold = beat "grab the paper chart"). Mitigation ladder if it's slow:
PL/pgSQL → **pgrx (in-DB Rust)** → external Rust — see
[ADR-0002](spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md). *This spike is the go/no-go
on the approach.*

---

## Docs restructure (this session)

The spec was split from a single versioned file into **`docs/spec/`** (one file per aspect) + an
**ADR log** (`docs/spec/decisions/`), with a Markdown→HTML pipeline (MkDocs Material; callouts
authored in GitHub/Obsidian syntax). Conventions changed: **no filename versioning, no in-file
changelogs** — git is the history, ADRs are the *why* (see updated `CLAUDE.md` and the new
[layout](#read-these-first-the-durable-state) above). Build verified with `mkdocs build --strict`.
**Still deferred (on the menu):** a polished non-developer landing page (frontend-design work), and
optionally single-sourcing the mission prose between root `README.md` and `spec/index.md`.

---

## Decided in conversation, NOT yet written into the documents

These are real decisions that still need to be reflected in the canonical files:

1. **Governance / CONTRIBUTING document** is identified as the next principles document to
   write, but does not exist yet. STEWARDSHIP-OF-THE-NAME.md is intended for inclusion in it.
2. **Status line:** README and spec both say "specification / architecture phase." Still
   accurate today — flip when implementation begins.

*(Reference for context — already written: name is **Cairn** / repo **cairn-ehr**; domains
`cairn-ehr.org` canonical + `cairn-ehr.com` defensive redirect, both registered, reflected in
STEWARDSHIP-OF-THE-NAME.md.)*

---

## Time-sensitive (do soon, before squatters do)

- **Reserve the project namespaces defensively** while the name is fresh: GitHub organization,
  and package registries (PyPI / crates / npm). Same logic as the domains — parking them now
  is trivial; reclaiming them later is painful.

---

## Open questions / where we'd pick up

Spec §11 still lists the remaining open questions (1, 2 and 11 are now struck-through/resolved).
The standing synthesis point not captured in the spec body:

- **Dynamic sync-scope handoff (§11.3)** — the patient transferred ED→ICU mid-partition; who
  owns scope reassignment during a partition. This is the last genuinely unsolved
  distributed-systems problem in the design, and it stands largely independent of the now-resolved
  Postgres-intelligence cluster — so it's a clean standalone session.

**The recurring menu** when resuming (pick one):
- The **dynamic sync-scope handoff (§11.3)** — the cleanest remaining hard problem.
- **Write the GOVERNANCE / CONTRIBUTING document** (folding in STEWARDSHIP-OF-THE-NAME.md).
- **Define the Pi-benchmark spike** in enough detail to be the first implementation task (it
  validates the [ADR-0001](spec/decisions/0001-fat-postgres-thin-daemon.md) architecture; see
  "Resolved this session" above).
- **Polish a non-developer landing page** for the generated site (frontend-design work; the current
  Home is the spec index).
- More clinical **case-mining** — the most productive mode so far: the user (an EM physician)
  brings real failure modes from practice, and we test whether the existing primitives absorb them.
  The event-overlay primitives (link / unlink / repudiate / reattribute / identify / dispute) have
  absorbed every case raised so far without new architecture; continuing to stress-test this is
  high-value. *(Now also testable against the v0.6 in-DB algebra concretely.)*
- Other still-open §11 items: schema migrations across offline nodes (§11.4), tombstones/GDPR
  erasure in an append-only system (§11.5), attachment strategy (§11.6), locale-pluggable matcher
  comparators (§11.7), visibility-scope ↔ sync-scope interaction (§11.8), armed write-context
  model (§11.9), notification economy (§11.10), authentication vs. paper-parity (§11.12).

---

## Parked (deliberately not decided yet — don't re-litigate without reason)

- **Stewarding legal entity & jurisdiction.** Options floated: German Stiftung /
  gemeinnütziger Verein, US 501(c)(3), or an umbrella (e.g. Software Freedom Conservancy or a
  health-specific foundation). Deferred until the project has momentum and funding/adoption
  geography is clearer. Interacts with the trademark question below.
- **Formal trademark / wordmark registration.** Principle is recorded now (stewardship doc);
  the legal instrument is deliberately deferred to avoid premature legal scaffolding — file it
  when there is enough substance to be worth capturing.

---

## Working context for whoever resumes

- The user is a senior Physician with an interest in ML / AI / health IT; codes
  mostly in Python. Brings real ED and hospital system experience from several nations and health systems — case-mining sessions are unusually productive.
- The project's founding motivation is explicitly **anti-capture / anti-vendor-lock-in**, rooted
  in the user's experience of government EHR committees being sabotaged by commercial interests.
  Decisions consistently favour the mission over convenience; treat that as the tie-breaker.
- Two governing principles run through everything and are the right lens for new decisions:
  **(a) append-only + causal ordering** so sync is set-union plus a small enumerated set of
  clinically-reasoned merge policies; **(b) identity is a claim, never a fact** — never merge,
  always link; never erase, always overlay. A third, added later: **(c) paper-parity** — no
  workflow may be slower / harder / more cognitively demanding than its paper equivalent
  (malfeasance excepted). When a new design choice arises, check it against these three first.
