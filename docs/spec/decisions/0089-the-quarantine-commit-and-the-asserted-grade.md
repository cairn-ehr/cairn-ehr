# ADR-0089 — The quarantine commit and the `Asserted` grade

- **Status:** Accepted (no decision of [ADR-0087](0087-the-first-note-stream-and-its-draft.md) or
  [ADR-0088](0088-whos-here-and-a-drafts-parts.md) is superseded; it rules on the `M > N` question ADR-0088's
  Consequences left to the maintainer, [#974](https://github.com/cairn-ehr/cairn-ehr/issues/974), decision 13)
- **Date:** 2026-11-04
- **Spec version at acceptance:** 0.91
- **Issues:** draft PR [#981](https://github.com/cairn-ehr/cairn-ehr/pull/981) (GUI sign-in slice 3b-2a). Design:
  `docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md` (§3.3, §3.7, §4.1, §7);
  plan: `docs/superpowers/plans/2026-11-04-sign-in-slice-3b-2a-quarantine-commit-and-asserted.md` (its four
  "Decisions taken with the maintainer on 2026-11-04" are cited below as *maintainer decision 1* to *4*, never to be
  confused with this ADR's own numbered decisions). Filed at planning:
  [#979](https://github.com/cairn-ehr/cairn-ehr/issues/979) (the terminal of an unattributed commit, deferred) and
  [#980](https://github.com/cairn-ehr/cairn-ehr/issues/980) (a co-signed discard, not built). Filed by the build:
  [#982](https://github.com/cairn-ehr/cairn-ehr/issues/982) (a stuck draft not yet due promises a time). Bounds the
  life of [#968](https://github.com/cairn-ehr/cairn-ehr/issues/968)'s revision rows (commented; it stays open). Rules
  on [#974](https://github.com/cairn-ehr/cairn-ehr/issues/974) (the maintainer closed it).
- **Builds on:** [ADR-0088](0088-whos-here-and-a-drafts-parts.md) (a draft's parts, one event per part) ·
  [ADR-0087](0087-the-first-note-stream-and-its-draft.md) (the note and its draft) ·
  [ADR-0083](0083-the-human-key-vault.md) (the vault) · [ADR-0053](0053-per-write-human-authorship.md) (the human
  signs, the node seals) · [ADR-0007](0007-authorship-and-accountability.md) (authorship is compositional;
  accountability is separable) · [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) (principle 4)

## Context

On paper an entry left unsigned on the progress sheet is simply in the record. It stays in the folder, anyone who
opens it can read it, and it is plainly not signed. Nobody acts to put it there. A Cairn draft (ADR-0087, ADR-0088)
is not in the record until someone signs it, so a draft its writers walked away from would be invisible to the next
clinician and would never reach a peer.

The slice 3 design (§3.3) answers this with a quarantine: a draft nobody has typed into for the site's quarantine
time goes into the record by itself, **unsigned**. The maintainer decided on 2026-10-31 that such a note is
*recorded, not attested*: the writer's key is not available, so it cannot carry their signature, and a later vouch
is an overlay. ADR-0088 split slice 3b in two; 3b-2 was split again on 2026-11-04 (maintainer decision 1):
**3b-2a**, recorded here, builds the clock, the site's knob, the commit door, the runner and the `Asserted` grade.
**3b-2b** builds the audited discard, a new event type, under its own plan.

## Decision

### The quarantine (`db/067_note_draft_commit.sql`)

1. **The clock runs from the draft's last real change, on the database's own clock** (design §3.3). The input is
   the draft header's `last_keystroke`, which db/066's save door stamps from the database clock on every real
   change and which an idle re-save leaves alone (ADR-0088 decision 8). The window never sends a time. A draft is
   overdue when `last_keystroke + quarantine <= clock_timestamp()`, in the database that holds it. A step in that
   clock loses nothing: forward only commits early (still honestly unsigned), backward only delays. The quarantine
   is operational only and never touches the stated time, `t_effective` or `t_recorded`. *Continue here* (the take
   door) moves a draft by changing `note_draft.terminal_id` alone, so it does **not** restart the clock: a take
   changes no text (controller ruling, 3b-2a Task 1). Pins: db/tests/067's `not_overdue_commits_nothing` (179
   minutes), `overdue_returns_parts_and_spends_by_timer` (181 minutes).

2. **The quarantine is site policy: one row, set only at the owner's console, outside every backup.**
   `note_policy` holds `quarantine_minutes`, 1 to 43 200 (30 days), with no row or NULL meaning **180**. The table is
   revoked from PUBLIC, `cairn_agent` and `cairn_node`, as db/061's `vault_policy` is. `cairn_note_set_quarantine`
   raises outside the range and has no grant to any runtime role; the verb
   `cairn-node note-set-quarantine (--minutes N | --default)` refuses a database on another machine
   (`conn_posture::require_local`) and prints the value now in force, read back from
   `cairn_note_quarantine_minutes()`. The policy is custody state, not the record: Cairn's backup medium carries
   `node_event` and `event_log`, and the local-state export reads neither `note_policy` nor any draft table (ADR-0083
   decision 2's posture), so a restore comes back with the default. A shorter policy applies to drafts already
   waiting: nothing is grandfathered. Pins: db/tests/067's `default_quarantine_is_180`, `set_quarantine_bounds`,
   `a_shorter_policy_applies_to_waiting_drafts`, `the_console_door_has_no_window_grant`;
   `cli_note_quarantine.rs::set_quarantine_refuses_a_remote_database`, `set_quarantine_parses_strictly`.

3. **"Overdue" is decided in one door, under the draft's lock, in the same call that consumes it.**
   `cairn_note_draft_commit_overdue(draft, event)` takes the draft's advisory lock (`'note_draft:' || draft_id`),
   then the header row `FOR UPDATE`, which is every draft door's lock order (ADR-0088's "Harder"), and only then
   compares the header's `last_keystroke` against the policy. An absent, already spent or not yet overdue draft
   returns no rows and writes nothing. Otherwise it returns the non-blank parts in writing order, with the same
   columns as db/066's `cairn_note_draft_spend_parts`, and deletes the draft's revisions and its header (the parts
   cascade). `cairn_note_drafts_overdue` lists candidates oldest first and takes no lock: it is a hint, never the
   decision. So two windows ticking at once, or a Sign racing the timer, put exactly one set of notes in the log, and
   a raw-SQL `cairn_window` client cannot commit a draft early. A NULL argument raises. Pins: db/tests/067's
   `a_spent_draft_commits_nothing`, `overdue_list_filters_by_chart`, the NULL case;
   `note_commit.rs::two_commits_race_one_note` (two connections), `a_sign_after_the_timer_is_refused_committed`.

4. **A timer's spend is marked, and a later save or take says `committed`.** `note_draft_spent` gains
   `by_timer boolean NOT NULL DEFAULT false`, created in **db/065**, the file that creates the table, with the paired
   `ALTER TABLE … ADD COLUMN IF NOT EXISTS` (#207), because db/066's save and take doors read it and a door body may
   read only a column created in its own file or an earlier one. The commit door records its spend with
   `by_timer = true`; db/065's commit-time trigger is unchanged and still insists that the spend names a
   `clinical.note.asserted` of this chart inserted in this same transaction. The save and take doors keep their
   signatures and return shape (ADR-0088 decision 13) and gain one answer word: `committed` for a timer's spend,
   `spent` for a Sign's. A Sign that finds the draft committed is refused `SignRefusal::Committed`. Pins: db/tests/067's
   `save_after_a_timer_commit_answers_committed`, `take_after_a_timer_commit_answers_committed`,
   `save_after_a_sign_still_answers_spent`, `a_timer_commit_still_needs_its_note`.

5. **An overdue draft whose every part is blank is deleted, never committed** (design §3.5). The door deletes it with
   its revisions and records no spend, and the caller commits that deletion. "Blank" is `cairn_twin_is_present`, the
   floor's one test (ADR-0088 decision 10). With no spend row, a later save of the same draft id starts a fresh draft
   rather than answering `committed`: nothing went into the record. Pins: db/tests/067's
   `an_all_blank_overdue_draft_is_deleted_without_a_spend` (NBSP and U+3000) and its source pin;
   `note_commit.rs::an_all_blank_overdue_draft_leaves_nothing`.

### The commit (`cairn_node::note::commit`, `cairn_event::contributor`)

6. **One node-signed event per non-blank part, in one transaction, chained as a Sign chains them.** `commit_overdue`
   calls the door with a fresh event id for the first part, then builds, seals and submits each part exactly as
   `sign_note` does (ADR-0088 decision 14): the first non-blank part is the note, each later one `addendum-to` the
   one before, each takes its own HLC tick inside the transaction, and the submits run with no savepoint. Either all
   of it commits or none of it does. The signer is the **node** key (`prepare_sealed_as(…, node_sk)`): the
   signature proves only "this node recorded these words". Pins:
   `note_commit.rs::an_overdue_two_writer_draft_goes_in_as_two_node_signed_events`,
   `window_role_grants_notes.rs::the_windows_quarantine_commit_runs_under_its_own_login`.

7. **The contributors say who wrote it and that nobody answers for it.** A named writer's part is
   `[{writer's current key, "drafted"}, {node, "recorded"}]` (`with_drafting_writer`); a part by a writer not known is
   `[{node, "recorded"}]` alone, the device default. There is never an `attested` or `authored` entry and never a
   human signer: such a note is never attested (maintainer, 2026-10-31). The writer's key is
   `cairn_vault_member_key(writer)`, decided by key as a Sign decides it. **No wire change:** no new event type, no
   new body key; `drafted` is already a ratified contributory role. Pins: `contributor.rs::with_drafting_writer_shapes`;
   `note_commit.rs::an_unknown_writers_part_is_the_node_alone`.

8. **A named writer with no vault key makes the draft STUCK, never "writer not known"** (principle 4, as ADR-0088
   decision 15's `NoSuchWriter`). When `cairn_vault_member_key(writer)` is NULL (the writer's member row is gone),
   `commit_overdue` drops its transaction, so the door's deletes roll back and the draft stays exactly as it was,
   and it answers `CommitOutcome::Stuck`. One stuck draft never holds the others back: each draft is its own
   transaction. A Sign of the same draft is refused `NoSuchWriter`, so only enrolling that writer again moves it:
   then the next check commits it, or a Sign can. Pins: `commit.rs::a_named_writer_without_a_key_is_stuck`;
   `note_commit.rs::a_writer_with_no_key_is_stuck_and_kept`, `one_stuck_draft_does_not_stop_the_others`.

### The grade (`cairn_event::contributor`, `cairn_node::note::{read, view}`)

9. **`Asserted` is the grade of "no responsibility-bearing contributor, but a named human contributory one"**
   (design §4.1). `classify_authorship_with_humans(contributors, signer, attester, human_kids)` answers as
   `classify_authorship_confidence` does, except that where that would be `Device`, a contributory entry whose role
   is not `recorded` (the capture device's own, ADR-0051) and not an unknown-partition role, and whose `actor_id` is
   in `human_kids`, makes it `Asserted`. A bearing contributor always wins. "Human" is passed in as resolved key ids,
   so the classifier stays pure; `classify_authorship_confidence` passes none and never answers `Asserted`. The enum
   stays exhaustive (#412), so the new variant broke every consumer's build until each decided how it reads. SQL's
   `cairn_claim_authority` answers `'unverified'` for this shape, the same as for `Unverified` and `Device`. Pins:
   `contributor.rs::a_drafted_human_with_no_bearer_is_asserted`, `a_drafted_non_human_stays_device`,
   `recorded_is_never_the_human_role`, `an_unknown_partition_role_is_never_the_human_role`,
   `a_bearer_wins_over_asserted`, `the_old_classifier_never_says_asserted`;
   `authority_lockstep.rs::asserted_in_rust_is_unverified_in_sql`.

10. **The note read takes "human" to mean "a key this node can name"; the byline says "went in unsigned".** The
    note list reads the `drafted` contributor's key and passes it as human only when `cairn_note_signer_name` names
    it (db/065, through db/061's `cairn_vault_key_holder`: a key the actor registry enrolled or rotated to, named by
    its vault member's display name or staff id, else by the handle the registry pinned). `Asserted`
    reads `NoteGrade::WentInUnsigned`, "went in unsigned", never "signed": the byline is
    **"written by ?Nurse A · went in unsigned"**, spoken "written by unverified Nurse A · went in unsigned"
    (ADR-0088 decision 16's marks). A part with no writer reads "writer not known", as any node-recorded note does.
    Pins: `view.rs::every_authorship_grade_has_its_words`, `every_grade_and_writer_reads_true`,
    `a_went_in_unsigned_byline_marks_the_name_in_speech`; `note_commit.rs::the_read_grades_a_committed_part_asserted`.

### The runner (`cairn-gui-tauri/src/notes_commit.rs`, `notes.rs`, `src-ui/notes_save.js`)

11. **The window is the runner, in two places** (design §7). A **tick**: `run_ticks`, spawned once in `main`'s setup
    on Tauri's runtime, waits `COMMIT_TICK` (60 s) and then commits every chart's overdue drafts, forever; it waits
    first, so a launch is never slowed. It is a **Rust task, not a webview timer**, because WebKit throttles the
    timers of a hidden window and a minimised window must still commit. And **before every chart read**:
    `note_list_impl` commits that chart's overdue drafts before it reads, so the pane never shows a draft the next
    tick is about to take away. Neither does anything in fixture mode or without the node key. Each draft is its own
    connection hold, after one short hold that lists them, so a clinician's press waits behind one draft at most. The
    tick never reopens a lost connection (`lock_if_open`; the next press does); the pre-read commit may. A failed
    commit is never a failed read: its error goes to stderr with the draft id and cause only, never text or a
    patient's name, and the draft stays listed. The tick says a stuck draft on stderr once per window run. Pins:
    `notes_commit_tests.rs::the_tick_is_spawned_once_in_setup`,
    `each_draft_is_its_own_hold_and_the_tick_never_reopens`, `fixture_mode_commits_nothing`,
    `a_failed_commit_never_blocks_the_read`, `opening_a_chart_commits_its_overdue_draft_first`,
    `a_stuck_draft_is_said_once`.

12. **An open draft says when it goes in, from the same row the door reads.** A fifth window door,
    `cairn_note_drafts_due(patient)` (db/067), answers each open draft's `due` and `overdue` from the HEADER's
    `last_keystroke` plus the policy, the commit door's own input, and `stuck` by the commit's own rule (a non-blank
    part whose writer has no vault key). Reading the parts' times instead could disagree with the door after a
    backward clock step, and a displayed time is a claim. The draft's words end with one of three sentences:
    "goes into the record unsigned at HH:MM if nobody types in it"; once overdue, "overdue — not in the record yet;
    the window tries again at every check to put it in unsigned"; and for a stuck draft, "overdue, but cannot go into
    the record by itself — a writer of it is no longer enrolled on this node; it goes in once they are enrolled
    again". A save that answers `committed` keeps what was typed since the last save as a **new draft**, with its own
    three sentences ("This note went into the record unsigned after it was left untouched. …"), **even when this
    window's Sign set `mayBeSigned`**: a timer's commit is never this window's Sign (ADR-0088 decision 12's fork,
    with one more answer). Pins: `notes_commit_tests.rs::due_words_before_and_after`, `a_stuck_draft_reads_stuck`;
    `note_commit.rs::an_open_draft_reads_due_and_overdue`, `due_follows_the_header_not_the_parts`;
    `notes_tests.rs::committed_sentence_exact`; `notes_fork_webview_tests.rs::a_timer_commit_forks_even_after_a_sign_press`.

### Decisions taken with the maintainer

13. **#974 is not an `M > N` defect** (maintainer decision 4). *Continue here* with nobody named costs 2 acts (the
    press and the who's-here pick) where the plan counted 1, and getting a draft back after a window restart costs
    about 2 where slice 3a's target was 0. Neither is worse than paper, whose counterpart is walking back to where
    the sheet was left, or having it fetched: the same ruling as ADR-0088 decision 5's bookmark. The maintainer
    has since marked #974 done on GitHub.

14. **The terminal of an unattributed commit is not recorded** (maintainer decision 3; design §10 finding 3,
    ADR-0088 decision 18). `clinical.note/1` has no slot for it, and a `/2` profile would hide the note from older
    nodes, which carry a newer profile but never project it (ADR-0087 decision 6). Deferred:
    [#979](https://github.com/cairn-ehr/cairn-ehr/issues/979).

15. **3b-2b's discard rule, recorded here so it is not lost** (maintainer decision 2). A draft may be discarded only
    when every non-blank part is the discarder's own (by key) or by a writer not known (ADR-0088 maintainer decision
    6: any signer may discard an unknown writer's draft). A draft with two or more named writers is refused and goes
    in by the timer. A co-signed discard is [#980](https://github.com/cairn-ehr/cairn-ehr/issues/980), not built.

## What is not guaranteed

- **With no window running, nothing commits.** A draft waits, safe, and is shown overdue to the next reader, whose
  chart open commits it first. Until it commits it is only on this node: drafts never sync, so no peer sees the note
  (design §3.3, last bullet).
- **A pane already open on a chart keeps showing a draft the tick has committed** until that chart is read again;
  nothing pushes to the screen. Typing into it is safe: the next save answers `committed` and forks what was typed
  since the last save into a new draft, with the committed sentence (decision 12).
- **The commit door cannot prove that the notes a `cairn_window` client submits carry the draft's text.** db/065's
  trigger checks that the spend names a note of this chart inserted in this transaction, not what it says. This
  grants nothing new: a raw-SQL client holding the node key could already submit any node-signed note.
- **A stuck draft not yet due still says "goes into the record unsigned at HH:MM"**, which is false for it; the stuck
  sentence appears only once it is overdue ([#982](https://github.com/cairn-ehr/cairn-ehr/issues/982)).
- **A note that went in unsigned reads `Asserted` only where its writer's key can be named.** A node whose vault and
  registry cannot name the `drafted` key reads "written by someone not named here · no person signed for it" (the
  `Device` grade): true, and weaker. Being nameable is the read's whole test of "human": the classifier trusts the
  key ids it is given.
- **No vouch upgrades `Asserted` yet.** Finishing or vouching for a note that went in unsigned is 3c's addendum.
- **Revision rows now live at most the quarantine plus one tick** after a draft's last real change, while a window
  runs ([#968](https://github.com/cairn-ehr/cairn-ehr/issues/968), commented, still open): each real save still
  writes a full-text copy, a draft typed into for hours keeps every one, and a stuck draft or a node with no window
  keeps them indefinitely.
- **Which terminal an unattributed draft came from is not recorded** (decision 14, #979).
- **The tick's own cost and its stderr lines are not measured or surfaced in the window.** A stuck draft is said on
  stderr once per window run, and a failed commit on every pass, where only an operator reading the log sees it.

## Consequences

- **Easier.** An abandoned note is in the record, unsigned and saying so, as paper's unsigned entry is, with no act
  asked of anyone and nothing new on the wire. Steps (paper *N* → built *M* → target *K*): walking away from a draft
  0 → 0 → 0; reading a note that went in unsigned 0 → 0 → 0 ("written by ?Nurse A · went in unsigned", in words and
  speech, never a glyph or a colour alone); setting the quarantine is not a clinical act (the owner, at the console,
  once); finishing one later is 3c's addendum (design §6, 3 → 3 → 3). #974's two cases are 1 → 2 and 0 → about 2,
  ruled not defects (decision 13). **A delay worth naming, not an `M > N` case:** paper's unsigned entry is in the
  folder at once, Cairn's only after the quarantine, and a peer sees it only then. Time budgets, owed by a human in
  RUNBOOK §20: an overdue draft goes in within one tick (≤ 60 s past due) while any window runs, and a chart open that
  first commits an overdue draft stays within the single-chart open budget + 200 ms.
- **Harder.** Every future draft door must keep the lock order (advisory lock, header, parts) and must treat
  `by_timer` as a second kind of spend. A new window that reads drafts must take its due time from
  `cairn_note_drafts_due`, never from the parts. A new consumer of `AuthorshipConfidence` must decide how `Asserted`
  reads.
- **The bet.** That a node-signed note naming a `drafted` writer, read "went in unsigned", is honest enough to put in
  the record without anyone's signature. We would know it is wrong if readers take "went in unsigned" notes as
  vouched for, or if sites set the quarantine so short that drafts go in mid-thought.
- **Pinned by** `db/tests/067_note_draft_commit_test.sql`, `crates/cairn-event/src/contributor.rs`'s tests,
  `crates/cairn-node/src/note/{commit,view}.rs`'s tests,
  `crates/cairn-node/tests/{note_commit,authority_lockstep,cli_note_quarantine,window_role_grants_notes}.rs` and
  `cairn-gui-tauri/src/{notes_commit,notes,notes_fork_webview}_tests.rs`. RUNBOOK §20 carries the human
  measurements.
