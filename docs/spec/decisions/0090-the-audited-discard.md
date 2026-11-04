# ADR-0090 — The audited discard

- **Status:** Accepted (builds [ADR-0089](0089-the-quarantine-commit-and-the-asserted-grade.md) decision 15's
  rule as recorded there, with [ADR-0088](0088-whos-here-and-a-drafts-parts.md) maintainer decision 6; supersedes no
  ADR decision. It narrows the slice 3 design's §3.5, which put a discard "only in the chart's audit view": decision
  9, maintainer 2026-11-04)
- **Date:** 2026-11-05
- **Spec version at acceptance:** 0.92
- **Issues:** draft PR [#984](https://github.com/cairn-ehr/cairn-ehr/pull/984) (GUI sign-in slice 3b-2b). Design:
  `docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md` (§3.5); plan:
  `docs/superpowers/plans/2026-11-04-sign-in-slice-3b-2b-audited-discard.md`. Filed by the build:
  [#985](https://github.com/cairn-ehr/cairn-ehr/issues/985) (a raw-SQL Sign can spend a colleague's draft with an
  unrelated note; pre-existing, found in review) · [#986](https://github.com/cairn-ehr/cairn-ehr/issues/986) (Sign's
  words lead a failure whose commit outcome is unknown with "Could not sign this note:") ·
  [#987](https://github.com/cairn-ehr/cairn-ehr/issues/987) (a discarder's own raw-SQL client can sign a discard whose
  text differs from the draft). Still open: [#980](https://github.com/cairn-ehr/cairn-ehr/issues/980) (a co-signed
  discard).
- **Builds on:** [ADR-0089](0089-the-quarantine-commit-and-the-asserted-grade.md) (the quarantine commit, `by_timer`)
  · [ADR-0088](0088-whos-here-and-a-drafts-parts.md) (a draft's parts; "own part" by key) ·
  [ADR-0087](0087-the-first-note-stream-and-its-draft.md) (the note, its draft, R8, R9) ·
  [ADR-0053](0053-per-write-human-authorship.md) (the human signs, the node seals) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) (born-sealed bodies) ·
  [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) (principle 4)

## Context

On paper you may tear up your own page before it goes into the folder. You may not tear a colleague's writing out
of it. A Cairn draft (ADR-0087, ADR-0088) is different from that page in one way that matters: it is autosaved, a
colleague can take it over, and a colleague may have read it and acted on it. A doctor orders levels on "took 40
paracetamol at 02:00", and then the draft is thrown away. If the discard left no trace, "it never existed" would be
a quiet falsehood (principle 4). Cairn has no audit table; the signed log is the audit trail.

So a discard is a signed event that says the draft existed and what it said (design §3.5). ADR-0089 decision 15
recorded the rule for who may do it. This ADR records the slice that builds it, **3b-2b**: a new stream, its floor
and projection (`db/068`), one new draft door (`db/069`), an arm of db/065's commit-time trigger, the one-transaction
discard in `cairn_node::note::discard`, and the window's button and marker.

## Decision

### The stream (`db/068_clinical_note_discard.sql`, `cairn_event::note_discard`)

1. **A discard is a signed clinical event, never a note.** Its type is `clinical.note-discard.asserted`, registered
   `additive` and not targeting another author. It stays **off** db/048's `cairn_event_type_has_no_thread` list, as
   notes do, so the conservative sensitivity bound applies. It never enters `clinical_note`. Pins: db/tests/068's
   `registration_and_the_type_is_not_threadless`;
   `note_discard_floor.rs::a_sealed_discard_submits_and_projects_its_parts_not_a_note`.

2. **The wire shape, frozen from the first signed discard.** `schema_version` is `clinical.note-discard/1`. The
   payload has one key, `parts`: a non-empty array, in writing order, of objects with exactly these keys:
   - `writer`: `"discarder"` or `"not-known"`. It names no key, because under the rule every named writer IS the
     discarder (decision 6), so a key here would be a second, unchecked claim. #980 can add a key-bearing value
     additively.
   - `narrative`: the part's text, escaped once by `EscapedNarrative::of_typed`, the note's own escape (drafts hold
     raw text, ADR-0087 R4).
   - `stated_time`: an RFC 3339 instant, or JSON `null` when the writer named no time.
   - `recording_started`, `recording_ended`: RFC 3339 instants, the database's stamps for the part (its first save,
     and its `last_keystroke`).

   The envelope's `t_effective` is the **discard instant**, minted by the door (decision 7). The window never sends
   a time. The node builds a device-shaped body (`[{node, recorded}]`) and `prepare_sealed` with `Some(author)`
   rewrites it to `[{discarder, authored}, {node, recorded}]`, signed by the discarder's key (ADR-0053); the node
   keeps custody of the sealed body. Pins: `note_discard.rs` (cairn-event)'s
   `the_payload_has_exactly_the_frozen_keys`, `the_narrative_is_escaped_once`;
   `note_discard.rs` (cairn-node tests)'s `discarding_own_draft_records_one_signed_event` (the signer, the parts, the
   escape, `t_effective` = the spend's `spent_at`).

3. **The twin is mechanical and names no time.** Its first line is exactly `Draft discarded; it was never entered in
   the record.` Then, for each part, a blank line, a heading `Part N, written by the discarder:` or `Part N, writer
   not known:`, and the part's text unescaped. A time in the twin would be a second, unchecked copy of the
   envelope's. The twin row requires a non-empty twin (#173). Pins: `the_twin_is_the_typed_text_under_part_headings`,
   `the_twin_names_no_time`.

4. **The floor follows db/064 rule for rule** (`cairn_check_note_discard`, revoked from PUBLIC). It refuses a
   missing payload; a schema tag other than `/1`; `parts` that is not a non-empty array; a part that is not an
   object; a `writer` outside the two words; a narrative that is not a string or is blank by `cairn_twin_is_present`;
   a narrative failing `cairn_check_note_markup`, or `cairn_check_note_anchors(text, '[]'::jsonb)` (a discard carries
   no media, so every anchor dangles); recording times that are not strings or not instants with an explicit offset
   (the `cairn_t_effective` validator); `recording_started` after `recording_ended`; a missing `stated_time` key, or
   one that is neither `null` nor an instant; an unset `t_effective`. At the remote door a newer profile is carried
   unchecked and never projected (ruling R8, as db/064), because refusing it would wedge sync on every older node.
   Pins: db/tests/068 (each refusal by its own message); `note_discard_floor.rs::the_strict_door_refuses_a_hostile_discard_with_the_floors_own_sentence`.

5. **One projection row per discard, read only for the marker.** `clinical_note_discard` holds the event id, the
   chart, `parts` as received (narratives still escaped), `t_effective`, the HLC, the origin and the
   `content_address`. Its applier reads the clear payload (ADR-0052), projects `/1` alone, inserts or does nothing,
   and is registered `heal_safe`, so the late-custody path can re-run it when a key lands. The applier is revoked
   from PUBLIC. A shred scrubs the row by `content_address` (db/037's `cairn_execute_shred`, edited in place and
   guarded by `to_regclass`), because the parts ARE clinical plaintext. Pins: db/tests/068's
   `a_well_formed_discard_is_projected`, `a_shred_scrubs_the_discard_row`;
   `note_discard_floor.rs::a_shred_scrubs_the_discard_projection`.

### The rule, the door and the floor's proof (`db/069_note_draft_discard.sql`, db/065 in place)

6. **The rule, as built: compare by KEY.** `cairn_note_draft_discard(draft, patient, terminal, event,
   discarder_key)` refuses `not-yours` when any part that counts has a writer who is neither NULL (not known) nor a
   member whose current vault key (`cairn_vault_member_key(writer)`) equals `discarder_key`: the key is what
   the signature proves (ADR-0088 decision 15), and what decision 8's trigger checks. A part counts only when
   it is non-blank by `cairn_twin_is_present`, the floor's one blank test, so a colleague who blanked their own part
   no longer holds the draft. A named writer with no key at all makes the discard `not-yours`; it is never read as
   "not known", which would quietly lose who wrote it (principle 4). The discarder must be a vault member found by
   `signing_key_id` whose role may act clinically (`cairn_vault_purpose_allowed(role, false, 'clinical')`); otherwise
   the answer is `no-such-signer`, so a superuser or an admin may not discard. So a draft that holds a colleague's
   writing is refused and goes in by the timer (ADR-0089), or stays stuck while a writer of it has no key; a co-signed discard is #980. Pins: db/tests/069's
   `own_draft_is_discarded_and_spent_with_the_key`, `unknown_writer_draft_any_clinical_signer_may_discard`,
   `a_colleagues_part_is_not_yours`, `a_named_writer_without_a_key_is_not_yours`,
   `a_blank_colleague_part_does_not_count`, `an_admin_key_is_no_such_signer` (an admin's, a superuser's and an unknown
   key); `note_discard.rs::a_colleagues_draft_is_refused_not_yours_and_kept`.

7. **The door keeps every draft door's lock order and mints the discard instant under the lock.** NULL arguments
   raise. The signer check is a plain read of `vault_member` before the lock, as the save door's writer check is.
   Then the door WAITS for the advisory lock `'note_draft:' || draft_id` (only the timer's door tries it), reads
   `clock_timestamp()` once, AFTER the lock, so a discard queued behind a save is never stamped before that save's
   last keystroke. A spent draft then answers `discarded`, `committed` or `spent`; otherwise the door takes the
   header `FOR UPDATE`, and a missing header answers `gone`, another chart's `elsewhere`, another terminal's
   `moved`; then the rule. A refusal is one row with every other column NULL and writes nothing. Success returns one `ok` row per
   non-blank part in writing order, all carrying the same `discarded_at`, then deletes the revisions and the header
   (the parts cascade) and records the spend with `discarded_by` = the discarder's key and `spent_at` = that instant.
   The door is granted to `cairn_window` alone, and its return shape is final. Pins: db/tests/069's
   `discard_after_timer_answers_committed`, `discard_after_sign_answers_spent`, `moved_and_elsewhere_and_gone`, the
   NULL case; `note_discard.rs::discard_racing_sign_one_wins` (two connections),
   `discarded_at_follows_a_save_that_held_the_lock` (a smoke test: it cannot catch a clock read before the door's lock,
   since the node's save takes that lock first).

8. **The door's yes is only a claim until db/065's trigger ties it to a signature and an instant.** The door trusts
   the key its caller names, and a key id is public. So `note_draft_spent_names_its_note`, the deferred commit-time
   trigger, gains a discard arm: a spend with `discarded_by` set must name an event that is a
   `clinical.note-discard.asserted`, of the same chart, inserted in the same transaction (`xmin`), **signed by
   `discarded_by`**, and whose **`t_effective` equals the spend's `spent_at`**. Only the private key can make such a
   row, because the strict door writes only a row whose signature verifies. And no event signed before the door's
   call can carry an instant the door minted inside it, so a raw-SQL client holding a colleague's genuine discard
   (captured from another node, not yet synced here) cannot replay it against a draft. A client that names a
   colleague's key gets the draft consumed, then the whole transaction refused at COMMIT. Hence no savepoint around
   the submit: a subtransaction's row carries a subxid. The note arm (`discarded_by` NULL) is unchanged, and a
   discard event never backs a Sign's spend. Pins: db/tests/069's `the_commit_refuses_a_spend_without_its_signed_event`,
   `the_commit_refuses_an_event_signed_by_another_key`, `the_commit_refuses_a_discard_event_of_another_instant`
   (with the other-chart, note-not-discard and subtransaction cases beside them);
   `note_discard.rs::a_raw_door_call_without_its_event_is_refused_at_commit` (a real COMMIT refusal).

9. **A discarded draft answers `discarded`, never `spent`.** db/066's save and take doors (edited in place, same
   signatures and return shapes) answer `discarded` when the spend row carries a `discarded_by`, because the window
   words `spent` as "already signed", which is false of a discard. `note_draft_spent.discarded_by` is created in
   **db/065**, the table's own file, with its paired `ALTER TABLE … ADD COLUMN IF NOT EXISTS` (#207), because
   db/066's doors read it. A Sign of a discarded draft is refused `SignRefusal::Discarded` (its save answers
   `discarded` first). Pins: db/tests/069's `save_after_discard_answers_discarded`,
   `take_after_discard_answers_discarded`; `note_discard.rs::sign_after_discard_is_refused_discarded`.

10. **An all-blank draft is deleted, with no event and no spend** (design §3.5: nothing to audit). The door deletes
    its revisions and header, records no spend and answers `empty`; the node commits that deletion and answers
    `DiscardOutcome::Empty`. With no spend row, a later save under the same draft id starts a new draft rather than
    answering `discarded`. Pins: db/tests/069's `all_blank_is_deleted_empty_no_spend` (NBSP and U+3000),
    `a_save_after_an_empty_discard_starts_a_new_draft`; `note_discard.rs::an_all_blank_draft_is_empty_and_gone`.

### The node (`cairn_node::note::discard`, `read`)

11. **One transaction, `sign_note`'s shape step for step.** `discard_draft` saves the text on screen as the on-screen
    writer's part (`save_part`), so the audit copy holds what was on screen when Discard was pressed, never an older
    autosave; a refused save writes nothing. It then calls the door with the discarder's key, builds ONE event from
    the door's rows with `t_effective` exactly the door's `discarded_at` (RFC 3339 to the microsecond, which is
    `timestamptz`'s precision), takes its HLC tick inside the transaction, seals it through `prepare_sealed`, submits
    it with no savepoint, and commits. A door word this build does not know is an error, never a default. A Recorded
    COMMIT whose answer was lost is marked by `db_diagnosis::commit_failure`; the Empty path's eventless commit is
    not, so a lost answer there reads "Not discarded:" ([#989](https://github.com/cairn-ehr/cairn-ehr/issues/989)). Pins: `discard.rs`'s
    `refusal_words_map_every_door_word`, `an_unknown_word_is_an_error`;
    `note_discard.rs::discard_carries_the_text_on_screen`.

12. **The marker reads the projection, by five named keys, and never as a note.** `list_patient_notes` returns
    `discards` (newest `discarded_at` first) beside the notes and counts `discards_withheld_on_linked`: a marker on a
    chart other than the opened one, in a set holding a doubted link, is counted, not shown, as a note is. The read
    takes from each part only `writer`, `narrative`, `stated_time`, `recording_started` and `recording_ended`, never
    the part as a whole, because db/068 stores `parts` as received and a key a raw-SQL client added beside them must
    never reach the screen. Its times go through `cairn_t_effective`, the validator the floor admitted them with. The
    discarder's name is `cairn_note_signer_name` of the signing key. `NOTES_SQL` is unchanged, so ADR-0087 R8 still
    holds: a chart's notes are read only from `clinical_note`. Pins: `note_read.rs::the_marker_lists_the_discard_not_as_a_note`,
    `a_doubted_set_withholds_a_linked_charts_marker`. The withheld line counts both: "2 note(s) on this patient's
    linked charts are not shown: … 3 discarded draft(s) on those charts are held back for the same reason." With no
    note withheld it leads with the discards ("3 discarded draft(s) on this patient's linked charts are not shown:
    …"), never "0 note(s)". Pin: `notes_parts_tests.rs::the_withheld_sentence_counts_discards`.

### The window (`cairn-gui-tauri/src/notes_discard.rs`, `src-ui/notes_discard.js`, `whos_here.js`)

13. **The chart shows each discard as a one-line marker, its text behind a click, never as a note** (maintainer,
    2026-11-04). This replaces design §3.5's "only in the chart's audit view": no audit view exists, and a trace
    nobody can see on the chart does not tell the next clinician that something was thrown away. Each marker is a
    closed `<details>` in its own list (`#note-discards`, never `#note-list`), its summary "Draft discarded by Nurse A
    · 03:40" (dated as a note's time is), opening to each part's heading ("Part 1, written by Nurse A · started 03:12" or "Part 2, writer
    not known · started 03:31") and its text exactly as typed, every word set as text, never HTML. A discarder the node cannot name
    reads "someone this node cannot name". Pins: `notes_parts_tests.rs::the_marker_summary_and_headings_are_exact`,
    `the_withheld_sentence_counts_discards`, `the_pane_lists_discard_markers`;
    `notes_discard_webview_tests.rs::a_marker_is_a_details_not_a_note`, `markers_are_text_never_html`.

14. **Discard draft sits beside Sign note, with the same strip and no confirmation dialog** (principle 3: a dialog is
    a step paper never charges, and the record itself is the safety). `note_discard_impl` follows `note_sign_impl`:
    the chart on screen, fixture mode, the verdicts before the key, the terminal, the key, the one-transaction call.
    An empty editor is allowed (the window checks no text; the node answers `Empty` only when every
    part is blank). A Discard shares Sign's in-flight flags, so both buttons
    are down and the editor read-only while it runs, and the idle re-lock and a who's-here switch wait for it. The
    press cancels a pending save and carries the text on screen, so a discard pressed during the debounce keeps the
    last second of typing. It marks the draft `mayBeSigned`, so a late save answering `discarded` after this window's
    own press forks nothing; a save answering `discarded` otherwise keeps what it had not saved as a new draft, in a
    discard's own words. A refusal gives the mark back (unless the outcome is not known) and saves the text on screen
    again, because the refused transaction rolled back its own save. The gesture is timed as `note-discard` (db/044,
    in place, in both the `CREATE TABLE` line and the drop/re-add). Pins:
    `notes_discard_webview_tests.rs::discard_cancels_the_pending_save_before_it_discards`,
    `discard_sends_the_credential_and_the_text_on_screen`, `the_discard_button_never_asks_to_confirm`,
    `the_editor_is_read_only_while_a_discard_is_in_flight`, `a_discard_marks_the_press_as_carrying_the_text`,
    `a_discarded_save_is_handled_as_spent`, `a_refused_discard_saves_the_text_on_screen_again`; db/tests/069's
    `note_discard_is_a_timing_kind`; `notes_discard_tests.rs::note_discard_impl_discards_under_the_window_login`;
    `window_role_grants_notes.rs::the_windows_discard_runs_under_its_own_login`.

15. **A discard's words never borrow Sign's.** Every refusal but `discarded` ("This draft was discarded; it is no
    longer open.") is led "Not discarded:" and says what is true of a discard: "nothing was thrown away; your text
    is still here" where it is. A colleague's draft reads "Not
    discarded: someone else wrote part of this draft, and only its writer may throw their part away — nothing was
    thrown away; your text is still here.". It promises no time, because a stuck draft never meets one, and it does
    not point at the draft's own line, because a draft held in the editor has none on screen; the chart's draft row
    carries `cairn_note_drafts_due`'s words, and showing them in the editor is
    [#988](https://github.com/cairn-ehr/cairn-ehr/issues/988). A save that finds its draft discarded in another window
    says "This draft was discarded in another window …", never "signed". A failure that is not a worded refusal is
    led by its retry class: "Whether this draft was discarded is NOT known:" when the commit may have landed, else
    "Not discarded:". A Recorded COMMIT whose answer was lost reads "Whether this draft was discarded is NOT known — no
    ordinary answer came back …" and is not a timing sample; the Empty path's eventless commit is not marked, so a
    lost answer there reads "Not discarded:" ([#989](https://github.com/cairn-ehr/cairn-ehr/issues/989)). The who's-here picker words a discard's outcome as a
    discard's ("Your draft was discarded.", "Your draft was NOT discarded — open the chart to see where it stands.",
    "Whether your draft was discarded is not known — check the chart."), never as a signature. Sign's matching flaw
    is #986. Pins: `notes_discard_tests.rs::discard_words_each_refusal`, `discard_words_each_outcome`,
    `a_failure_is_led_by_what_it_means_for_the_discard`, `a_discarded_save_reads_like_spent_and_says_discarded`,
    `take_words_discarded`; `notes_tests.rs::discarded_fork_words_never_say_signed`;
    `notes_discard_webview_tests.rs::a_switch_during_a_discard_says_what_the_discard_did`.

## What is not guaranteed

- **(a) A discarder's own raw-SQL client can sign a discard whose text differs from the draft it consumed**
  ([#987](https://github.com/cairn-ehr/cairn-ehr/issues/987)). The trigger checks the event's type, chart,
  transaction, signer and instant, not its parts: by COMMIT the draft's rows are deleted, so the floor has nothing to
  compare with. The window builds the event from the door's own rows; only a client writing its own SQL with its own
  released key can do this, and the false copy is then signed by that key.
- **(b) A raw-SQL Sign can spend a colleague's draft with an unrelated note**
  ([#985](https://github.com/cairn-ehr/cairn-ehr/issues/985)). Pre-existing (db/066's `cairn_note_draft_spend_parts`
  and the note arm of db/065's trigger), found by this slice's review, unchanged here.
- **(c) A draft holding a non-blank part by any named writer other than the discarder cannot be discarded**; it goes
  in by the timer, or stays stuck while a writer of it has no key (ADR-0089 decision 8). A co-signed discard is
  [#980](https://github.com/cairn-ehr/cairn-ehr/issues/980).
- **(d) The discarded text stays searchable.** It is in the event's plaintext twin, the full-text/RAG substrate
  (§3.13), for any reader holding the body's custody, as long as the event is not shredded. The twin's first line
  says it was never entered in the record.
- **(e) The marker is the only display.** `cairn_window` runs no audit view; a reader who does not open the
  `<details>` sees only who discarded a draft and when.
- **A writer is a claim** (ADR-0088 decision 9). Someone who claims to be colleague C at who's-here can blank C's
  last part and then Discard with their own key. Blank parts do not count (db/069), so the door answers `empty` and
  deletes the draft and its revisions without an event. This is not a new erasure: the blanking already hid the text,
  and 3b-2a's timer deletes an all-blank draft after the quarantine anyway. The discard only removes the owner-only
  revision trace sooner. See [#985](https://github.com/cairn-ehr/cairn-ehr/issues/985).
- **The marker reaches a peer only with the event.** Drafts never sync (design §3.3), so a peer never saw the draft;
  it sees the marker once the discard event arrives and its custody lets it project the body.

## Consequences

- **Easier.** A clinician can throw away their own draft, or one nobody can name the writer of, and the record keeps
  what it said, signed by them. Steps (paper *N* → built *M* → target *K*): discarding your own draft 1 → 1 (Discard +
  the strip, counted as Sign is) → 1; discarding a draft of unknown writing 1 → 1 → 1; a draft holding a colleague's
  part: paper permits no tear-up → refused → no gesture; seeing that a draft was discarded: paper leaves no trace →
  0 (the marker is in the list) → 0, and reading the text is one press. No `M > N`. Time budget, owed by a human in
  RUNBOOK §21: a discard ≤ 3 s, credential included, the same as Sign's (RUNBOOK §16), by stopwatch; the node's `note-discard` sample
  times the write half.
- **Harder.** Every future draft door must treat `discarded_by` as a third kind of spend, beside a Sign's and
  `by_timer`, and answer `discarded`. Every future reader of `clinical_note_discard.parts` must read the named keys
  only. A future change to the trigger's discard arm must keep both the signer and the instant.
- **The bet.** That a one-line marker, its text one press away, is enough for the next clinician to notice that a
  draft they may have acted on is gone, without cluttering the notes. We would know it is wrong if clinicians miss a
  discard that mattered, or if markers crowd out the notes on a busy chart.
- **Pinned by** `db/tests/068_clinical_note_discard_test.sql`, `db/tests/069_note_draft_discard_test.sql`,
  `crates/cairn-event/src/note_discard.rs`'s tests, `crates/cairn-node/src/note/discard.rs`'s tests,
  `crates/cairn-node/tests/{note_discard,note_discard_floor,note_read,window_role_grants_notes}.rs` and
  `cairn-gui-tauri/src/{notes_discard,notes_discard_webview,notes_parts,notes}_tests.rs`. RUNBOOK §21 carries the
  human measurements.
