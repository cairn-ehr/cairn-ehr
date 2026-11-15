# #1014 — supervision rows on the node's HLC (plan)

**Issue:** [#1014](https://github.com/cairn-ehr/cairn-ehr/issues/1014). **Refines:** ADR-0094 decision 5 ("the
comparison is at the millisecond") and closes its limit (d). **New ADR:** ADR-0107.

Paper-parity: not clinical-surface — no gesture is added or changed; only the clock the existing "— not co-signed"
judgement compares on moves, so step counts and timings are untouched.

## The defect

Supervision rows (`actor_event`'s six ops) are timed by `recorded_at = clock_timestamp()`. Note parts and vouches are
timed by the node's HLC wall. One peer event up to 24 h ahead ratchets the local HLC, so parts signed afterwards
carry a future wall: a `lift-cosign` recorded later in real time judges them as signed after it and clears their
"— not co-signed" (the slice's anti-goal); a `require-cosign` marks earlier parts; an `unlink-supervisor` revokes
earlier co-signs.

**Not in the issue, and load-bearing:** after a ratchet every local part AND the lift share ONE wall and differ only
in the HLC counter. Stamping the rows with the HLC wall but comparing walls alone would still clear the parts. The
compare must be on `(wall, counter)`.

## Decisions

1. **Additive columns** `actor_event.hlc_wall BIGINT`, `hlc_counter INTEGER` (db/004, `ADD COLUMN IF NOT EXISTS`),
   both or neither (CHECK, added once in a guarded `DO`).
2. **Only the six supervision ops are stamped**, by `node_hlc_tick()` inside `cairn_actor_append`, as the LAST step
   before the INSERT (hlc_state is the hot lock, always taken last — db/005's lock-order note). The vault ops
   (`confirm`, `rotate-key`, …) are never compared to an HLC and stay off that lock.
3. **`cairn_supervision_record()`** returns `hlc_wall` and `hlc_counter` too, ordered by them then `seq`. A row with
   no stamp (written before this fix, or restored from an older export) reads as
   `(floor_ms(recorded_at), 0)` — exactly today's millisecond rule, so old rows behave as before. The return type
   changes, so the function is dropped first, only when its old shape is present.
4. **Rust compares `(wall, counter)` lexicographically, `<=`.** `NoteRow` gains `recorded_counter`, `PartVouch` gains
   `counter`; `SupervisionRow` carries the stamp. `from_rows` no longer floors (the door does it for old rows).
5. **Restore carries the stamp.** `ActorRegistryRow` gains `hlc_wall`/`hlc_counter`
   (`skip_serializing_if = "Option::is_none"`, so an export with no stamped rows is byte-identical — a stamped one is a
   one-way door, as `related_actor` was). `restore_actor_registry` validates both-or-neither integers, inserts them,
   refuses a present row differing in them, and **merges `hlc_state` forward** to each stamp
   (`cairn_node_hlc_merge`): otherwise a part signed after a restore could sort before a restored `require-cosign`.

## Residual (stated in the ADR, not fixed)

A **peer's** part carries the peer's HLC. Against a local lift, its order is the HLC's happens-before order: a
causally-later part is always later, but a concurrent part from a peer up to 24 h ahead can still sort after a lift
it never saw. No local clock can fix that; it is bounded by db/001's drift ceiling.

## Tasks (TDD — each test RED first)

1. **db/004:** columns + CHECK + stamping in `cairn_actor_append`. Test (`db/tests/004_actors_test.sql`): a
   supervision op gets a stamp strictly above an HLC ratcheted ahead; a vault op gets none; both-or-neither refused.
2. **db/072:** the record door's columns, coalesce and order. Test (`db/tests/072_supervision_test.sql`).
3. **Rust `note/supervision.rs` + `read.rs` + `vouch_read.rs`:** `(wall, counter)` compare. Pure test: a lift on the
   same wall as a part but a higher counter does NOT clear it. Update every `NoteRow`/`PartVouch` literal (both trees).
4. **DB-gated Rust test (`note_supervision.rs`):** ratchet the HLC 2 h ahead with a peer stamp, sign a part, lift,
   read: the part still owes its co-sign. RED on `main`.
5. **Restore:** db/052 + `ActorRegistryRow` + `localstate_read.rs` + to_json; tests in
   `restore_actor_registry_supervision.rs` (round-trip, tamper refused, hlc_state merged) and `localstate.rs`.
6. **Docs:** ADR-0107, spec version bump, ADR index, durable-rules block, HANDOVER/ROADMAP.
