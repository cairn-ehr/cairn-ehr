# Design — `verify-backup` asks the clinical-plane question

- **Date:** 2026-09-13
- **Closes:** [#567](https://github.com/cairn-ehr/cairn-ehr/issues/567) — *`verify-backup`'s OK is
  still federation-only, so it says nothing about the clinical plane a restore now applies.*
- **Produces:** one new pure module `crates/cairn-node/src/backup/clinical_verdict.rs`; about ten
  lines in the `Cmd::VerifyBackup` arm of `crates/cairn-node/src/main.rs`; one new refusal
  (`backup SHORT`). **No migration, no `SCHEMA_GENERATION` bump, no wire or medium-format change, no
  new CLI flag.** The event core is untouched.
- **No ADR, by precedent.** Every exit-code rule `verify-backup` already enforces (torn tail, #502's
  EMPTY, Task 12's kit verdict, the unsound-medium refusal) lives in code, doc comments and tests,
  and none has an ADR. This slice adds one more rule of the same kind.
- **Predecessors, not re-derived here:** [ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md)
  (the backup/restore ceremony), [ADR-0067](../../spec/decisions/0067-a-restore-reads-the-clinical-plane.md)
  (a restore applies the clinical plane, which is what turned this command's federation-only scope
  from a safety property into a gap), and DR slice 2c's Task 12 (the kit verdict and
  `health_describes_medium`).
- **Exit-code policy decided with the maintainer (2026-09-13):** *fail only on evidence* (§3).

---

## 1. Why this piece exists

`verify-backup` is the cron health check. Its one job is to answer *"can I still recover from this
medium?"* while the node's disk is still alive and the answer can still be acted on.

Since slice 2d, `restore` applies **both** planes. `verify-backup` still prints one line about the
federation plane and nothing at all about the clinical one. An operator reads green and rotates the
drive, and a solo clinic depends on exactly the half the green says nothing about.

`verify-backup` is already deliberately **stricter** than `restore`. A torn tail makes `restore`
recover the intact prefix, and makes `verify-backup` fail. The two commands answer different
questions: `restore` asks *what is the most I can recover now?*, and `verify-backup` asks *is this a
complete backup?* #567's scope section suggests matching `restore`'s leniency ("partial beats total
loss"). That argument belongs to the restore. A health check that fails loses nobody's data; it
catches the problem on a day the node can still write a good medium.

---

## 2. Three findings that reshaped #567's scope

All three came from reading the code before designing against it.

### 2.1 The chain-break case is already a hard failure

#567 lists *"a mid-file chain break, so a restore will refuse to trust everything after it"* as
something `verify-backup` misses. It does not miss it. The arm calls `refuse_unsound_medium` →
`cairn_medium::assess(m).sound()` before any OK can print, and `sound()` requires
`chain.chain_intact()`, which is `faults.is_empty()`.

In `cairn_medium::chain::chain_report`, every condition that stops `verified_through` from advancing
(`IndexMismatch`, `EmptySegment`, `ChainBroken`, `AttestationInvalid`) also pushes a `SegmentFault`.
So on a medium that passes `sound()`, every segment's link held, `verified_through` is the last
segment, and `plane_records_with_accounting(..).gated_out` is **0**. (`SelfIdUnbound` and
`UnknownPlane` do not retract `verified_through`, but both push a fault, so neither survives
`sound()` either.)

Wiring `untrusted_clinical_notice` in after that check would be dead code that claims to handle a
case it can never see. The arm already rejects that shape once, for the unknown-plane warning. So
this slice **pins the invariant instead of wiring the notice** (§6.1, §6.3).

### 2.2 A gap notice would fire permanently

#567 asks for a `clinical_gap_notice` over `cairn_medium::chain::seq_gaps`. That function was never
written, and [#549](https://github.com/cairn-ehr/cairn-ehr/issues/549) explains why it should not be
written yet. Both `event_log` insert doors end in `ON CONFLICT (event_id) DO NOTHING`, PostgreSQL
consumes the `GENERATED ALWAYS AS IDENTITY` value before conflict arbitration, and set-union sync
re-delivers duplicates routinely. **Every duplicate apply burns a `seq`**, so holes in a medium's
`source_seq` run are normal on any federating node. A gap notice would cry wolf on every such node
and hide the one hole that is a genuinely lost event.

**Deferred to #549, in writing:** a code comment at the site, and a comment on #549 saying that
`verify-backup` is waiting for a known-burned set before it reports gaps.

### 2.3 An empty clinical plane is ambiguous from the bytes, and the sidecar is not

The remaining case in #567 is a clinical plane that is **empty** on a medium whose federation plane
is healthy. From the medium's bytes alone, three situations look identical:

| situation | a restore brings back | should the health check fail? |
|---|---|---|
| a fresh clinic (or a federation-only node) that has never written a chart | exactly what the node held | no — it is a correct backup |
| a copy cut off at a **section boundary** (a failed `cp`/`dd`, a dying drive) | fewer charts than the node held, possibly none | yes |
| an older copy put back at the path the nightly backup writes to | the charts as they were then | yes, until the next backup catches it up |

A cut exactly at a section boundary is **not** a torn tail: every remaining section is complete, so
`truncated_tail` stays false and `sound()` holds.

`backup_to` never produces a node-plane-only medium itself. It captures both planes into one buffer
and discards the buffer on any capture error (`backup.rs`, "The order of operations"), so on media
this build writes an empty clinical plane means either "nothing to capture" or "these bytes were
changed after the write".

What tells those apart is **`backup-status.json`**. `backup_to` writes `clinical_watermark` into it,
derived from the durable bytes with the same `cairn_medium::watermark` function that
`clinical_watermark_of` applies to the file under test. When the sidecar describes this exact path
and records a watermark, the node has itself stated what the last backup wrote there.

The same reasoning covers a clinical plane that is **short** rather than empty: a medium whose
watermark is below the sidecar's is also not what the last backup wrote.

---

## 3. The decision

**`verify-backup` fails on a clinical-plane shortfall only when it has evidence; everything the bytes
cannot settle is reported loudly and exits 0.**

The **evidence rule**, stated once (two axes since the final review's correction — see below):

> The sidecar at `health_path_for(--key)` exists and **describes the medium named by `--from`**
> (`health_describes_medium`); and EITHER
>
> 1. **newest seq:** it records `clinical_watermark = Some(s)`, and the medium's own newest trusted
>    clinical seq is `None` or strictly less than `s`; OR
> 2. **record count:** it is a v2-or-newer sidecar (`version >= SUPPORTED_HEALTH_VERSION`) recording
>    `clinical_events = n`, and the medium's RAW clinical record count
>    (`plane_counts(&image).clinical`) is strictly less than `n`.

When it holds, the command fails with `backup SHORT`, naming the axis (or both) that fell short.
Otherwise it prints what the clinical plane holds and continues.

**Cases the rule deliberately does not fail:**

- **No sidecar.** No evidence, so a note and exit 0.
- **A sidecar with `clinical_watermark = None`** (serde default, or a fresh clinic): no newest-seq
  evidence, and a recorded count of 0 can never be undercut. This covers a fresh clinic.
- **A v1 sidecar.** It recorded no per-plane counts; serde defaults `clinical_events` to 0, which is a
  claim nobody made (`describe_health` already refuses to render it). The count axis is not consulted.
- **A sidecar that describes a different path.** Not evidence about this file. A medium with clinical
  content still meets Task 12's existing `CoverageUnknown` refusal, unchanged. An empty one stays
  green, which `verify_backup_is_clean_on_an_empty_clinical_medium_even_with_a_mismatched_sidecar`
  already pins.
- **A medium AHEAD of the sidecar on either axis.** This happens when a backup wrote the medium durably
  and then failed to write the sidecar ("Health was NOT advanced"). The medium holds more than the
  sidecar says, which is not a shortfall. The axes are independent: ahead on one never excuses short
  on the other.

> [!NOTE]
> **Correction made by the final review, 2026-09-14.** This section first compared the newest seq
> alone, and justified it with a paragraph headed *"Why the watermark and not the record count"*: the
> sidecar's `clinical_events` counts raw records including byte-identical re-captures, which the
> accounting collapses, so a count comparison would need reconciliation. That reasoning was wrong, and
> the rule it defended missed a real shortfall.
>
> - **The comparison it rejected is raw against raw.** The sidecar's `clinical_events` is
>   `plane_counts(&written_image).clinical` — every record on the medium `backup` wrote — and
>   `plane_counts(&image).clinical` is the same raw count over the file under test. Nothing is
>   collapsed on either side, so nothing needs reconciling.
> - **The newest seq alone misses a medium short BELOW its newest seq.** A capture backfills
>   late-committing holes under the watermark (`capture/plane.rs`). Night 1 captures seqs 1–100 while
>   seq 97's transaction is still uncommitted (yielding 99 records); nothing new is written before night 2, whose
>   capture only backfills 97, so the sidecar records watermark 100 and 100 clinical records. Put the night-1
>   copy back: its newest seq is 100, level with the evidence, and the watermark-only rule exits 0
>   over a medium missing an event. Its raw count is 99 < 100, which the count axis catches.
> - **v1 sidecars are excluded from the count axis**, because their 0 is a serde default, not a
>   recorded fact.
>
> One wording consequence: on the count axis ALONE the refusal does not claim a certain loss. The
> missing records could all have been byte-identical re-captures of records still present, which a
> restore collapses anyway, so the message says a restore brings back less *unless* that is so. With
> the newest seq short, the newest recorded event is absent and the loss is certain.

### 3.1 The consequence to state plainly: same-mount-point rotation

Two drives rotated through **one mount point** share a path, so the sidecar describes both. The drive
that did not receive the latest backup verifies `backup SHORT` until its own next backup catches it
up. That red is **true**: if the node died at that moment, that drive would restore less than the
node's last backup captured. It is also consistent with today's behaviour for drives at **different**
paths, which already fail `COVERAGE-UNKNOWN` whenever the medium carries clinical content.

It narrows [#551](https://github.com/cairn-ehr/cairn-ehr/issues/551)'s same-path false green (a
behind drive no longer reads `Restorable`) and **does not close it**. The coverage figure still lives
in a node-global file, and a kit copied to another machine carries no evidence at all. That machine
gets the honest note and exit 0. A comment on #551 says so.

---

## 4. What the operator sees

After `federation-plane events OK: N/N verified`, and before the local-state export line:

| medium | stdout | stderr | exit |
|---|---|---|---|
| CAIRNB3, clinical records present, no shortfall | `clinical-plane records OK: N verified, newest seq S` (plus `, K byte-identical re-capture(s) collapsed` when K > 0) | — | continues |
| …and two *different* records share a `source_seq` | the same line | a straddled-duplicate advisory worded for a check that has applied nothing (exit unaffected, as in `restore`) | continues |
| CAIRNB3, clinical plane empty, no evidence | `clinical plane: EMPTY — this medium would restore NO patient data. If this node holds charts, they are NOT on this medium.` | — | continues |
| CAIRNB1/CAIRNB2 (legacy), no evidence | `clinical plane: NONE — this CAIRNB1/CAIRNB2 medium predates the clinical plane and carries no patient data at all.` | — | continues |
| any of the above with evidence of a shortfall — a newest clinical seq below the recorded one, or fewer raw clinical records than recorded (§3; corrected 2026-09-14) | the plane line first, then the refusal | `backup SHORT: …` naming the axis, or both | **1** |

The `SHORT` text says what **this node's last backup to this path** recorded — *this path*, not *this
medium*: in a rotation it was a different drive — and what the file holds, for each axis that fell
short: "newest clinical seq N" against the medium's newest (or "no clinical records at all"), and the
two record counts. It says "newest clinical seq N", never "clinical events through seq N", which would
imply no gaps below N. It says the file at this path is **not what the last backup wrote** (typically
a truncated or older copy), and gives two remedies: run `backup --to` this path again while the node
is alive, or locate the complete copy; the rotation hint stays. It must not suggest re-establishing an
escrow or upgrading the node; those remedies belong to other failures (the #502 lesson).

**Why before the export checks.** Export coverage (`kit_verdict`) compares the export against the
medium's watermark. Over a short medium the export looks *ahead* and the kit verdict says
`Restorable`. The shortfall is the more fundamental finding, and its remedy is different.

**Why "N verified" is an honest claim here.** The line prints only after `refuse_unsound_medium` has
passed, which verified every record signature on the medium, clinical records included, and the
chain.

---

## 5. The shape

### 5.1 A new module, because every neighbour is already over the limit

`backup.rs` is ~2 470 lines and `main.rs` ~6 500. The new logic goes in
`crates/cairn-node/src/backup/clinical_verdict.rs`, declared from `backup.rs` the way `restore.rs`
declares `restore/clinical.rs` and `restore/recovery_code.rs`. Neither existing file is restructured.

### 5.2 What is pure

One pure function computes the whole decision from facts the caller has already gathered:

- **Inputs:** the clinical `PlaneRecords` accounting (from `clinical_plane_accounting`), whether the
  image is legacy, the medium's raw clinical record count (`plane_counts`), and the **evidence** —
  the sidecar's `clinical_watermark` and (v2 and newer only) `clinical_events`, supplied only when
  the sidecar describes this medium. The caller reduces "no sidecar" and "a sidecar for another
  path" to the same `None`; a recorded fact that is absent (no watermark, a v1 count) is `None`
  inside the evidence. (Corrected 2026-09-14 with §3.)
- **Output:** the stdout summary line, an optional advisory for stderr (the straddled-duplicate
  finding — the position search is shared with `restore`, but the wording is not: `restore`'s
  notice says the copies "were applied", which would be false here), and an optional refusal
  message.

`health_describes_medium` touches the filesystem (it canonicalizes paths), so it stays in the
caller. The verdict never sees a path. The exact type names and signature belong to the plan.

### 5.3 Where it is wired

In the `Cmd::VerifyBackup` arm, after the `federation-plane events OK` line and before
`export_verdict_line`. The arm already reads the sidecar further down for the kit verdict; the plan
moves that read up so it happens once and both checks use it, without changing the kit verdict's
behaviour. The arm's `⚠️ THAT SCOPING IS NOW A KNOWN GAP` comment is rewritten to say what the
command now claims, what it does not (gaps → #549, off-node evidence → #551), and why
`untrusted_clinical_notice` is not wired here (§2.1).

### 5.4 What deliberately does not change

`restore`; `kit_verdict` and its four pinned tests; `refuse_unsound_medium`; the torn-tail refusal;
#502's EMPTY and INCOMPLETE refusals (they fire earlier, on the federation plane); the sidecar format;
`status`.

---

## 6. Testing (TDD, every item written failing first)

### 6.1 `cairn-medium`, no database — the invariant §2.1 rests on

For each fault kind — broken link, index mismatch, empty segment, invalid attestation, unbound
self-id, unknown plane — reusing the `health`/`chain` test fixtures where they exist and building the
rest, assert that **either** `assess` is not
`sound()` **or** `gated_out` is 0 for both planes. If a future change makes some fault stop
retracting `verified_through` without failing `sound()`, this test fails and says to wire
`untrusted_clinical_notice` into `verify-backup`.

### 6.2 `cairn-node`, pure unit tests of the verdict

1. Records present, no evidence → OK line naming the count and newest seq; no refusal.
2. Records present with collapsed duplicates → the line names the collapsed count.
3. Two different records at one seq → advisory present; no refusal.
4. CAIRNB3, empty, no evidence → EMPTY line; no refusal.
5. Legacy, no evidence → NONE line; no refusal.
6. Empty, evidence `Some(s)` → refusal naming `s` and "no clinical records at all".
7. Watermark `m < s` → refusal naming both.
8. Watermark `m == s` → no refusal (the boundary).
9. Watermark `m > s` → no refusal (the failed-sidecar-write case).
10. Legacy with evidence → refusal (a legacy file where this node last wrote clinical events is not
    what the last backup wrote).

Added by the 2026-09-14 correction (§3):

11. Same newest seq, fewer raw records than recorded → refusal naming the two counts, and not
    claiming a certain loss.
12. More raw records than recorded → no refusal; the same count → no refusal (the boundary).
13. v1 evidence (no recorded count) with a lower medium count → no count-based refusal.
14. Both axes short → ONE refusal naming both.
15. Ahead on one axis, short on the other → refusal (the axes are independent).
16. The refusal says "this node's last backup to this path" and "newest clinical seq N", never
    "this medium" or "through seq N".
17. The adapter: a v1 sidecar yields no count evidence; a sidecar for another path yields none.

### 6.3 `cairn-node`, DB-gated, driving the real binary

1. **The shortfall that motivated the slice.** A clinic backs up to path P with no clinical events,
   the medium is copied aside, a chart is written, and a second backup goes to P. The older copy is
   put back at P → `verify-backup --from P` exits non-zero and stderr carries `backup SHORT`.
2. **A sound medium with clinical content** verifies, exits 0, and stdout carries the clinical line
   with the right count.
3. **A clinical chain break with intact record signatures** (parse a real medium, change one clinical
   segment's `prev_commitment`, re-serialize with `serialize_v3`) → non-zero exit, and stdout carries
   no `clinical-plane records OK` line. The existing
   `verify_backup_refuses_a_medium_whose_clinical_plane_is_corrupt` flips a byte inside a record, which
   exercises the signature path. This test exercises the chain path.
4. **Unchanged, and must stay green:**
   `verify_backup_is_clean_on_an_empty_clinical_medium_even_with_a_mismatched_sidecar`.
5. **A sidecar for another path is not evidence, even when it records clinical events.** Back up to B
   with no clinical events, write a chart, back up to A (the sidecar now describes A and records a
   clinical watermark) → `verify-backup --from B` exits 0 with the EMPTY line and no `SHORT`. This is
   the only test that can see the caller's "describes this medium" condition, because 6.3.4's
   sidecar records no clinical watermark either way. What it pins, and why, is in §7.

### 6.4 Mutations to run before calling the tests meaningful

Flip `<` to `<=` in the evidence rule (kills 6.2.8); drop the "describes this medium" condition in the
caller (kills 6.3.5); treat a `None` medium watermark as "no shortfall" (kills 6.2.6 and 6.3.1); print
the clinical OK line before `refuse_unsound_medium` instead of after it (kills 6.3.3).

---

## 7. What is still broken when this merges

- **Gaps in `source_seq`** are not reported: #549.
- **A kit verified on another machine** gets no evidence and can read EMPTY with exit 0 over a copy
  that lost its clinical plane: #551.
- **An empty drive rotated through a DIFFERENT path stays green even when this node has charts**
  (6.3.5 pins it). The node-global sidecar does prove that *this node* captured clinical events, but
  `verify-backup` cannot tell whose medium `--from` is: it does not bind the medium's self-marker to
  `--key`'s node, so a sidecar about another path may describe a different node entirely. A non-empty
  drive at a different path already fails `COVERAGE-UNKNOWN`, so this asymmetry predates the slice. It
  comes from Task 12's `medium_seq.is_some()` guard, whose justification ("nothing for an export to
  cover") is about export coverage, not about charts that should be there. Closing it needs a kit
  identity the sidecar can bind to, which is #551's fix. The comment on #551 names this case.
- **`verify-backup` still cannot say that a medium predates a shred.** `security.md` owes that to
  "DR slice 2e", a label since retired. The docs pass on this branch re-homes that sentence to an
  issue, so no stale label is left standing.
- **A capture is O(whole medium), and so is this check**: #552. This slice adds one more pass over
  records that are already in memory, and no new I/O.

## 8. Documentation owed by the branch

- `docs/spec/security.md` §"the event log survives": one clause saying that `verify-backup` reports
  the clinical plane and fails a medium that is short of what the node last wrote (#567). Whether
  that needs a spec-version bump follows `docs/spec/decisions/README.md`'s rule, checked rather than
  assumed.
- HANDOVER / ROADMAP: #567 closed; #549 and #551 remain, with this slice's narrowing recorded.

## Paper-parity benchmark (§1.2)

Paper-parity: not clinical-surface — this changes an unattended operator health check's exit code and
adds no human act. In the one case where it changes the outcome, a red while the node is still alive
replaces a green that would have cost the clinic its charts at the disaster.
