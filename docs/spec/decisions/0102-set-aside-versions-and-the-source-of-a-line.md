# ADR-0102 — Set-aside versions, and the source of a line

- **Status:** Accepted (builds batch charting slice 2a: the wire, the floor, the projections, the list's "From:" and
  "Set aside", and Sign)
- **Date:** 2026-11-13 (built), recorded 2026-11-13
- **Spec version at acceptance:** 0.104
- **Issues:** PR [#1088](https://github.com/cairn-ehr/cairn-ehr/pull/1088) (the design, the plan and the build).
  Design: `docs/superpowers/specs/2026-11-13-batch-charting-slice-2-versions-wire-design.md` (approved by the
  maintainer 2026-11-13). Refs [#1072](https://github.com/cairn-ehr/cairn-ehr/issues/1072) (a warning is never
  allergy-only; a set-aside version raises none).
- **Builds on:** [ADR-0099](0099-charting-a-medication.md) (the charted line, `info_source`, the check) ·
  [ADR-0100](0100-charting-a-list-under-one-signature.md) (the list, one Sign, the spend in the line's transaction) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) and [ADR-0053](0053-per-write-human-authorship.md) (sealed bodies, per-write
  authorship) · [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) (structure at both doors,
  vocabulary at the authoring door) · [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (a peer's event of a
  type this node does not know is admitted, never refused) · [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (soft policy in the window)

## Context

A clinician taking a medication history meets two sources that disagree: the GP's letter says atorvastatin 40 mg, the
packet the patient brought says 20 mg. On paper the clinician writes the line they believe, and beside it, crossed
but legible, the other figure and where it came from. The record must be able to say the same: the line believed, the
version not believed, who said it, and why the clinician chose. Two gaps in charting slice 1a
([ADR-0099](0099-charting-a-medication.md)) and batch charting slice 1
([ADR-0100](0100-charting-a-list-under-one-signature.md)) stood in the way. A line could carry only the category
`info_source` ("patient-reported"), never "the GP letter of 3 March" and never a reason. And a version that was not
believed had nowhere to go: charted as a line it would be a current medication that the allergy check, the duplicate
check and every reader would treat as taken.

The builder split the design (§7) into two slices. Slice 2a, this one, writes and shows the version. Slice 2b
restores a version and lets a reviewer mark one "checked: not current".

## Decision

### The source of a line: two keys, never one

1. **`source_detail` and `why` are two optional keys** on `clinical.medication.asserted`, not one note.
   `source_detail` says where the claim came from ("GP letter", "daughter, from the packet"); `why` says why the
   clinician charted it this way. Fused into one box, a reader (or a later matcher) cannot tell a provenance from a
   rationale, and a clinician who has only one would be pushed to invent the other (principle 4). The builder
   (`crates/cairn-event/src/medication/source_note.rs`) inserts a key only when present, so an assertion without
   them serialises byte-identically to one built before they existed. The twin appends ` — from {detail}` and
   ` — why: {why}` in that order (twin = wire).
2. **The floor is `db/079` §1**, called from `db/031`'s assertion check. Both doors refuse a present key that is not
   a string; a JSON null reads as absent at both doors (as `db/077` reads every optional key), so a peer
   that emits nulls is not penned. The authoring door alone also refuses a blank value or one with a control
   character (newline and tab included: these are one-line labels). `db/077` gains the two projection columns on
   `medication_charting`, paired with `ALTER TABLE … ADD COLUMN IF NOT EXISTS` for a table made before them.
3. **"From:" is a preset of the window, not a wire value.** The list shows one "From:" above its lines with six
   labels, and the window maps each to the wire category `info_source`, never guessing the category from typed text
   (`info_source_of`): *Patient* and *Carer / family* to `patient-reported`; *GP letter* and *Pharmacy* to
   `external-record`; *Packets / Webster pack seen* to `clinician-observed`; *Other* to `unknown`. The free text beside
   it becomes `source_detail`, or, when blank, the label itself (`stamp`).
4. **"From:" stamps only a NEW line**: a line with no stored row yet and no source of its own. A line
   typed before "From:" keeps whatever it stored, none included, so a typo fix never signs a provenance nobody gave
   and never costs the line its hand tick. A line may name its own source, which "From:" never overwrites. With no
   "From:" set, nothing is stamped and slice 1a's press default stands for a charted line. A set-aside version with no
   source of its own signs `unknown`, never the press default: the press says who reported a line charted as current,
   and nothing about a figure the clinician set aside (principle 4; `version_to_sign`).
5. **The preset is stored with the writer.** `medication_draft_from` holds one row per (chart, writer), a NULL writer
   being its own bucket, so the next line starts with it after a crash. It stores the exact label with a `CHECK` on
   the six, so `info_source_of` can never silently miss a stored label; renaming a label is not an in-place edit: the
   constraint reaches an existing table only through a guarded block that adds it when absent, so it must be dropped
   and re-added, and `info_source_of` and the window's preset list changed with it. The row is working state like the
   lines: never exported, never backed up. The delete door drops it with the writer's last line on the chart, and
   every Sign spends through that door, so a Sign that empties the writer's list resets "From:"; one that leaves a line
   behind (unticked, or a version kept) keeps it. Not yet guaranteed: a "From:" set with no line ever added is never
   dropped (#1090).

### The set-aside version: its own type, never a flag

6. **A version is its own event type**, `clinical.medication-set-aside.asserted` (schema
   `clinical.medication-set-aside/1`, class `additive`, `db/079` §2), built by `set_aside_body` from the assertion's
   body so the drug is spelled once on the wire. It drops `medication_id` (a version is not a thread) and
   `charted_as`, `warning_override` and `safety`, and adds `version_id` and a **mandatory `alternative_to`**, the
   `medication_id` of its companion. Its twin reads `{from} reported {drug line}. Set aside, not current` and
   ` — {why}` when given, `{from}` being the `source_detail`, else the `info_source` word.
7. **It is never a current medication.** It is written to its own table, `medication_set_aside`, keyed by the event's
   content address (what a shred scrubs by, `db/037`); no reader of current lines (`medication_statement`,
   `patient_medication*`) sees it, and it has no thread. **Tempting alternative rejected:** a flag on the ordinary
   type, which every existing reader would then have to remember to filter.
8. **The floor refuses `charted_as`, `warning_override` and `safety` at BOTH doors.** A version was never charted
   and overrode nothing, so no honest writer emits them and refusing them pens nothing legitimate. The `safety` ban
   has a specific reason: `db/020` copies the envelope's `safety` into `event_log.safety` for every type,
   and `cairn_patient_safety` reports any event carrying one, so a peer's claim on a set-aside would become a live
   safety signal on the chart. The structural shape (ids, `substance.term`, `info_source`, the coding, route,
   strength, components, the two notes) is checked at both doors too.
9. **The references are checked at the authoring door only** (ADR-0051): the schema tag, that `alternative_to` is a
   thread on this chart, that it is still current (not stopped), and that `version_id` is new. A companion submitted
   earlier in the same transaction counts, because it is already projected.
10. **The remote door tolerates an orphan.** A peer may send a version before its companion has synced; the event is
    admitted and projected, and because every reader joins on `alternative_to`, an orphan is shown nowhere. A version
    whose companion is on ANOTHER chart is not refused on remote apply but flagged by
    `cairn_guard_medication_patient`, exactly as a cessation's is. A body the applier cannot represent is skipped,
    never raised (a sealed peer body without custody never met the
    structural floor, and a raise would fail the late-custody heal).
11. **Readers filter on `patient_id` AND join on `alternative_to`** (`read_set_asides`). A flagged remote version keeps
    its own `patient_id` but another chart's `alternative_to`; joining on `alternative_to` alone would show it on the
    wrong chart. `version_id` is not unique at the remote door, so nothing keys on it.
12. **A version's sensitivity is its companion's.** `db/048`'s thread resolution unions `medication_set_aside` on
    `alternative_to`, behind its own `to_regclass` probe (a fourth source file, so a loader that stops between files
    never meets a missing table).

### On the list, and at Sign

13. **A line is marked a version of a companion**: `medication_draft_line` gains `info_source` (checked against the
    four categories), `source_detail`, `why` and `set_aside_of`, `line:<uuid>` (another draft line) or `med:<uuid>` (a
    charted medication), shape-checked. A companion must be the same coded drug: `companion_candidates` offers only
    lines of one single-part `drugref-moiety` coding, so an uncoded line or a combination has no "Set aside" button
    (only single-part coded lines can be versions), and a charted candidate is a current line of the OPENED chart, not
    a linked chart's.
14. **A set-aside line is never ticked**, by default or by hand; the door forces its hand tick off. It reads "set
    aside — not current" where the tick's words were.
15. **Provenance edits are content changes, in the door.** A change to a line's source, why or mark clears a hand
    tick, because the signature vouches for them. It does NOT clear a stored untick (`held`): only the drug's words
    do, otherwise adding a "why" to a line the clinician had unticked would re-derive the default tick and the line
    would tick itself. The save door treats an omitted parameter as null, so the window sends all four own fields on
    every save; and a read brings back the line's own fields even while it is being typed, unless a local edit changed
    that field since the last send. The save that stamps "From:" answers the stamp, and the window puts it on the line
    as soon as that save answers `saved`, unless the clinician changed the line's source after the save was sent; so
    the line's next save carries the stamp whether or not a read has landed in between, and an autosave never clears a
    stamp the clinician did not take off. Picking "not set" for a line's own source clears its category and detail, so
    a stale stamp can be taken off; the press's default then stands at Sign.
16. **Unmarking leaves the line held**. Removing the mark through the save door, or deleting a companion (which
    unmarks its versions, with a fresh `saved_at`), leaves the line unticked with its tick gone: a version the
    clinician believed wrong must never become default-ticked. A companion's spend at Sign is such a delete, so it
    also unmarks and holds each of its versions kept for their own reason (a blank part, or the companion now another
    drug); the outcome then says "{the line as typed} — no longer set aside: its line was charted; set it aside
    again", never "version kept", which is said only of a version whose companion was not charted.
17. **A version signs only in or after its companion.** At Sign `plan_versions` derives every version's companion
    from the STORED mark on the fresh read, never from the webview, and `version_holds` re-checks it. A
    version beside a ticked list line goes in that line's one transaction (`chart_draft_line`: the companion's event
    first, then each version's, because `alternative_to` needs the companion's thread). A version beside a charted
    current line signs alone, with its spend, in its own transaction (`set_aside_draft_line`), after every ticked
    line, even when nothing is ticked. Any other version stays in the list with its line saying why ("its line was
    not charted", or the companion is gone, stopped, or now another drug). A version not yet finished is kept ("it is
    not finished") wherever its companion is: a version has no tick, so this is the guard an unfinished line's
    never-default tick is for a line, and a half-typed figure is never signed.
18. **The spends run the other way round**: every version's draft row is spent before its companion's,
    because the delete door unmarks and re-stamps the versions of a line it deletes, and a version spent after that
    would answer `changed` and roll everything back.
19. **A version's refusal rolls back its companion.** The floor refusing a version signed with its list companion
    (a control character in its why, say, [#1091](https://github.com/cairn-ehr/cairn-ehr/issues/1091)) abandons the
    shared transaction: the companion is reported not charted and both stay in the list. A version signed alone
    beside a charted line stopped since the read is refused alone and kept, saying why. A lost commit is reported not
    known for the companion and its versions together.
20. **Sign says it records versions.** Near Sign the list reads "Sign also records {n} set-aside version(s), as not
    current" when it will, because a Sign with nothing ticked still signs a version beside a charted line. The count
    comes from the same grouping Sign uses, so it never names a version Sign would keep.

### Reading it back

21. **A charted line shows its versions collapsed under it**, one closed `<details>`, "{n} other version(s)", read in
    the same snapshot as the chart. A line is `{source}: {drug line} — set aside, not current`, then ` — {why}` when
    given. The drug line is `drug_line_twin`'s, rebuilt from the projected columns, so there is one spelling of a drug
    line and a version differing only in route reads differently from its companion. That twin carries the
    `info_source` word in parentheses and, when the coding's display differs from the term, the display in square
    brackets, so the line reads e.g. "GP letter: Lipitor 25 mg (external-record) [atorvastatin] — set aside, not
    current". The versions of a since-stopped line stay shown under it. A failed read of the versions never hides the
    list: it is worded above the lines ("Set-aside versions could not be read: none is shown under the lines"), so
    an empty space under a line is never mistaken for none set aside. A version whose companion is in no row of the
    list is not shown ([#1089](https://github.com/cairn-ehr/cairn-ehr/issues/1089)).
22. **Soft policy.** Which lines may be set aside, the pick ("Version of which line?" when two or more qualify), the
    copy and the collapse live in the window. The type, its refusals and the projection are the floor and the same
    for every client.

## What is not guaranteed

- **No restore and no "checked: not current"** (slice 2b). A version can be set aside and read; "use this version
  instead" is not yet an act, and the version stays until that slice.
- **No warning threshold** (slice 3). A set-aside raises no warning and takes no part in a check.
- **Drafts do not cross nodes** ([#963](https://github.com/cairn-ehr/cairn-ehr/issues/963)): the lines, their marks and
  "From:" live on the node that typed them. The signed version does cross.
- **Only single-part coded lines can be versions.** An uncoded line or a combination cannot be set aside.
- **A peer's orphan is invisible until its companion arrives.** It is held, not lost, and shown nowhere meanwhile.
- **A line typed before "From:" is not stamped**; its source is set by hand. A "From:" change waits, saying so,
  while a line typed before it has not saved.
- **The draft door is not the Sign floor.** It stores a control character in a source or why that the floor then
  refuses at Sign ([#1091](https://github.com/cairn-ehr/cairn-ehr/issues/1091)), and a version's refusal at Sign is
  worded against its companion line ([#1092](https://github.com/cairn-ehr/cairn-ehr/issues/1092)). The "From:" door
  checks less than the line door ([#1094](https://github.com/cairn-ehr/cairn-ehr/issues/1094)).
- **A peer version that opens malformed is skipped without a trace**
  ([#1093](https://github.com/cairn-ehr/cairn-ehr/issues/1093)).
- **The figures are owed by a human.** Every stopwatch figure, the real window's WebKit and VoiceOver, and the DB-gated
  window tests (which CI does not run, #905) are measured per RUNBOOK §33.

## Consequences

- Paper *N* = 1 act (write the believed line, the other figure crossed beside it with its source) → architecture *M* = 1
  human signature (a line and its version are two events in one transaction) → UI *K* = 1 press of "Set aside" and 1
  Sign. A "From:" is set once for the lines after it. Budgets: "From:" at most 3 s, "Set aside" at most 2 s, the Sign
  of a 10-line list with 2 versions within slice 1's 3 s.
- `SCHEMA_GENERATION` moves 78 to 79 (`db/079` is a new file); the `db/031`, `db/037`, `db/048`, `db/077` and `db/078`
  edits are in place and reach an existing database by the content hash
  ([ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md)).
- A version is the first event type whose whole point is to be kept and never current. Slice 2b's restore must be a
  stop and a new line in one transaction, never an edit of the version.
