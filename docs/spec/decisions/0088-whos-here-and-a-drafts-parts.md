# ADR-0088 — Who's here, and a draft's parts

- **Status:** Accepted (partly supersedes [ADR-0087](0087-the-first-note-stream-and-its-draft.md) decisions 1, 10, 11
  and 12; narrows [ADR-0041](0041-progress-note-narrative-format.md) decision 1's atomic arrival for a multi-writer
  note, see Consequences; supersedes rule I1 of the slice 3 design, §3.2)
- **Date:** 2026-11-04
- **Spec version at acceptance:** 0.90
- **Issues:** PR [#966](https://github.com/cairn-ehr/cairn-ehr/pull/966) (GUI sign-in slice 3b-1), which builds
  [#958](https://github.com/cairn-ehr/cairn-ehr/issues/958) (`moved` split from `elsewhere`; the PR closes it). Design:
  `docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`; plan:
  `docs/superpowers/plans/2026-11-04-sign-in-slice-3b-1-whos-here-and-draft-parts.md` (its "Decisions taken with the
  maintainer on 2026-11-04" are cited below as *maintainer decision 1* to *6*, never to be confused with this ADR's
  own numbered decisions). Filed at planning: [#963](https://github.com/cairn-ehr/cairn-ehr/issues/963)
  (drafts across terminal-nodes, slice 5), [#964](https://github.com/cairn-ehr/cairn-ehr/issues/964) ("unknown is
  allowed" as a deployment policy), [#965](https://github.com/cairn-ehr/cairn-ehr/issues/965) (draft visibility once
  sealed episodes reach notes). Filed by the build: [#967](https://github.com/cairn-ehr/cairn-ehr/issues/967) ·
  [#968](https://github.com/cairn-ehr/cairn-ehr/issues/968) · [#969](https://github.com/cairn-ehr/cairn-ehr/issues/969) ·
  [#970](https://github.com/cairn-ehr/cairn-ehr/issues/970) · [#971](https://github.com/cairn-ehr/cairn-ehr/issues/971) ·
  [#972](https://github.com/cairn-ehr/cairn-ehr/issues/972) · [#973](https://github.com/cairn-ehr/cairn-ehr/issues/973) (built by
  the final fix wave, decision 16); by the review: [#974](https://github.com/cairn-ehr/cairn-ehr/issues/974) (the who's-here
  pick on resuming, `M > N` to judge), [#975](https://github.com/cairn-ehr/cairn-ehr/issues/975) (three small window defects)
  and [#976](https://github.com/cairn-ehr/cairn-ehr/issues/976) (a whitespace-only first keystroke closes a moved draft's
  editor). It builds the first half of [#945](https://github.com/cairn-ehr/cairn-ehr/issues/945) (decision 12).
- **Builds on:** [ADR-0087](0087-the-first-note-stream-and-its-draft.md) (the note and its draft) ·
  [ADR-0083](0083-the-human-key-vault.md) (the vault, the held key, the terminal lock) ·
  [ADR-0053](0053-per-write-human-authorship.md) (the human signs, the node seals) ·
  [ADR-0007](0007-authorship-and-accountability.md) (authorship is compositional; accountability is separable) ·
  [ADR-0041](0041-progress-note-narrative-format.md) decision 8 (`addendum-to`) · ADR-0008 §5.11 (point-of-care
  identity)

## Context

On a ward several people write into one note before one of them signs it: the nurse's observations, then the
doctor's plan below them. On paper each hand is plainly its own, nobody can rub out a colleague's ink, and the sheet
travels with whoever carries it. Slice 3a (ADR-0087) built a note and its draft for one unknown writer at one
terminal. Its next part, 3b, was split in two on 2026-11-04 (maintainer decision 1): **3b-1**, recorded here, adds who is at the terminal,
the idle re-lock, a draft made of per-writer parts with their revisions, moving a draft to another terminal, and
signing one event per part. **3b-2** adds the commit runner after the quarantine timer, the `Asserted` grade and the
audited discard. 3c adds addenda to signed notes and "sign off this note".

ADR-0087 is immutable, so where 3b-1 changes what it says, this ADR records a partial supersession by decision
number.

## Decision

### Who's here (`cairn-gui-tauri/src/whos_here.rs`, `src-ui/whos_here.js`; `cairn_node::note::whos_here`; db/066)

1. **Who's here is a claim, never authentication** (design §3.1). Naming someone signs nothing and releases no key,
   and reading a chart never asks for a name. A name is asked only when writing begins (**New note**, **Continue
   here**) or when someone presses **Who's here?** or Ctrl+Shift+H (Cmd+Shift+H on macOS). "Don't know / just
   reading" is an allowed answer (`Claim::Unknown`); until somebody answers, the claim is `NotAsked` and owns no draft
   part. Every signature still needs the signer's own key, released by their staff ID and passphrase (or still held
   from that release on a key-hold terminal).

2. **The directory is the vault's clinical members.** `cairn_whos_here_directory()` lists every `vault_member`
   whose role passes `cairn_vault_purpose_allowed(role, false, 'clinical')` (db/061), so never a superuser or an
   admin, who confer no clinical authorship (§5.1). It returns staff id, display name and role, and no key or sealed
   blob. The window lists this terminal's recent people first (at most eight, newest first) and accepts a pick only
   for a staff id the directory holds. Pins: db/tests/066's `directory_lists_clinical_roles_only`;
   `note_draft_parts.rs::the_directory_omits_superusers`; `whos_here_tests.rs::the_directory_lists_recent_people_first`,
   `a_pick_of_someone_not_enrolled_names_nobody`.

3. **A pick and the re-lock forget the held key first** (design §3.1, review finding). On a key-hold
   terminal an act with no staff ID signs with the held key, so Dr B, switched in, could otherwise sign as Dr A.
   `whos_here_pick_impl` calls `AppState::forget_held_key` and forgets the old claim before it reads the directory,
   so a refused pick leaves nobody named. *Back* closes the picker with no pick, so the claim, and any held key, stay
   as they were. A **switch** first waits for a Sign in flight to settle, then closes the
   chart, awaiting its queued saves, and only then opens the picker: Nurse A's last words are saved under Nurse A.
   A Sign that settled while a switch waited is said in the picker in words that name no patient. Opening the picker
   empties the signer strip's passphrase. Pins: `whos_here_tests.rs::pick_forgets_the_held_key`,
   `relock_forgets_claim_and_key`; `whos_here_webview_tests.rs::switch_awaits_the_save_queue_before_picking`,
   `a_sign_settled_during_a_switch_is_said_in_the_picker`, `staff_id_prefilled_passphrase_never`.

4. **The bar says who is writing, and a claimed name read by others carries a `?` in text.** A full-width bar at the
   top reads "Writing as Nurse A", "Writer not known" or "Nobody has said who is at this terminal". It is a polite
   live region, rewritten only when its words change. On this screen the claim is the user's own statement, so the
   bar has no `?`. Where others read a claimed writer the name is `?Nurse A`, text, never a glyph or a colour alone:
   in a draft's parts with the accessible name "unverified Nurse A" (`whos_here::claimed_name`), and in a signed
   part's byline, heard "written by unverified Nurse A" (`byline_spoken`, decision 16). After every pick the signer strip's staff ID is the claim's, so only the passphrase
   is typed (design §3.6); "Don't know" leaves it empty. Pins: `whos_here_tests.rs::claimed_name_words`,
   `the_bar_says_who_is_writing`; `whos_here_webview_tests.rs::the_bar_is_aria_live`, `parts_drawn_through_cell`.

5. **The bookmark is offered, pre-focused, never auto-opened** (maintainer decision 3; design §10 finding 1). When a named
   person opens a chart from a list on screen (`open_chart_impl`), it becomes their bookmark at this terminal. Their
   next pick shows one pre-focused button, "Return to …", and puts the chart into `AppState::shown`, so opening it
   passes `open_chart`'s rule that only a chart some list on screen showed can be opened. Esc or a search dismisses
   it. The maintainer ruled that the extra press is not an `M > N` defect: paper's counterpart includes walking back
   to wherever the folder was left, or having it fetched. The bookmark is the chart only (the design's "(chart, tab)":
   the window has no tabs to remember). Pins: `whos_here_tests.rs::pick_named_then_forget_keeps_bookmarks`,
   `remember_chart_ignored_unless_named`, `an_offered_chart_becomes_openable`.

6. **The claim, the recent people and the bookmarks live in window memory only** (`whos_here::Here`, one field of
   `AppState`): no file and no browser storage. A bookmark holds a chart header, i.e. a patient's name, which must not
   sit on disk in the clear. A window restart forgets them. Pin: `parts_drawn_through_cell` refuses
   `localStorage`, `sessionStorage` and `indexedDB` in `notes.js`, `whos_here.js` and `idle.js`; `Here` performs no
   I/O.

7. **The idle re-lock** (`src-ui/idle.js`, design §3.3). After `--idle-lock-minutes` (1 to 240, default 15; separate
   from the key-hold cap) with nobody touching the terminal, the window closes the chart (its queued saves awaited),
   the window forgets the held key and the claim (the `relock` command), the signer strip is emptied and the
   terminal passphrase is asked for again. Only a person resets the clock: `keydown`, `pointerdown`, `input` or
   `wheel`, listened for in the capture phase so a keystroke in any field counts. No poll, interval or IPC answer
   does (main.js's lock-poll lesson); so the clock is first armed by the first touch. A Sign in flight is waited for
   first, and a touch meanwhile cancels the re-lock. The re-lock keeps the recent people and the bookmarks. Pins:
   `whos_here_webview_tests.rs::idle_resets_only_on_user_input`, `relock_waits_for_a_sign_in_flight`;
   `whos_here_tests.rs::idle_lock_minutes_bounds`.

### A draft's parts (`db/066_note_draft_parts.sql`, `cairn_node::note::draft`)

8. **A draft is a header plus an ordered list of parts, one writer per part.** `note_draft` keeps the header
   (chart, terminal, `recording_started`, and `last_keystroke`, the latest real change over its parts); the text
   moved to `note_draft_part` (`draft_id`, `seq`, `writer`, `narrative`, `stated_time`, `recording_started`,
   `last_keystroke`). A save rewrites the **tail** part only when the tail's writer is the saver; any other save
   **appends** part tail + 1. Nurse A, then Dr B, then Nurse A again is three parts. No door takes a part number, so
   no call can address, change or remove another writer's part: the design's "the save door refuses a save that
   changes another writer's part" holds by construction, not by a refusal. **Shortcut:** a writer's own earlier part
   is read-only too once a colleague has written below it (#967). An idle re-save of unchanged text and time writes
   nothing (ADR-0087 ruling R1), and `last_keystroke` is never stamped before its `recording_started`, as in 3a. Pins:
   db/tests/066's `first_save_makes_part_1`, `same_writer_rewrites_the_tail`, `another_writer_appends`,
   `a_b_a_is_three_parts`, `b_cannot_reach_part_1`; `note_draft_parts.rs::two_writers_read_back_as_two_parts`.

9. **The writer is an enrolled clinical member, or NULL for "not known"; it is captured when the editor opens and
   travels with every save.** The save door normalises the writer with `cairn_vault_staff_id` and answers
   `no-such-writer`, writing nothing, for anything but a member whose role may act clinically, so a superuser, an
   admin or an empty string is refused even from raw SQL. The window captures the writer once, when the editor opens
   (`noteDraft.writer` in `notes.js`), and sends it verbatim with every save and with Sign; it is never re-read from
   the current claim at save time, so a debounced save landing after a switch still names the person who typed it.
   The window decides only which part the editor opens on (`notes_parts.rs`: the tail, when the draft is held here
   and the tail is the current claim's), and the node's rule is the one that holds. The note list names the claim it
   chose that part for (`NotePane.claim`), and a reopened draft's editor writes as THAT claim, so the part it opens on
   and the writer it saves as cannot diverge; a list read for another claim than the one held now opens nothing. Pins: db/tests/066's
   `an_unknown_staff_id_answers_no_such_writer`, `a_staff_id_in_other_case_is_normalised` and its superuser and admin
   cases; `whos_here_webview_tests.rs::the_writer_is_set_once_at_editor_open`;
   `notes_tests.rs::save_passes_the_given_writer_not_the_claim` (a source-text guard: the window crate has no chart
   fixture); `notes_parts_tests.rs::only_the_current_writers_tail_is_editable`.

10. **"Blank" is the floor's one test, `cairn_twin_is_present`** (db/005), never a regex class whose answer for a
    no-break space depends on the database's collation. A new writer's blank save makes no part (a colleague clicking
    in and out writes nothing). A writer's own tail may be blanked and is kept, but a blank part is never signed. A
    new draft is created with its part 1 whatever the text. Pin: db/tests/066's
    `an_empty_save_by_a_new_writer_makes_no_part`, `spend_returns_nonempty_parts_in_order_and_clears_revisions` (NBSP
    and U+3000 read blank) and its source pin on both doors.

11. **Every real change leaves a revision row** (`note_draft_revision`: draft, part, writer, terminal, the part's
    whole text, stated time, when). A new draft's part 1, a rewritten tail and an appended part each write one; an
    idle re-save and a new writer's blank save write none. The rows cascade with their draft, and the spend deletes
    them explicitly, so no revision outlives the draft it records. **No door reads them**: today only the node's
    owner can, over SQL. They exist so that text a later save replaced stays findable while the draft is open (ADR-0087
    "what is not guaranteed", first bullet). Pin: db/tests/066's `every_real_change_is_a_revision`.

12. **"Continue here" at another terminal MOVES the draft there** (maintainer decision 2; supersedes the design's rule I1, which
    started an addendum draft, and partly supersedes ADR-0087 decision 11, "one draft, one chart, one terminal").
    `cairn_note_draft_take` is one statement on the node holding the draft, under the draft's advisory lock: it
    changes `note_draft.terminal_id` and nothing else, and answers `taken`, `spent`, `elsewhere` (another chart's) or
    `gone`. The old terminal's next save answers **`moved`**: still this chart, another terminal holds it, split from
    `elsewhere`, another chart's (#958). That window keeps what the moved draft lacks as a **new draft of its own**
    (a fresh id, the same writer and chart), so nothing typed is lost: the text typed since its last successful save
    when the editor still starts with that saved text; nothing, and the editor closes, when nothing new was typed; the
    whole editor when the saved text was itself edited. Each case says its own sentence. The new draft is a separate
    note, not a part of the moved one. **A save that answers `spent` is kept the same way** (final review I1,
    [#945](https://github.com/cairn-ehr/cairn-ehr/issues/945)): the draft was continued and signed in another window, so
    what was typed here since the last save is in no note; the window keeps it as a new draft by the same rule, with
    its own three sentences ("This note was signed in another window. …"). The save the window sends after a Sign
    refused "already signed" does the same, so the editor is never left on a signed draft. A Sign marks its draft
    (`mayBeSigned`) before it is sent, and only a refusal that recorded nothing takes the mark back: so a `spent`
    answer for a draft this window's own Sign may have signed forks nothing, and the text that Sign carried is never
    written twice. Once the editor's text has been kept, a save of the old draft still queued keeps nothing again. When the draft's node cannot be reached, the clinician can only start a new
    note. With nobody named at this terminal, *Continue here* asks who's here first, so it is two acts, not one
    ([#974](https://github.com/cairn-ehr/cairn-ehr/issues/974)). Once terminals are nodes (slice 5), how a draft reaches the facility's other terminal-nodes, partitioned
    case included, is #963. Pins: db/tests/066's `moved_vs_elsewhere`, `take_moves_and_the_old_terminal_is_moved`,
    `take_of_spent_is_spent`, `take_of_unknown_is_gone`, `take_naming_another_chart_is_elsewhere`;
    `note_draft_parts.rs::a_take_then_the_old_terminal_saves_moved`; `notes_tests.rs::moved_sentence_exact`,
    `spent_sentence_exact`; `notes_fork_webview_tests.rs::moved_forks_a_new_draft`,
    `a_draft_signed_in_another_window_forks_a_new_draft`.

13. **3a's three draft doors are removed, never reshaped** (partly supersedes ADR-0087 decision 10's door list). A
    door's return shape is final, and a `CREATE OR REPLACE` of a same-named function with a new shape would make
    db/065's replay fail on every existing database. So `cairn_note_draft_save`, `cairn_note_drafts` and
    `cairn_note_draft_spend` are deleted from db/065 in place and dropped by db/066 (`DROP FUNCTION IF EXISTS`), and
    the part-aware doors take new names: `cairn_note_draft_part_save`, `cairn_note_draft_parts`,
    `cairn_note_draft_take`, `cairn_note_draft_spend_parts`, beside `cairn_whos_here_directory`. All five keep 3a's
    posture: `SECURITY DEFINER SET search_path = public, pg_temp`, revoked from PUBLIC, granted to `cairn_window`
    only; the two new tables are revoked from PUBLIC and granted to nobody. A 3a draft is migrated once, while
    `note_draft.narrative` still exists, into part 1 by an unknown writer (3a never knew who typed), and the two 3a
    columns are dropped. `SCHEMA_GENERATION` is 66. Pins: db/tests/066's `migrated_3a_draft_is_part_1_unknown_writer`,
    `the_3a_doors_are_gone`, `no_table_grant`; `window_role_grants_notes.rs`.

### Signing (`cairn_node::note::sign`, `cairn_event::contributor`, `cairn_event::note`)

14. **One event per non-blank part, in one transaction** (design §4.3; partly supersedes ADR-0087 decision 12's one
    note per draft). Sign saves the on-screen text as the captured writer's part, then
    `cairn_note_draft_spend_parts` returns the non-blank parts in writing order and spends the draft, and each part
    becomes one sealed, signed `clinical.note.asserted`. The first non-blank part is the note; each later non-blank
    part carries `refs: [{"rel": "addendum-to", "event": <the previous non-blank part's event id>}]`, so a blank part
    (part 1 included) is skipped in the chain. The spend records the **first non-blank part's** event id (the id
    `sign_note` mints before the spend and hands to that part), so db/065's commit-time trigger is unchanged. Each part takes
    its own HLC tick **inside** the transaction, and the submits run with no savepoint (ADR-0087 ruling R9). A refused
    part rolls back every part and leaves the draft as it was. A draft whose every part is blank is refused `Empty`
    and is not spent. A draft taken to another terminal is refused `Moved`. Pins: `note_sign_parts.rs::
    a_two_writer_draft_signs_as_two_events_chained_by_addendum_to`, `a_refused_second_part_rolls_back_the_first`,
    `empty_parts_are_skipped_and_an_all_empty_draft_is_refused_empty`, `a_moved_draft_is_refused_moved_and_writes_nothing`;
    `sign.rs::a_later_part_is_an_addendum_to_the_one_before`.

15. **Each part's contributor set says whose words it is** (design §3.4; partly supersedes ADR-0087 decision 1's
    single contributor shape). A part is the signer's own when its writer's **current vault key**
    (`cairn_vault_member_key`) is the signing key, decided by key and never by comparing staff-id text, because the key
    is what the signature proves:
    - the signer's own part: `[{signer, "authored"}, {node, "recorded"}]`, exactly 3a's (`with_human_author`);
    - another writer's part: `[{writer's key, "drafted"}, {signer, "attested"}, {node, "recorded"}]`, signed by the
      signer (`with_attesting_signer`): the signer answers for words they did not write (ADR-0007);
    - a part by a writer not known: `[{signer, "attested"}, {node, "recorded"}]`. No writer is invented
      (principle 4), even when the signer typed it under "Don't know".

    All three grade `Attested` under `classify_authorship_confidence`. A named writer with no member row any more
    refuses the whole sign (`NoSuchWriter`) rather than signing as "not known". **No wire change:**
    `clinical.note/1`'s `refs` slot already existed, the floor already checks `addendum-to` against an event this
    node holds (db/064 rule 6), and the earlier part is held because it was inserted earlier in the same
    transaction. ADR-0087 decision 1's `refs` is `[]` only for the first non-blank part now. Pins: `contributor.rs::attesting_signer_shape`,
    `attesting_signer_unknown_writer`, `attested_part_grades_attested`; `sign.rs::a_parts_author_is_decided_by_key`;
    `note_sign_parts.rs::a_one_writer_draft_signs_as_the_signers_own_authored_note`, `an_unknown_writers_part_is_attested_by_the_signer`,
    `a_part_whose_writer_left_is_refused_no_such_writer`.

16. **A signed part reads with its writer, and never like the signer's own when it is not.** A chart's note list
    carries `addendum_to` (an `addendum-to` ref, the window writes one, read back as `::uuid::text`, Postgres's
    canonical form, so a peer's other spelling cannot make the whole list fail to read) and `writer`
    (`view::PartWriter`), read from the event: whether it carries a `drafted` contributor, that key's display name
    through `cairn_note_signer_name` when this node can name it, and whether the signer's own role is `attested`
    rather than `authored`. The byline (`view::byline_words`) is "written by ?Nurse A · signed by Dr B" for a named
    drafter, "written by someone not named here · signed by Dr B" when the drafter's key cannot be named, "writer not
    known · signed by Dr B" for a part the signer attested with no writer recorded, and plain "signed by Dr B" for the
    signer's own. Beside the device grade a drafted writer reads "… · no person signed for it", never "writer not
    known". Each byline has a spoken form (`byline_spoken`, "written by unverified Nurse A · …"), which a screen
    reader hears in place of the shown one ([#973](https://github.com/cairn-ehr/cairn-ehr/issues/973)). Each part is
    listed as its own note; the compound note with its "N parts" is 3c's. Pins:
    `view.rs::a_drafted_name_heads_the_byline_unverified`, `an_attested_part_never_reads_as_the_signers_own`,
    `every_grade_and_writer_reads_true`; `note_read.rs::an_unknown_writers_part_reads_writer_not_known_signed_by`;
    `note_sign_parts.rs::a_two_writer_draft_signs_as_two_events_chained_by_addendum_to`;
    `notes_parts_tests.rs::the_pane_words_each_note_and_any_withheld_ones`;
    `whos_here_webview_tests.rs::parts_drawn_through_cell`.

17. **ADR-0087 decision 9 is confirmed** (maintainer decision 4): drafts are in no backup and no export. Moving a draft between
    terminals changes one column on one node and never touches a backup.

### What 3b-1 leaves out

18. **Not built here, by the design's own table** (maintainer decision 5): the picker's pending-sign-off counts and the person
    menu (§3.6, 3c); the `Asserted` grade, with its only producer, the commit runner after the quarantine timer
    (3b-2); the audited discard (3b-2), where a draft whose writer is unknown may be discarded by **any** signer
    (maintainer decision 6, recorded here so it is not lost); the terminal of an unattributed commit (design §10 finding 3,
    3b-2). This ADR builds part of [#243](https://github.com/cairn-ehr/cairn-ehr/issues/243)'s durable drafts: a draft
    has a subject and a writer per part, never a session, but it is still held by a terminal, not by `(author,
    patient)`.

## What is not guaranteed

- **A misclaim writes under another's name.** Nurse B, who forgot to switch, types under "Writing as Nurse A"; the
  part's writer is `nurse-a`, and if Dr C signs, the event says `{Nurse A's key, drafted}` and the byline reads
  "written by ?Nurse A". That is what the terminal asserted, marked as a claim, and no stronger. The revision rows
  record the terminal and time of each save, but only while the draft is open and only for the node's owner over SQL;
  the spend deletes them. Guards: the loud bar, the re-lock clearing the claim, and Sign needing the signer's own
  key, released by their staff ID and passphrase (or still held from that release on a key-hold terminal).
- **Two "Don't know" writers at one terminal share one tail part.** The floor cannot tell two unknown people apart
  (NULL is NULL), so the second one's save rewrites the first one's tail. The window shows that tail's text for
  editing, so the second writer normally adds to it; an empty editor would replace it. The revision rows keep the
  earlier text while the draft lives.
- **A writer's own earlier part is read-only** once a colleague has written below it ([#967](https://github.com/cairn-ehr/cairn-ehr/issues/967)).
- **The revision rows cost one full-text copy of the part per real save**, about one a second while typing, for the
  life of the draft, which is unbounded until 3b-2's runner ([#968](https://github.com/cairn-ehr/cairn-ehr/issues/968)).
- **A window restart forgets the claim, the recent people and the bookmarks** (decision 6). So a draft held here is
  not reopened into the editor until somebody says who is writing: getting it back after a crash or relaunch takes
  about two acts (*Continue here* and the pick), where slice 3a needed none ([#974](https://github.com/cairn-ehr/cairn-ehr/issues/974)).
- **The re-lock is a window posture, not a node one.** It is a timer in the webview. The node does not know the
  terminal re-locked, and the window's own commands are not refused while the lock shows: a raw-SQL `cairn_window`
  client, or a webview that stops running its timers, is not re-locked. It leaves the enrol, re-key and reseal forms'
  passphrase fields as they were ([#969](https://github.com/cairn-ehr/cairn-ehr/issues/969)).
- **The terminal id stays caller-asserted** (ADR-0087), and the take door adds a way to use that: any `cairn_window`
  client can take any open draft of a chart it names, mid-typing or not. The old terminal keeps its unsaved text as
  a new draft (decision 12), so nothing typed is lost, but that text then signs as its own note. When the saved text
  had itself been edited, the whole editor is kept and the part saved before the move appears in both.
- **An `addendum-to` target that arrives after its part at the remote door is refused** (ADR-0087's
  [#948](https://github.com/cairn-ehr/cairn-ehr/issues/948)). ADR-0087 expected this to become live with 3c's addenda;
  it is live now, because a two-writer draft signs part 2 as an addendum to part 1. Part 2's HLC follows part 1's,
  but set-union sync does not promise delivery in order. The refusal happens when the receiver holds the body's key;
  a sealed arrival without custody is admitted unchecked, since its floor cannot read the ciphertext (db/020's step 8).
- **The directory lists members whose key is repudiated or retired** ([#971](https://github.com/cairn-ehr/cairn-ehr/issues/971)).
  Naming them signs nothing, and a repudiated or retired key cannot sign here.
- **Only a chart opened from a list on screen is bookmarked**: one opened at launch by `--patient` or by registering
  is not, and the offer shows the header's age rather than a date of birth ([#972](https://github.com/cairn-ehr/cairn-ehr/issues/972),
  with [#673](https://github.com/cairn-ehr/cairn-ehr/issues/673)).
- **A Sign's outcome after a switch**: an "already signed" refusal is said as "NOT signed … sign it again", a double
  hot-key press during the chart's close clears the first press's outcome line, and the idle re-lock never says a
  Sign's outcome, by ruling: the next person at the terminal must not read what the last one did
  ([#970](https://github.com/cairn-ehr/cairn-ehr/issues/970)).
- **The bookmark offer shows a colleague's last patient to anyone who picks that colleague's name.** A pick is a
  claim and needs no proof, so whoever says "Nurse A" at this terminal is offered "Return to …" with the name of the
  chart Nurse A last opened here. That discloses no more than the terminal already does: it is unlocked, and every
  chart on it is readable without a name.
- **Text typed after an unknown-outcome Sign can be lost** ([#945](https://github.com/cairn-ehr/cairn-ehr/issues/945)'s
  second half). When the save after that Sign gets no answer the editor stays live; if the Sign did sign, every later
  save answers `spent`, nothing is kept (decision 12's `mayBeSigned`), and the editor stays on the signed draft until
  the chart is reopened. It needs an interrupted commit and an unanswered save in a row.
- **A whitespace-only first keystroke after a move or a signing elsewhere closes the editor**
  ([#976](https://github.com/cairn-ehr/cairn-ehr/issues/976)). The first keystroke of a burst is saved at once; when
  it is Enter or a space, the remainder is blank, so the editor closes (decision 12's nothing-new case) and the
  letters typed next go to no field.
- **A note naming two `addendum-to` refs** (none from this window, which writes one) reads with an arbitrary one of
  them as `addendum_to`: the read takes the first row with no order.
- **"Unknown is allowed" is not a site setting yet** ([#964](https://github.com/cairn-ehr/cairn-ehr/issues/964)),
  and **a draft is visible to everyone who opens the chart**, with no gating for a sealed sensitive episode
  ([#965](https://github.com/cairn-ehr/cairn-ehr/issues/965)).

## Consequences

- **Easier.** Several people write one note, each person's words stay theirs, and the signed record says exactly
  who typed each part and who answers for it, at event granularity, with nothing new on the wire. ADR-0020's bet,
  which ADR-0041 decision 1 shares, that sub-note span-level provenance is not needed, holds: provenance is exact
  per event. Steps (paper *N* → built *M* → target *K*): reading 0 → 0 → 0; writing and signing 3 → 3 → 3 (the
  who's-here pick once per stint replaces typing the staff ID at signing, design §6); adding to a colleague's draft
  1 → 1 → 1; finishing at another terminal 1 (plus the walk) → **1** when that terminal already names the writer,
  **2** (*Continue here* and the pick) when nobody is named; resuming after the re-lock 2 (plus the walk) → 3 → 3
  (maintainer decision 3's ruling); getting a draft back after a window restart, which paper never needs, 3a's 0 →
  about **2**. Whether the two `M > N` cases are defects is the maintainer's to judge
  ([#974](https://github.com/cairn-ehr/cairn-ehr/issues/974)).
- **Narrower: a multi-writer note can arrive in parts.** ADR-0041 decision 1 made a note one signed event, so that
  "arrival is atomic (a note can never half-arrive)", and rejected a chained shape for that sync reason. 3b-1 keeps
  every event a whole note on its own, but a note several people wrote is now several events chained by
  `addendum-to`, and that compound CAN arrive in parts: a peer may hold Nurse A's part without Dr B's, or (where it
  can check) refuse Dr B's for want of Nurse A's (#948). This narrows decision 1's atomicity for a multi-writer
  note; a one-writer note is still one event and arrives whole. The trade was made for exact per-writer provenance
  and no wire change (design §4.3).
- **Harder.** A reader of a multi-writer note must follow `addendum-to` to see it whole until 3c's compound view. Any
  future door on the draft tables must keep the tail-only rule and the lock order (advisory lock, header, parts),
  and must take a new name rather than reshape an old one.
- **The bet.** That a claim marked `?` everywhere others read it, plus a loud bar and a re-lock, is enough to make
  "who typed this" useful without a passphrase per writer. We would know it is wrong if misclaimed parts turn up
  often in practice, the case the revision rows and the `?` exist for.
- **Pinned by** `db/tests/066_note_draft_parts_test.sql`, `db/tests/065_note_draft_test.sql`,
  `crates/cairn-event/src/{contributor,note}.rs`'s tests, `crates/cairn-node/src/note/{sign,view}.rs`'s tests,
  `crates/cairn-node/tests/{note_draft_parts,note_sign_parts,note_sign,note_read,window_role_grants_notes}.rs` and
  `cairn-gui-tauri/src/{whos_here,whos_here_webview,notes,notes_parts,notes_webview,notes_fork_webview}_tests.rs`. RUNBOOK §19 carries
  the human measurements.
