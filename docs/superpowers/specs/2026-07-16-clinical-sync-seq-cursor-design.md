# Design — clinical-plane seq cursor + periodic full sweep (issue #196, review finding B1)

**Date:** 2026-07-16 · **Issue:** [#196](https://github.com/cairn-ehr/cairn-ehr/issues/196)
(P2 / B1) · **Plane:** clinical (`cairn-sync`) · **Tier:** safety-critical (§9 Rust + in-DB)

## Problem

The clinical-plane pull (`cairn-sync do_pull`) fetches
`event_log WHERE (hlc_wall, hlc_counter) >= watermark ORDER BY hlc_wall, hlc_counter, node_origin`
and never does a full sweep. The watermark (`sync_state.hlc_wall/hlc_counter`) is the max HLC
applied from that peer. **Any event that lands in the peer's `event_log` with an HLC _below_ an
already-advanced watermark is never fetched again** — a silent set-union / convergence violation
(the flagship guarantee). Two concrete ways it happens:

- **Multi-hop.** Node B learns an older-HLC event from node C _after_ node A's watermark for B has
  already advanced past that HLC → A never converges.
- **Self-stamped low HLC.** An L2 agent submitting through `submit_event` with an older `hlc_wall`
  (nothing structurally prevents a low wall) lands below every peer's watermark.

The only signal today is a `do_fingerprint` mismatch nobody is forced to compare.

The **node plane already solved this exact hazard in #38** (`db/007` + `cairn-node/src/sync.rs`):
it cursors on a monotonic, node-**local** insertion-order `seq` (never the HLC), plus a periodic
full sweep as the correctness floor. The clinical plane never inherited that treatment. This design
ports it.

## Approach (decided)

Port the #38 treatment onto the existing clinical plane (not a rebuild onto the node engine — the
two transports differ: node = async framed streaming, clinical = sync JSON batch). Three approved
sub-decisions:

1. **Migrate the quarantine re-offer floor from HLC to a seq `refused_seq`** (uniform with the node
   plane; removes the last HLC-keyed fetch point, which shares the same skip hazard).
2. **Ship the per-event seq as an additive `seqs: Vec<i64>` array** on the existing JSON
   `EventsResponse` (minimal, matches the current clinical transport; pagination/framing stays the
   separate issue #101).
3. Keep the two planes as **parallel implementations of the same pattern** — a deliberate, accepted
   duplication given the different transports (unification is possible later, not now).

## Mechanism

### 1. `event_log.seq` — the monotonic local cursor key

New migration **`db/036_clinical_sync_seq.sql`**, containing **only idempotent ALTERs** (no CREATE
TABLE widening, exactly like `node_event.seq` in db/007 line 44 — so there is no CREATE/ALTER
divergence and no `migration_replay_widening.rs` `WIDENED` entry is needed):

```sql
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS seq BIGINT GENERATED ALWAYS AS IDENTITY;
CREATE INDEX IF NOT EXISTS event_log_seq_idx ON event_log (seq);
```

- Node-**local** insertion order: a newly-_learned_ event (even one carrying an old HLC) gets a
  fresh high `seq`, so new knowledge always sorts _above_ any puller's cursor and can never be
  silently skipped.
- Never signed, never on the wire core — sync transport metadata only (principle 12 untouched).
- `GENERATED ALWAYS AS IDENTITY` is assigned at INSERT, so the existing `submit_event` /
  `apply_remote_event` INSERT column lists need **no change** (they never name `seq`; `GENERATED
  ALWAYS` also forbids an explicit value, which none supply). Additive per ADR-0012:
  `ADD COLUMN IF NOT EXISTS` does not fire the append-only UPDATE/DELETE trigger. On an existing
  large `event_log` the ALTER backfills the identity column once (a one-time upgrade cost, as the
  node plane accepted for `node_event`).

### 2. Per-peer seq cursor

```sql
ALTER TABLE sync_state       ADD COLUMN IF NOT EXISTS last_seq    BIGINT NOT NULL DEFAULT 0;
ALTER TABLE sync_quarantine  ADD COLUMN IF NOT EXISTS refused_seq BIGINT NOT NULL DEFAULT 0;
```

- `sync_state.last_seq` — the highest serving-node `seq` this node has pulled from `peer`
  (mirrors `sync_cursor.last_seq`). `cairn-sync`'s role already does raw DML on `sync_state`
  (node-local operational state, outside the append-only floor), so the checkpoint stays a raw
  **advance-only** `UPDATE ... SET last_seq = GREATEST(last_seq, $new)` — no new SECURITY DEFINER
  door (the node plane needed one only because its `cairn_node` role does zero raw DML).
- `sync_quarantine.refused_seq` — the serving `seq` at which an unverifiable event was refused
  (mirrors `node_event_quarantine.refused_seq`). Set on **INSERT only**; the dedupe/re-offer UPDATE
  leaves it (and `reason`/`first_seen`) untouched as the forensics. Legacy rows default to `0`
  (re-offer from the log start until they resolve — safe, self-limiting, matching the node plane's
  documented `refused_seq = 1 → after_seq = 0` known cost).
- The HLC watermark columns (`sync_state.hlc_wall/hlc_counter`, `quarantine_floor_wall/counter`)
  become **vestigial** and are **kept, deprecated-in-place** — NOT dropped. Dropping cleanly would
  require editing two foundational migrations (db/001 + db/021, else db/021's `ADD COLUMN IF NOT
  EXISTS` re-adds them every connect → per-connect add/drop churn), and — decisively — a `DROP
  COLUMN` is the non-additive move ADR-0012 / principle 11 forbid: an older cairn-sync binary
  reading a db/036-migrated DB (an _expected_ fleet state, since schema version is decoupled from
  binary version) still `SELECT`s `hlc_wall` and would break. Adding columns is downgrade-safe;
  dropping one is not. So db/036 and `do_pull` carry a one-line comment marking these columns
  superseded by `last_seq`/`refused_seq` (overlay the old meaning, never erase it). The pull metrics
  report the seq cursor instead of the HLC watermark.

### 3. Wire (additive, principle-12-clean)

```rust
enum Request {
    EventsAfter    { wall: i64, counter: i32 }, // KEPT: an older puller still works
    EventsAfterSeq { after_seq: i64 },          // NEW: the seq-cursor path; 0 = full sweep
    BlobSlice { .. },
}

struct EventsResponse {
    events: Vec<String>,
    #[serde(default)] attestations:  Vec<Option<String>>,
    #[serde(default)] attester_keys: Vec<Option<String>>,
    #[serde(default)] seqs: Vec<i64>,            // NEW: parallel per-event serving seq
    #[serde(default, skip_serializing_if = "Option::is_none")] signing_context: Option<String>,
}
```

Serve side for `EventsAfterSeq { after_seq }`:
```sql
SELECT seq, encode(signed_bytes,'hex'), encode(attestation,'hex'), encode(attester_key,'hex')
FROM event_log WHERE seq > $1 ORDER BY seq
```
`ORDER BY seq` is load-bearing: the puller relies on ascending-seq order to freeze the cursor at
the _contiguous_ handled prefix (same reasoning as node `sync.rs`).

**Version-skew honesty:** a new puller sends `EventsAfterSeq`; an _old_ serve that lacks the variant
fails to decode the request and the pull fails **loudly** (never silent). An old puller's
`EventsAfter` is still served. Both are documented as principle-12-additive changes.

### 4. `do_pull` rewrite (net simplification)

The seq model is simpler than the HLC `pin` logic it replaces:

1. Read the cursor: `last_seq` from `sync_state`.
2. Read the floor: `SELECT min(refused_seq) FROM sync_quarantine WHERE peer = $1 AND NOT acked`.
3. `after_seq = if full_sweep { 0 } else { floor.map_or(last_seq, |f| (f-1).min(last_seq)) }`
   (the `-1` because serve is strict `seq > after_seq`, mirroring node `pull_into`).
4. Request `EventsAfterSeq { after_seq }`; keep the existing `signing_context` skew pre-check and
   the hex-decode-as-unverifiable handling verbatim.
5. Per `(seq, event)` in ascending order — mirroring node `pull_into`:
   - **applies** → count; if `floor.is_some()`, auto-release any pen row for these bytes
     (`DELETE FROM sync_quarantine WHERE content_digest = event_address(bytes)`).
   - **verifiable but failed to apply** (transient/deterministic) → freeze: stop advancing the
     cursor below this seq (retried next cycle; never skipped).
   - **unverifiable** → pen with `refused_seq = seq`; on pen-quota refusal → freeze.
   - Track `max_seq` = highest **contiguous** handled seq (advance only while `!frozen`).
   - The event's `seq` comes from the parallel `seqs[i]` array; if the response carries events but
     an empty/short `seqs` (a malformed or unexpectedly-old serve), fail **loudly** rather than
     checkpoint blindly — the seq is load-bearing for the cursor.
6. Checkpoint advance-only: `UPDATE sync_state SET last_seq = GREATEST(last_seq, $max_seq), ...`
   when `max_seq > after_seq || full_sweep`.
7. **Loud failure** at cycle end if any unacked refusal remains (unchanged
   `PullIntegrityError` contract + diagnosis; `count(*) ... WHERE NOT acked` for the pending
   signal).

`do_pull` gains a `full_sweep: bool` parameter.

### 5. Periodic full sweep (the correctness floor)

- `const FULL_SWEEP_EVERY: u64 = 10;` (mirrors the node plane).
- `cmd_run`: `let full_sweep = cycle % FULL_SWEEP_EVERY == 0;` passed into `do_pull`.
  (`#[allow(clippy::manual_is_multiple_of)]` for MSRV 1.74, as the node plane does.)
- `cmd_pull` (one-shot CLI) gains a `--full` flag (default incremental); a manual operator sweep is
  then available without waiting for the cadence.

The sweep is **not** optional: it reconciles the residual BIGSERIAL out-of-order-commit gap (a txn
assigned `seq = 5` can commit before `seq = 4`; a puller reading in between would checkpoint past
`seq = 4`). Incremental = optimization; full sweep = correctness. They ship together.

### 6. `connect_checked` schema probe

Update the "newest piece" markers from `sync_state.quarantine_floor_wall` to the db/036 columns
(`event_log.seq` + `sync_quarantine.refused_seq`), so a DB predating this migration is caught with
the existing legible "run `cairn-sync init`" message instead of failing at runtime.

### 7. SCHEMA registration

`db/036_clinical_sync_seq.sql` is added to **both** explicit SCHEMA lists — `cairn-sync`
(`crates/cairn-sync/src/main.rs` `SCHEMA`) and `cairn-node` (`crates/cairn-node/src/db.rs`
`SCHEMA`) — since both load an explicit curated list (neither globs). Omitting it from cairn-node
would leave a node's clinical `event_log` without `seq`.

## Testing (TDD — RED first)

The load-bearing acceptance criterion inherited from #38: _prove an incremental pull cannot drop an
event under a low-HLC/out-of-order arrival, before any wire change is trusted._

New DB-gated tests in **`crates/cairn-sync/tests/clinical_pull.rs`** (the #199 real-binary A→B rig):

1. **`low_hlc_below_cursor_still_converges` (the headline RED test).** Author events on A, pull to B
   (B's cursor advances). Then land on A a fresh event whose **HLC is lower** than B's current
   cursor position. `submit_event` always advances A's HLC monotonically, so the faithful way to get
   a low-HLC event onto A is the **multi-hop path**: a third signer authors an event carrying an
   explicitly low `hlc_wall`, applied to A through A's remote-apply door — it lands in A's
   `event_log` with the low HLC but a **fresh high `seq`**. Pull B again (incremental). Assert B now
   holds that event. This test **fails on the current HLC-fetch code** (the event sorts below B's
   watermark and is never re-served) and passes on the seq cursor — the regression guard.
2. **`full_sweep_reconciles_a_skipped_seq`.** Deterministically force the out-of-order-commit gap:
   insert an event, then manually advance the cursor past its seq (as a commit race would), assert
   an incremental pull misses it, then a `--full` sweep applies it. Proves the sweep is the floor.
3. **`repull_from_zero_converges` (idempotence).** A full sweep from `after_seq = 0` re-applies the
   whole log as set-union no-ops and reaches an identical read-state — the ADR-0004 "watermark is a
   hint" invariant, now on seq.

Plus in-file/unit coverage in `main.rs` tests:

4. **Seq-based quarantine re-offer + auto-release.** An unverifiable event pins `refused_seq`; later
   pulls re-offer from below it; a repaired/re-signed version auto-releases the row and the pull
   goes quiet. (Adapts the existing clinical quarantine tests from the HLC floor to the seq floor.)
5. **Wire back-compat decode.** `EventsResponse` with no `seqs` field decodes (serde default) to an
   empty vec; the existing `events_response_decodes_pre_attestation_wire_format` test is extended to
   confirm the new field is additive.

Existing `do_pull`/watermark tests that assert on `sync_state.hlc_wall` are migrated to assert on
`last_seq`. `cargo fmt` + `clippy -D warnings` + full `cargo test --workspace` must be green.

## Non-goals / out of scope

- **#101 pagination** (the whole suffix still ships in one JSON batch on a sweep) — unchanged; a
  separate issue. The seq cursor _reduces_ steady-state re-shipping but does not paginate.
- Unifying the clinical and node sync engines into one module — deliberate parallel duplication.
- Dropping the vestigial HLC watermark/floor columns — additive-only; left in place.
- The #220 arrival-time-only veto re-check — a separate follow-up.

## Files touched

- `db/036_clinical_sync_seq.sql` (new) — the three additive ALTERs + index.
- `crates/cairn-sync/src/main.rs` — `SCHEMA` (+036), `Request` (+`EventsAfterSeq`), `EventsResponse`
  (+`seqs`), `serve_conn` (+ the seq arm), `do_pull` (seq cursor rewrite + `full_sweep` param),
  `quarantine_event` (+`refused_seq`), `cmd_run` (sweep cadence), `cmd_pull` (+`--full`),
  `connect_checked` (probe markers), `FULL_SWEEP_EVERY` const.
- `crates/cairn-node/src/db.rs` — `SCHEMA` (+036).
- `crates/cairn-sync/tests/clinical_pull.rs` — the three acceptance tests.
- HANDOVER.md / ROADMAP.md at close.
