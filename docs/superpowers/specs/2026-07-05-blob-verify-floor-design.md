# Design — the blob self-verification in-DB floor (ADR-0013 point 11)

**Date:** 2026-07-05
**Spec home:** §3.14 (attachments — content-addressed blobs) / §6.6 (the lazy byte tier) /
ADR-0013 point 11 (blast radius: "content-verification on fetch" is safety-critical).
**Status:** design; implements a settled decision — no ADR, no spec edit, no new event type.

## 1. What this slice is

`db/003_blobs.sql` records an honest gap in its own comments (lines 27–32): the blob tier's
*self-verifying* property — bytes marked `present = TRUE` actually hash to the `blob_address`
that names them — is enforced only in L2 (`cairn-sync` verifies BLAKE3 before flipping
`present`). PostgreSQL cannot restate the check (pgcrypto has no BLAKE3), so the in-DB
`CHECK` constraint covers only length consistency. **A client with raw SQL access can today
store arbitrary bytes as any named blob**, and every honest consumer downstream (viewers,
renditions, exports, the swarm-serving path) would serve the wrong bytes under a
signed-event-referenced content address.

That is precisely the failure ADR-0013 point 11 names as the tier's one safety-critical seam:
*"a wrong-hash blob must never be served as the named one."* And principle 12 (uniform core,
plural edges) requires the floor to hold **in the database, unbypassably** — "even a client
talking raw SQL cannot break it." Every other safety property of the event core already lives
at that altitude (signature verification via `cairn_verify`, the twin floor, the attestation
gate); the blob tier's content binding is the one that was left as an L2 promise, with the
`cairn_pgx` fix explicitly named in the db/003 comment.

This slice closes the gap. It is deliberately **outside** the demographics/matcher/identity
territory under active parallel construction (`db/010`–`025`, `matcher/`,
`cairn-event::{demographics,evidence,identity,john_doe}`).

## 2. The mechanism — one implementation, thin wrapper, trigger floor

Following the ADR-0002 / cairn_pgx house pattern (ONE verify implementation, in-DB via a thin
pgrx wrapper — never a second implementation in SQL):

1. **`cairn_pgx` 0.2.0 → 0.3.0** gains two functions, mirroring the `cairn_verify` /
   `cairn_verify_error` pair:
   - `cairn_blob_verify(addr bytea, content bytea) → boolean` — true iff
     `cairn_event::blob_address(content) == addr` (the same function `cairn-sync` and the
     walking skeleton use; BLAKE3 multihash `0x1e 0x20` + 32 bytes). A malformed address
     (wrong length / wrong prefix) is simply *false* — fail closed, never an error path a
     hostile caller can steer.
   - `cairn_blob_verify_error(addr bytea, content bytea) → text` — NULL when the pair
     verifies, else a legible reason ("address is not a BLAKE3 multihash…", "content hashes
     to X, address names Y"). Diagnostics only; the floor gates on the boolean.

2. **`db/026_blob_verify_floor.sql`** — a trigger floor on `blob_store`:
   - `cairn_blob_present_guard()`: raises (with the legible error as DETAIL) unless
     `NEW.content` is non-NULL and `cairn_blob_verify(NEW.blob_address, NEW.content)`.
   - `BEFORE INSERT … WHEN (NEW.present)` and
     `BEFORE UPDATE … WHEN (NEW.present AND (NOT OLD.present
     OR NEW.content IS DISTINCT FROM OLD.content
     OR NEW.blob_address IS DISTINCT FROM OLD.blob_address))`.
     The UPDATE condition re-verifies on any transition into `present`, any content swap
     under a present row, and any re-keying — while a metadata-only update (media_type,
     fetched_at) never re-pays the hash.
   - A trigger, not a `SECURITY DEFINER` door + REVOKE: `blob_store` legitimately receives
     raw DML from the byte tier (`do_blobd`, `put-blob`) and the reference-learning helper;
     a trigger binds every writer including those, with zero call-site churn.
   - `db/003`'s honest-gap comment is rewritten to point at the now-real floor.

3. **Version floor**: `cairn-sync`'s `REQUIRED_PGX_FLOOR` moves `0.2.0 → 0.3.0` — a stale
   `.so` would otherwise fail schema load with an illegible `undefined function` error
   instead of the existing legible "rebuild + reinstall" message (issue #109 pattern).

## 3. What the floor deliberately does NOT cover (honest limits, recorded)

- **`blob_chunk` rows are not in-DB verified.** A chunk is bao-slice-verified by the fetching
  L2 against the address root *before* insert, but the DB cannot restate that: bao slice
  verification needs the interleaved tree nodes from the wire encoding, which are not stored.
  The floor sits at the present-flip — wrong chunks can only ever assemble into a whole-blob
  verify that FAILS, so they waste space, never serve wrong bytes as named ones.
- **`outboard` is not verified.** A wrong outboard makes this node serve slices that the
  *fetching* peer's bao decode rejects against the signed address root (self-verifying fetch,
  zero trust in the source — ADR-0013 point 4). It degrades availability, never integrity.
  Verifying it in-DB would cost a full bao re-encode per write for no integrity gain.
- **Reference-only rows (`present = FALSE`) are untouched** — a reference learned from a
  signed event stays cheap to record; the floor prices only the present-flip (one BLAKE3
  pass, ~measured at multi-GB/s even on Cortex-A76 — Bet B4).
- **A superuser can drop the trigger.** Same standing as every other floor piece (the
  submit_event grants, the CHECK constraints): the floor binds applications and agents, not
  the DBA. Nothing new here.

## 4. Blast radius / tier (§9)

Safety-critical, minimal surface: ~10 lines of Rust delegating to the existing
`cairn_event::blob_address` (already covered by cairn-event unit tests + the Bet A/B4
spikes), one PL/pgSQL guard, two trigger declarations. No new dependency (blake3 is already
a cairn-event dependency; AGPL-compatible: CC0-1.0 OR Apache-2.0).

## 5. Tests (TDD)

- `cairn_pgx` pg_tests: accept matching pair; reject flipped byte; reject malformed address
  (fail closed, both truncated and wrong-prefix); error text legible + NULL on success.
- `crates/cairn-node/tests/blob_floor.rs` (DB-gated, house harness): the hostile-client
  matrix — raw INSERT of wrong bytes as present → refused with legible DETAIL; verified
  bytes → accepted; content swap under a present row → refused; metadata-only update on a
  present row → allowed (no false rejection, no re-hash gate); `present=TRUE` without
  content → refused; `blob_note_reference` reference-only path unaffected; UPDATE flip
  (the `do_blobd` assembly path shape) with verified bytes → accepted.
- Existing `cairn-sync` blob tests (put-blob / gen-blob / blobd E2E) must stay green —
  they always wrote verified bytes, so the trigger must be invisible to them.
