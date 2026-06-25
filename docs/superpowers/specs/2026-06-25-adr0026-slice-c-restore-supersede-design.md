# ADR-0026 slice C — backup restore (apply) + new-identity supersede

- **Date:** 2026-06-25
- **Issue:** [#50](https://github.com/cairn-ehr/cairn-ehr/issues/50)
- **Refs:** [ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) §7.10;
  slice B module `crates/cairn-node/src/backup.rs`; federation floor `db/007_node_federation.sql`.
- **Status:** Approved (brainstorming) — pending implementation plan.

## Why this exists

Slice B exports a node's signed `node_event` set to a self-verifying cold-peer medium. The
**apply/restore-into-a-DB** half was deferred because it needs a door the live federation floor
deliberately does not provide: a *self-trusting* apply that rehydrates a node's **own** history into a
fresh DB **without** a peer-trust set (which a fresh node does not have yet). That door is a
federation-admission bypass if mis-scoped, so it is designed together with the identity ceremony, not
bolted onto the live `apply_remote_node_event` path.

This slice realizes ADR-0026 points 1/2/4 at the node-federation tier: a solo node, restored from its
medium plus a fresh recovery secret, recovers its **event history** (verifiably), comes back under a
**new** supersede-linked identity (the private signing key is never backed up), and **re-peers** from
empty.

## Two keystone decisions (resolved in brainstorming)

1. **Fence the restore door with an empty-genesis precondition.** `restore_node_event` fails closed
   unless `local_node` is empty (node not yet enrolled). This is structural, in-DB, and fail-closed: a
   live node *always* has a genesis, so the door is a permanent no-op there — no extra session state, no
   second role to provision. (Alternatives weighed: a distinct restore role grant; both. Rejected as more
   moving parts for a door the empty-genesis check already neutralizes on a live node.)
2. **Resolve the dead node_id by auto-detect + explicit fallback.** If the medium carries exactly one
   `enroll` (the solo-clinic case — ADR-0026's primary deployment), restore auto-detects it as self,
   zero-config. If the medium carries multiple enrolls (a federated node whose log also holds peers'
   genesis events), restore requires an explicit `--superseded-node <hex>` and fails closed otherwise.
   No medium-format change — the medium stays pure (ADR-0026 point 2), and this is forward-compatible with
   the deferred sealed local-state export (point 3), which can later supply the old identity automatically.

## The restore flow

`cairn-node restore --from <medium>` runs against a **fresh, schema-loaded, un-enrolled** DB:

1. **Verify the medium** — reuse slice B `backup::verify_medium_bytes`; bail on any tamper/bit-rot. The
   medium is only read, so a previous good medium is never at risk.
2. **Resolve the dead node_id** — single-enroll auto-detect, else require `--superseded-node <hex>`.
3. **Mint a fresh sealed keypair** — exactly like `init` (`keystore::generate_sealed`: operational
   passphrase + a one-time recovery code shown once). The old signing key is never on the medium and is
   never reconstructed.
4. **Apply old events** through the new self-trusting `restore_node_event` door — works only while
   `local_node` is empty. Appends old genesis + old peer/revoke events as historical, signature-verifiable
   rows. Does **not** touch `local_node`.
5. **Author new genesis** (`submit_node_event`, `node.enrolled`, new key) → sets `local_node = NEW`, which
   **permanently fences the restore door closed**.
6. **Author supersede** (`submit_node_event`, `node.superseded`, new key, subject = OLD node_id).
7. `trust_peer` is now empty (old peer events have `author ≠ NEW`) → the node must **re-peer**, exactly as
   ADR-0026 point 4 intends. No code.

**Ordering is load-bearing:** every old event must apply in step 4 *before* step 5 closes the door. The
DB mutations are arranged so a mid-restore failure leaves a cleanly re-runnable state (an un-enrolled DB
with no `local_node`); exact transaction boundaries are an implementation-plan detail. To retry an
interrupted restore, start from a clean DB — the medium is immutable and a fresh key is minted per run
(see Honest limitations).

## Schema — `db/009_node_supersede_and_restore.sql`

- **Widen the op CHECK (additive, ADR-0012):** `ALTER TABLE node_event DROP CONSTRAINT IF EXISTS
  node_event_op_check`, then `ADD CONSTRAINT node_event_op_check CHECK (op IN
  ('enroll','peer','revoke','supersede'))`. Widening a CHECK to a superset is forward-compatible — it
  rejects nothing it previously accepted.
- **`restore_node_event(p_signed bytea) RETURNS uuid`** — `SECURITY DEFINER SET search_path = public`.
  Verifies `cairn_verify` + the content-address invariant (the same checks slice B proves catch a tampered
  medium), **but performs no peer-trust check**. Fenced fail-closed:
  `IF EXISTS (SELECT 1 FROM local_node WHERE id) THEN RAISE EXCEPTION 'restore_node_event: node already
  enrolled; restore is only into a fresh node (the live apply path is apply_remote_node_event)'`. Inserts
  the event (any op) `ON CONFLICT (node_event_id) DO NOTHING`; merges the HLC forward like the live apply
  path. **Never writes `local_node`.** Grants: `REVOKE EXECUTE ... FROM PUBLIC`, `GRANT EXECUTE ... TO
  cairn_node`.
- **Extend `submit_node_event`** — add `WHEN 'node.superseded' THEN 'supersede'` to the op map. The
  `supersede` op is authored only by this node's current key (same guard as peer/revoke); subject =
  `payload.superseded_node_id_hex` (a **distinct** field, not `peer_node_id_hex`, for legibility — the
  superseded node is not a peer). Reject a missing `superseded_node_id_hex` with a legible error rather
  than storing a `\x00` subject.
- **`node_lineage` view** — exposes supersede edges (`new_node_id ← superseded_node_id`) for `status` and
  audit, keyed off `node_event WHERE op = 'supersede'`.

All of the above lives in one `BEGIN; … COMMIT;` migration, consistent with `db/007`.

## Rust

- **`identity::author_supersede(db, sk, key_id, node_origin, old_node_id_hex)`** — mirrors
  `author_peer`/`author_unpeer`: ticks the HLC, builds a `node.superseded` body with
  `payload.superseded_node_id_hex`, signs, submits via `submit_node_event`.
- **`restore.rs` (new module)** — the restore orchestration (verify → resolve dead id → mint key →
  apply loop → new genesis → supersede), kept out of `main.rs`/`identity.rs` so each file stays focused
  and under the ~500-line house guide. Pure helpers (dead-node-id resolution from a parsed event set) are
  separated from the DB/IO orchestration so they unit-test without a DB.
- **`status`** — add a lineage line when a supersede exists, e.g.
  `identity      <new>… (supersedes <old>…)`.

## Testing (TDD, red-first)

**DB-gated (`crates/cairn-node/tests/restore.rs`):**
- `restore_door_rejects_on_a_live_node` — with a genesis present, `restore_node_event` raises the legible
  "already enrolled" error.
- `restore_door_accepts_into_an_empty_db` — into a fresh DB, a validly-signed event is appended.
- `restore_door_rejects_a_tampered_event` — a bit-flipped medium event fails the door's signature check.
- `restore_round_trip` — back up a node, restore into a fresh DB under a new identity, then assert:
  (a) old events present; (b) `local_node` = new id; (c) supersede recorded with subject = old id;
  (d) `trust_peer` empty; (e) the restore door is now fenced closed.

**Pure unit (`restore.rs`):**
- dead-node-id resolution: single-enroll auto-detect; multi-enroll requires an explicit arg (error
  otherwise); an explicit arg overrides auto-detect.

**SQL (`db/tests/009_*`):**
- the op-CHECK now accepts `supersede` and still rejects an unknown op;
- `node_lineage` resolves a supersede edge.

## Scope boundary / honest limitations

**Out of scope (deferred, per issue #50):**
- the sealed **local-state export** (ADR-0026 point 3 — node config, draft/scratchpad store,
  sealed-episode DEKs) — independent of the apply path, its own future slice;
- **shred-replay-before-projection** (ADR-0026 point 6) — N/A at the node-federation tier (no clinical
  bodies / DEKs in `node_event`); it lands when the clinical event tier exists;
- Shamir M-of-N, QR rendering, TPM/keyring escrow.

**Honest limitations (documented, not engineered away):**
- Restore targets a fresh un-enrolled DB; an interrupted restore is retried from a clean DB (the medium is
  immutable; a fresh key is minted per run). Partial-restore resumption is not built in this slice.
- A false-death node that later resurrects reconciles via `revoke` (both identities valid, never merged —
  ADR-0026 point 4); that reconciliation UX is out of scope here.
- The private signing key deliberately does not survive (ADR-0026 point 4); re-peering after restore is
  expected and is a no-op for a solo node.
