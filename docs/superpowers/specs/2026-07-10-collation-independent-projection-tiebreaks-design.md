# Design — Collation-independent projection winner tiebreaks (#69)

**Date:** 2026-07-10 · **Issue:** [#69](https://github.com/cairn-ehr/cairn-ehr/issues/69) ·
**Spec bump:** v0.45 → v0.46 · **New ADR:** 0045

## Problem

The trigger-maintained projections resolve their "current winner" with a tuple comparison whose
tiebreak keys include **TEXT columns compared under the database's default collation** —
`node_origin`/`asserted_origin`, and the final total-order keys `value` / `display` / `use_key`.
Default collation is a *node-local property* (locale/ICU config), not a function of the event set.

So two nodes running **different default collations** could replay the identical append-only event
set and converge to **different display winners** for an exact `(provenance_rank, hlc_wall,
hlc_counter)` tie. This is a silent violation of the set-union convergence guarantee (principle 1) in
the safety-critical projection layer. It is **not** data loss — full history is retained append-only
in `event_log`; the divergence is confined to which retained assertion is *displayed* as current.

**Why this is real, not merely Byzantine.** #115 hardened the same layer against a *Byzantine
same-origin* HLC-triple collision (a broken signer reusing its own `(wall, counter, origin)` triple)
by appending the collation-free `content_address` (BYTEA) as the final tiebreak. But #115 explicitly
deferred the `origin` TEXT comparison to this issue. The **cross-origin** `(wall, counter)` tie —
two *different* honest nodes independently stamping the same millisecond wall + same counter — needs
**no misbehavior at all**, just coincidence, and it is decided by the collation-sensitive `origin`
compare *before* `content_address` is ever consulted. So the cross-origin case this issue closes is
strictly *more likely* than the Byzantine case #115 already fixed.

## Mechanism

`COLLATE "C"` forces byte-order comparison of a TEXT value's stored encoding. Every Cairn node stores
the **same UTF-8 bytes** for the same signed event (Postgres `server_encoding = UTF8`), so a
`COLLATE "C"` comparison yields an **identical total order on every node regardless of that node's
default/locale collation**. That is exactly the convergence property the projection layer needs.

`COLLATE "C"` is preferred over the alternatives:
- vs `convert_to(x, 'UTF8')::bytea` comparison — identical result (C *is* byte order of the UTF-8
  encoding), but cheaper (no per-comparison conversion) and more idiomatic.
- vs carrying `content_address` into the demographic projection tables (mirroring #115's Family-1
  fix) — truly canonical but a schema change to five projection tables plus threading
  `content_address` through every demographic trigger, and `value`/`display` are still needed as
  columns for display regardless. Rejected as disproportionate.

**Empirically confirmed** on the local UTF-8 cluster (2026-07-10): the DB default collation and `"C"`
genuinely disagree (`(1,'B') > (1,'a')` is true under default, false under `"C"`), and `COLLATE "C"`
is respected inside a row-tuple comparison, inside an `IMMUTABLE sql` function body, and in a mixed
`bigint`/`text`/`bytea` row. There is existing precedent at `db/012:63` (the `use_key` fold).

## Blast radius — the complete set of comparison sites

### Family 1 — five standing-state overlays (one shared predicate)

All five route through `cairn_hlc_overlay_wins(...)` in `db/002`, which compares
`(wall, counter, origin TEXT, content_address BYTEA)`. Fixing the **one** function fixes all five:
`patient_chart` (db/002), `patient_link` (db/018), `chart_dispute` (db/023),
`chart_identity_state` (db/024), `name_repudiation` (db/025).

**Change:** `origin` compared under `COLLATE "C"`. `content_address` (BYTEA) is already
collation-free and remains the final Byzantine-collision tiebreak.

### Family 2 — demographic projections (inline tuple compares + two VIEWs)

Each has its own winner comparison, over `asserted_origin` **and** a `value`/`display`/`use_key`
final total-order key:

| File | Projection | TEXT compare keys | Sites |
|---|---|---|---|
| db/010 | `patient_identifier` | `asserted_origin`, `value` | trigger WHERE tuple |
| db/011 | `patient_demographic` | `asserted_origin`, `value` | trigger WHERE tuple (**superseded by db/013** — fixed anyway, see below) |
| db/013 | `patient_demographic` | `asserted_origin`, `value` | trigger WHERE, **both** CASE branches (provenance-first + recency-first) |
| db/012 | `patient_name` | `asserted_origin`; `use_key`, `value` | trigger WHERE tuple + `patient_name_current` VIEW `ORDER BY` |
| db/014 | `patient_address` | `asserted_origin`; `display` | trigger WHERE tuple + `patient_address_current` VIEW `ORDER BY` |

Untouched (not TEXT comparison keys): `provenance_rank` (INT), HLC columns (BIGINT/INT),
`content_address` (BYTEA), and `facets`/`geo`/`structured` (JSONB, carried not compared). `provenance`
(TEXT) is carried but never a comparison key — its INT `provenance_rank` is.

**db/011 micro-call:** db/013 `CREATE OR REPLACE`s `patient_demographic_apply()` and its trigger, so
db/011's function body is dead at runtime. It is fixed anyway (2 annotations) so no migration in the
tree displays a now-known-wrong tiebreak pattern — reviewer-legibility over minimal-diff.

## ADR + spec

- **ADR-0045 — Collation-independent projection winner tiebreaks.** Invariant: *every projection
  winner tiebreak comparison over a TEXT key MUST be made under `COLLATE "C"`* so set-union
  projection converges across a federation of mixed default collations. Records the context (the
  cross-origin tie needs no misbehavior), the mechanism, the rejected content_address-carry
  alternative, and the consequence: **future demographic/overlay projection slices must follow the
  invariant** or silently reintroduce a collation-sensitive winner. Refines principle 1's convergence
  guarantee; relates to ADR-0031 (canonical identifiers) and #115 (`content_address` tiebreak).
- **Spec:** one line in `docs/spec/sync.md` (convergence / set-union projection section) naming the
  `COLLATE "C"` tiebreak invariant; version bump v0.45 → v0.46 in `docs/spec/index.md`.

## Tests (TDD, DB-gated)

**Design principle:** choose origin/value strings the DB default collation and `COLLATE "C"` order
**oppositely** (e.g. `'B'` vs `'a'` — default here orders `'B' > 'a'`, C orders `'a' > 'B'`). A test
that then asserts the projection picks the **C-order** winner proves collation-*independence*, not
merely in-DB determinism — because C order is defined by the (identical-on-every-node) bytes.

- **Pure predicate** — `cairn_hlc_overlay_wins` with a collation-divergent cross-origin `(wall,
  counter)` tie → winner follows C byte-order (and differs from the default-collation order for the
  constructed pair).
- **Family 1** — extend `crates/cairn-node/tests/overlay_tiebreaker.rs`: a collation-divergent
  cross-origin tie applied through the remote-apply door in **both arrival orders** converges to the
  same C-order winner.
- **Family 2** — new `crates/cairn-node/tests/projection_collation_convergence.rs`: one convergence
  case per projection —
  - `patient_identifier` (db/010)
  - `patient_demographic` provenance-first (dob/sex-at-birth) **and** recency-first (gender-identity)
    (db/013, both CASE branches)
  - `patient_name` retained-set **and** `patient_name_current` VIEW (db/012)
  - `patient_address` retained-set **and** `patient_address_current` VIEW (db/014)

  Each asserts (i) the DB default collation would order the constructed pair one way, (ii) the
  projection/VIEW winner follows the C order, (iii) both arrival orders agree.

**Optional, not in scope unless requested:** a real second database created with a different
`LC_COLLATE`/ICU locale running the same event set. The same-DB C-vs-default proof already
establishes collation-independence (C order = the bytes, identical everywhere) without depending on a
specific locale being installed, so this is redundant for correctness.

## Non-goals / scope boundaries

- **Projection-read-side only** — no wire/event-format/floor-gate change, no new event type, no
  on-wire SCHEMA change. The signed plane and the winner *ordering semantics* are unchanged; only the
  *collation* of the TEXT tiebreak keys is canonicalized.
- Does **not** alter the #115 `content_address` Byzantine tiebreak or the #157 collision signal —
  those remain the final/orthogonal mechanisms; this makes the `origin` step ahead of them
  collation-safe.
- Does not touch node-event-plane projections (db/007) — out of the issue's clinical-projection
  scope; noted for a future sweep if any node-event winner uses a TEXT tiebreak.

## Process

Small, safety-critical, well-bounded. brainstorm → design → plan → subagent-driven TDD with per-task
spec+quality review and a whole-branch opus review before PR. No file approaches the 500-line
guideline. New artifacts: 1 ADR, 1 test file; edits to db/002/010/011/012/013/014 + spec.
