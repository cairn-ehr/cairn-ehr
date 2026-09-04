# Design — DR slice 2c: the backup captures both planes

- **Date:** 2026-09-04
- **Part of:** DR slice 2, the programme that closes
  [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) (*the backup medium carries no clinical
  event*). **This slice does not close it** — it makes the medium *hold* the record; #500 closes when
  2d restores from it and the guarantee is proven end to end. See §8 before quoting this slice
  anywhere.
- **Produces:** `db/051` (SCHEMA 50 → 51) — one view and one set-returning function; a clinical
  capture in `cairn-node`'s `backup`; per-record custody on the medium; an actor registry and
  read-after-write verification on the `CAIRNL1` export; `BackupHealth` v2 with per-plane scope.
  **No ADR and no spec bump** (2e owns both), **one migration**, one additive `CAIRNL1` field.
- **Predecessors:** [2a](2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md) — the format
  (`crates/cairn-medium`, `CAIRNB3`); [2b](2026-09-02-dr-slice-2b-transport-seam-and-paged-pull-design.md)
  — the seam and the paged pull (`crates/cairn-wire`);
  [#511](2026-09-04-custody-newtypes-secret32-publickey32-design.md) — the custody newtypes, sequenced
  deliberately before this slice because 2c is where key material starts moving again.

---

## 1. Why this piece exists

ADR-0026 decision 1 promises that on total hardware loss of a solo node *"the **clinical event log
survives**"*. `crates/cairn-node/src/backup.rs::read_event_set` is still

```rust
.query("SELECT signed_bytes FROM node_event ORDER BY seq", &[])
```

`node_event` is the federation plane. No `event_log` row travels — not clinical, not demographic, not
identity, not registration, not erasure. The clinic backs up nightly, `verify-backup` passes, the disk
dies, and `restore` recovers **who it peered with and zero patients**.

2a built the format that can carry both planes. 2b built the seam and the paging. Neither reads a
clinical row. **This slice is the writer.**

---

## 2. Three decisions taken before the design, and why

These were settled with the maintainer on 2026-09-04. They are recorded here with their reasoning
because two of them look, from the outside, like the wrong call.

### 2.1 Custody travels on BOTH paths (the medium *and* the export)

The obvious design is one authority: put the wrapped DEKs in the `CAIRNL1` export, which is rewritten
whole on every run and can therefore apply the `erasure_shred_log` filter retroactively, and leave
`MediumRecord.dek_wrapped` permanently `None`. One filter, one place; a shredded key can never reach a
backup **by construction**.

That was the first recommendation and it is wrong, because of a freshness asymmetry:

| | lives where | changes how often |
|---|---|---|
| the unwrap **secret** | export only (never the DB, by design) | ~never — it is the node's long-lived custody key |
| the **DEK set** | export and/or medium | grows with every sealed write |

The export is the artifact this codebase treats as **optional**. `backup` writes and verifies the
medium, then *attempts* the export and degrades to a warning with **exit 0** on three separate paths:
no `CAIRN_KEY_PASSPHRASE` (i.e. every unattended cron run), an unusable `.lsk` escrow, or an unwrap key
it could not load. All three are deliberate — the medium is the load-bearing copy and a nightly backup
must not be failed over an optional sidecar — and all three are correct in isolation.

The realistic disaster is therefore not *"no export"*. It is **tonight's medium beside a weeks-old
export**. With custody only in the export, every event sealed since that export is unreadable
ciphertext forever, and the node reported success every night in between. With custody on the medium,
those bodies carry their own DEKs, the stale export still supplies the stable secret, and the record
comes back.

**Decision: both.** The medium's copy is co-fresh with the events it unlocks; the export's copy is the
retroactively-filtered one. Data loss outranks erasure completeness (the maintainer's ranking:
*"data loss is the most catastrophic event possible next to data falsification"*).

**What follows is not a defect, and the first draft of this design mislabelled it as one.** The
maintainer's framing, which governs (2026-09-05):

> A backup is only a backup if it can restore the state of the system at the time the backup was taken.
> Taking care of invalidated backups is a policy issue, not a core enforcement one. The core will only
> guarantee availability and integrity of data.

Read the append-only medium against that definition and the behaviour is exactly right, not
best-effort:

| the body was shredded… | in a medium captured before | in a medium captured after |
|---|---|---|
| …before that capture | n/a | unreadable — the capture-time filter reproduces the state as it stood |
| …after that capture | **readable, correctly** — at the moment that medium was taken, it *was* readable | unreadable |

So the medium's capture-time filter is **not** a weaker version of the export's retroactive one. It is
the point-in-time semantic, and it is the export's whole-file rewrite that departs from it — the export
carries *current* custody because its job is to carry the long-lived unwrap secret, not to be a
point-in-time artifact. A restore therefore reads a body if **either** carrier still holds its key, and
the medium half of that is a faithful snapshot rather than a leak.

**The consequence a practice must be told, and the line where core ends.** Erasure does not propagate
backwards into media already written. Destroying a key on the live node completes an erasure *there*;
completing it across backups is **rotation** — capture a fresh medium, destroy the old — and the
rotation interval **is** the maximum time an erasure takes to complete across all copies. That number
is the clinic's policy call, not Cairn's: principle 9 (mechanism, never policy) and ADR-0005's
*deletion is best-effort and declared, never guaranteed*. Cairn's obligations are to make the residue
**legible** (`verify-backup` must be able to say a medium predates a shred) and to state it plainly —
**2e's ADR owes that sentence in as many words.** What Cairn must never do is describe the two carriers
as symmetric, or imply that a shred reached a medium it cannot reach.

Two things this framing settles that would otherwise be re-litigated: the retraction behaviour is
**not** on any defect list, and no future slice should "fix" it by filtering old segments — an
append-only medium that rewrote its own history would forfeit the integrity guarantee that is the
core's actual job.

### 2.2 The shred predicate gets ONE home, in the database

The capture needs, per event: `signed_bytes`, `attestation`, `attester_key`, the DEK still wrapped for
*this* node, and `seq`. `cairn-sync`'s serve door already has exactly that query, shred filter
included — but `cairn-sync` is binary-only and unreachable from `cairn-node` (2b §2), so 2c cannot call
it.

Writing it again would give the predicate *"a shredded body's key must not travel"* **three**
hand-written spellings in two crates:

| where | spelling |
|---|---|
| `localstate_read.rs` (the export) | `WHERE NOT EXISTS (SELECT 1 FROM erasure_shred_log s WHERE …)` |
| `cairn-sync/src/main.rs` (the serve door) | `LEFT JOIN erasure_shred_log s … CASE WHEN s.target_event_id IS NULL THEN …` |
| 2c's capture | a third one |

That is this repository's most persistent defect class — [#182](https://github.com/cairn-ehr/cairn-ehr/issues/182),
[#404](https://github.com/cairn-ehr/cairn-ehr/issues/404) and
[#441](https://github.com/cairn-ehr/cairn-ehr/issues/441) (*"two hand-maintained mirror lists"*, filed
2026-08-20 and still open) — and here the mirrored thing is a **safety predicate**, not a list of
table names.

**Decision: one in-DB definition, per [ADR-0001](../../spec/decisions/0001-fat-postgres-thin-daemon.md)
(fat Postgres, thin daemon).** `db/051` adds it; all three callers select from it. It also puts the
predicate in the floor layer, where a client talking raw SQL inherits it rather than re-implementing
it — the ADR-0021 argument for why the floor is the compatibility boundary.

### 2.3 The actor registry is written by 2c, installed by 2d

`actor_event` (db/004) has no `signed_bytes` and replicates nowhere, while every clinical apply door
gates on `actor_current` (*"signer % is not an enrolled, non-revoked actor"*). Without it a restored
node **refuses its own history**. 2a's table put the whole question in 2d.

But the registry can only ride `CAIRNL1`, and `CAIRNL1` is written by `backup` — the command 2c owns
and is already changing. Splitting write from install across two slices means 2d has to re-open this
slice's file, and it means 2c ships a medium+export pair that cannot restore a node **by
construction**.

**Decision: 2c writes it, 2d installs it.** One additive `LocalState` slot; the #511 wire pins are
extended, never reordered.

---

## 3. `db/051` — the capture's source of truth

Two objects. **Neither may widen who can read custody**, which is the trap this migration is most
likely to spring: `db/037` `REVOKE`s `event_dek` from `PUBLIC` *and* from `cairn_agent`, granting
`SELECT` only to `cairn_node` ("serve-side custody reads"). A plain Postgres view reads its base tables
as the **view's owner**, so an un-guarded `event_custody_surviving` would hand every role that can
select the view exactly the custody access db/037 refused them. The view is therefore declared
`WITH (security_invoker = true)` and granted to match db/037 exactly, and the migration owes a test
that `cairn_agent` still cannot read custody **through the new objects** — the #430/#431 shape (a
decoy path around a floor that looked correct at its own site).

```sql
-- Custody that SURVIVES: the one definition of "this event's key may travel".
CREATE OR REPLACE VIEW event_custody_surviving AS
  SELECT d.event_id, d.dek_wrapped
    FROM event_dek d
   WHERE NOT EXISTS (SELECT 1 FROM erasure_shred_log s
                      WHERE s.target_event_id = d.event_id);

-- One page of the clinical plane, in the shape a peer response and a medium
-- segment both need.
CREATE OR REPLACE FUNCTION cairn_clinical_page(after_seq bigint, page_limit int)
RETURNS TABLE (seq bigint, signed_bytes bytea, attestation bytea,
               attester_key bytea, dek_wrapped bytea) …
```

Three callers, no fourth spelling: `cairn-sync`'s serve door, `localstate_read`'s export (via the view
alone — it needs `event_id`/`dek_wrapped` for every surviving row, not a page), and 2c's capture.

**The probe-row rule travels with the function, not with each caller.** 2b established that
`rows.len() == limit` cannot distinguish *"the log ends exactly here"* from *"there is one more"*, and
that a wrong `complete: true` at that boundary strands every event above it forever. The function
therefore takes `page_limit` and the caller asks for `limit + 1`, exactly as the serve door does today
— the arithmetic stays at the call site, because it is the caller that owns the `complete` claim.

**Guards this migration owes** (the repo's pinned-count discipline): `db/tests/051_*.sql` mirror; a
`SCHEMA_GENERATION` bump to 51 with its pinned-count updates; and a test asserting the shred predicate
appears in **exactly one** SQL definition across `db/`, so a fourth spelling cannot be added quietly.

---

## 4. The capture loop

`cairn-node backup` gains a clinical pass. Per plane it is:

1. **Read the watermark** — the highest `source_seq` in the last *verified* clinical segment of the
   existing medium. Derived from verification, never from the file tail: an unverifiable trailing
   segment does not advance the cursor, so a torn append costs exactly one increment and its records
   are re-captured rather than lost (2a §6).
2. **Page** from that watermark through `cairn_clinical_page`, `DEFAULT_PAGE_EVENTS` at a time —
   2b's constant, reused, not re-invented.
3. **Build one `Segment` per page**, verify it before it can touch the medium
   (`serialize_and_verify_v3` — 2a built this precisely so a corrupt `bytea` read cannot be signed into
   a valid-looking attestation), append, `sync_all()`.
4. **Advance health only after the last page is durable**, preserving today's rule that health can
   only ever under-claim.

A nightly capture over an unchanged log appends nothing and writes no segment — the property that makes
a nightly backup of a growing log affordable, and the reason CAIRNB3 exists.

**The node plane keeps its current whole-set write for now.** It is small, it is what
`read_event_set` already produces, and changing both planes' write strategy in one slice would cost
this slice its ability to attribute a regression. Named as a deferral, with 2d as the slice that
revisits it if restore needs it.

---

## 5. Custody on the medium

`MediumRecord.dek_wrapped` is filled from the page function's `dek_wrapped` column — **copied verbatim,
never re-wrapped**. It is already wrapped to this node's unwrap public key, which is the key a restored
node inherits (ADR-0066), so there is nothing to translate and no plaintext key passes through the
capture path. `None` means exactly what the record's doc already says: unsealed, no custody here, or
shredded.

The asymmetry from §2.1 is stated at the two sites that could mislead a reader — the capture function
and `localstate_read`'s filter comment — in the form *"this filter is retroactive; the medium's is
capture-time only, and an already-signed segment cannot be retracted"*.

---

## 6. The export

Two changes, both on the path `backup` already runs:

- **`actor_registry`** — a new optional `LocalState` slot carrying `actor_event` rows. Additive: the
  `CAIRNL1` wire pins from #511 gain the new field's encoding and keep every existing byte, and the
  `None` case is pinned as well as the `Some` case (#511's lesson: a `skip_serializing` mutant that
  deserializes to `None` passes every round-trip test).
- **Read-after-write** — the export is re-read, re-parsed and its seal re-checked before health
  advances, the same defence the medium has had since slice B. Today it is `atomic_write` and done,
  which is the one artifact carrying this node's custody key off the machine.

`LocalState`'s producer set stays closed (#511): the registry arrives through `from_custody`'s
successor, not through a new struct literal.

---

## 7. Health and scope honesty

`BackupHealth` v1 records one `event_count` — a true count of what the medium holds, which is exactly
how #500 stayed invisible while every surface was honest. v2 records **scope**:

- per-plane event counts and watermarks (`node`, `clinical`);
- `clinical_watermark` — the medium's newest clinical `seq`;
- `export_covers_seq` — the node's `max(event_log.seq)` **at the moment the export was written**,
  recorded only when the export actually succeeded.

**Why a `seq` and not the export's own contents.** The first draft of this design said the export's
watermark was *"the newest `event_id` its custody set covers"*. `event_id` is a UUID: there is no
newest one, so the comparison it was supposed to feed does not exist. The two artifacts do share one
ordered quantity — this node's local `event_log.seq` — so that is what both sides record.

It goes in the **plaintext health sidecar**, not inside the sealed bundle: `verify-backup` is the cron
health check and must be able to answer without a passphrase. `max(seq)` is no more disclosing than
the `event_count` the sidecar already carries. If the export is skipped, the field must **not** advance
— health may only ever under-claim, the rule slice B already holds to.

`verify-backup` gains the comparison and **exits non-zero** when the export is absent, unusable, or
**stale** relative to the medium. This is the one place the slice deliberately changes an exit code:
`backup` keeps warn-and-exit-0 (it wrote a good medium; failing it would page an operator over a
success), while `verify-backup` — whose entire job is to be the cron health check — stops printing
`backup OK` over a kit that provably cannot restore what the medium holds.

`status`'s one-line summary follows the same split: it must be able to say *"clinical: 41,204 events to
seq 91,338; custody export 6 days stale"*.

---

## 8. What is STILL broken when this merges — read before quoting

- **[#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) stays open.** The medium holds the
  clinical record; nothing restores it. `restore` still applies a federation-plane medium.
  `dr_clinical_guarantee_gap.rs` is rewritten, not deleted: the half that pins *"no clinical event
  travels"* inverts, and a new half pins *"and nothing yet reads one back"*.
- **Erasure does not propagate backwards into media already written** (§2.1). This belongs on this list
  as a fact an operator must know, **not as a defect**: it is the point-in-time semantic working, and a
  future slice that "fixed" it by rewriting old segments would trade the integrity guarantee for a
  policy job that is the clinic's. 2e's ADR declares it; completing an erasure across copies is
  rotation.
- **Restore cannot use the registry** this slice writes — 2d installs it.
- **The node plane still rewrites whole** (§4).
- Every deferral here names the slice that retires it, per 2a's rule: *a deferral is only honest while
  its stated precondition holds, and nothing in the repo watches for one expiring.*

---

## 9. Testing

The gate is `scripts/run-db-gated-tests.sh` — a migration means the `db/tests/*.sql` mirrors run too,
and this slice touches `cairn-sync`, so `-p cairn-node` alone would miss the cross-crate suite (#503's
lesson, and the reason `clinical_pull.rs` exists).

What must be proven, beyond the obvious round trip:

1. **A clinical event reaches the medium** — the inversion of `dr_clinical_guarantee_gap`'s pin, on the
   medium file `backup_to` actually writes, not on a fixture built by the test (a pin whose fixture is
   built by the test leaves the production site unpinned).
2. **A shredded body's DEK does not enter a NEW segment** — capture after a shred, assert `None`.
3. **A shredded body's DEK REMAINS in a segment captured before the shred** — the point-in-time
   semantic, pinned as a *test that must pass*, named so no reader mistakes it for a known bug awaiting
   a fix (`a_medium_restores_the_state_at_capture_time`, not `…_leaks_a_shredded_dek`). It is the most
   important test in the slice: it makes §2.1's declared behaviour falsifiable rather than a paragraph,
   and it is the guard against a future session "tidying" old segments and forfeiting integrity.
4. **A torn append costs exactly one increment** — truncate mid-segment, re-capture, assert the
   watermark did not advance past the last verified segment and that the lost records return.
5. **An unchanged log appends nothing** — byte-identical medium across two runs.
6. **A stale export fails `verify-backup`** and a fresh one passes, with the medium identical in both.
7. **Mutation-check the guards**, per 2a's 19-of-19 lesson: a round trip through one encoder/decoder
   pair cannot catch a mirrored change, so the new health fields get golden pins.

---

## Paper-parity benchmark (§1.2)

**Counterpart:** the practice's nightly backup ritual — insert the volume, run the backup, take the
medium off-site.

| | acts |
|---|---|
| paper *N* | 1 — mount the medium, run one backup |
| architecture-forced *M* | **1** — `cairn-node backup`, unchanged flags, both planes |
| UI bundling target *K* | 1 |

**`M > N` is an architecture defect and gets filed, not accepted** (house rule 7). The two ways this
slice could break it, both to be checked before it merges: adding a *separate* clinical-capture
command, and requiring a passphrase where an unattended cron run previously needed none. The second is
the live risk — the export path already prompts, and 2c must not let the *clinical* capture inherit
that dependency. The medium's custody copy is exactly what keeps it from having to.

**Time budget:** a nightly capture on an unchanged log must append nothing and finish in the time a
`max(seq)` read takes. A first full capture of a real clinic's log is bounded by the page loop, not by
a whole-file rewrite — that is the property 2a's segment chain bought, and this slice is where it is
first measured. Measurement is owed by the slice that first exposes a runnable capture: **this one.**
