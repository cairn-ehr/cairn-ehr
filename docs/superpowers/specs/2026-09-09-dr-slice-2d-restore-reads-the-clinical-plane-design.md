# Design — DR slice 2d: restore reads the clinical plane back

- **Date:** 2026-09-09
- **Closes:** [#554](https://github.com/cairn-ehr/cairn-ehr/issues/554) — *restore does not read the
  clinical plane back: the medium holds the record, nothing gives it to a node.* This slice is the
  **read half**, and it is the half a solo clinic's survival actually depends on.
- **Produces:** `db/052` (SCHEMA 51 → 52) — a self-trusting, set-shaped actor-registry restore door
  (§4), a shared `cairn_quarantine_event` pen door (§6.1), and one additive column on the quarantine
  pen; the `verified_through` + `source_seq` derivation lifted into `cairn-medium` and consumed by both
  `cairn-wire` and `cairn-node` (§5.0); a clinical reader in `cairn-node`'s `backup`; a reordered
  `restore` ceremony; per-event failure accounting with durable quarantine. **One ADR (2e's, which this
  slice now owes rather than defers)** — see §8 for what it must contain and what remains of 2e.
- **Touches four crates, so the gate is the full workspace.** `cairn-node`, `cairn-medium`,
  `cairn-wire`, and `cairn-sync` (§2.2's `do_requeue`, §6.1's pen caller). A `-p cairn-node` run misses
  the cross-crate suite — #503's lesson, and the reason `cairn-sync/tests/clinical_pull.rs` exists.
- **Migration bookkeeping the slice must not forget** (each has bitten before): the `db/tests/052_*.sql`
  mirror; `SCHEMA_GENERATION` 51 → 52 in `crates/cairn-event/src/schema_generation.rs`, whose guard
  reads `db/` at test time; the **hand-written migration list** in `crates/cairn-node/src/db.rs`; and
  `crates/cairn-node/tests/schema_version_guard.rs`. The `sync_quarantine` column is added by
  `ALTER TABLE … ADD COLUMN IF NOT EXISTS` in `db/052` and the `CREATE TABLE` in `db/021` is **not**
  widened — if a later tidy-up widens it, that pair needs an entry in
  `crates/cairn-node/tests/migration_replay_widening.rs` (#207).
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
is exactly what `cairn-sync` already does on every pull — `do_pull` unwraps each slot's DEK for its own
key and hands the plaintext to `apply_signed`, which passes it straight to the door's fourth argument.
(The unwrap is at the *call site*, not inside `apply_signed`; restore mirrors the same split.) The
round-trip is deliberate — it re-derives custody through the one door that owns `event_dek`, rather than
teaching a second site how to write it.

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
an incremental pull fetches from `min(last_seq, floor_seq - 1)` — the `-1` is load-bearing, because the
serve streams `seq > after_seq`, so fetching from `floor_seq` itself would skip the very slot being
re-offered (`cairn-sync`'s `do_pull`). A **full sweep ignores the floor entirely** (`after_seq = 0`).
**No peer re-offers a medium.** A floor pinned by a restore
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

`recorded_at` is the **primary ordering key deciding who may author** (`ORDER BY ae.actor_id,
ae.recorded_at DESC, ae.seq DESC` — `seq` is only the tiebreak). A restored `enroll` whose timestamp
defaulted to empty-or-now would outrank a genuine older `revoke` and silently re-authorise a recalled
actor — arriving through the door built to restore the registry.

That is the resurrection hazard `cairn_actor_id_key_conflict` (db/004, ~29 lines below `actor_current`)
guards against on the *enroll* path. Two precisions, because §4 leans on this: that guard's **primary**
purpose is #152's silent identity merge (a row bound to a *different* key); resurrection is the
deliberate second effect of its **NULL-key** case, which is what matches a `revoke`/`supersede` row.
The other `IS DISTINCT FROM` in db/004 (`cairn_key_actor_id_conflict`) is #166 key-reuse and is a
different guard — do not conflate them.

**Decision: drop the `#[serde(default)]` from `recorded_at` and refuse a row without it.** Harmless
until now only because nothing installed these rows; #554 item 4 asked for the decode-refusal test, and
this is what that test finds.

---

## 3. The ceremony reorders: `finalize_identity` moves LAST

Today (`main.rs`, the `restore` arm):

```
mint new key → apply node plane (needs an UN-ENROLLED db) → finalize_identity (writes local_node,
fences the door) → apply_local_state_export (installs the unwrap key)
```

`main.rs` already carries the note that this is temporary: *"this runs AFTER `finalize_identity`, which
is correct only while no CLINICAL event is applied here … When the medium starts carrying clinical
events (#500) this block moves up ahead of step 5."* Finding 2.1 confirms it and finding 3 extends it.

**New order:**

```
mint new key
  → apply node plane          (restore_node_event; un-enrolled fence)
  → apply_local_state_export  (installs the unwrap key + REGISTERS its public half; installs the actor registry)
  → apply CLINICAL plane      (apply_remote_event, plaintext DEK per record)
  → finalize_identity         (new genesis + supersede; fences everything closed)
```

(The Rust entry point is `apply_local_state_export`; `apply_local_state` is its inner worker in
`localstate.rs`. Both carry the ordering note quoted above.)

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

**That claim is only true if the registry door is resumable, and the obvious fence design makes it
false.** The registry is installed at step 3, *before* the clinical apply at step 4. A per-row door that
"refuses if `actor_event` holds any row" would therefore refuse the re-run it just argued for: the
failed attempt leaves `local_node` empty **and `actor_event` populated**, and the second attempt is
turned away at the door. The recovery would be a fresh database — precisely the outcome this ordering
exists to avoid. **§4's door is specified as set-shaped and idempotent for exactly this reason**; the
resume path is a first-class requirement of §3, not an afterthought of §4.

Note what this does *not* license. `enroll_actor` contains **zero** references to `local_node`, so a
populated `actor_event` on an un-enrolled database is **not** provably the residue of a failed restore —
it could be a half-provisioned node that enrolled an actor before minting identity. A blanket
`TRUNCATE actor_event` on the un-enrolled fence alone would therefore be unsound, and §4 does not do
that.

One consequence to state plainly: `restore_node_event`'s fence (`local_node` empty) now protects a
longer window. That is the direction that fails safe — the door stays a permanent no-op on any live
node, unchanged.

---

## 4. `db/052` — the actor-registry restore door

```sql
restore_actor_registry(p_rows JSONB) RETURNS integer   -- rows inserted
```

**It takes the whole set in one call, not one row per call.** That is what makes it a transaction
boundary and what makes the resume path §3 depends on expressible at all; a per-row door cannot tell a
partial prior attempt from a foreign registry. It returns the number of rows it actually inserted, so a
resume reports honestly.

SECURITY DEFINER, `SET search_path = public, pg_temp`, `REVOKE EXECUTE … FROM PUBLIC`, granted to
`cairn_node` **only** — explicitly *not* to `cairn_agent`. This is the highest-value new privilege in
the slice (it writes the trust anchor every clinical apply gates on, and it deliberately bypasses the
collision guards below), so the grant is a tested property, not a comment (§7).

**Fenced twice.**

1. It refuses if `local_node` is non-empty — the `restore_node_event` fence. A live node is never a
   restore target.
2. It refuses if `actor_event` holds any row whose `actor_event_id` is **not** among `p_rows`.

Fence 2 is the set-shaped form of "an empty table," and it is strictly better than it. It is still
structurally unable to inject into a populated registry — one foreign actor row and the whole call is
refused — but a registry that is a **subset of what is being restored** is the signature of an
interrupted restore, and the door completes it instead of refusing it. So:

- fresh database → inserts everything;
- interrupted restore, re-run → inserts only the missing rows, in order, and says how many;
- half-provisioned node that enrolled its own actor → **refused**, because that row is not in `p_rows`.

**Ordering.** Rows are inserted in **ascending source-`seq` order**, and `seq BIGINT GENERATED ALWAYS AS
IDENTITY` assigns fresh values. The restored relative order is therefore exact, without `OVERRIDING
SYSTEM VALUE` (which db/004's own comment asks to keep loud in review) and without leaving the identity
counter behind the restored maximum — the bug an explicit-`seq` restore would plant for the *next*
`enroll_actor`. A resumed call inserts its remainder after the rows already present, so the
relative order survives the interruption too.

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

### 5.0 This derivation already exists — do not write a second one

**An earlier draft of this section specified a fresh `clinical_plane_records` in `cairn-node` and
re-derived the ordering from first principles. That was wrong on both counts, and the review caught
it.** 2b already built the reader, in `cairn-wire::MediumTransport`, whose `servable` field is
documented verbatim as:

> Clinical records within `verified_through`, ascending by `source_seq`. Materialised at construction…

That is the whole of what §5 was about to re-derive — **and one thing more, which the re-derivation had
dropped**:

> TRUST STOPS AT `verified_through` (2a invariant 5). Serving past it would hand a puller records whose
> chain link never held — and the puller's cursor would then advance over them. `None` (nothing
> verified) yields an empty set, never "all".

A second reader that sorts but does not gate would apply records from a torn tail or past a broken chain
link. Each is still individually signature-verified by `apply_remote_event`, so nothing forged gets in —
but 2a built the chain pass precisely to narrow the **segment-splice** residual, which per-event
signatures do not address. Re-deriving the sort while losing the gate is the worst of both.

**Decision: one derivation, and it moves to the crate that owns the chain.** The
`within(verified_through) → sort by source_seq` derivation is lifted into **`cairn-medium`** (which owns
`chain`, and which *both* `cairn-wire` and `cairn-node` already depend on), and `MediumTransport` is
re-expressed on top of it rather than keeping its own copy. `cairn-node`'s restore consumes the same
function.

The alternative — add a `cairn-wire` dependency to `cairn-node` and call `MediumTransport` directly —
was considered and **rejected**: `MediumTransport` is a *serving* abstraction with paging, a label and a
logging latch, none of which a restore wants, and `cairn-node` currently depends on `cairn-medium`
alone. Lifting the pure derivation keeps the dependency graph as it is and leaves exactly one
implementation of 2a invariant 5. Note this is a **course correction against 2a's programme table**
("restore pulls the medium through `apply_remote_event` unchanged" via the 2b seam); it follows 2c's
Erratum E2 precedent, which already reads media in `cairn-node` via `parse_any`, and it is recorded here
rather than left for the next session to discover.

### 5.1 The shape restore consumes

`backup::clinical_plane_records(&MediumImage) -> Result<Vec<MediumRecord>, BackupError>` remains the
`cairn-node`-side entry point, but it is now a thin adapter over the lifted derivation — it does not
re-implement the gate or the sort. It returns whole `MediumRecord`s rather than bare `Vec<u8>` because
the clinical plane carries three things the federation plane does not: the attestation pair and the
wrapped DEK.

`node_plane_events` keeps its exact current shape, its `Legacy`/`V3` arms and its tests.

**Why the sort is load-bearing** (retained, because the reason must survive even though the code is
now inherited): 2c's capture backfills burned-`seq` gaps **newest-first** under a bounded probe budget
(`MAX_GAP_PROBES_PER_CAPTURE = 64`), so a record with a *lower* `source_seq` can legitimately sit in a
*later* segment — `capture/plane.rs` says so in as many words. Applying in raw medium order would then
offer an overlay before the event it targets, and db/020 would refuse it with *"apply_remote_event:
overlay targets unknown event"* — a self-inflicted pen entry, on a medium that carried everything
needed.

Sorting by `source_seq` restores causal order because `event_log.seq` **is** causal on the node that
wrote it: a locally-authored overlay is inserted after its target by construction, and a replicated one
could not have been admitted at all before its target (db/020 refuses it, and the puller pens it), so no
overlay ever holds a lower `seq` than the event it targets.

A duplicate `source_seq` — the same event captured by both the watermark pass and a gap probe — needs no
special handling: the apply door is idempotent (`ON CONFLICT (event_id) DO NOTHING`, with a
content-address check that refuses a *substitution* under the same id), so the second offer is a
set-union no-op.

### 5.2 Legacy, unknown planes, and provenance

**A legacy (CAIRNB1/B2) medium.** It has no clinical plane at all, and `clinical_plane_records` returns
empty for it. 2b made the same case a hard `UnsupportedByMedium` on the *serving* path, reasoning that
an empty success "is #500's exact signature, reproduced inside the machinery built to close it" — and
that reasoning applies here too. **Decision: restore reports it as an explicit, named outcome, not as a
silent zero.** The operator is told *"this is a CAIRNB1/B2 medium: it predates the clinical plane and
carries no patient data — the federation plane was restored, and no clinical events exist on this
medium to restore."* Same bytes recovered either way; the difference is whether a solo clinic reads
"restored" and believes it has its charts back.

**An `Unknown(tag)` segment is not read as clinical.** `plane_counts().unknown` surfaces it, and
`verify-backup` **refuses** such a medium outright (`refuse_unsound_medium` → `needs_a_newer_build()`).
`restore` does **not** refuse — it prints a note and continues. That asymmetry is deliberate and stays:
refusing a restore because part of the medium needs a newer build would convert a partial recovery into
a total loss, which is the trade §6 rejects everywhere else. The note must name the record count.

**Provenance (2a's deferral to this slice, now decided).** 2a left open *"whether an unsigned segment
should ever be restorable without operator confirmation … 2d decides."* **Decision: clinical segments
inherit the same `Provenance` treatment the node plane already gets.** An unsigned or
non-sole-enroll-signed medium requires the operator's identity confirmation before its clinical records
are applied — the third human act the paper-parity benchmark counts, and #512's `M = 3`. This is why
that act cannot be quietly dropped from the step count: it is a safety gate, and this slice is the one
that extends it to patient data.

> **Superseded 2026-09-10 by [ADR-0068](../../spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md)
> ([#571](https://github.com/cairn-ehr/cairn-ehr/issues/571)).** The paragraph above supports two
> readings and the shipped code implements the other one: the node plane's `Provenance` treatment is a
> **printed warning**, never a blocking prompt, so "inherits the same treatment" was already satisfied
> the day this was written. **Provenance warns; it never gates.** The wording is left standing because
> the divergence is the point — a design sentence with two readings and no test is how this sat unnoticed
> through a merge. The `M = 3` claim in the last sentence does not follow either; #512's count is
> re-derived by measurement.

---

## 6. Failure policy: skip, count, pen, report

**Per-event.** A refusal from `apply_remote_event` — an unenrolled signer, a DEK that will not unwrap,
an overlay whose target the medium lost at a burned `seq` (#549) — does not abort the restore. It is
counted by reason and penned into `sync_quarantine` with:

- `peer = '(restore)'` — an explicit sentinel, not an empty string. `peer` is `NOT NULL` and the
  per-peer quota probes (#197) filter on it (`WHERE peer = $5`), so a restore-penned row must be
  identifiable as one rather than blend into an unnamed link. (An earlier draft also claimed "the
  mixed-version diagnosis groups on it." **That is false** — the mixed-version diagnosis is composed in
  Rust from the peer name and the first signing context and never queries `sync_quarantine`; there is no
  `GROUP BY peer` over that table anywhere in the tree. The `db/021` comment asserting otherwise was
  itself inaccurate and **is corrected in this PR** — it invited exactly the opposite conclusion about
  what a shared or sentinel `peer` value would break, which is the question §6.2 turns on.)
- `reason` — a legible refusal text prefixed to name the restore as its origin. **Not always "the
  door's" text:** per §2.1 the unwrap now happens in Rust *before* `apply_remote_event` is called, so an
  unwrap failure never reaches the door and has no door text. Restore-side refusals carry their own
  legible reason; door refusals carry the door's, prefixed. This matters because #536's counting depends
  on the cause being legible.
- `dek_wrapped` = the record's custody, **preserved** (finding 2.2).
- **no** `quarantine_floor_seq` pinned (finding 2.3).

This follows 2c's torn-medium ruling directly: in the one command that exists for the disaster where
re-running the backup is impossible, converting a partial loss into a total one is the wrong trade.
No confirmation dialog (principle 3).

### 6.1 Where the pen lives, and why it cannot simply call the sync one

`quarantine_event` lives in `crates/cairn-sync/src/main.rs`. **`cairn-sync` is a binary-only crate** —
no `lib.rs`, no `[lib]` target — and `cairn-node` does not depend on it. So §6 as first drafted had no
implementation available to it, and the obvious repair (copy the `INSERT` into `cairn-node`) forks the
quota and dedupe logic across two crates.

**Decision: the pen becomes an in-DB door in `db/052`**, `cairn_quarantine_event(…)`, and *both*
`cairn-sync` and `cairn-node` call it. This is ADR-0001's direction (fat Postgres, thin daemons) applied
to a floor that is already about refusing things safely, and it leaves one implementation of the quota
rather than two. `cairn-sync`'s `quarantine_event` becomes a thin caller; its behaviour on the sync path
is unchanged, which its existing tests pin.

### 6.2 The per-peer quota is unsafe on the restore path, and the sentinel makes it worse

**This is the defect the sentinel decision above walked into.** The pen's quota is
`MAX_QUARANTINE_ROWS_PER_PEER = 10 000` rows and `MAX_QUARANTINE_BYTES_PER_PEER = 64 MiB`, both scoped
`WHERE peer = $5 AND NOT acked`. Penning every restore refusal under the single sentinel `(restore)`
puts the entire restore in **one quota bucket**.

The no-export path (below) pens *every record that carries custody*. Under born-sealed (ADR-0052) that
is substantially the whole clinical log — and #512's budget scale is **100 000 events**, an order of
magnitude past the row cap and far past 64 MiB.

At the quota the pen returns `Err` whose text reads *"refusing to grow it; **the watermark freezes
instead (delayed, never lost)**."* That guarantee is a **sync-path** guarantee: it needs a cursor to
freeze and a peer that will re-serve. A restore has neither — §2.3 deliberately pins no floor, there is
no peer, and `finalize_identity` fences the node immediately afterwards. Inheriting the sync quota
unexamined would therefore **lose those events and their custody at exit**, which is #500's shape one
layer down, inside the slice built to end it — the same sentence §2.2 writes about the requeue path.

**Decision: the quota does not apply to a restore-originated pen.** `cairn_quarantine_event` takes the
quota as a caller-supplied policy rather than a constant, and the restore caller passes *unbounded*.
The justification is that the quota's purpose does not obtain here:

- It exists to stop a **hostile or broken peer** from filling local disk with refused bytes. A restore's
  input is the operator's own medium, already on local disk, already `verify-backup`-checked. There is
  no adversary to bound and no unbounded stream — the medium is finite and known.
- The bytes it would refuse to store are bytes the node **is about to lose permanently**. A resource
  budget that trades a clinic's record for disk it already spent is the wrong trade, and it is the exact
  trade §6 rejects for torn media.

**What replaces it** — because "unbounded" must not mean "unreported": the restore reports the pen's row
count and byte total in the operator summary, and if the pen exceeds the ordinary per-peer quota the
summary says so explicitly, naming `cairn-sync quarantine` and the disk cost. A bound the operator can
see beats a bound that silently drops the record.

**No usable export.** If `apply_local_state_export` installed no unwrap key — no passphrase (every unattended
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
2c already has. The list below is the full set; a decision in §2–§6 with no entry here is a decision
this slice is not entitled to claim.

**The guarantee**

1. **The inverted pin.** `nothing_yet_restores_a_clinical_event_from_a_medium` becomes
   `a_clinical_event_restores_from_a_medium` — inverted, never deleted, with its doc rewritten to say it
   is now a GUARANTEE and what reddens it. Precisely: its two count assertions (`event_log`,
   `event_dek`) invert; its **leg-1 equality** (`node_plane_events(&image) == federation`) stays as it
   is, because §5.1 keeps `node_plane_events` unchanged.
2. **The end-to-end guarantee (the one that matters).** Seal a real clinical body on node A, capture it,
   restore into a fresh node B from medium + export, and **read the payload back in clear** through its
   projection. The only test that can distinguish a correct restore from the double-wrap of finding 2.1,
   because a double-wrapped `event_dek` row is present, well-formed, and the right length.
3. **The double-wrap regression, named.** A direct assertion that the DEK reaching `apply_remote_event`
   is the plaintext one — so a future "simplification" that passes `dek_wrapped` through reddens here
   with a legible reason rather than at (2) with a decryption failure.

**Custody**

4. **Custody survives the pen.** Pen a sealed event during a restore, then `requeue` it, and assert the
   body opens. Fails loudly against today's `None`-passing requeue.
5. **A sealed body with no custody restores anyway.** A body **shredded before its first capture**
   arrives sealed with `dek_wrapped = None`; it must restore custody-less, exactly as it stands on the
   dead node. This is the §6 rule (*the test is the record's custody, not whether the event is sealed*)
   and it is the case an earlier draft of test 8 would have forbidden.
6. **The no-export path, keyed on custody.** A restore with no usable export recovers the federation
   plane and **every record whose `dek_wrapped` is `None`** — sealed or not, per (5) — pens **every
   record that carries a `dek_wrapped`** with that custody intact, and exits non-zero. Worded on
   custody, never on sealedness.
7. **The pen is not silently capped, and the fixture is volume-bearing.** A restore whose refusals
   exceed `MAX_QUARANTINE_ROWS_PER_PEER` pens **all** of them under `peer = '(restore)'` and loses none
   (§6.2). **This test must run at a volume above the cap** — at a handful of events it passes against
   the unfixed quota and proves nothing, which is exactly how this defect would have shipped green.

**The registry door**

8. **The fences.** `restore_actor_registry` refuses on a node with `local_node` set, and refuses when
   `actor_event` holds a row that is **not** among `p_rows` (the half-provisioned node).
9. **The resume path** (§3's argument, and it has to be tested or §3 is unproven). Install a partial
   registry, then re-run with the full set: it inserts **only** the remainder, reports that count, and
   the resulting `actor_current` is identical to a clean install. Reddens against the "refuses if
   `actor_event` holds any row" fence §3 shows to be unsound.
10. **The privilege.** `cairn_agent` and `PUBLIC` **cannot execute** `restore_actor_registry`. The
    #430/#431 shape — a decoy path around a floor that looked correct at its own site — applied to the
    most dangerous new door in the slice.
11. **The registry ordering property.** A dead node whose history is `enroll(actor) → revoke(actor)`
    restores to a registry where that actor is **absent from `actor_current`**. **The fixture must pin
    both rows to the same `recorded_at`**, because `actor_current` orders on `(recorded_at, seq)` and
    with distinct timestamps `recorded_at` alone decides — the ordering half would be untested
    otherwise, which is what an earlier draft of this test claimed to cover and did not.
12. **The counter is not left behind.** After a restore, the **next** `enroll_actor` succeeds — the bug
    §4 says an explicit-`seq` restore would plant.
13. **The serde contract, both structs** (#554 item 4 in full, which an earlier draft half-discharged).
    A CBOR `ActorRegistryRow` with no `recorded_at` fails to decode; an `EpisodeDek` missing a field
    fails to decode; and **`deny_unknown_fields` is pinned on both** — deleting it must redden
    something, which today it does not.

**The reader**

14. **Nothing beyond `verified_through` is applied.** A medium with a broken chain link mid-file
    restores the verified prefix and **not one record past it** (2a invariant 5, §5.0). Reddens against
    the second, ungated reader the review caught.
15. **The sort, with a fixture that can fail.** An overlay whose target sits in a **later segment at a
    lower `source_seq`** (2c's newest-first gap backfill) restores without a pen entry. A fixture in
    capture order passes either way — 2b named this trap and it is repeated here deliberately.
16. **A duplicate `source_seq` is a no-op**, and a *different* body under the same `event_id` is
    refused as a substitution.
17. **A legacy medium is a named outcome, not a silent zero** (§5.2), and an `Unknown(tag)` segment is
    noted with its record count rather than applied.
18. **Provenance warns before any clinical record is applied, and never blocks.** ~~Provenance gates
    the clinical plane. A non-sole-enroll-signed medium requires the operator's identity confirmation
    before clinical records are applied (§5.2, and the third act #512 counts).~~ **Restated 2026-09-10
    by [ADR-0068](../../spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md)**
    (closing [#571](https://github.com/cairn-ehr/cairn-ehr/issues/571)). The struck wording is left
    standing because it is what the plan said when it was accepted, and the divergence between it and
    the shipped `eprintln!` arms is the thing worth being able to see. There is no gate, on either
    plane, and there is no third act: refusing converts a partial loss into a total one, a prompt
    ratifies an identity rather than a record set, and principle 3 forbids the mechanism by name. #512's
    step count is re-derived by measurement, not by this line.

**The ceremony and the report**

19. **The order is load-bearing.** `finalize_identity` runs **last**; a clinical apply that fails
    catastrophically leaves `local_node` empty, and the same database re-restores to completion (with
    9). This is what distinguishes "last" from the minimal "later" reordering §3 rejects — test 2 passes
    under both.
20. **The two §3 preconditions are pinned, not merely read.** Source guards: `db/020_apply_remote_event.sql`
    contains zero `local_node` references, and `cairn_register_unwrap_key` reads none. A future migration
    adding one breaks the ceremony silently otherwise — the same discipline 2c demanded for the shred
    predicate.
21. **No floor pinned.** After a restore that penned an event, `sync_state.quarantine_floor_seq` is
    still NULL, and a subsequent pull is not dragged backwards.
22. **The summary survives the failure.** The `new node` / `supersedes` / `re-peer with …` lines print
    **before** the non-zero exit, and refusals are reported **counted by reason** with a legible cause
    (§6's `reason` rule, including the Rust-side unwrap failures that carry no door text).
23. **The AEAD caveat is printed at restore time** (§4) — the doc's own justification for printing it is
    that a limitation living only in a design doc is one nobody finds, which applies equally to a
    limitation living only in an untested `eprintln!`.

---

## 8. The ADR this slice owes

2c deferred an ADR to "2e". The decisions below are architectural rather than local, and an
undocumented decision is one the next session re-litigates. **It is ADR-0067** (0066 is the current
highest).

**Scope check against what 2e was defined to own.** 2a specified 2e's ADR as: *"supersedes ADR-0026
decision 2's implementation wording; records the two planes, the export-borne registry and its caveat,
and what is still not true."* An earlier draft of this section claimed 2e's ADR while omitting the
supersession — its headline purpose. Taking the ADR means taking that too (item 4). 2c also said *"no
ADR and no spec bump (2e owns both)"*; the **spec bump belongs to this slice as well** (item 5), or
nothing owns it.

1. **The actor registry re-enters on container AEAD alone** — the single exception to verify-on-apply in
   a restore, and why it is accepted (§4).
2. **Erasure does not propagate backwards into media already written**, and completing it across
   backups is **rotation** — capture fresh, destroy old — whose interval *is* the maximum time an
   erasure takes to complete across all copies. That is the clinic's policy call, not Cairn's
   (principle 9; ADR-0005's *deletion is best-effort and declared*). 2c's design §2.1 established this
   and HANDOVER trap 7 records it; the ADR owes it **in as many words**, which is what 2c said and did
   not do.
3. **The restore ceremony's order is load-bearing**, not incidental: custody and the registry precede
   the clinical apply, and identity is minted last so a failed restore leaves a restorable database —
   which is only true because the registry door is resumable (§3, §4).
4. **Supersedes ADR-0026 decision 2's implementation wording** — 2e's headline purpose, inherited here.
   Decision 2 describes backup as "a configuration of the existing sync daemon" whose restore is
   "set-union apply through the existing verify-on-apply path." The built system is a **medium format
   plus a capture/restore command in `cairn-node`**, not a cold-peer daemon configuration, and its
   registry re-entry is explicitly *not* verify-on-apply (item 1). The mechanism changed; the guarantee
   did not. Saying so is what stops the next reader trusting decision 2's wording as a description of
   the code.
5. **The resource-budget carve-out** (§6.2): a restore-originated pen is not subject to the per-peer
   quarantine quota, because the quota bounds a hostile peer and a restore has none, and because the
   bytes it would refuse are bytes the node is about to lose permanently.

**Spec bump.** The spec version in `docs/spec/index.md` moves with this ADR. 2c deferred it to 2e; 2e's
ADR is now this slice's, so the bump is too.

**What remains of 2e.** After this slice, 2e is no longer an ADR-owing slice. What is left under that
name is operational: the DR kit's per-kit restorability figure (#551) and the foreign-legacy-medium
succession hazard (#553). If nothing else attaches, "2e" should be retired as a label rather than left
as an empty container that future sessions defer into.

ADR-0026 decision 1's clinical promise may be cited as met **only** once this merges — and promise 2
(*node-default data-at-rest keys survive*) still has **no subject**: no node-default key tier exists
(the only `node_default` in the tree is the empty `node_default_deks` slot), so it is neither honoured
nor violated. Do not let this slice's success be read onto it.

---

## 9. What is still broken when this merges

- **#549** — a burned IDENTITY `seq` is indistinguishable from a lost clinical event, and `unfilled_gaps`
  has no operator surface. This slice makes the consequence visible (such an overlay pens with a
  legible reason) without closing the diagnosis.
- **#552** — a nightly capture is O(whole medium). This slice adds *parses* on the read side at the same
  seam. Restore is a once-per-disaster command, so the cost lands where it is affordable, but the
  measurement in the paper-parity benchmark must be taken rather than assumed.
- **Peak memory on the read side is unbudgeted.** The reader materialises the servable clinical set and
  sorts it; 2a deferred streaming parse to 2b, 2b did not decide it, and this slice doubles the peak.
  Fractal topology means a Pi or an Android node is a legitimate restore target, so this is named rather
  than assumed away — **streaming stays deferred**, and #552 is where it lands.
- **The `source_seq`-gap operator surface** (`chain::seq_gaps`) — 2b named this as belonging to *"2d's
  restore report, which is the first caller with an operator to tell,"* and this slice does **not**
  deliver it. It is re-filed under #549 rather than closed: recording the re-deferral, because 2a's own
  lesson is that a deferral is only honest while its stated precondition holds.
- **#551** — the kit-restorability figure still has no per-kit home; a same-mount-point rotation can
  still false-green.
- **#553** — an unmarked foreign legacy medium can still be destroyed by succession.
- **#536** — an unopenable DEK is counted nowhere on the sync path. This slice counts it on the *restore*
  path only; the sync half stays open.
- **#512** — the `M > N` paper-parity defect for this ceremony. This slice **measures** it (see the
  benchmark) and does not close it; the `K = 2` bundling remains future UI work.

**Two issues an earlier draft of this section listed as open are closed, correctly, and are recorded
here so the next session does not re-open them:**

- **#523** (*a corrupt section length under the cap is indistinguishable from a torn tail*) was **closed
  by 2c** (PR #555) — the CAIRNB3 section-framing guard makes a header vouch for its own length. An
  earlier draft listed it as open *and* described it as "a zero-filled tail is refused rather than
  recovered," which was never what #523 said.
- **#500** is closed **as titled**, deliberately and by the maintainer's call (commit `94c6ced`): the
  medium does now carry clinical events, so that title's sentence is false. The read half is #554, this
  slice. It is not the closing-keyword accident that closed #500 once before (2026-09-01 → 09-04).

---

## Paper-parity benchmark (§1.2)

**This benchmark does not restate the numbers — it inherits them.** The `M > N` defect for *this exact
ceremony* is already filed as
[#512](https://github.com/cairn-ehr/cairn-ehr/issues/512), by the 2026-08-24 DR-clinical-tier design,
and it is **open**. House rule 7 permits filing, never arguing away; redefining the paper baseline or
the bundling target inside the slice being measured is how a falsifiable benchmark stops being
falsifiable. The numbers below are #512's, unchanged.

**Paper counterpart:** the off-site duplicate chart — the practice that copies its records, keeps the
copy in another building, and carries the box back after a fire.

**Steps:** paper *N* = **2** (fetch the box; shelve it) → architecture-forced *M* = **3** (attach the
medium; run `cairn-node restore` and answer its prompts; **confirm the echoed identity when provenance
is not sole-enroll-signed**) → UI bundling target *K* = **2**. `M > N`, and it is **filed as an
architecture defect (#512), not argued away** — the extra act is the identity confirmation, which has
no paper counterpart because a paper box carries no cryptographic identity to mis-assign.

**This slice does not change any of the three numbers.** It changes what those steps *recover*. Two
consequences follow, and both are obligations rather than reassurances:

- The third act is the **identity confirmation**, not "supply the recovery code" (that is part of the
  second act's prompts). §5 therefore owes a `Provenance` ruling for clinical segments — 2a assigned
  that decision to this slice and it is taken in §5, not skipped. A design that dropped the
  confirmation would be reporting `M = 2` by deleting a safety step, not by bundling one.
- #512's `K = 2` argument (the escrow secret and the invocation are one interactive ceremony, and
  `Provenance::Signed` on a sole-enroll medium is already unambiguous, so the confirmation disappears
  for the solo-clinic case) **survives this slice untouched**. Nothing here forecloses it.

**Time + cognitive load:** cognitive load is unchanged by construction — the operator types the same
command and reads a longer scope line. The standing time budget is #512's and is **not adjusted here**:
*a restore of a 100 000-event medium completes in ≤ 10 min, and the operator needs one secret and no
knowledge of the dead node's configuration.* #512 records that measurement as owed by the slice that
first exposes a runnable surface recovering a readable record — **this slice**.

**Measured by this slice**, against that budget, at #512's scale. A per-event figure is reported
alongside it as *evidence*, not as a replacement budget: a restore parses, verifies a signature per
event, unwraps a DEK per sealed event and re-wraps it, so it is several times the per-event work of
2c's append (whose measured `< 60 s` for 10 000 events is the comparison point). If the measurement
falls outside #512's budget, **that is the finding — file it against #512, never adjust the budget.**
