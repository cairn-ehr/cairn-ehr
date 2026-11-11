# ADR-0100 — Charting a list of medications under one signature

- **Status:** Accepted (builds slice 1 of the batch-charting design; answers the `M > N` defect
  [#1072](https://github.com/cairn-ehr/cairn-ehr/issues/1072) for the list's first half; slices 2 and 3 follow)
- **Date:** 2026-11-11 (built), recorded 2026-11-11
- **Spec version at acceptance:** 0.102
- **Issues:** PR [#1078](https://github.com/cairn-ehr/cairn-ehr/pull/1078) (the design, the plan and the build).
  Design: `docs/superpowers/specs/2026-11-11-batch-charting-1072-design.md` (approved by the maintainer
  2026-11-11); plan: `docs/superpowers/plans/2026-11-11-batch-charting-slice-1-the-list-and-one-sign.md`. Refs
  [#1072](https://github.com/cairn-ehr/cairn-ehr/issues/1072) (it stays open until slice 2's versions land) and
  [#963](https://github.com/cairn-ehr/cairn-ehr/issues/963) (drafts do not cross nodes). Filed by this slice:
  [#1081](https://github.com/cairn-ehr/cairn-ehr/issues/1081) (the pick carries no form),
  [#1082](https://github.com/cairn-ehr/cairn-ehr/issues/1082) (the doors trust the window's writer).
- **Builds on:** [ADR-0099](0099-charting-a-medication.md) (the one line, the check at charting, the re-check at Sign,
  `warning_override`) · [ADR-0098](0098-allergy-coding-and-the-medication-check.md) (the allergy check) ·
  [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) (a defect on one line never
  invalidates another; transaction scope matches clinical atomicity) · [ADR-0087](0087-the-first-note-stream-and-its-draft.md) / [ADR-0088](0088-whos-here-and-a-drafts-parts.md)
  (the note draft store, who's here as the writer, "Continue here") · [ADR-0095](0095-the-person-menu.md) (the person
  menu) · [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (soft policy in the window)

## Context

On paper the admission medication history is a list of lines written under each other and **one signature at the
bottom**. Slice 1a (ADR-0099) charted one line per Sign, so a twelve-line home list cost twelve signatures. The
maintainer ruled that an `M > N` defect under house rule 7 (#1072).

The real case behind the design: an elderly patient brings a handwritten list, an old GP letter and photos of his
packets. The three disagree, the clinician is called away several times with the list half written, and in practice
works it out on paper first and types it in afterwards. The list must therefore survive interruption, belong to the
person writing it, and never go into the record by itself.

This slice builds the list and its one Sign. It adds **no wire change**: every event it signs is slice 1a's
`clinical.medication.asserted`, with slice 1a's optional keys.

## Decision

### The list is a durable, per-writer draft (`db/078`)

1. **A mutable store beside the record.** `medication_draft_line` holds one row per unsigned line: the raw text as
   typed, the window's split of it (`typed_drug`, `directions`), the coded parts, the press (`charted_as`), the reason,
   the tick state and `finished`. It is never signed or synced. No event plane of the backup holds it, and the backup
   reads no draft table, so it is in no backup or export (ADR-0087's ruling for note drafts).
2. **Reached only through definer doors.** `cairn_med_draft_line_save`, `cairn_med_draft_lines`,
   `cairn_med_draft_line_delete`, `cairn_med_draft_take` and `cairn_med_drafts_by_writer` are `SECURITY DEFINER` with
   `search_path = public, pg_temp`, revoked from `PUBLIC` and granted to `cairn_window` only. `cairn_window` holds no
   privilege on the tables.
3. **Per writer.** A line's `writer` is the vault staff id (normalised by `cairn_vault_staff_id`) of the person named in
   who's here when the list was read, or NULL when nobody is named (principle 4: never force a name). A named writer
   must be a vault member whose role may act clinically, else the save answers `no-such-writer`. A line's writer never
   changes: a save naming another writer answers `not-yours`. The window shows a list only to its writer, and the
   NULL-writer list only while nobody is named. Save and delete are sent under the writer the list was READ under, so
   a debounced save landing after a who's-here switch still names the person who typed it.
4. **One chart, one terminal.** A save or delete of a line on another chart answers `elsewhere`, and one from another
   terminal answers `moved`. "Continue here" (`cairn_med_draft_take`) moves the writer's whole list on that chart to
   this terminal in one statement, as a note draft's does (ADR-0088). All the writing doors take one advisory lock per
   (chart, writer) list, which also serialises `seq`, assigned on insert only.
5. **Never committed by a timer.** No door, runner or tick turns a draft line into an event; only Sign does. Unlike a
   note draft (ADR-0089) there is no quarantine: a timer that charted a medication on someone's behalf would put words
   in a prescriber's mouth, and the safe default is inaction.
6. **Removing a line is unaudited** (maintainer, design §9 Q3): only what can have clinical consequences is audited,
   and a medication draft cannot go in by itself. The delete leaves a tombstone in `medication_draft_line_deleted`
   (`line_id`, `deleted_at`, nothing else). It is not an audit record. It exists because the window autosaves: a save
   already in flight when the line was removed would otherwise insert it again, perhaps with its hand tick. A save of a
   tombstoned id answers `deleted` and writes nothing.
7. **The list is counted in "Waiting for you"** (ADR-0095). `cairn_med_drafts_by_writer` groups named writers' lines by
   chart. The person menu adds one item per chart to its first group, "{n} unsigned medication line(s)", with "Open
   medication list" bringing the chart's medication pane into view. The picker's per-person counts come from the same
   builder. The door reads one small table, grouped, so #1021's rig was not re-run; this is not a measured figure.

### Ticks (`cairn_node::medication::draft_tick`, pure)

8. **The default tick is derived at every read and never stored.** A line gets it only when all of these hold: the list
   has a named writer; the line is finished (Enter, Escape, or focus leaving it, never while typed); it has at least one
   part and every part is coded; the directions read a form, a dose (amount AND unit) and a frequency (or `prn`, or
   `stat`), by ADR-0099's `read_directions`; the check ran (`!unverified`); it carries no allergy mark; the chart's drug
   allergies are not "not yet asked"; and it is not a duplicate. An adverse-effect note alone does not withhold it (the
   design's default threshold until slice 3).
9. **"Allergies not yet asked" withholds every default tick** (maintainer, design §9 Q1): a chart with no allergy
   records checks clean, but clean against nothing is not clean. Charting is never blocked by it: the unconscious
   patient with no records is charted by hand ticks, and the status stays "not yet asked".
10. **The duplicate guard.** A single-part line coded to a drugref moiety is a duplicate when another list line, or a
    current single-moiety line on the chart, has the same code. It gets no default tick and says "already on the list:
    {line}", naming the line it duplicates. Uncoded lines and combinations never group (a missed grouping leaves two
    lines each needing its own decision, the safe failure). Two chart lines sharing a code are not a list matter.
11. **A hand tick is possible at any stage**, complete or not: the prescriber takes responsibility. It is stored with
    `ticked_against`, the warnings `{source, ref}` it vouches for. Rust builds those at the save from a fresh check,
    keeping only the warnings that were also drawn on the line (`seen`): the webview can narrow them, never add one.
    The door takes them only when a tick is GIVEN; while a stored tick stands on unchanged content it keeps its own and
    ignores later saves', so a warning arriving after the tick is never folded into it by the next autosave.
12. **Changed content clears a tick in the door.** A save that changes `line_text`, `parts`, `charted_as`, `typed_drug`
    or `directions` stores `hand_ticked = false`, `ticked_against = []` and `held = false`, whatever the caller sent, so
    a stale webview or a raw-SQL client cannot sign new words on an old tick.
13. **An untick is stored as a veto (`held`).** Unticking a line by hand stores `held`, so a line the default tick
    would give never comes back ticked after a closed chart, a re-lock or a restart. Changed content or a hand tick
    clears it. Ticking or unticking by hand also marks the line finished.

### Sign (`cairn-gui-tauri`: `med_list_sign.rs`, `med_list_judge.rs`; `cairn_node::medication::draft`)

14. **Every refusal of the whole Sign comes before the key, and is for a changed picture only.** In order: the chart is
    on screen and writable; a staff id typed in the strip must normalise to the list's writer, else
    `This list was written by {name}; only they can sign it`; a fresh read in one snapshot (the lines, the chart set,
    its allergies and its current medication lines) must show the set on screen, and every ticked id must be a line of
    it held by this terminal, else `The list changed since it was shown: nothing was signed. Look at it again`.
15. **A tick that no longer holds refuses the whole Sign, and nothing is signed.** On the fresh read, a default tick
    holds while `default_tick` still gives it and the line is not held; a hand tick holds while every warning the fresh
    check finds is one it was given against. Any ticked line that fails loses its hand tick through the save door, and
    the window says `{n} ticked line(s) changed since they were ticked: nothing was signed. Look at them again`. A
    second press signs the rest. Nothing was written, so this is no rollback (ADR-0060 decision 5).
16. **A missing reason is a per-line refusal, not a changed picture** (ruling R3). A *Start* line with a seen mark and
    no reason, or a reason against a check that could not run, keeps its tick, and slice 1a's per-line checks refuse
    that line alone (`NEEDS_A_REASON`, `CHECK_DID_NOT_RUN`). The per-line checks (the press, what the parts sign, the
    reason) all run before the key, so a Sign that could chart nothing never asks for a passphrase.
17. **One key release, checked by KEY.** The key is released once (`signer_for_act`). For a named writer, the writer's
    current vault key is read before the release and compared after it: a key held for someone else is refused
    `MED_LIST_NOT_YOURS` with nothing written (ruling R-B, as `note::sign`). This is the one refusal after the key: a
    typed staff id has already matched the writer (decision 14), so it fires on a typed credential only when that id's
    released key is not its current one (a re-key race), or when the writer has left the vault, and only then is a
    passphrase spent on a refusal. A NULL-writer list has no default ticks, so anyone may hand-tick and sign it.
18. **Each line commits in its own transaction** (ADR-0060 decision 7). `chart_draft_line` seals the body before any
    transaction, then in one transaction submits the event and spends the draft row through the delete door. The
    spend must answer `deleted`, else the transaction rolls back and the line is not charted ("the line changed since
    the list was read"). The spend names the `saved_at` the Sign read, so a line saved again after the read is never
    spent on content it did not sign. A line leaves the list exactly when it enters the record. There is no shared
    transaction across lines.
19. **A per-line defect never refuses the others** (ADR-0060 decision 1): it is skipped with its reason, and the rest
    sign. The outcome reads `Charted {k} of {n}`, then `{line} — not charted: {reason}` for each line not charted.
20. **A commit with no answer stops the loop.** That line's outcome is worded NOT known (ADR-0099's lead), and every
    line after it is reported "not tried: the line above may have been charted; look at the list again" and stays in
    the list, ticked. Every line's spend is in its own transaction, so the re-read list is the truth.
21. **What is signed is rebuilt in Rust from the stored row**, never from what the webview sends at Sign: the parts,
    the typed drug and the directions are the row's, the check and its `against` are the fresh read's. The node times
    the Sign as `medication-chart` (`ui_gesture_timing`), its count the lines charted.

### The window (`med_list.rs`, `med_list_judge.rs`, `med_list_sign.rs`; `med_list.js`, `med_list_sign.js`)

22. *Start medication* and *Already taking* add a line to the list; each line has a tick box, Remove and slice 1a's line
    editor, and the list has one **Sign ticked lines**. A line saves one second after its last change. A line still
    being typed reads `unfinished` and is never drawn ticked; a line not drawn ticked reads "not ticked: Sign leaves it
    unsigned". On a *Start* line with a mark, the box stays disabled until the reason holds non-blank text. The webview
    sends as ticked only the lines drawn ticked when Sign was pressed. A list held by another terminal says so and
    offers "Continue here". Opening the chart and a who's-here change re-read the list, which re-checks every line. A
    who's-here switch takes the last person's list off the screen before anyone can type into it, and waits for a
    list Sign in flight. No dialog anywhere.

## What is not guaranteed

- **No versions or set-aside yet** (slice 2): two lines of one drug are flagged and withheld from the default tick, but
  there is no "Set aside", no set-aside event type, no "From:" source and no "why". All of those are wire.
- **The warning threshold is fixed** at allergy marks (slice 3 makes it an administrator's policy, lowerable only).
- **The form must be typed.** The pick's display is not read as the formulation, so `amoxicillin 500mg tds` picked
  without a form word never gets the default tick: most lines are hand-ticked today
  ([#1081](https://github.com/cairn-ehr/cairn-ehr/issues/1081), a decision).
- **The doors trust the window's writer** ([#1082](https://github.com/cairn-ehr/cairn-ehr/issues/1082), a decision):
  save and delete take the writer the webview sends (decision 3), and the doors check only that it is a clinical vault
  member, as the note draft's do. A `cairn_window` client could put lines in another clinician's list, or read it.
  Every Sign check still runs on them, but they would be signed under that clinician's key.
- **A refused Remove is said, not explained.** The pane reports "This line was not removed (the list answered …)" with
  the door's word, as a refused save does, and the re-read draws the line back.
- **Drafts do not cross nodes** (#963): a list typed on one node is not on another until slice 5.
- **The lone-line Sign is unused.** The webview no longer calls slice 1a's `med_chart` command (the presses append to
  the list); it stays registered, with its tests, and is reachable only by those tests.
- **A failed untick is only logged.** If clearing a changed line's hand tick fails at the save door, the stale tick
  stays stored, but it is never drawn ticked or signed while it fails to hold: the view and every Sign re-derive it
  from a fresh read.
- **The DB-gated window tests run locally only** (#905); the real window's WebKit, the real Tauri IPC, VoiceOver and
  every stopwatch figure are owed by a human (RUNBOOK §31).

## Consequences

- A twelve-line home list is twelve lines and one Sign: paper *N* = 12 lines + 1 signature → *M* = 1 human signature
  act (12 events) → *K* = 12 lines + 1 Sign, plus one more Sign only when a warning arrived meanwhile (paper never
  re-checks). Resuming after an interruption costs nothing (the list comes back on opening the chart); removing a line
  is one press, as crossing it out.
- The list is a second draft store beside the note's, with the same shape of rules (definer doors, per writer, bound
  to a terminal, "Continue here") and two differences: it never commits by itself, and removing from it is not audited.
- `SCHEMA_GENERATION` 77 → 78. A database that loaded an earlier build of `db/078` converges on replay: the paired
  `ALTER … ADD COLUMN IF NOT EXISTS`, the shape checks added `NOT VALID`, and the superseded door signatures dropped.
