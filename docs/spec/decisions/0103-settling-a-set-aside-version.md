# ADR-0103 — Settling a set-aside version: the restore and "checked: not current"

- **Status:** Accepted (builds batch charting slice 2b: the restore and the check on the wire, their floor and
  projections, the list's restore line, Sign, and how a settled version reads)
- **Date:** 2026-11-14 (built), recorded 2026-11-14
- **Spec version at acceptance:** 0.105
- **Issues:** PR [#1096](https://github.com/cairn-ehr/cairn-ehr/pull/1096) (the plan, the wire, the restore floor and
  the "checked" type) and its continuation PR [#1098](https://github.com/cairn-ehr/cairn-ehr/pull/1098) (the list,
  the node, the window and these docs). Design:
  `docs/superpowers/specs/2026-11-13-batch-charting-slice-2-versions-wire-design.md` §3.3 and §3.4, amended by
  the build's rulings R1 and R3–R13 (recorded below; R2 was superseded by R5). Refs
  [#1097](https://github.com/cairn-ehr/cairn-ehr/issues/1097) (an erased link in a restore chain).
- **Builds on:** [ADR-0102](0102-set-aside-versions-and-the-source-of-a-line.md) (the set-aside version, "From:", the
  two source keys; its Consequences required a restore to be a stop and a new line) ·
  [ADR-0100](0100-charting-a-list-under-one-signature.md) (the list, one Sign, the spend in the line's transaction) ·
  [ADR-0099](0099-charting-a-medication.md) (the charted line, `charted_as`, the per-line checks before the key) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) and [ADR-0053](0053-per-write-human-authorship.md) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (a reference is an authoring-door rule) ·
  [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) (erasure is best-effort and declared) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (the floor is in the database; soft policy in the window)

## Context

The morning after the admission history, the reviewer rings the GP about the atorvastatin the letter put at 40 mg and
the packet at 20 mg. Either the set-aside figure was right after all, often with the dose changed since, or it was
checked and is not what the patient takes now. On paper the reviewer rewrites the line, or initials the crossed-out
figure, and notes the call. Slice 2a ([ADR-0102](0102-set-aside-versions-and-the-source-of-a-line.md)) recorded the
version and showed it, but nothing could settle it, so every version stayed an open question for good.

The design's §3.3 had a restore emit no cessation when the companion was "already replaced by another version's
restore". The build found that this leaves two current lines of one drug: the earlier restore's line still stands.
Ruling R3 replaced it (decision 2).

## Decision

### The restore: a stop and a new line, in one transaction

1. **A restore is a new line, never an edit of the version.** It is an ordinary `clinical.medication.asserted` on a
   new thread with one new optional top-level key, **`restores`** = the version's `version_id`
   (`crates/cairn-event/src/medication/charting.rs`, `Charting::restores`). The builder inserts the key only when
   present, so an assertion without it serialises byte-identically to one built before it existed. The twin's tail
   gains ` · restores a set-aside version`, after the warning-override clause (twin = wire). The version event is
   never touched (principle 2).
2. **A restore stops whatever stands for the companion** (ruling R3). That is the companion itself when it is current,
   or else whatever is current anywhere in the companion's rooted restore tree (decision 3): a line that restored one
   of its versions, a line that restored a version of THAT line, and so on. A companion stopped for another reason,
   with nothing current in its tree, needs no cessation. This supersedes the design's §3.3 sentence quoted above.
3. **What stands is ONE SQL function, read by the floor and by the window alike** (rulings R4 and R10):
   `cairn_medication_restore_lineage(companion, patient)` (`db/080` §1). It walks UP first, from a restore line to the
   companion of the version it restored, to the original line, then returns the whole tree below it, following
   `medication_restore` → `medication_set_aside.alternative_to` within the one chart. Walking down only from the
   companion is not enough: a version may be set aside beside a restore line, and its restore would then miss a
   sibling restore under the original line, leaving two current lines (R10). Both walks use `UNION`, so a cycle a
   peer could forge still ends. The function is `STABLE SECURITY DEFINER` with a pinned `search_path`, and EXECUTE is
   granted to `cairn_window` and `cairn_agent`. The floor's commit check and the node's `version_for_restore`
   (`standing`) read it, so what Sign stops and what the floor demands cannot drift apart.
4. **The floor checks a restore twice.** `cairn_check_medication_restores`, PERFORMed from `db/031`'s assertion
   check, judges the key's shape at BOTH doors: a uuid string, or JSON null, which reads as absent. At the authoring
   door only, and only for an event not already in the log, it requires that `restores` names a set-aside version of
   this chart (one submitted earlier in the same transaction counts) and that the line is a NEW thread. It then records
   the debt: one `medication_restore_pending` row per (restore event, companion), with the companion read at submit, so
   the commit check never needs the version's row to survive until then.
5. **The pairing is checked at COMMIT.** A `DEFERRABLE INITIALLY DEFERRED` constraint trigger on the pending table runs
   once per row: unless the restore's own thread has a cessation, no other member of the lineage may be current
   (projected in `medication_statement` with no `medication_cessation` row). Otherwise the whole transaction fails and
   every write in it rolls back; no client of the authoring door, raw SQL included, can sign a restore that leaves two
   current lines among those the lineage can see (two multi-node limits are under "What is not guaranteed"). The row
   is then deleted, so the table is empty outside a transaction. The stop and the line may be
   submitted in either order. A client that sets the constraint `IMMEDIATE` only makes the check stricter: the stop
   must then come first. Two restores of one companion in one transaction commit when only one of them stands.
6. **The commit check never reads the `cairn.remote_apply` setting.** Only the authoring door owes the pairing, but
   the strict door sets that setting to `''` for its own duration only, and at COMMIT it reads whatever the transaction
   last set (`cairn_reproject` leaves it `'on'`). The pending row's existence is the record that the restore came
   through the authoring door. **Tempting alternative rejected:** read the setting in the deferred trigger. That
   would judge the wrong door, and a reprojection in the same transaction would silently waive the check.
7. **The remote door checks structure only.** A peer's restore that arrives before its version has synced is
   admitted and projected. A reprojection, a late-custody heal and a shred re-fold run the appliers only, never a
   check, so none of them writes a pending row. A re-submit of an event already in the log is `db/005`'s silent no-op
   and is judged no further. A set-aside version carrying `restores` is refused at both doors (`db/079`, even as JSON
   null): a version restores nothing.
8. **The floor does not compare the new line's drug with the version's.** The reviewer may edit the line ("right
   drug, but the dose has changed since"), and the floor cannot tell an edit from a different drug.
9. **`restores` has its own projection, `medication_restore`** (ruling R5): one row per restore event, keyed by the
   event's content address, insert-or-nothing, through its own applier registered `heal_safe`, and deleted by content
   address on a shred (`db/037`). **Tempting alternative rejected:** a column on `medication_charting`. That table
   keeps one HLC-winner row per thread, so a sealed restore that heals late would lose to a newer re-assertion of its
   thread, and a shred that scrubs by the producing event's content address could miss it.

### "Checked: not current": its own type, with no verdict field

10. **A check is its own event type**, `clinical.medication-set-aside-checked.asserted` (schema
    `clinical.medication-set-aside-checked/1`, class `additive`, `db/080` §2), built by
    `crates/cairn-event/src/medication/version_checked.rs`. Its payload is `version_id`, `info_source` and the
    optional `source_detail` and `why`. **There is no verdict field: the type is the verdict.** The opposite outcome
    is a restore, so each outcome has one spelling on the wire and there is no flag to get backwards. Its twin reads
    `Set-aside version checked: not current — from {source}`, then ` — {why}` when given, `{source}` being the
    `source_detail`, else the `info_source` word.
11. **The floor.** At BOTH doors: `version_id` is a uuid string; `info_source` is a non-blank string; `charted_as`,
    `warning_override` and `restores` are absent, refused even as JSON null; the two source keys are judged as a
    charted line's (`db/079` §1); and a §5.9 safety claim is refused. At the authoring door only: the schema tag,
    and a `version_id` that names a set-aside version of this chart. A peer's check that arrives before its version is
    admitted and shown nowhere, because readers join on `version_id` AND `patient_id`.
12. **Its projection is `medication_set_aside_checked`**: one row per check, keyed by content address, with its
    time taken from the event's HLC wall, and scrubbed by content address on a shred (`db/037`). Its sensitivity is
    its version's companion thread's: `db/048`'s thread resolution reaches the companion through the version, behind
    its own `to_regclass` probe.
13. **A check locks nothing.** A second check of the same version is admitted and shown (two reviewers). A checked
    version can still be restored. A check is never a medication line: it is not in `medication_statement`.

### On the list, and at Sign

14. **"Use this version instead" signs nothing.** The press adds a restore line to who's here's list through
    `db/078`'s `cairn_med_draft_restore_add`, the only door that sets the draft line's `restores`. The door answers
    `no-such-version` for a version not on this chart, and starts the line finished, unticked and not held. The save
    door has no `restores` parameter, so an edit keeps it and a plain line never becomes one, and it forces a restore
    line's set-aside mark to NULL. Before adding, the window refuses fixture mode, a version not on the OPENED chart
    beside a companion present there (`VERSION_GONE`; a linked chart's version is never restored from here, design
    §2), and a list held on another terminal. The webview then opens the one editor on the new line.
15. **The prefill** (ruling R8) is the version's term as the drug, one part with the version's coding, and the
    version's `sig` as the directions, with its dose and route put in front only when the `sig` does not already say
    them (doubled, they would read as two doses). The line's `charted_as` is `started-here` when the companion was
    started here; anything else gives `pre-existing`: a companion charted before slice 1a, with none (the
    maintainer's ruling), and `unknown` or any word a peer sent (ruling R11), because a list line takes only the two
    presses. This window has no control that changes a list line's press.
16. **A restore line never takes "From:" or the press default.** "From:" stamps only a line with no stored row
    (ADR-0102 decision 4), and a restore line is stored by its press, so it is never stamped. A restore line with no
    source category of its own signs `unknown`, never the press default (the maintainer's ruling): the press says
    who reported a line being charted, nothing about what corroborated a version. The reviewer gives the line's own
    "From:" and "why" for what the call found.
17. **A restore line is not a duplicate of the lines it will stop.** It is judged against the chart without the rows
    made wholly of its `standing` lines, but still against every other current line and every other list line, so
    two restores of one drug (the same moiety code; uncoded lines never duplicate) duplicate each other and neither
    is default-ticked; ruling R9 a catches the rest at Sign (decision 18). A restore whose `standing` holds a
    restore line is never default-ticked (ruling R9 b): replacing a restore charted earlier must be deliberate. Nor
    is a restore whose `standing` is empty (ruling R12): its line was stopped since the press, by another source, and
    Sign would restart a stopped drug without a word (the chart-held rule). Its mark reads
    `restores the {source} version — its line has since been stopped`. Nor is a restore whose version no longer
    reads, which Sign refuses. A hand tick on a restore line vouches for what stood for its companion as drawn when
    it was ticked (the version and each `standing` line, kept beside its allergy refs in `ticked_against`), and holds
    at Sign only while the fresh `standing` is exactly that set. So a tick given before a colleague's stop or restore
    fails at Sign and the line comes back unticked, while a tick given after the mark (vouching for an empty
    `standing`) signs, as a deliberate restart.
18. **Sign re-derives the restore from the STORED row on the fresh read**, never from the webview. Each restore
    line's version is read on the OPENED chart with what stands for it. Two per-line refusals come before the key:
    `RESTORE_VERSION_GONE` ("the version it restores can no longer be read"), and, for a later ticked restore of the
    same lineage as one about to sign (the same companion, or a shared standing line), `SAME_LINE_RESTORED` (ruling R9
    a): its stops were read before the first one charts, so the floor would refuse it in its own words. It reads
    "another restore of the same line is signed first; signed after it, this one replaces it" (ruling R13), saying
    what a later Sign does.
    `chart_draft_line` then signs one cessation of each standing line, reason `replaced by the {source} version`,
    FIRST, then the line, then its spend, in ONE transaction; a failed spend rolls the stops back with the rest. A
    caller's own `charting.restores` is refused, with or without a restore, so the key has one source. A companion
    stopped by someone else between the read and the commit still leaves one current line: `db/031` admits a second
    cessation of a stopped thread, and `medication_cessation` keeps one row for it, the HLC winner's (see "What is not
    guaranteed").
19. **The check is an inline row** under the version: "From:" with slice 2a's presets, the document or person,
    *Why*, its own Sign and Cancel. The row signs nothing until its Sign. Sign checks that the chart is on screen and
    writable, re-reads the version on the OPENED chart, releases the key once and signs one event in its own
    transaction. A preset maps as the list's "From:" does. With no preset the source is `unknown` and the typed detail
    is kept (ruling R1: the category is never guessed). A blank why is none. No dialog, here or for the restore.

### Reading it back

20. **The settlements are read in the same snapshot and savepoint as the versions** (`read_settlements`), so a
    version never shows as unsettled because only its settlements failed to read. A check shows only for a version
    on its own chart beside a companion present there. A restore is read from `medication_restore` and joined to its
    version's companion on the same chart.
21. **The words.** Under the version: `checked by {who}, {when}: not current — from {source}`, then ` — {why}`, one line
    per check, oldest first; or `restored by {who}, {when}`. Under the new line,
    `replaced: {old drug line} — {who}, {when}`, the companion spelled by `drug_line_twin` without its coding, which
    this read does not join (`medication_coding`). The summary reads `{n} other version(s)`, then `, {u} to check`
    when `u > 0`; a version is to check when nothing settled it and its line is current. `{when}` is the tab crate's
    `local_instant`.
22. **`{who}` is a person or nobody** (ruling R6): the vouched attester when there is one (`db/065`), else the
    envelope signer, which is the human under ADR-0053. It is named only when the registry enrolled that key for a
    human (`cairn_note_key_is_human`), else it reads `signer not known`: a device or agent key is never read as the
    person who checked or restored.
23. **The buttons** are offered only on the OPENED chart's versions and only while the version is not restored. A
    checked version keeps both. The versions of a stopped line are not "to check", but keep their buttons.
24. **Soft policy.** The buttons, the prefill, the check row, the duplicate rule for a restore line and the copy live
    in the window. The types, the pairing at commit and the projections are the floor, the same for every client.

## What is not guaranteed

- **An erased link breaks a restore chain** ([#1097](https://github.com/cairn-ehr/cairn-ehr/issues/1097)). The
  lineage walks projected rows. A shredded version in the middle of a chain hides the subtree below it, and a
  shredded restore event whose line stays current through a later re-assertion drops that line from every lineage. A
  later restore of a sibling version then passes, leaving two current lines. Both stay visible on the chart (an
  over-statement, never a silent loss), and erasure is best-effort and declared (ADR-0005).
- **A restore may chart a different drug than its version.** The floor does not compare the drug fields, and the
  editor lets the reviewer change the drug.
- **No "Waiting for you" group for unsettled versions.** The "to check" count is per line, on the chart only.
- **"replaced:" names the version's companion** (ruling R7), whatever the restore actually stopped: in a chain, the
  original line rather than the intermediate restore line, and, for a companion stopped for another reason, a line
  this restore did not stop.
- **The window's conflict rule is not "same root".** When nothing in a tree stands (every line stopped), two
  hand-ticked restores of versions with different companions in that tree are not caught by
  `SAME_LINE_RESTORED`. The first charts; the floor refuses the second at commit, and the list says so in the
  floor's raw text.
- **A line carrying `restores` twice joins two trees.** The authoring door admits `restores` only on a new thread, but
  a peer's re-assertion of a restore line naming a second version gives that line two parents. The lineage then spans
  both trees, and a restore in either must stop the current line of the other.
- **A receiving node may briefly show none current, or two.** There the stop and the restore are separate events,
  and the remote door checks structure only. When the stop arrives first (the usual order: the stops take their HLCs
  first) the drug briefly shows no current line; when the restore arrives first, two. For one restore, both settle
  when both arrive; the next bullet is when they do not.
- **Restores made apart can leave two current lines for good.** Two offline nodes that each restore a different version
  of one line each stop it and each chart their own line; after sync, both lines are current. So are two lines when a
  node restores a version while a peer's restore line of the same line is ahead of its version: the lineage walks
  `medication_restore` through `medication_set_aside`, cannot reach the peer's line, and stops nothing for it. The floor
  cannot prevent either (availability over consistency: each node signed what it could see). They surface only where the
  two lines share a duplicate key (`patient_medication_reconciliation_flag`, as `db/033` redefines it: the coding pair,
  else the term). Separately, shredding a restore line's only assertion leaves its companion stopped, reading "replaced
  by the … version", with no replacement on the chart.
- **A colleague's stop can lose the shown reason to a restore's.** A stop of the companion that lands between Sign's
  fresh read and its commit is overwritten in `medication_cessation` (one row per thread) by the restore's own stop,
  when that stop holds the later HLC. The log keeps both cessations, and the colleague's reason is still listed
  beside the line as another reason (`db/060`), but the reason the line shows first is "replaced by the … version".
- **Drafts do not cross nodes** ([#963](https://github.com/cairn-ehr/cairn-ehr/issues/963)): a restore line lives on
  the node that pressed it. The signed restore and check do cross.
- **The figures are owed by a human.** Every stopwatch figure, the real window's WebKit and VoiceOver are measured
  per RUNBOOK §34; the webview was walked in Chromium only. The check's write half is not timed (it has no
  `ui_gesture_timing` kind). The DB-gated window tests run locally only; CI does not run them
  ([#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)).

## Consequences

- Restore: paper *N* = 2 acts (rewrite the line, note the call) → architecture *M* = 1 signed act (the stops and the
  new line are events under one key release and one transaction) → UI *K* = 1 press + the line's "From:" + "why" +
  Sign, plus the edits when the dose changed, as paper's rewrite carries them. Checked: *N* = 1 (initial the
  alternative) → *M* = 1 → *K* = 1 press + "why" + Sign. Reading what was settled: *K* = 1 (open the collapsed
  versions). Budgets: the restore press to the line in the editor at most 2 s; the check row's Sign at most 3 s.
- `SCHEMA_GENERATION` moves 79 to 80 (`db/080` is a new file); the `db/031`, `db/037`, `db/048`, `db/078` and `db/079`
  edits are in place and reach an existing database by the content hash
  ([ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md)).
- The deferred constraint trigger with a pending table is the floor's first check that must wait for COMMIT and must
  know which door an event came through. A later pairing rule can reuse the shape: record the debt at submit, judge it
  at COMMIT, and never read a transaction-local setting there.
