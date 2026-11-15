# ADR-0107 — The supervision record is stamped on the node's HLC

- **Status:** Accepted (fixes [#1014](https://github.com/cairn-ehr/cairn-ehr/issues/1014): the supervision
  record and the note parts it judges are now compared on one clock)
- **Date:** 2026-11-15
- **Spec version at acceptance:** 0.109
- **Issues:** [#1014](https://github.com/cairn-ehr/cairn-ehr/issues/1014). Plan
  `docs/superpowers/plans/2026-11-15-1014-supervision-on-the-hlc.md`.
- **Supersedes in part:** [ADR-0094](0094-supervision-and-the-mandatory-co-sign.md) decision 5's bullet "The comparison
  is at the millisecond", and closes its limit (d). Every other part of ADR-0094 stands.
- **Builds on:** [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) (the HLC is `t_recorded`) ·
  [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (an in-place db/ edit replays by its content
  hash; no new file, `SCHEMA_GENERATION` 82 unchanged)

## Context

On paper a probationer's entry waits for the supervisor's countersignature, and lifting the probation afterwards
does not countersign yesterday's entries. ADR-0094 built that as "as of *t*": each note part, and each vouch, is
judged against the supervision record as it stood when the part entered the log.

The two sides were on two clocks. A supervision row (`actor_event`: `link-supervisor`, `unlink-supervisor`,
`require-cosign`, `lift-cosign`, `grant-assigner`, `withdraw-assigner`) was timed by `recorded_at`, the database's wall
clock. A part and a vouch are timed by the node's HLC. The HLC ratchets to the newest wall it has seen, and db/001
admits a peer event up to 24 h ahead. After one such event, every local part signed afterwards carries that future
wall. A `lift-cosign` recorded later in real time was then judged EARLIER than those parts, and cleared their
"— not co-signed". That is the anti-goal ADR-0094 was built to prevent. A `require-cosign` could likewise mark parts
signed before it, and an `unlink-supervisor` could revoke co-signs given before it.

## Decision

1. **The six supervision ops are stamped on the node's HLC.** `actor_event` gains `hlc_wall BIGINT` and `hlc_counter
   INTEGER` (additive, both or neither: `actor_event_hlc_pair`). `cairn_actor_append` takes a `node_hlc_tick()` for
   those six ops only, as the last step before its INSERT. `hlc_state` is the hot lock and is always taken last
   (db/005's lock-order note). A rollback rolls the tick back with the row, which is harmless because nothing else saw
   it. The vault ops (`confirm`, `rotate-key`, …) are never compared to an HLC. They stay unstamped and off that lock.
   `recorded_at` stays as written, and `actor_current` and the clock-behind refusal still read it.
2. **The compare is on `(wall, counter)`, never the wall alone.** After a ratchet, every local part AND every
   supervision act shares one wall and is ordered only by the HLC counter. A wall-only compare would still let the
   lift clear the parts. `note::supervision` compares `Stamp = (i64, i32)` tuples with `<=`. `NoteRow` gains
   `recorded_counter` and `PartVouch` gains `counter` (their `event_log.hlc_counter`). The millisecond floor in
   `Supervision::from_rows` is gone.
3. **Rows written before this fix keep their old rule.** `cairn_supervision_record()` returns `hlc_wall` and
   `hlc_counter`, ordered by them and then by `seq`. A row with no stamp reads as `(recorded_at floored to the
   millisecond, 0)`. With `<=`, that row is in effect at any part in its millisecond, which is exactly ADR-0094's
   millisecond rule, so an old row judges as it did. The door's return type grew, so db/072 drops the old shape first,
   and only when that shape is present, so a later replay never drops it again.
4. **Restore carries the stamp and moves the HLC forward.** `ActorRegistryRow` gains `hlc_wall` / `hlc_counter`
   (`skip_serializing_if = "Option::is_none"`). An export with no stamped row is byte-identical to the older one. One
   that carries a stamp will not decode on an older build (`deny_unknown_fields`), the same one-way door as
   `related_actor` (ADR-0094 decision 7). `restore_actor_registry` (db/052) refuses a half-pair or a non-integer, naming
   both fields. It refuses a wall past now plus db/001's drift ceiling, because no honest node minted one and merging
   it would park the HLC in the future. It refuses a present row that differs in either field. It merges `hlc_state`
   forward past every stamp it brings back (`cairn_node_hlc_merge`). Without that merge, a fresh node's HLC starts
   behind the restored stamps, and a part signed after the restore could sort before a restored `require-cosign` and
   escape it.

## Consequences

- The DB test suites no longer reset `hlc_state` in their setup to work around the two clocks.
  `note_supervision.rs::a_lift_after_a_ratcheted_hlc_leaves_the_earlier_part_waiting` ratchets the HLC two hours
  ahead on purpose and still finds the part waiting.
- A supervision act now takes `hlc_state`'s row lock until its transaction ends. Each act is one door call in its own
  transaction, so the lock is held for the length of that statement.

## What is not guaranteed

- **A peer's part carries the peer's HLC.** Measured against a local act, its order is the HLC's happens-before
  order. A part the peer signed after seeing the act is always later. A CONCURRENT part from a peer whose clock is
  ahead (up to db/001's 24 h ceiling) can still sort after a lift it never saw. No local clock can fix that. The
  record itself stays node-local (`actor_event` does not sync), as ADR-0094 says.
- Rows written before this fix still compare `recorded_at` against an HLC wall, with limit (d)'s exposure. Pre-clinical,
  no such record matters today.

Pins: `db/tests/004_actors_test.sql` (#1014 block), `db/tests/072_supervision_test.sql` (3.);
`note/supervision.rs`'s `a_lift_on_the_parts_wall_with_a_later_counter_does_not_clear_it`,
`an_unstamped_rows_millisecond_is_in_effect_at_the_part`;
`note_supervision.rs::a_lift_after_a_ratcheted_hlc_leaves_the_earlier_part_waiting`;
`restore_actor_registry_supervision.rs`'s `the_supervision_ops_survive_a_restore`, `a_tampered_hlc_stamp_is_refused`.
