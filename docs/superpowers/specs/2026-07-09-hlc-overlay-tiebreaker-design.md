# Design — Deterministic HLC-overlay tiebreaker (shared helper)

**Date:** 2026-07-09
**Issue:** [#115](https://github.com/cairn-ehr/cairn-ehr/issues/115) (part 1 + the HLC-overlay bullet of part 2)
**Scope:** the safety-critical in-DB projection floor. No wire/event-format change, no ADR/spec bump.

## Problem

Every standing-state overlay maintainer folds a new event into its projection with the same
HLC-guarded upsert, comparing the incoming event's `(hlc_wall, hlc_counter, node_origin)` against
the stored winner's triple:

```sql
ON CONFLICT (...) DO UPDATE SET ...
WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin)
    > (tbl.hlc_wall, tbl.hlc_counter, tbl.origin);
```

If two **distinct** events targeting the same key ever share an identical
`(hlc_wall, hlc_counter, origin)` triple, the strict-`>` guard is false in both directions, so the
winner is decided by **arrival order**. Two honest nodes receiving the two events in different
orders converge to **different** standing state — a silent cross-node divergence in the
safety-critical projection layer. For `chart_dispute` this is directly clinician-visible: node X
settles `open` (chart reads *under-review*), node Y settles `resolved` (reads *confirmed*).

Reachability is low: a conformant node's `node_hlc_tick` makes `(wall, counter)` strictly
monotonic per origin, so a same-origin triple collision requires a Byzantine/broken signer reusing
its own triple across two different signed bodies. Cairn handles Byzantine events via signatures +
recall, not projection tiebreaks — so this is a *robustness* gap, not an everyday bug. But it is
silent, and it is in the layer that must converge for "sync = safe set-union" to hold.

## Finding: the overlay sites are not uniform — only five have the acute gap

Tracing all ten overlay upserts, they split cleanly.

**Group A — the acute gap.** Comparison tuple is exactly `(wall, counter, origin)` with **no
deterministic final key anywhere**, and the projected value is a *state that flips*:

| Site | Table | What flips on divergence |
|---|---|---|
| `db/002` | `patient_chart` | winning name / dob / sex (`CASE`-form upsert) |
| `db/018` | `patient_link` | link vs unlink — **merge vs un-merge** |
| `db/023` | `chart_dispute` | open vs resolved — **under-review vs confirmed** |
| `db/024` | `chart_identity_state` | pending vs identified |
| `db/025` | `name_repudiation` | repudiated vs reversed |

All five share one comparison shape. These are exactly the sites #115 names plus their
identity-algebra siblings.

**Group B — already deterministic, deferred to [#69](https://github.com/cairn-ehr/cairn-ehr/issues/69).**
The demographic overlays converge to a deterministic winner already:

- `db/010 patient_identifier`, `db/011 patient_demographic`, `db/013 sex/gender` carry `value`
  *inside* the comparison tuple.
- `db/012 patient_name`, `db/014 patient_address` carry `value`/`display` both in the upsert
  conflict key and as the display view's final `ORDER BY` key.

Their residual divergence is purely **TEXT-collation** of those `origin`/`value`/`display`
comparisons — exactly what #69 tracks (their own code comments say so: *"add content_address … the
'C' fix for the origin/text comparisons is #69's remit"*). Their tuples are non-uniform and
policy-dependent (`db/013` reorders columns by field policy), so forcing `content_address` into
them would bloat a safety-critical PR and overlap #69. **Out of scope here.**

## Design (Group A)

### 1. One shared pure helper

Defined at the top of `db/002_projection.sql` (the first projection file, so every later overlay
can call it — deferred PL/pgSQL name resolution is not relied on):

```sql
-- Deterministic overlay-winner predicate: does the incoming event outrank the stored winner?
-- Compares (wall, counter, origin) exactly as before, then breaks a full tie with the event's
-- content_address (BYTEA multihash of the signed bytes) — canonical, UNIQUE, byte-compared
-- (collation-free), and never shared by two distinct events. This makes the overlay converge to
-- the same winner on every node even under a Byzantine (wall,counter,origin) collision, closing
-- the arrival-order divergence in the safety-critical projection layer (#115).
CREATE OR REPLACE FUNCTION cairn_hlc_overlay_wins(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT (new_wall, new_counter, new_origin, new_addr)
         > (COALESCE(cur_wall, -1), COALESCE(cur_counter, -1),
            COALESCE(cur_origin, ''), COALESCE(cur_addr, '\x'::bytea));
$$;
```

- `content_address` is the final tiebreaker: canonical, `UNIQUE` (a pure function of the signed
  bytes — two distinct events never share it, barring SHA-256 collision), `BYTEA` so it is
  byte-compared and immune to the #69 TEXT-collation concern.
- The `COALESCE(cur_*, …)` generalizes `db/002`'s existing `-1 / ''` defaults for its note-only
  partial rows. Empty bytea `'\x'` sorts below any real `\x1220…` (34-byte) address, so a note-only
  row (no demographic yet) always loses to a real demographic event. The four `WHERE`-guard sites
  always populate the row on insert, so their `COALESCE` is a harmless no-op.
- The **new** side is always a real event's values → never null; only the *current* side can be
  null (the `db/002` note-only case), so only it is COALESCEd.
- `IMMUTABLE` + `LANGUAGE sql` (one expression) → inlinable, once-tested.

### 2. Per-site change (all edited in place)

For each of the five Group-A tables:

1. Add a `content_address BYTEA` column (stores the winning event's content address).
2. The `INSERT` supplies `NEW.content_address`.
3. The `DO UPDATE` sets `content_address = EXCLUDED.content_address` (in the winning branch).
4. Route the comparison through `cairn_hlc_overlay_wins(...)`:
   - `db/018`, `db/023`, `db/024`, `db/025`: replace the `WHERE (EXCLUDED…) > (tbl…)` guard with a
     single `WHERE cairn_hlc_overlay_wins(EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin,
     EXCLUDED.content_address, tbl.hlc_wall, tbl.hlc_counter, tbl.origin, tbl.content_address)`.
   - `db/002` (`patient_chart`, `CASE`-form): replace each inline
     `(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin) > (COALESCE(...))` condition with a
     `cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)` call. Column
     name `demo_content_address` (parallel to `demo_hlc_wall`/`demo_hlc_count`/`demo_origin`). The
     `note.added` branch leaves it null (no demographic winner yet).

No `db.rs` SCHEMA-array length change (all in-place edits to existing migrations). No new event
type, no floor-gate change, no `submit_event`/`apply_remote_event` logic change beyond the overlay
triggers they already fire.

### 3. Tests (TDD — failing first)

**Helper unit tests** (SQL, pure, in a new `db/tests/` fixture or an existing test file — pick the
lightest home during planning):
- A `(wall, counter, origin)` collision is decided deterministically by `content_address`
  (`wins(addr_hi) = true`, `wins(addr_lo) = false`, for the same triple).
- Strictly-greater `wall` wins regardless of `content_address`; likewise `counter`, `origin`.
- Null current side (`COALESCE` path) → new wins.
- Byte-order determinism (a fixed pair of addresses orders the same way every call).

**Convergence regression** (DB-gated Rust integration — the real point): submit two *distinct*
events carrying an *identical* `(hlc_wall, hlc_counter, node_origin)` triple but different bodies
(→ different `content_address`) through the **remote-apply door** (`apply_remote_event`, `db/020` —
the only door that takes the wire HLC verbatim rather than assigning a fresh monotonic tick, and the
realistic foreign-node scenario). Apply them in **both arrival orders** (two independent patient
subjects / dispute ids, seeded oppositely) and assert the **same winner** each way. At minimum for
`patient_link` and `chart_dispute` (the clinician-visible flips); extend to the others if cheap.

Crypto material in tests is runtime-derived, never a literal (house rule 6).

## Explicitly out of scope

- **Group B** (demographic overlays `db/010`–`014`): already deterministic modulo TEXT-collation →
  deferred to #69.
- **#69** itself (collation-sensitivity of the intermediate `origin`/`value`/`display` TEXT
  comparisons): complementary and separately tracked. Adding `content_address` as the *final* key
  does **not** fix a divergence caused by two *different* origins collating differently under
  `(wall, counter)` tie — that is #69's remit. This design closes only the
  `(wall, counter, origin)`-all-equal (Byzantine self-reuse) case.
- **Part 2 of #115** other than the HLC-overlay bullet — the `cairn_event_twin` hard-require-twin
  registry and the `cairn_require_uuid` helper remain separate follow-ups.

## Why this is safe against the immutable-wire posture

The change is projection-read-side only. Signed event bodies, the wire format, content-addressing,
and every write-gate check are untouched. `content_address` already travels on every event; this
design only *stores it in the projection* and *consults it as a tiebreaker*. Convergence is a
property the spec already assumes; this makes the projection layer actually deliver it under an HLC
collision.
