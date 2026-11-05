# ADR-0091 — The compound note and the addendum

- **Status:** Accepted (builds the slice 3 design's §3.4 addenda and §3.7 compound note, read with
  [ADR-0088](0088-whos-here-and-a-drafts-parts.md) decision 14 (Sign's parts); adds one authoring-door arm to
  [ADR-0087](0087-the-first-note-stream-and-its-draft.md) decision 3's floor rule 6; settles
  [#948](https://github.com/cairn-ehr/cairn-ehr/issues/948), which ADR-0087 and ADR-0088 left open; supersedes no ADR
  decision)
- **Date:** 2026-11-05
- **Spec version at acceptance:** 0.93
- **Issues:** draft PR [#992](https://github.com/cairn-ehr/cairn-ehr/pull/992) (GUI sign-in slice 3c-1). Design:
  `docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md` (§3.4, §3.7, §5); plan:
  `docs/superpowers/plans/2026-11-05-sign-in-slice-3c-1-compound-note-and-addendum.md`. Filed by the build:
  [#993](https://github.com/cairn-ehr/cairn-ehr/issues/993) (no "Add to this note" on a linked chart's note) ·
  [#994](https://github.com/cairn-ehr/cairn-ehr/issues/994) (buttons drawn before the editor opens stay up and do
  nothing) · [#995](https://github.com/cairn-ehr/cairn-ehr/issues/995) (the sealed cases of #948 are untested) ·
  [#996](https://github.com/cairn-ehr/cairn-ehr/issues/996) (the reviews' minor residuals).
- **Builds on:** [ADR-0090](0090-the-audited-discard.md) (the discard) ·
  [ADR-0089](0089-the-quarantine-commit-and-the-asserted-grade.md) (the timer commit, `Asserted`) ·
  [ADR-0088](0088-whos-here-and-a-drafts-parts.md) (parts; Sign = one event per part, chained by `addendum-to`) ·
  [ADR-0087](0087-the-first-note-stream-and-its-draft.md) (the note, its draft, R8) ·
  [ADR-0041](0041-progress-note-narrative-format.md) decision 8 (`addendum-to`) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (logic below the UI)

## Context

On paper, a colleague adds to a note by writing below it in the same folder, and signs. The original is never
touched. The page reads as one entry, top to bottom, and the handwriting shows who wrote which lines.

Cairn already signs a two-writer draft as one event per writer's part, each part `addendum-to` the part before
(ADR-0088). Until this slice the chart showed each part as a note of its own, and nobody could add to a note once it
was signed. This slice, **3c-1**, adds both: the parts of a note read as **one compound note**, and any clinician may
press **"Add to this note"**, type and Sign (or let the timer commit it). The result is an addendum to that note,
never a new note. It needs no wire change: the `refs` slot and its `addendum-to` rel exist since ADR-0041.

## Decision

### The maintainer's rulings (2026-11-05)

1. **3c is three PRs.** **3c-1** (this ADR): the compound note, "Add to this note" and #948, with no wire change.
   **3c-2**: the vouch overlay, "Sign off this note", the `student` label, "not co-signed", the per-part "signer
   unconfirmed". **3c-3**: the pending-sign-offs menu and #918's display half.

2. **#948 stays penned.** An addendum that reaches a peer before the note it adds to is refused, penned by the
   puller and admitted once the note lands, where its body can be read (decision 14). No code changes for it; this
   slice adds a test and this record (decision 14).

3. **3c-2's vouch shape is decided; 3c-2's ADR builds it and records it in full.** A vouch is **one event per vouched
   part**, of a new type `clinical.note-attestation.asserted`. Its contributors are `{human, attested | co-signed,
   responsibility}`. Its payload names the target event, that event's content address, and the act, `addendum` or
   `sign-off`. It is offline-first at the remote door. The strict door checks the target and its content address.

### The addendum and its draft (`db/065`, `db/070`)

4. **An addendum is an ordinary note that names the ROOT of the note it adds to.** Its first non-blank part carries
   `refs: [{"rel": "addendum-to", "event": <root>}]`; each later part of the same draft chains to the part before, as
   ADR-0088 decision 14 decided. The root is the compound's first part (decision 9), which the window sends as
   `CompoundView.root_event_id`. Two clinicians adding to one note at once both name the root, so both join it.
   Pins: `note_addendum_sign.rs::signing_an_addendum_names_the_root`,
   `a_two_writer_addendum_chains_part_two_to_part_one`, `two_addenda_signed_at_once_both_join_the_note`.

5. **The draft header remembers the target, and nothing ever changes it.** `note_draft.addendum_to uuid` (NULL = a
   new note) is created in **db/065**, the table's own file, with its paired `ALTER TABLE … ADD COLUMN IF NOT EXISTS`,
   because db/070's doors read it. Only db/070's door sets it, once, when it makes the header; no door updates it.
   So a reader may read it without the draft's lock. A spend deletes the header, so the target must be read before
   the spend. Pins: `note_addendum_doors.rs::an_addendum_draft_names_its_note`, `a_later_plain_save_keeps_the_target`.

6. **One new save door owns the header and hands the text to db/066's door.**
   `cairn_note_draft_addendum_part_save(draft, patient, terminal, writer, narrative, stated_time, addendum_to)`:
   - A NULL in any argument but the writer and the stated time raises (a caller bug).
   - It takes the draft's advisory lock first, as every draft door does. A spent draft answers db/066's words:
     `discarded`, `committed` or `spent`.
   - With no header yet: the target must be a row of `clinical_note` with `patient_id` = the patient, or the door
     answers `no-such-note`. It reads the projection, not `event_log`: only a note this node shows on this chart is
     a target. Blank text (`cairn_twin_is_present`) answers `saved` and makes no header. Otherwise it inserts the
     header with the target and `recording_started` = `last_keystroke` = `clock_timestamp()`.
   - With a header whose target differs (a new note's NULL included): `elsewhere`. A new note's draft never becomes an
     addendum, nor one addendum another's.
   - Then it returns `cairn_note_draft_part_save(…)` with the same first six arguments, so the writer check, the tail
     rule and the revision row stay one rule in one place. If that door refuses and this call made the header (only
     `no-such-writer` can), the header is deleted too, so the refusal wrote nothing.

   A second door, `cairn_note_draft_addendum_to(draft)` (`STABLE`), returns the header's target: NULL for a new
   note's draft or no such draft. Both doors are `SECURITY DEFINER SET search_path = public, pg_temp`, revoked from
   `PUBLIC` and granted to `cairn_window`. `SCHEMA_GENERATION` is **70**. Pins: db/tests/070;
   `note_addendum_doors.rs::a_blank_first_save_makes_no_header`, `a_target_on_another_chart_is_no_such_note`,
   `a_target_that_is_not_a_note_is_no_such_note`, `a_new_notes_draft_refuses_a_target_later`;
   `window_role_grants_notes.rs::the_windows_addendum_doors_run_under_its_own_login`; `floor_execute_grants.rs`.

7. **Sign and the timer take the target from the header, never from the window.** The window's `addendum_to` only
   picks the save door: `Some` the addendum door, `None` db/066's. Sign reads the header's target after its own save,
   while its transaction holds the draft's lock; the timer reads it before `commit_overdue_parts`, with no lock,
   because the column never changes. The first part's body then gets `addendum_to` = that target. So Sign pressed
   inside the one-second save debounce still makes an addendum (its own save makes the header), and a restarted
   window that sends `None` for an existing addendum draft still signs an addendum. A Sign whose save would make an
   addendum header for a note this node does not show on this chart is refused `SignRefusal::NoSuchNote` ("The note
   this adds to is not on this chart; nothing was saved."). Discard passes the target too; its `no-such-note` (no
   header, so nothing to throw away) is `DiscardRefusal::Gone`. The drafts list reads each draft's target in the
   same statement as its parts. Pins:
   `note_addendum_sign.rs::sign_takes_the_target_from_the_header_not_the_window`,
   `sign_inside_the_debounce_still_makes_an_addendum`, `the_timer_commits_an_addendum_as_an_addendum`,
   `an_addendum_target_on_another_chart_refuses_the_sign`, `the_drafts_list_carries_the_target`.

### The floor (`db/064` rule 6a, in place)

8. **At the authoring door, an `addendum-to` target must be a note of the same chart.** Inside rule 6's loop, after
   its existence check, when the floor is authoring (`cairn.remote_apply` is not `on`): an `addendum-to` target must
   be an `event_log` row of type `clinical.note.asserted` whose `patient_id` is the event's own. Otherwise it refuses
   "note assertion: an addendum-to target must be a note on this chart, got …". So a raw-SQL client that skips the
   door still cannot sign an addendum to another patient's note or to a medication event. `correction-of` and
   `transcript-of` are not tightened (nothing writes them yet). **The remote door keeps the existence check alone**,
   on purpose: its bare `RAISE` (`P0001`) is what the puller pens (decision 14), and a peer's event is carried, not
   re-judged. Pins: `note_addendum_doors.rs::the_floor_refuses_an_addendum_to_another_charts_note`,
   `the_floor_refuses_an_addendum_to_a_medication_event`, `the_remote_door_still_admits_a_held_target_on_another_chart`;
   db/tests/064 and db/tests/070.

### The compound (`cairn_node::note::compound`, pure)

9. **Grouping is a pure function in `cairn-node`, so any UI can use it.** `compounds(notes)` follows each note's
   `addendum_to` through the notes in the read. A note with no target is a root. A note whose target is not in the
   read is a root marked **orphan**: its target is withheld on a doubted linked chart, or held but not shown here (a
   newer profile R8 keeps unprojected, or a sealed body this node cannot read). The walk is bounded by the read's
   length; a row still unresolved after the bound (a cycle: no honest client makes one, but a bound costs nothing)
   becomes its own orphan root, so every row lands in exactly one compound. Parts: the root first, then the rest by
   `(recording_started, event_id)`. Compounds: newest root `t_effective` first, ties by root id. Pins:
   `compound.rs`'s tests (`a_multi_part_sign_is_one_compound_in_writing_order`, `two_addenda_to_one_root_join_it_in_recording_order`,
   `an_addendum_whose_note_is_not_read_is_an_orphan_root`, `compounds_are_newest_root_first`, `a_cycle_is_bounded`).

10. **The compound's state, in exactly these words.** A one-part note has none (its byline says it). Otherwise:
    every part signed → "every part signed"; any part `WentInUnsigned` or `WriterNotKnown` → "includes an unsigned
    part"; else, with a `Claimed` part → "includes a part whose signature this node cannot verify". The match over
    grades has no `_` arm, so a new grade breaks the build there. Pin: `compound.rs::the_state_words_are_exact`.

### The window (`notes_compound.rs`, `notes_parts.rs`, `src-ui/notes_compound.js`, `notes.js`)

11. **A compound reads as one note, headed by its root.** A one-part note is drawn as before 3c-1: its text
    (`CompoundView.text`), then its byline and time. A note of two or more parts is drawn heading first: the root's
    byline and time, then its state line. Drawn after the text, the root writer's "signed by …" would sit under the
    last writer's words and read as signing them. Then comes the text, a part at a time in part order
    (`CompoundView.segments`). Each later part is preceded by a **seam** line, "Added: " + that part's byline + " · "
    + its time, e.g. "Added: written by ?Nurse A · went in unsigned · 03:30 (stated)". Its spoken form hears the `?`
    as "unverified" (#973). So who wrote each part, and when it applies, are read at the join, not only behind the
    disclosure. "Added" says only that the part was added to the note, never that it applies later: a stated time
    may precede the root's. The root has no seam; the heading is its. Last comes a closed `<details>` whose summary
    is "{n} parts", holding each part with its own byline. An orphan's shown and spoken bylines are led by "Adds to a
    note not shown here · ". A compound keeps its place in the list by its ROOT's time (decision 9), so adding to a
    note never moves it; the seam carries the later part's time. Every word of the compound is Rust's and is set as
    text; only the button's label "Add to this note" and its accessible-name prefix (decision 12) are the script's.
    Pins: `notes_compound_tests.rs::a_one_part_note_has_no_parts_label_or_state`,
    `a_three_part_note_reads_as_one_text`, `each_later_part_is_seamed_with_its_writer_and_time`,
    `an_orphan_says_it_adds_to_a_note_not_shown`;
    `notes_compound_webview_tests.rs::a_compound_is_one_text_with_its_parts_behind_a_disclosure`,
    `a_compound_is_headed_by_its_root_and_seamed_at_each_part`.

12. **"Add to this note" is offered only on a note of the chart on screen, while the editor is free.**
    `CompoundView.can_add` is true when the root's chart is the opened chart. A linked chart's note has no button:
    the draft would live on a chart this pane does not list ([#993](https://github.com/cairn-ehr/cairn-ehr/issues/993)).
    The button's accessible name is "Add to this note: " + the note's time, and it sits behind the same who's-here
    gate as New note (`claimAsked()`, else `askWhosHere`). The pane draws the notes AFTER its loop that reopens a
    draft, so a reopened draft holding the editor never leaves a dead button. Pins:
    `notes_compound_tests.rs::a_linked_charts_note_cannot_be_added_to`;
    `notes_compound_webview_tests.rs::add_is_offered_only_on_this_charts_note_while_the_editor_is_free`,
    `add_asks_whos_here_first_and_names_the_root`, `the_notes_are_drawn_after_a_draft_reopens`.

13. **An addendum draft says what it adds to, on its own line.** Above the editor's text box and above the draft's
    line in the drafts list, the window shows Rust's words: "Adding to the note of {time}" when the root's time was
    stated, "Adding to the note begun {when its writing began}" otherwise (an unstated time would read "no stated
    time · writing began …" and run into the draft's own words), and "Adding to a note not shown here" when the pane
    shows no part with the target's id. The line is `CompoundView.adds_line`, which Add opens the editor with, and
    `DraftView.adding_to`, which the draft reads back with, so the two are always the same words. It is never part of
    `DraftView.words`. The window sends the draft's own target back with every save, Sign and Discard; text kept as a
    new draft after a fork keeps the target, because text typed as an addition must never become a new note. Pins:
    `notes_compound_tests.rs::an_addendum_draft_says_what_it_adds_to`;
    `notes_compound_webview_tests.rs::every_save_sign_and_discard_carries_the_target`,
    `a_reopened_draft_keeps_its_target_and_its_line`, `the_adding_to_line_shows_only_for_an_addendum`.

### #948, as built

14. **An addendum ahead of its note is penned where its body is readable, and admitted once the note lands.** At the
    remote door, rule 6's existence check refuses it with a bare `RAISE` (`P0001`, "… is not held on this node (a
    dangling reference)"). `cairn-sync` reads `P0001` as a deliberate refusal: it pens the bytes and re-offers them
    every sweep until they apply. Meanwhile the receiving node shows nothing of the addendum. The cost is one pen row
    on that node, and a loud (failing) pull cycle for each sweep that re-offers it before the note lands: the
    refusal counts as `refused_verifiable`, so `cycle_is_loud` (`crates/cairn-sync/src/main.rs`) turns the cycle into
    a `PullIntegrityError`, not a clean exit. Pins: `note_addendum_ahead_of_target.rs` (the door half,
    unsealed bodies: P0001 first, the same bytes admitted once the note is applied, both projected);
    `cairn-sync`'s `clinical_pull.rs::refused_apply_pens_the_bytes_and_recovers_without_loss` (the puller half, with
    a different P0001, an un-enrolled signer).

    **Where the body cannot be read, nothing is penned.** A sealed addendum that arrives WITHOUT its DEK (a node that
    is not a custody holder, or a byte-lazy pull) is admitted uninterpreted by db/020 step 7: the floor cannot see its
    `refs`, so rule 6 never runs. It is not projected until its body can be read. When a key does land, both doors
    that can land one run the floor on the clear body first (db/020 step 8; db/005's `submit_event`), so a key
    arriving while the note is still missing meets rule 6 again. The sealed cases, with and without the DEK, are
    untested ([#995](https://github.com/cairn-ehr/cairn-ehr/issues/995)).

## What is not guaranteed

- **(a) No "Add to this note" on a linked chart's note** ([#993](https://github.com/cairn-ehr/cairn-ehr/issues/993)).
  The clinician opens that chart first: one step more than paper (consequences).
- **(b) Buttons drawn before the editor opens stay up and do nothing** until the next pane read
  ([#994](https://github.com/cairn-ehr/cairn-ehr/issues/994)). New note, Add or Continue here opening the editor
  leaves "Add to this note" and "Continue here" on screen; pressing one does nothing. "Continue here" already did
  this before 3c-1.
- **(c) #948's sealed cases are untested** ([#995](https://github.com/cairn-ehr/cairn-ehr/issues/995)); decision 14
  says what the SQL does.
- **(d) An honest under-claim until 3c-2.** Signing an addendum vouches for that addendum only. A note whose earlier
  part went in unsigned still reads "includes an unsigned part" after its writer comes back and adds to it; nothing
  over-claims.
- **(e) The door and the floor read different tables.** The door asks the projection (`clinical_note`), the floor
  asks `event_log`. So a note this node holds but does not show (a `/2` note, a sealed note without its key) is
  never a target from the window, though a raw-SQL client may name one.
- **(f) An orphan offers Add on its own chart, naming the orphan part.** Its root is the part whose target is not
  shown, so a new addendum names that part. The floor admits it (a note of this chart), and when the missing note
  is shown later all of them join one compound. The orphan's "Adding to …" line names the orphan part's time, not the
  missing note's.
- **(g) A peer's addendum to another chart's note is admitted** at the remote door, where only existence is checked
  (decision 8). It shows inside its root's compound when both charts are in the read, and as an orphan otherwise.
- **(h) Minor residuals of the reviews** are [#996](https://github.com/cairn-ehr/cairn-ehr/issues/996): among them, a
  mismatched target on an existing draft answers `elsewhere`, which the window words as "another chart" (the window
  always sends the draft's own target, so only a raw or faulty client meets it).

## Consequences

- **Easier.** A note and everything added to it read as one note, and anyone may add to a note on the chart. Steps
  (paper *N* → built *M* → target *K*): adding to a note and signing it 3 → 3 → 3 (Add on the note, type, Sign +
  the strip); reading a note with additions 0 → 0 → 0 (the compound and its state line show at once); seeing who
  wrote which part, and when it applies, 0 (the handwriting, the dated line) → 0 (the read carries every part's
  writer and time) → 0 (each later part's seam line names them, in the one continuous text of design §3.7; "{n}
  parts" drills down to each part's own byline); the state line keeps the safety-relevant fact, an unsigned part,
  at 0 presses too. Adding to a
  linked chart's note 3 → 3 → **4** (open that chart first): a UI limit, not `M > N`
  ([#993](https://github.com/cairn-ehr/cairn-ehr/issues/993)). Time budget, owed by a human in RUNBOOK §22: add and
  sign ≤ 3 s (Sign's, RUNBOOK §16; the `note-sign` timing kind); open a compound's parts ≤ 1 s.
- **Harder.** Every future draft door must leave `note_draft.addendum_to` alone once set. Any future reader of a
  chart's notes that shows them grouped must use `compounds`, not its own walk. A future change to rule 6 must not
  tighten the remote door: that would turn #948's pen into a permanent refusal for a peer whose floor is older.
- **The bet.** That one continuous text with a state line, a seam line at each later part and a closed "{n} parts"
  is enough for the next clinician to see that a note was added to, by whom and when, without opening the parts.
  We would know it is wrong if clinicians miss who wrote a line that mattered, or open the parts on every note.
- **Pinned by** `db/tests/064_clinical_note_test.sql`, `db/tests/070_note_addendum_test.sql`,
  `crates/cairn-node/src/note/compound.rs`'s tests,
  `crates/cairn-node/tests/{note_addendum_doors,note_addendum_sign,note_addendum_ahead_of_target,window_role_grants_notes,floor_execute_grants}.rs`
  and `cairn-gui/cairn-gui-tauri/src/{notes_compound,notes_compound_webview,notes_parts,notes}_tests.rs`.
  RUNBOOK §22 carries the human measurements.
