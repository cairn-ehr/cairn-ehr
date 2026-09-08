# Design — DR slice 2d: restore reads the clinical plane back

- **Date:** 2026-09-09
- **Closes:** [#554](https://github.com/cairn-ehr/cairn-ehr/issues/554) — *restore does not read the
  clinical plane back: the medium holds the record, nothing gives it to a node.* This slice is the
  **read half**, and it is the half a solo clinic's survival actually depends on.
- **Produces:** `db/052` (SCHEMA 51 → 52) — a self-trusting actor-registry restore door and one
  additive column on the quarantine pen; a clinical reader in `cairn-node`'s `backup`; a reordered
  `restore` ceremony; per-event failure accounting with durable quarantine. **One ADR (2e's, which this
  slice now owes rather than defers)**, one migration, one additive `sync_quarantine` column.
- **Predecessors:** [2a](2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md) — the format
  (`crates/cairn-medium`, `CAIRNB3`); [2b](2026-09-02-dr-slice-2b-transport-seam-and-paged-pull-design.md)
  — the seam and the paging; [#511](2026-09-04-custody-newtypes-secret32-publickey32-design.md) — the
  custody newtypes; [2c](2026-09-04-dr-slice-2c-both-planes-captured-design.md) — **the write half**,
  whose §2.1 (custody on both carriers) governs this slice and is not re-derived here.

---

## 1. Why this piece exists

A solo clinic backs up nightly. `verify-backup` passes. The disk dies. `cairn-node restore` rehydrates
the medium — and the node comes back knowing who it had peered with and **zero patients**.

That sentence is unchanged by slice 2c. What 2c changed is that the bytes now *exist* off-machine to be
given back: the medium is a CAIRNB3 image carrying every `event_log` row with its wrapped DEK beside
it, and the `CAIRNL1` export carries the node's unwrap secret and its actor registry. Before 2c, a dead
disk was total loss and no later slice could have recovered it. Now only the reader is missing.

`backup::node_plane_events` returns the federation plane alone, deliberately, and that is the seam this
slice moves.

---

## 2. Four findings that shaped the design

These came out of reading the seam before designing against it. Two of them would have produced a
green, shipped slice that silently destroyed custody.

### 2.1 `apply_remote_event`'s `p_dek` is a PLAINTEXT DEK, not a wrapped one

`db/020_apply_remote_event.sql` step 9:

```sql
INSERT INTO event_dek (event_id, dek_wrapped)
VALUES (v_event_id, cairn_wrap_dek(p_dek, v_pub))
```

The door **wraps what it is handed**. Both carriers — the medium's `MediumRecord.dek_wrapped` and the
export's `EpisodeDek.dek_wrapped` — hold keys that are *already* wrapped to this node's unwrap public
key. Piping either straight into `p_dek` would **double-wrap every key in the clinic's record**,
producing `event_dek` rows that unwrap to noise. Every test would pass: the rows exist, the counts
agree, `verify-backup` is green. The defect surfaces only when a clinician opens a chart, months later,
on a node that can no longer be re-restored.

The door also *needs* the plaintext: `cairn_unseal_body(container, dek, event_id)` takes the DEK
itself (the body is sealed under the DEK; the DEK is what the unwrap key protects). Without it there is
no clear view, so no twin, no projection, no chart — the "zero patients" outcome in a new costume.

**Decision: `restore` unwraps each DEK in Rust with the inherited unwrap secret and passes the
plaintext to the door**, which re-wraps it to the registered public half. This is not a new idiom: it
is exactly what `cairn-sync`'s `apply_signed` already does on every pull, where the puller unwraps for
its own key before knocking. The round-trip is deliberate — it re-derives custody through the one door
that owns `event_dek`, rather than teaching a second site how to write it.

**Consequence, and it is the whole reason §3 reorders the ceremony:** the unwrap key must be installed
*before* any clinical event is applied.

### 2.2 The quarantine pen holds no custody, and for a restore that is fatal

`sync_quarantine` (db/021) stores `signed_bytes`, `attestation`, `attester_key` — never a DEK. The
requeue path passes `None` and says why:

> No sidecar DEK on the requeue path: the quarantine pen holds only the refused signed bytes +
> attestation pair, never custody. A re-queued sealed event is admitted structurally without custody;
> its DEK rides a later normal pull once the peer serves it.

**That reasoning is sound for sync and false for restore.** A restored solo node has no peer. The
medium is the only carrier of that key, and `finalize_identity` fences the restore door behind the
operator. Penning a sealed clinical event as-is would preserve the bytes, silently drop the key, tell
the operator *"quarantined — requeue after fixing the cause"*, and the requeue would then admit
permanently-unopenable ciphertext at exit 0. That is #500's own shape one layer down, inside the slice
built to end it.

**Decision: the pen gains an additive `dek_wrapped BYTEA` column** and `do_requeue` unwraps and passes
it. This strictly improves the sync path too — a penned sealed event whose peer is later decommissioned
becomes recoverable instead of lost. The column is nullable: an unsealed event, and every row penned
before this migration, legitimately has none.

### 2.3 A restore must NOT pin the re-offer floor

`sync_state.quarantine_floor_seq` exists so a peer keeps re-offering a refused slot until it is fixed;
pulls fetch from `min(watermark, floor)`. **No peer re-offers a medium.** A floor pinned by a restore
would make the restored node's *first real pull* re-fetch from a position no peer will ever resolve,
wedging federation over an event that has nothing to do with any peer.

**Decision: restore pens without pinning a floor.** `cairn-sync requeue` is the release mechanism for a
restore-penned row, and it reads the pen directly rather than through any watermark.

### 2.4 `ActorRegistryRow.recorded_at` must refuse, not default

The field carries `#[serde(default)]`, justified in its own doc as degrading *"rather than refusing the
whole row over one audit field."* It is not an audit field. `actor_current` (db/004) resolves the trust
anchor with

```sql
WHERE NOT EXISTS (SELECT 1 FROM actor_event r
                  WHERE r.actor_id = ae.actor_id AND r.op = 'revoke'
                    AND (r.recorded_at, r.seq) >= (ae.recorded_at, ae.seq))
```

`recorded_at` is the **primary ordering key deciding who may author**. A restored `enroll` whose
timestamp defaulted to empty-or-now would outrank a genuine older `revoke` and silently re-authorise a
recalled actor — the resurrection hazard the `IS DISTINCT FROM` guard a few lines below exists to
prevent, arriving through the door built to restore the registry.

**Decision: drop the `#[serde(default)]` from `recorded_at` and refuse a row without it.** Harmless
until now only because nothing installed these rows; #554 item 4 asked for the decode-refusal test, and
this is what that test finds.

---

## 3. The ceremony reorders: `finalize_identity` moves LAST

Today (`main.rs`, the `restore` arm):

```
mint new key → apply node plane (needs an UN-ENROLLED db) → finalize_identity (writes local_node,
fences the door) → apply_local_state (installs the unwrap key)
```

`main.rs` already carries the note that this is temporary: *"this runs AFTER `finalize_identity`, which
is correct only while no CLINICAL event is applied here … When the medium starts carrying clinical
events (#500) this block moves up ahead of step 5."* Finding 2.1 confirms it and finding 3 extends it.

**New order:**

```
mint new key
  → apply node plane          (restore_node_event; un-enrolled fence)
  → apply_local_state         (installs the unwrap key + REGISTERS its public half; installs the actor registry)
  → apply CLINICAL plane      (apply_remote_event, plaintext DEK per record)
  → finalize_identity         (new genesis + supersede; fences everything closed)
```

**Two preconditions, verified rather than assumed:**

- `cairn_register_unwrap_key` (db/037) never reads `local_node` — it is a singleton registrar guarding
  only against a *differing* key. It works on an un-enrolled database.
- `db/020_apply_remote_event.sql` contains **zero** references to `local_node`. Its HLC merge goes
  through `cairn_node_hlc_merge`, which updates `hlc_state` (created in db/001, not gated on
  enrollment).

**Why last is better than merely later.** The whole restore now happens inside the un-enrolled fence, so
a clinical apply that fails catastrophically leaves a database with **no genesis written** — still
legitimately restorable, from the same medium, into the same database. Under the minimal reordering
(finalize where it is, clinical after it) the same failure leaves a node already identity-minted and
already fenced, whose only recovery is a fresh database: trap 4's neighbourhood, reached by an ordinary
failure rather than an operator error.

One consequence to state plainly: `restore_node_event`'s fence (`local_node` empty) now protects a
longer window. That is the direction that fails safe — the door stays a permanent no-op on any live
node, unchanged.

---

## 4. `db/052` — the actor-registry restore door

```sql
restore_actor_event(p_row JSONB) RETURNS void
```

SECURITY DEFINER, `SET search_path = public, pg_temp`, granted to `cairn_node` only.

**Fenced twice.** It refuses if `local_node` is non-empty (the `restore_node_event` fence), *and* it
refuses if `actor_event` already holds any row. The second fence is what makes the door structurally
unable to inject into a live registry: a node that has ever enrolled an actor is not a restore target.

**Ordering.** Rows are inserted in **ascending source-`seq` order** into what the fence guarantees is an
empty table, and `seq BIGINT GENERATED ALWAYS AS IDENTITY` assigns fresh values. The restored relative
order is therefore exact, without `OVERRIDING SYSTEM VALUE` (which db/004's own comment asks to keep
loud in review) and without leaving the identity counter behind the restored maximum — the bug an
explicit-`seq` restore would plant for the *next* `enroll_actor`.

`recorded_at` is carried verbatim and is **required** (finding 2.4).

**It deliberately bypasses `enroll_actor`'s collision guards (#152 / #166), and this is the one place
that needs saying out loud.** Those guards refuse a *fresh* enroll that would silently merge two actors
or resurrect a retired one. A restore is replaying a history that already passed them on the dead node;
re-running them would refuse this node's own legitimate `revoke` and `supersede` rows — every one of
which trips `cairn_actor_id_key_conflict` by construction, because prior registration history is
exactly what they are. The door therefore validates *shape* (`op` in the CHECK set, `actor_id` present,
`recorded_at` present) and replays; it does not re-adjudicate.

**What authenticates these rows.** Nothing per-row. They arrive inside the `CAIRNL1` export, authenticated
by that container's AEAD and nothing else — **the one part of a restore that is not verify-on-apply.**
The clinical events around them are each individually signature-verified by `apply_remote_event`; the
registry is not. This is accepted deliberately: whoever holds the export *and* its passphrase or
recovery code already controls the restored node completely, so refusing here costs the record and buys
nothing. It is recorded in the ADR (§8) and printed to the operator at restore time, because a
limitation that lives only in a design doc is a limitation nobody will find.

**And one additive column** on the pen, per finding 2.2:

```sql
ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS dek_wrapped BYTEA;
```

---

## 5. The reader

`backup::clinical_plane_records(&MediumImage) -> Result<Vec<MediumRecord>, BackupError>` — the sibling
of `node_plane_events`, returning whole `MediumRecord`s rather than bare `Vec<u8>` because the clinical
plane carries three things the federation plane does not: the attestation pair and the wrapped DEK.

`node_plane_events` keeps its exact current shape, its `Legacy`/`V3` arms and its tests. A legacy
(CAIRNB1/B2) medium has no clinical plane at all, so `clinical_plane_records` returns empty for it —
honestly, since those media were written before the plane existed. An `Unknown(tag)` segment is **not**
read as clinical: `plane_counts().unknown` already surfaces it, and `verify-backup` already refuses a
medium carrying a plane this build cannot read.

**Records are returned sorted by `source_seq` ascending across the WHOLE medium, not in medium order,
and the difference is load-bearing.** 2c's capture backfills burned-`seq` gaps **newest-first** under a
bounded probe budget, so a record with a *lower* `source_seq` can legitimately sit in a *later* segment.
Applying in raw medium order would then offer an overlay before the event it targets, and db/020 would
refuse it with *"overlay targets unknown event"* — a self-inflicted pen entry, on a medium that carried
everything needed.

Sorting by `source_seq` restores causal order because `event_log.seq` **is** causal on the node that
wrote it: a locally-authored overlay is inserted after its target by construction, and a replicated one
could not have been admitted at all before its target (db/020 refuses it, and the puller pens it), so no
overlay ever holds a lower `seq` than the event it targets.

A duplicate `source_seq` — the same event captured by both the watermark pass and a gap probe — needs no
special handling: the apply door is idempotent, so the second offer is a set-union no-op.

---

## 6. Failure policy: skip, count, pen, report

**Per-event.** A refusal from `apply_remote_event` — an unenrolled signer, a DEK that will not unwrap,
an overlay whose target the medium lost at a burned `seq` (#549) — does not abort the restore. It is
counted by reason and penned into `sync_quarantine` with:

- `peer = '(restore)'` — an explicit sentinel, not an empty string. `peer` is `NOT NULL`, and both the
  per-peer quota probes (#197) and the mixed-version diagnosis group on it, so a restore-penned row
  must be identifiable as one rather than blend into an unnamed link.
- `reason` = the door's legible refusal text, prefixed to name the restore as its origin.
- `dek_wrapped` = the record's custody, **preserved** (finding 2.2).
- **no** `quarantine_floor_seq` pinned (finding 2.3).

This follows 2c's torn-medium ruling directly: in the one command that exists for the disaster where
re-running the backup is impossible, converting a partial loss into a total one is the wrong trade.
No confirmation dialog (principle 3).

**No usable export.** If `apply_local_state` installed no unwrap key — no passphrase (every unattended
cron run), a corrupt `.lsk`, an export that carries rows but no key — then:

- a record whose `dek_wrapped` is `None` **restores normally**, with `p_dek` NULL;
- a record that **carries** a `dek_wrapped` is refused *by this slice*, not by db/020's lenient arm, and
  penned with that custody intact.

The test is the record's custody, **not** whether the event is sealed, and those are different questions.
A body shredded before its first capture is sealed and arrives with `dek_wrapped = None` — its ciphertext
travels, its key was destroyed (trap 7's mirror half: *a shred destroys the key, never the event*) — and
it must restore, custody-less, exactly as it stands on the dead node. Keying the decision off sealedness
would pen it forever over a key that does not exist and is not supposed to.

The distinction matters and is the reason not to reuse db/020's existing behaviour here. That arm
downgrades a missing unwrap key to a `WARNING` and admits the event without custody — correct for a
*puller*, which will see the DEK again on a later cycle. For a restore it would admit ciphertext into a
node that `finalize_identity` then fences, with no second delivery ever. Penning instead keeps both the
bytes and the key, so recovering the export later and running `cairn-sync requeue` completes the
restore without redoing it.

**Exit code.** Non-zero if any clinical event was refused, after the full operator summary has printed —
the same discipline the local-state block already uses, and for the same reason: the `new node` /
`supersedes` / `re-peer with …` lines are the operator's next step and must not be lost to a late
failure.

---

## 7. Testing

The proof this slice owes is **end-to-end and clinical**, not structural. Structural evidence is what
2c already has.

1. **The inverted pin.** `nothing_yet_restores_a_clinical_event_from_a_medium` becomes
   `a_clinical_event_restores_from_a_medium` — inverted, never deleted, with its doc rewritten to say
   it is now a GUARANTEE and what reddens it.
2. **The end-to-end guarantee (the one that matters).** Seal a real clinical body on node A, capture it,
   restore into a fresh node B from medium + export, and **read the payload back in clear** through its
   projection. This is the only test that can distinguish a correct restore from the double-wrap of
   finding 2.1, because a double-wrapped `event_dek` row is present, well-formed, and the right length.
3. **The double-wrap regression, named.** A direct assertion that the DEK reaching `apply_remote_event`
   is the plaintext one — so a future "simplification" that passes `dek_wrapped` through reddens here
   with a legible reason rather than at (2) with a decryption failure.
4. **Custody survives the pen.** Pen a sealed event during a restore, then `requeue` it, and assert the
   body opens. Fails loudly against today's `None`-passing requeue.
5. **The registry ordering property.** A dead node whose history is `enroll(actor) → revoke(actor)`
   restores to a registry where that actor is **absent from `actor_current`**. Reddens if the rows are
   inserted out of order or if `recorded_at` is defaulted.
6. **`recorded_at` refuses.** A CBOR `ActorRegistryRow` with no `recorded_at` fails to decode
   (#554 item 4's decode-refusal test, aimed at the field finding 2.4 identifies).
7. **The no-export path.** A restore with no export recovers the federation plane and every unsealed
   event, pens every sealed one **with its DEK**, and exits non-zero.
8. **The fences.** `restore_actor_event` refuses on a node with `local_node` set, and refuses on a
   database whose `actor_event` is non-empty.
9. **No floor pinned.** After a restore that penned an event, `sync_state.quarantine_floor_seq` is
   still NULL, and a subsequent pull is not dragged backwards.

---

## 8. The ADR this slice owes

2c deferred an ADR to "2e". Three decisions here are architectural rather than local, and an
undocumented decision is one the next session re-litigates:

1. **The actor registry re-enters on container AEAD alone** — the single exception to verify-on-apply in
   a restore, and why it is accepted (§4).
2. **Erasure does not propagate backwards into media already written**, and completing it across
   backups is **rotation** — capture fresh, destroy old — whose interval *is* the maximum time an
   erasure takes to complete across all copies. That is the clinic's policy call, not Cairn's
   (principle 9; ADR-0005's *deletion is best-effort and declared*). 2c's design §2.1 established this
   and HANDOVER trap 7 records it; the ADR owes it **in as many words**, which is what 2c said and did
   not do.
3. **The restore ceremony's order is load-bearing**, not incidental: custody and the registry precede
   the clinical apply, and identity is minted last so a failed restore leaves a restorable database.

ADR-0026 decision 1's clinical promise may be cited as met **only** once this merges — and promise 2
(*node-default data-at-rest keys survive*) still has **no subject**: no node-default key tier exists, so
it is neither honoured nor violated. Do not let this slice's success be read onto it.

---

## 9. What is still broken when this merges

- **#549** — a burned IDENTITY `seq` is indistinguishable from a lost clinical event, and `unfilled_gaps`
  has no operator surface. This slice makes the consequence visible (such an overlay pens with a
  legible reason) without closing the diagnosis.
- **#552** — a nightly capture is O(whole medium). This slice adds *parses* on the read side at the same
  seam. Restore is a once-per-disaster command, so the cost lands where it is affordable, but the
  measurement in §10 must be taken rather than assumed.
- **#551** — the kit-restorability figure still has no per-kit home; a same-mount-point rotation can
  still false-green.
- **#553** — an unmarked foreign legacy medium can still be destroyed by succession.
- **#523** — a zero-filled tail is refused rather than recovered (`cairn-medium`'s to fix).
- **#536** — an unopenable DEK is counted nowhere on the sync path. This slice counts it on the *restore*
  path only; the sync half stays open.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** a practice whose premises burn recovers its charts from off-site storage — the
physical box of folders, or the microfiche/scanned archive held by a records service.

**Step count.** Paper *N*: retrieve the box, and the charts are readable — 1 human act, with the
retrieval itself often taking days. Architecture-forced *M*: mount the medium, run `restore`, supply
the recovery code — **3**, and it is the same 3 the command already has today; this slice adds no
operator step, it changes what those steps recover. UI bundling target *K* = 3. `M > N` in raw count
but not in kind: paper's single act hides the days of retrieval that Cairn's three acts replace with a
local file read, and the paper box cannot be verified before it is trusted, which `verify-backup` can.

**Budget.** A restore of 10 000 clinical events completes in **< 120 s** on the reference rig, and the
first chart is readable immediately afterwards without further operator action. The budget is
deliberately loose against the capture's `< 60 s` for the same volume: a restore parses, verifies a
signature per event, unwraps a DEK per sealed event, and re-wraps it — several times the per-event work
of an append. **Measured by this slice** (it is the first to expose a runnable restore of clinical
content). If the measurement falls outside the budget, that is the finding — file it, never adjust the
budget.
