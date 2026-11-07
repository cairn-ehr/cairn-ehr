# Durable rules — what a tidy-up must not undo

Moved here from `docs/HANDOVER.md` on 2026-10-15 (R7) so the handover stays a short, current
scaffold. These are long-lived: each rule is a step a later session could undo in good faith, with
its pin (the test that fails if it is undone) and, where one exists, the tempting wrong fix. The
full argument for each is in its ADR, its test's header, the design page
(`docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`) or ROADMAP.
**Read the section for any code you are about to touch.** When a slice adds rules, add its block
here, newest first, and leave a one-line pointer in HANDOVER.

## Per-slice rules (the repair path, the funnel)

**⇒ GUI SIGN-IN SLICE 3c-3a — SUPERVISION AND THE MANDATORY CO-SIGN (ADR-0094, db/072, db/004 + db/052 in place) — do not undo**
(pins: `db/tests/{004_actors,072_supervision}_test.sql`, `crates/cairn-node/src/note/{supervision,cosign_tests,review}.rs`'s
tests, `crates/cairn-node/tests/{supervision_doors,supervision_doors_approver,note_supervision,note_review_cosign,restore_actor_registry_supervision}.rs`,
`cairn-gui/cairn-gui-tauri/src/{notes_review,notes_review_db,supervision_panel,notes_sign_off_webview}_tests.rs`):
- **"As of t" is the part's or the vouch's OWN `hlc_wall`.** `annotate` judges `needs_cosigning` at the part's
  `recorded_at` (its `event_log.hlc_wall`) and each vouch's `by_supervisor` / `writer_supervised` at that vouch's
  `at`. **Tempting wrong fixes:** judge the requirement as it stands now (lifting a probation would silently clear
  every part signed under it: the slice's anti-goal); judge it at `t_effective` (freely backdatable); judge "is a
  supervisor" now (a supervisor linked after a colleague's co-sign would clear it retroactively, and one who has since
  stepped away would stop counting). Pins: `supervision.rs`'s `annotate_judges_the_requirement_at_the_parts_time`,
  `annotate_judges_each_vouchs_supervisor_at_its_own_time`; `note_supervision.rs::a_lifted_requirement_leaves_the_earlier_part_waiting`.
- **Registry times are FLOORED to the millisecond before they are compared.** `recorded_at` has microseconds, an HLC
  wall milliseconds; `Supervision::from_rows` floors, and the compare is `<=`, so a row in the same millisecond as a
  signing counts as in effect. **Tempting wrong fixes:** drop the floor "because chrono compares fine" (a requirement
  recorded at .000700 and a part signed at .000 would read as signed before it); use `<`. Pins:
  `supervision.rs`'s `a_row_in_the_same_millisecond_as_the_part_is_in_effect`, `the_requirement_is_read_as_it_stood_at_t`.
- **The fallback accepts the strongest countersignature the floor allows ON THAT PART: a `co-signed` vouch, or,
  only where the part's envelope key is not a human's, an attested `sign-off`.** (The floor also admits an attested
  sign-off on a human-signed part; under the fallback that one clears nothing.) With no supervisor at the vouch's time, `clears_cosign` accepts a non-student's
  vouch whose role is `co-signed`, or whose act is `sign-off` when `signer_human_actor` is `None`. **Tempting wrong
  fixes:** demand `CoSigned` "because the old rule did" (db/071 refuses `co-signed` on a part that went in under the
  node's key, so such a student's part would wait for ever); accept any attested sign-off (it would clear a
  human-signed part that could have been co-signed). Pins: `cosign_tests.rs`'s
  `a_part_signed_later_by_its_writer_still_needs_its_co_sign`, `with_no_supervisor_any_clinicians_co_sign_clears_it`
  (an attested sign-off on a human-signed part does not clear); `note_supervision.rs::a_part_gone_in_unsigned_is_cleared_by_a_clinicians_sign_off`.
- **An unplaceable signer judges nothing.** `supervision::writer_actor` and `cosign::writer_signature` both answer
  `None` for a `signed_by` with an empty actor, and must keep agreeing: annotation judges the record for the writer the
  rules countersign. **Tempting wrong fix:** fall back to `drafted_actor` when `signed_by`'s actor is empty "to find
  the writer anyway" (the record would be judged for someone this node cannot show signed it). Pins:
  `supervision.rs::an_unplaceable_signer_is_no_writer`, `cosign_tests.rs::an_unplaceable_signer_owes_no_co_sign`.
- **While the writer had a supervisor at the co-sign's time, only a supervisor as of then clears a mandatory
  co-sign, and the offer stays until one does.** While the
  writer had a supervisor at the vouch's time, a non-supervisor's `co-signed` vouch is read and clears nothing.
  `needs_sign_off` is `signed_off_by(..).is_none() || any(awaits_cosign)`. **Tempting wrong fixes:** let any
  non-student's co-sign clear it, as before this slice; drop the `awaits_cosign` half "because `signed_off_by` already
  skips students" (a probationer signs every part herself, so her note would have no button). Pins:
  `cosign_tests.rs`'s `a_non_supervisors_co_sign_does_not_clear_it`, `the_offer_stays_while_a_mandatory_co_sign_is_missing`;
  `note_supervision.rs::only_a_supervisor_clears_a_mandatory_co_sign`.
- **"Co-sign as reviewed" never vouches another writer's part, and the KEY decides who reviews.** `review::reviewable`
  takes only a signed part that owes no co-sign, whose writer is not the presser and has the presser among their
  supervisors now, that no supervisor has vouched, and that the presser has not signed. The presser is the signing
  key's actor (`vouch_as_shown`). **Tempting wrong fixes:** make it `sign_off_note` under a second label (it would
  vouch every part, other writers' and unsigned ones included); take the presser from the who's-here claim; drop
  `!cosign_required` (an owed co-sign is Sign off's to clear). Pins: `review.rs`'s
  `review_vouches_only_the_pressers_supervisees_parts`, `review_skips_a_part_that_needs_a_co_sign`;
  `note_review_cosign.rs`'s `review_never_vouches_another_writers_part`, `the_key_not_the_claim_decides`;
  `notes_review_db_tests.rs::the_key_not_the_claim_decides_in_the_window`.
- **Nobody acts on their own supervision as the supervisee — enforced in the door.** `cairn_supervision_apply`
  refuses every op but the assigner pair when the approver's actor is the subject's. **Tempting wrong fixes:** narrow
  it to the design's "no self-link, no self-lift" (a supervisee holding the right could unlink every supervisor and
  fall into the fallback, where any clinician clears her co-sign); move it to the window (a raw `cairn_agent` client
  goes straight to the door). Pin: `supervision_doors.rs::a_supervisee_cannot_act_on_their_own_supervision_even_as_assigner`.
- **The panel releases the key for ONE act and never holds it.** `supervision_key`: refuse a blank passphrase,
  `forget_held_key`, lock, release `Clinical`, and only on `Release::RefusedPurpose` once more with `Enrol`, then
  `from_release_noting`. **Tempting wrong fixes:** call `signer_for_act` "like the note acts" (a superuser's key would
  enter the held session); drop `forget_held_key` (the last person's held key would outlive a new presser); retry
  `Enrol` on any refusal (a wrong passphrase would be counted twice). Pins: `supervision_panel_tests.rs`'s
  `a_supervision_act_wipes_then_releases_and_holds_nothing`, `a_superuser_key_is_released_for_enrol_after_a_refused_clinical`.
- **A good re-read clears a stale status sentence, and only when no act's outcome is being said.** Each refresh's
  success branch holds `if (!outcome) setMessage(el("…-status"), "");` (both supervision panels and the superuser
  panel). **Tempting wrong fixes:** clear unconditionally (the act's own outcome vanishes with the re-read); never
  clear (a "Nobody is named …" or read-failure sentence stays beside a full list, false). Pins:
  `supervision_panel_tests.rs::a_good_read_clears_a_stale_status`, `superuser_panel_tests.rs::a_good_read_clears_a_stale_status`.
- **A review's words are its own, never Sign off's.** `notes_review.rs`'s `REVIEWED` / refusal sentences and
  `whos_here.js`'s `REVIEWED_LINE` / `NOT_REVIEWED_LINE` / `REVIEW_UNKNOWN_LINE` for the outcomes "reviewed" /
  "not-reviewed" / "review-unknown". **Tempting wrong fix:** reuse Sign off's outcomes and lines "since it is the same
  transaction" (`SIGNED_OFF_LINE` says every part of the note now carries your signature, false of a review; the
  review's line names no parts because a review may skip some). The same every-sentence-true rule holds the lift's
  "… still need a co-signature if they have none." Pins: `notes_review_tests.rs::review_words_are_exact`,
  `notes_sign_off_webview_tests.rs::a_switch_during_a_review_says_what_the_review_did`,
  `supervision_panel_tests.rs::the_lift_sentence_claims_no_waiting_notes`.
- **`related_actor` is skipped when absent in the export.** `ActorRegistryRow.related_actor` keeps
  `skip_serializing_if = "Option::is_none"`, so a row with no `related_actor` keeps today's shape. An export carrying any of
  the six ops is a deliberate one-way door (ADR-0094 decision 7): a link row does not decode on an older build, and
  an older restore door refuses every new op word. **Tempting wrong fix:** drop the attribute "for symmetry" (every
  export changes shape). Pins: `localstate_wire_pins.rs` (unchanged by the slice);
  `restore_actor_registry_supervision.rs::the_supervision_ops_survive_a_restore`.

**⇒ GUI SIGN-IN SLICE 3c-3b — A REPUDIATED SIGNATURE COUNTS AS UNSIGNED (ADR-0093, no SQL change) — do not undo**
(pins: `crates/cairn-node/src/note/{vouch_tests,view_tests,compound,vouch_read}.rs`'s tests,
`crates/cairn-node/tests/note_repudiated_signer.rs`, `cairn-gui/cairn-gui-tauri/src/notes_sign_off_tests.rs`):
- **`vouch::re_signs(signer, v)` is THE one test of "the real person signed it again".** A part whose `signed_by` is
  repudiated counts only when a vouch on it is by a key that is NOT repudiated, of the SAME actor. `re_signed(p)` asks
  it of each vouch; `part_is_signed` and the byline's " — counts as unsigned" ask `re_signed`, and the byline's
  " · re-signed by …" asks `re_signs` of each vouch. **Tempting wrong fixes:** copy the conditions inline into a new
  reader (the words and the rules could then disagree about which vouch re-signs); let any later vouch clear the part
  "because someone vouched for it" (a colleague cannot say the words were the real person's: maintainer ruling);
  compare keys instead of actors (the re-keyed person never matches their own part); drop the `!v.by.repudiated` check (a re-sign by a key later
  repudiated would keep counting). Pins: `vouch_tests.rs`'s `a_colleagues_sign_off_does_not_clear_a_repudiated_part`,
  `only_the_real_persons_re_sign_clears_it`; `view_tests.rs::a_re_sign_by_a_key_since_repudiated_counts_nothing`.
- **Never count a repudiated signer in `signed_for`.** The repudiated key shares the re-keyed person's actor
  (`cairn_note_key_actor` does not filter repudiation), so `has_signed` would say they have signed, and
  `sign_off_vouches` would answer the real person `NothingToSign`. **Tempting wrong fix:** "simplify" `signed_for`
  back to `signed_by` plus every vouch's `by`. Pins: `vouch_tests.rs::the_re_keyed_writer_is_offered_their_repudiated_part`;
  `note_repudiated_signer.rs::a_repudiated_signers_part_reads_unsigned_until_they_re_sign`.
- **Ask `part_is_signed`, never `signed_by.is_some()`.** A repudiated `signed_by` is present and counts for nothing;
  `full_signers` yields nobody unless every part `part_is_signed`, so while a repudiated part is not re-signed, a
  colleague who vouched it never reads "signed off by" (once the real person re-signs, that earlier vouch counts as an
  ordinary sign-off of the same bytes: ADR-0093 decision 5). **Tempting wrong fix:** test `signed_by` in a new reader "because the grade is `Signed`".
  Pins: `vouch_tests.rs::a_repudiated_signature_counts_as_unsigned`,
  `a_colleagues_earlier_sign_off_counts_once_re_signed`; `compound.rs::a_repudiated_part_makes_the_note_include_an_unsigned_part`.
- **A repudiated, not re-signed `signed_by` is planned `Attested`, never `CoSigned`; only that one.** A co-sign would
  read "co-signed by" over a possibly forged signature. Once re-signed, the signature stands and is co-signed as any
  other (#1009). **Tempting wrong fixes:** refuse `CoSigned` whenever `signed_by` is not a standing `Some` (a part gone
  in unsigned under a human envelope key is co-signed by the floor's own rule); drop the `!re_signed(p)` half "to be
  safe" (a supervisor could then never clear a re-signed student part's "— not co-signed"). Pins: `vouch_tests.rs`'s
  `a_repudiated_part_is_never_planned_as_a_co_sign`, `a_re_signed_part_is_co_signed_again`,
  `sign_off_co_signs_only_what_the_floor_admits`; `note_repudiated_signer.rs`'s
  `a_colleagues_vouch_on_a_repudiated_part_is_attested`, `a_re_signed_students_part_is_co_signed_by_a_supervisor`.
  Known gaps: #1007 (a repudiated envelope key), #1010 (a sign-off made before a student's re-sign), #1011
  (re-signing one's own part also signs off the other writers' parts).
- **A vouch by a repudiated key counts nothing**: not as a signature of an unsigned part, not as the co-sign that
  clears a student's "— not co-signed", not as the sign-off that makes the note read "signed off by". Pins:
  `vouch_tests.rs::a_repudiated_keys_vouch_counts_nothing`, `view_tests.rs::a_repudiated_co_signer_does_not_clear_not_co_signed`,
  `compound.rs::a_repudiated_keys_sign_off_does_not_sign_the_note_off`.

**⇒ GUI SIGN-IN SLICE 3c-2 — THE VOUCH AND THE SIGN-OFF (ADR-0092, db/071, db/037 + db/044 in place) — do not undo**
(pins: `db/tests/071_clinical_note_attestation_test.sql`, `crates/cairn-event/src/note_attestation.rs`'s tests,
`crates/cairn-node/src/note/{vouch_tests,vouch_read,view_tests,compound,sign_vouch}.rs`'s tests,
`crates/cairn-node/tests/{note_attestation_floor,note_sign_off,note_addendum_vouch,note_vouch_remote_read}.rs`,
`cairn-gui/cairn-gui-tauri/src/{notes_sign_off,notes_sign_off_db,notes_sign_off_webview}_tests.rs`):
- **The applier holds the floor's closed vocabularies, as a SKIP.** `clinical_note_attestation_apply` drops a `/1`
  body whose bearing role is not `attested`/`co-signed`, whose act is not `addendum`/`sign-off`, whose addendum is
  `co-signed`, or whose target/address is malformed, because the heal path reaches a sealed vouch the floor never
  saw (db/020 step 7). **Tempting wrong fixes:** delete the "redundant" guard because "the floor already refuses
  these" (it never ran on that body); turn it into a table CHECK or a `RAISE` "to be loud" (either fails the heal).
  Pin: db/tests/071 rows 0013–0018 (each breaks one skip rule and asserts no projection row; 0017/0018, the
  bad uuid and bad hex, fail with a raise if the skip is removed).
- **A vouch counts only when its token is verified AND it names the part's own bytes, under the part's own chart.**
  The vouch column filters on `cairn_note_vouched_attester(a.event_id) = a.signer_key_id`, `a.target_address =
  e.content_address` AND `a.patient_id = n.patient_id`, inside the note statement (one snapshot). **Tempting wrong
  fixes:** join on `target` alone "since the strict door already checked the address and the chart" (the remote door
  did not: a vouch for another version, or one filed under another chart, would sign this part); read
  `event_log.attester_key` directly (db/020 stores a peer's token unverified); read the vouches in a second statement
  (a second snapshot). Pins: `note_sign_off.rs::a_forged_vouch_is_not_read`,
  `note_vouch_remote_read.rs::a_vouch_filed_under_another_chart_changes_no_byline`,
  `vouch_read.rs::only_a_verified_vouch_for_the_parts_own_bytes_is_read`.
- **People are compared by ACTOR, never by key, and an empty actor matches no one.** `vouch::same_actor`;
  `cairn_note_keys_share_actor` at the floor. **Tempting wrong fixes:** compare `signer_key_id`s "because it is
  simpler" (a re-keyed writer stops matching their own part); let `""` equal `""` (two unknown keys become one
  person, and an unknown key could sign off a note). Pins: `a_re_keyed_writer_vouches_for_a_part_drafted_under_the_old_key`,
  `vouch_tests.rs::a_key_this_node_does_not_know_matches_no_one`.
- **A sign-off plans `CoSigned` exactly where the floor admits it.** `sign_off_vouches` reads
  `NoteRow::signer_human_actor` (the ENVELOPE signer's actor, only for a human's key), the predicate db/071's co-sign
  rule applies. **Tempting wrong fix:** decide from `signed_by` "because that is who signed the part" (the grade's key
  may be an attester; the floor reads the envelope, refuses the co-sign, and the whole sign-off with it). Pin:
  `vouch_tests.rs::sign_off_co_signs_only_what_the_floor_admits`.
- **"Signed off by S" names the first full signer who PRESSED.** `compound_state` takes the first of
  `vouch::full_signers` with a `sign-off` vouch on the note. **Tempting wrong fix:** take `signed_off_by` and then
  check that one pressed (a doctor who signed every part but never pressed hides the colleague who did). Pin:
  `compound.rs::the_state_skips_a_full_signer_who_never_pressed_sign_off`.
- **The sign-off is as shown.** `sign_off_note` re-reads the compound in its transaction and answers `Changed` unless
  its parts are exactly the ids the window sent. **Tempting wrong fix:** drop `shown_parts` "because the node
  re-reads anyway" (a colleague's addendum landing after the read would then be signed unseen). Pins:
  `note_sign_off.rs::a_part_added_after_the_note_was_shown_refuses_changed`,
  `a_part_shredded_after_the_note_was_shown_refuses_changed`.
- **An addendum vouches `also_vouches ∩ addendum_covers`, for the signing KEY's actor.** The strip's list is computed
  for the who's-here CLAIM, so it is a ceiling, never the answer. **Tempting wrong fixes:** vouch whatever the strip
  listed "because the clinician saw the line" (Nurse B would sign Nurse A's part); take the actor from the claim
  (`cairn_note_staff_actor`) rather than the key (Task 4's mutant; the floor refused it loudly). Pins:
  `note_addendum_vouch.rs::a_listed_part_that_is_not_the_signers_is_not_vouched` (key and claim DIFFER),
  `an_unlisted_own_part_is_not_vouched`.
- **No note act starts while a press is in flight.** A sign-off is the first note act in flight with the editor
  FREE: `newNote`, `addToNote`, `continueHere`, `pressSignOff` and the editor's Sign and Discard listeners all start
  with a `signInFlight()` guard. **Tempting wrong fix:** remove a guard "because `noteDraft !== null` already covers
  it" (it does not with the editor free; a Sign opened mid-press reuses the strip's passphrase, and the two presses
  clear each other's flag). Pins: `notes_sign_off_webview_tests.rs::nothing_opens_the_editor_while_a_press_is_in_flight`,
  `sign_and_discard_refuse_to_start_while_a_press_is_in_flight`. The medication pane is not yet guarded (#1002).
- **A sign-off press settles with its outcome**, as Sign and Discard do: "signed-off", "not-signed-off" or
  "sign-off-unknown", which a who's-here switch's picker words (`whos_here.js`'s `SIGNED_OFF_LINE` and siblings).
  Since 3c-3a a "Co-sign as reviewed" press settles the same way with its own three, "reviewed", "not-reviewed" or
  "review-unknown" (`REVIEWED_LINE` and siblings; the 3c-3a block above). **Tempting wrong fix:** settle it as `null`
  "because the re-read chart shows the result" (the switch closes the chart, so the clinician never sees it). Pins:
  `notes_sign_off_webview_tests.rs`'s `a_switch_during_a_sign_off_says_what_the_sign_off_did`,
  `a_switch_during_a_review_says_what_the_review_did`.
- **`view::part_byline` is the one producer of a part's byline** (the root's heading, each part, each seam), with the
  "(student)", "— not co-signed", "(signer unconfirmed)" and vouch words. `byline_words` has no production caller
  left (its own tests and `note_read.rs` use it). **Tempting wrong fixes:** call `byline_words(grade, writer)` in a
  new window path "because it is the old name" (the student and vouch words vanish there); let any `CoSigned` vouch
  clear "— not co-signed" (a student's own co-sign admitted remotely, or one by a key this node cannot place, which
  might be the student's, would hide the pending mark). Pins:
  `view_tests.rs::a_students_own_co_sign_does_not_clear_not_co_signed`,
  `another_students_co_sign_does_not_clear_not_co_signed`, `an_unplaceable_co_signer_does_not_clear_not_co_signed`;
  the window's use only DB-gated (`notes_sign_off_db_tests.rs`, #905).

**⇒ GUI SIGN-IN SLICE 3c-1 — THE COMPOUND NOTE AND THE ADDENDUM (ADR-0091, db/070, db/064 + db/065 in place) — do not
undo** (pins: `db/tests/070_note_addendum_test.sql`, `db/tests/064_clinical_note_test.sql`,
`crates/cairn-node/src/note/compound.rs`'s tests,
`crates/cairn-node/tests/{note_addendum_doors,note_addendum_sign,note_addendum_ahead_of_target}.rs`,
`cairn-gui/cairn-gui-tauri/src/{notes_compound,notes_compound_webview}_tests.rs`):
- **The target is the ROOT, set once in the header, and Sign and the timer read it from there.** db/070's door sets
  `note_draft.addendum_to` when it makes the header; no door updates it; Sign reads it under the draft's lock after its
  own save, the timer before `commit_overdue_parts`. The window's `addendum_to` only picks the save door. **Tempting
  wrong fixes:** take the target from the window at Sign "because the window already knows it" (a restarted window
  sends `None`, and the draft would sign as a new note); let a later save update the column "to fix a wrong target"
  (a reader then needs the lock, and a new note's draft could become an addendum); name the compound's LATEST part
  (the target then depends on which parts this window has read; pinned by
  `note_addendum_sign.rs::signing_an_addendum_names_the_root`, `two_addenda_signed_at_once_both_join_the_note`). Pins:
  `note_addendum_sign.rs::sign_takes_the_target_from_the_header_not_the_window`,
  `sign_inside_the_debounce_still_makes_an_addendum`, `the_timer_commits_an_addendum_as_an_addendum`;
  `note_addendum_doors.rs::a_later_plain_save_keeps_the_target`, `a_new_notes_draft_refuses_a_target_later`.
- **The same-chart floor arm (db/064 rule 6a) is AUTHORING-ONLY; never tighten the remote door.** The remote door keeps
  rule 6's existence check alone, and its bare `RAISE` (`P0001`) is what `cairn-sync` pens and re-offers (#948).
  **Tempting wrong fix:** "make both doors agree" by running 6a at the remote door too: an addendum a peer's older floor
  admitted (to another chart's note) would then be refused for ever, and #948's pen would become a permanent refusal.
  Never give rule 6 a `USING ERRCODE` either (the puller reads only `P0001` as a verdict). Pins:
  `note_addendum_doors.rs::the_remote_door_still_admits_a_held_target_on_another_chart`;
  `note_addendum_ahead_of_target.rs::an_addendum_ahead_of_its_note_is_penned_then_admitted` (it asserts `P0001`
  exactly; Task 2 saw it fail when rule 6 raised `XX000`).
- **Grouping is pure and lives in `cairn-node`** (`note::compound::compounds`, `compound_state`, `state_words`), so
  every UI groups and words a note the same way (ADR-0021). **Tempting wrong fix:** group in the webview or in the
  window's Rust "because it is only display" (a second UI then walks `addendum-to` its own way, and two screens show
  one note differently). Pins: `compound.rs`'s tests, `the_state_words_are_exact`.
- **A door that delegates must not leave a half-made row behind on the delegate's refusal.** db/070 makes the header,
  then calls db/066's save; if that refuses (only `no-such-writer` can), the header it made is deleted, so "every
  answer but `saved` wrote nothing" stays true. **Tempting wrong fix:** copy db/066's writer check into db/070 "so
  it refuses first" (two copies of one rule drift). Pin: db/tests/070's `a_refused_writer_leaves_no_header`.
- **A pin on a condition's TEXT does not pin WHEN it is evaluated.** `renderNotes` draws the notes AFTER its loop that
  reopens a draft; drawn before it, `noteDraft === null` still held, and every reopen left a dead "Add to this note"
  (Task 6's review; the condition pin passed). **Tempting wrong fix:** move the draw back to the top "next to the
  withheld line". Pin: `notes_compound_webview_tests.rs::the_notes_are_drawn_after_a_draft_reopens`. The "Adding to …"
  line is its own field (`DraftView.adding_to`, `CompoundView.adds_line`), never a prefix parsed out of `words`.
- **A note of two or more parts is HEADED by its root, and each later part is SEAMED.** `compoundItem` draws the
  root's byline and the state line ABOVE the text, then the text a part at a time (`CompoundView.segments`), each
  later part after its Rust-worded seam, "Added: <its byline> · <its time>" (spoken form for the `?`, #973).
  **Tempting wrong fixes:** reuse `noteItem` for a compound "since its fields are a note's" (it draws text, then
  byline: the root writer's "signed by Nurse A" then sits under the LAST writer's words and reads as signing them —
  the final review's Important 1); draw the joined `text` "because it is simpler" (a later part's writer and time
  vanish behind the disclosure); re-sort compounds by their latest part (the note jumps in the list; the seam carries
  the time). Pins: `notes_compound_webview_tests.rs::a_compound_is_headed_by_its_root_and_seamed_at_each_part`;
  `notes_compound_tests.rs::each_later_part_is_seamed_with_its_writer_and_time`.

**⇒ GUI SIGN-IN SLICE 3b-2b — THE AUDITED DISCARD (ADR-0090, db/068 + db/069, db/037 + db/044 + db/065 + db/066 in
place) — do not undo** (pins: `db/tests/068_clinical_note_discard_test.sql`, `db/tests/069_note_draft_discard_test.sql`,
`crates/cairn-event/src/note_discard.rs`'s tests, `crates/cairn-node/tests/{note_discard,note_discard_floor,note_read,window_role_grants_notes}.rs`,
`cairn-gui-tauri/src/{notes_discard,notes_discard_webview,notes_parts,notes}_tests.rs`):
- **"Your own writing" is decided by KEY, never by staff id.** The door compares each non-blank part's
  `cairn_vault_member_key(writer)` with the discarder's key; a named writer with no key is `not-yours`; blank parts
  (`cairn_twin_is_present`) never count; the discarder must be a clinical member. **Tempting wrong fixes:** compare
  staff-id text "because it is simpler" (the key is what the signature proves, ADR-0088 decision 15, and what
  db/065's trigger checks; a rule in another currency is one the floor cannot back); read a keyless writer as "not known" so the draft can go (it quietly loses
  who wrote it, principle 4); test blank with `btrim(…) <> ''` (it disagrees with the floor under another collation).
  Pins: db/tests/069's `a_colleagues_part_is_not_yours`, `a_named_writer_without_a_key_is_not_yours`,
  `a_blank_colleague_part_does_not_count`, `an_admin_key_is_no_such_signer`;
  `note_discard.rs::a_colleagues_draft_is_refused_not_yours_and_kept`.
- **The trigger's discard arm checks the SIGNER and the INSTANT, not only type + chart + `xmin`.** The door trusts the
  key its caller names, and a key id is public; only a row signed by `discarded_by` proves the discarder's hand, and
  only `t_effective = spent_at` (minted by the door under the draft's lock) proves it is THIS discard. **Tempting
  wrong fix:** "simplify" the arm to type + chart + `xmin`, like the note arm (a raw-SQL client naming a colleague's
  key then discards her draft with any event it can sign, and a captured genuine discard of hers can be replayed
  against another draft). Also: read the instant before the lock (a discard queued behind a save is stamped before
  that save's keystroke). Pins: db/tests/069's `the_commit_refuses_a_spend_without_its_signed_event`,
  `the_commit_refuses_an_event_signed_by_another_key`, `the_commit_refuses_a_discard_event_of_another_instant`;
  `note_discard.rs::a_raw_door_call_without_its_event_is_refused_at_commit`,
  `discarded_at_follows_a_save_that_held_the_lock` (a smoke test only: the node's own `save_part` takes the lock
  first, so no test yet catches a door that reads the clock before its lock; reviewer-held).
- **`discarded` is not `spent`.** The save, take and discard doors answer `discarded` for a spend with `discarded_by`
  set, and `note_draft_spent.discarded_by` lives in db/065 with its paired `ALTER` (3b-2a's "a column a door body
  reads lives with its table"). **Tempting wrong fix:** answer `spent` "because the draft is gone either way" (the
  window words `spent` as "already signed", which is false: no note exists). Pins: db/tests/069's
  `save_after_discard_answers_discarded`, `take_after_discard_answers_discarded`;
  `note_discard.rs::sign_after_discard_is_refused_discarded`;
  `notes_tests.rs::discarded_fork_words_never_say_signed`.
- **The marker is never a note, and a part is read by its five named keys only.** `clinical_note_discard` is read
  only by `read.rs`'s `DISCARDS_SQL`, which takes `writer`, `narrative`, `stated_time`, `recording_started` and
  `recording_ended` and nothing else; `NOTES_SQL` does not know the table exists; the window draws a closed
  `<details>` in `#note-discards`, its words set as text. **Tempting wrong fixes:** render a part generically (every
  key it carries) "so a future field just shows" (db/068 stores `parts` as received, so a key a raw-SQL client added
  reaches the screen unchecked); join the discards into the notes list or count them as notes (ADR-0087 R8); show
  the text open "to save a click" (the notes crowd out). Pins: `note_read.rs::the_marker_lists_the_discard_not_as_a_note`,
  `a_doubted_set_withholds_a_linked_charts_marker`; `notes_discard_webview_tests.rs::a_marker_is_a_details_not_a_note`,
  `markers_are_text_never_html`; `notes_parts_tests.rs::the_marker_summary_and_headings_are_exact`.
- **A discard press marks the text as carried (`mayBeSigned`), and a refusal saves it again.** The node saves the text
  on screen and discards it in ONE transaction, so a refusal rolls back that save. **Tempting wrong fixes:** leave
  `mayBeSigned` unset "because nothing was signed" (a late save answering `discarded` after this window's own press
  then forks the discarded text back as a new draft); skip the re-save after a refusal (the "your text is still here"
  promise is true only until the window closes); discard the last autosave instead of the text on screen (the audit
  copy misses the last second of typing). Pins: `notes_discard_webview_tests.rs::a_discard_marks_the_press_as_carrying_the_text`,
  `a_discarded_save_is_handled_as_spent`, `a_refused_discard_saves_the_text_on_screen_again`,
  `discard_cancels_the_pending_save_before_it_discards`; `note_discard.rs::discard_carries_the_text_on_screen`.
- **A discard's words never borrow Sign's.** Each refusal and outcome has its own sentence ("Not discarded: …",
  "Whether this draft was discarded is NOT known: …"); the who's-here picker words a discard's outcome as a discard's
  (`noteSigning` renames a discard's `unknown` to `discard-unknown`). **Tempting wrong fix:** reuse the sign sentence
  with a prefix "to keep one set of words" — that copy is false here three ways this slice found ("nothing was
  signed", "says when it goes in by itself" for a stuck draft, the picker's "Whether your note was signed"). Sign's own
  matching flaw is #986. Pins: `notes_discard_tests.rs::discard_words_each_refusal`, `discard_words_each_outcome`,
  `a_failure_is_led_by_what_it_means_for_the_discard`;
  `notes_discard_webview_tests.rs::a_switch_during_a_discard_says_what_the_discard_did`.

**⇒ GUI SIGN-IN SLICE 3b-2a — THE QUARANTINE COMMIT AND THE `Asserted` GRADE (ADR-0089, db/067, db/065 + db/066 in
place) — do not undo** (pins: `db/tests/067_note_draft_commit_test.sql`, `crates/cairn-event/src/contributor.rs`'s
tests, `crates/cairn-node/tests/{note_commit,note_read,authority_lockstep,window_role_grants_notes}.rs`,
`cairn-gui-tauri/src/{notes_commit,notes,notes_fork_webview}_tests.rs`):
- **The door decides "overdue" — never the caller, never a terminal clock.** `cairn_note_draft_commit_overdue`
  re-checks the header's `last_keystroke + quarantine` against `clock_timestamp()` AFTER the draft's advisory lock
  and the header's `FOR UPDATE`, and consumes the draft in the same call; `cairn_note_drafts_overdue` is only a hint.
  **Tempting wrong fixes:** commit every draft the listing returned "since we just checked" (a save between the list
  and the commit restarts the clock, and a draft goes in mid-sentence); pass the window's `now` to the door "for
  testability" (a terminal clock then decides, and a raw-SQL client commits anyone's draft early); test overdue
  before taking the lock; make this door WAIT for the lock "as every other draft door that writes does" (it alone tries it: a
  held lock means a save or Sign is acting on the draft, and a waiting timer stalls a chart read behind that press).
  Pins: db/tests/067's `not_overdue_commits_nothing`, `a_spent_draft_commits_nothing`;
  `note_commit.rs::two_commits_race_one_note`, `a_held_draft_lock_makes_the_commit_skip_it`,
  `a_sign_after_the_timer_is_refused_committed`.
- **The window's due words come from the HEADER, through `cairn_note_drafts_due`** — the commit door's own input,
  plus its `stuck` rule. **Tempting wrong fix:** compute `due` in Rust from the parts' `last_keystroke` (db/066
  clamps a part's stamp apart from its header's after a backward clock step, so the shown time disagrees with the
  timer). Pin: `note_commit.rs::due_follows_the_header_not_the_parts`.
- **A column a door body reads lives with its table.** `note_draft_spent.by_timer` is created in db/065 (with its
  paired `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, #207), because db/066's save and take doors read it. **Tempting
  wrong fix:** move it into db/067 "with the rest of the slice" (PL/pgSQL binds a column when the body runs, so on
  an upgraded database db/066's doors raise until db/067 has committed). Pins: db/tests/067's
  `save_after_a_timer_commit_answers_committed`, `take_after_a_timer_commit_answers_committed`.
- **`committed` forks regardless of `mayBeSigned`.** A timer's commit is never this window's Sign, so what was typed
  since the last save is kept as a new draft even after a Sign press. **Tempting wrong fix:** treat `committed` as
  `spent` in `notes_save.js` ("both mean the draft is gone"): the `mayBeSigned` excuse then swallows text the timer's
  note does not hold. Pins: `notes_fork_webview_tests.rs::a_timer_commit_forks_even_after_a_sign_press`,
  `notes_tests.rs::committed_sentence_exact`.
- **A stuck draft is never committed as "not known".** A named writer whose `cairn_vault_member_key` is NULL drops
  the whole transaction (`CommitOutcome::Stuck`); the draft stays and says it is stuck. **Tempting wrong fixes:**
  commit the part with no `drafted` contributor "so the text is not lost" (it quietly loses who wrote it,
  principle 4); skip only that part (half a note goes in and the draft is spent); tell the clinician to "sign it" (a
  Sign is refused `NoSuchWriter` too). Pins: `commit.rs::a_named_writer_without_a_key_is_stuck`;
  `note_commit.rs::a_writer_with_no_key_is_stuck_and_kept`, `one_stuck_draft_does_not_stop_the_others`;
  `notes_commit_tests.rs::a_stuck_draft_reads_stuck`.
- **`recorded` is never the human contributory role.** `Asserted` needs a contributory entry other than `recorded`
  (the capture device's own, ADR-0051) whose key the registry enrolled for a `human` actor (db/067's
  `cairn_note_key_is_human`); a bearing contributor always wins.
  **Tempting wrong fixes:** count any contributory entry (a node key that is also a known human then grades its own
  capture `Asserted`); let the classifier decide "human" from the body itself (a claim read out of a body, the
  #412 trap; who is human is a registry fact the caller resolves and passes in); take "the read can name the key"
  as "human" (`cairn_note_signer_name` names device and agent keys by their pinned handle too). Pins:
  `contributor.rs::recorded_is_never_the_human_role`,
  `the_old_classifier_never_says_asserted`, `a_bearer_wins_over_asserted`;
  `authority_lockstep.rs::asserted_in_rust_is_unverified_in_sql`;
  `note_read.rs::a_note_drafted_by_an_agent_key_never_grades_went_in_unsigned`.
- **The runner is a Rust task, one connection hold per draft, and the tick never reopens a lost connection.**
  **Tempting wrong fixes:** move the tick into the webview as a `setInterval` (WebKit throttles a hidden window's
  timers, so a minimised window stops committing); hold the connection for the whole pass (a clinician's press waits
  behind every overdue draft on the node); reopen from the tick (a dead node is hammered every minute with nobody
  there to read why). Pins: `notes_commit_tests.rs::the_tick_is_spawned_once_in_setup`,
  `each_draft_is_its_own_hold_and_the_tick_never_reopens`, `a_failed_commit_never_blocks_the_read`.
- **A draft whose last commit FAILED says so; it never promises another try.** The window keeps the drafts whose
  last commit failed (`AppState::commit_failed`, cleared by any try that did not fail) and the pane reads them.
  **Tempting wrong fixes:** drop the commit's report at the chart read (`let _ =`) "because a failed commit must not
  block the read" (the read must not be blocked, but a failure repeating every pass then reads "the window tries
  again", and the next clinician leaves an abandoned note nobody will ever sign); show only "overdue" (the same
  broken promise). Pins: `notes_commit_tests.rs::due_words_before_and_after`, `a_failed_commit_never_blocks_the_read`.

**⇒ GUI SIGN-IN SLICE 3b-1 — WHO'S HERE AND A DRAFT'S PARTS (ADR-0088, db/066, db/065 in place) — do not undo**
(pins: `db/tests/066_note_draft_parts_test.sql`, `crates/cairn-event/src/contributor.rs`'s tests,
`crates/cairn-node/src/note/sign.rs`'s tests, `crates/cairn-node/tests/{note_draft_parts,note_sign_parts,note_read,window_role_grants_notes}.rs`,
`cairn-gui-tauri/src/{whos_here,whos_here_webview,notes,notes_parts,notes_fork_webview}_tests.rs`, `crates/cairn-node/src/note/view.rs`'s tests):
- **A save writes only the TAIL part, and only the saver's own.** Any other save appends a part; no draft door takes
  a part number, so nothing can address, change or remove another writer's text. A writer's own earlier part is
  read-only too once someone wrote below it: a deliberate shortcut, #967. **Tempting wrong fixes:** add a `p_seq`
  to the save door "so Nurse A can fix her typo in part 1" (then the door must prove the part is hers, which is where
  Dr Z rubs out Dr X's ink); or let a save replace the draft's whole text "for simplicity". Pins: db/tests/066's
  `b_cannot_reach_part_1`, `another_writer_appends`, `a_b_a_is_three_parts`.
- **The writer travels with the save; it is never re-read from who's here.** The editor captures it once, when it
  opens (`noteDraft.writer`), and sends it with every save and with Sign; `notes.rs` passes it through. A switch
  waits for a Sign in flight, then closes the chart awaiting its saves, before the picker opens. **Tempting wrong
  fix:** read the current claim in `note_draft_save_impl` "to drop a parameter" — a debounced save landing after a
  switch then files Nurse A's words under Dr B. Pins: `the_writer_is_set_once_at_editor_open`,
  `save_passes_the_given_writer_not_the_claim` (source text: keep its token match strict),
  `switch_awaits_the_save_queue_before_picking`.
- **"Own part" is decided by KEY** (`cairn_vault_member_key(writer)` = the signing key), never by comparing staff-id
  text in Rust, and a named writer with no member row refuses the sign (`NoSuchWriter`), never signs as "not known".
  **Tempting wrong fix:** normalise staff ids in Rust and compare them — a second normalisation rule that can drift
  from `cairn_vault_staff_id`. Pins: `sign.rs::a_parts_author_is_decided_by_key`,
  `note_sign_parts.rs::a_one_writer_draft_signs_as_the_signers_own_authored_note` (a `" NoteWriter "` writer is still the
  signer's own), `a_part_whose_writer_left_is_refused_no_such_writer`.
- **A door's return shape is final: new shape, new name.** 3a's `cairn_note_draft_save`, `cairn_note_drafts` and
  `cairn_note_draft_spend` were deleted from db/065 in place AND are dropped by db/066's `DROP FUNCTION IF EXISTS`
  lines; the part-aware doors are new names. **Tempting wrong fixes:** `CREATE OR REPLACE` an old name with a new
  signature or return type (db/065's replay then fails on every existing database); delete db/066's `DROP` lines as
  dead code (an upgraded database keeps the old doors, granted to `cairn_window`). Pins: db/tests/066's
  `the_3a_doors_are_gone`; `window_role_grants_notes.rs`.
- **One event per non-blank part, in ONE transaction, HLC ticks inside it, no savepoint; the spend records the first
  non-blank part's event id.** The first non-blank part is the note (a blank part 1 is skipped); each later non-blank
  part carries `addendum-to` the one before it. **Tempting wrong fixes:** commit each part in its own
  transaction "so one bad part does not block the rest" (half a note signs and the draft is spent); tick the HLCs
  before `BEGIN`; spend with the last part's id, or give the spend's id to `seq` 1 rather than to the first non-blank part (a
  blank part 1 would leave the spend naming no note); wrap a submit in a savepoint (db/065's trigger then refuses every
  sign, the 3a rule). Pins: `note_sign_parts.rs::a_refused_second_part_rolls_back_the_first`,
  `a_two_writer_draft_signs_as_two_events_chained_by_addendum_to`,
  `empty_parts_are_skipped_and_an_all_empty_draft_is_refused_empty`; db/tests/066's
  `spend_still_needs_its_note_in_this_transaction`.
- **"Blank" is `cairn_twin_is_present`, the floor's own test, in both draft doors.** **Tempting wrong fix:**
  `btrim(narrative) <> ''` or a `[[:space:]]` regex (the plan's own first wording): its answer for a no-break space
  depends on the database's collation, so a part the floor refuses as blank would reach Sign and make the draft
  unsignable. Pin: db/tests/066's source check on both doors.
- **The writer rule and the directory rule are one rule**: `cairn_vault_purpose_allowed(role, false, 'clinical')` in
  both `cairn_note_draft_part_save` and `cairn_whos_here_directory`. **Tempting wrong fix:** filter only the
  directory (a raw-SQL client then records a superuser or an admin as a writer). Pins: db/tests/066's superuser and
  admin `no-such-writer` cases and `directory_lists_clinical_roles_only`.
- **`moved` is not `elsewhere`, and a `moved` save keeps the unsaved remainder as a new draft** (#958); **so does a
  `spent` save, unless this window's own Sign may have signed the draft** (`mayBeSigned`, set BEFORE the Sign is
  sent, taken back only by a refusal that recorded nothing; final review I1, #945). **Tempting wrong fixes:** fold
  `moved` back into `elsewhere`; drop the fork "because the text is at the other terminal" (what was typed since the
  last save is not); treat `spent` as "nothing to do" (Nurse A's line typed after Dr B signed at terminal B is then in
  no draft, and lost on close); set `mayBeSigned` only after the Sign answers, or never clear it (an unknown-outcome
  Sign's follow-up save then forks text it just signed, or the save after an "already signed" refusal keeps the
  editor on a dead draft). Pins: db/tests/066's `moved_vs_elsewhere`;
  `notes_fork_webview_tests.rs::moved_forks_a_new_draft`, `a_draft_signed_in_another_window_forks_a_new_draft`;
  `notes_tests.rs::moved_sentence_exact`, `spent_sentence_exact`.
- **A reopened draft writes as the claim its list was read for** (`NotePane.claim`, final review item 7), and a list
  read for another claim than the one held now opens nothing. **Tempting wrong fix:** take `openDraft`'s writer from
  `whosHere()` "because it is the same thing" — a list read before a pick then opens an "unknown" tail for editing
  under Nurse A's name, and her first save APPENDS a copy of that tail as her part. Pins:
  `whos_here_webview_tests.rs::the_writer_is_set_once_at_editor_open`;
  `notes_parts_tests.rs::the_pane_names_the_claim_it_chose_the_editable_part_for`.
- **A part the signer only attested never reads like their own note.** The byline comes from the event's contributor
  set (`view::PartWriter`: a `drafted` contributor, named or not, and whether the signer's role is `attested`), never
  from `drafted_by` alone. **Tempting wrong fix:** infer "own" from a NULL drafter name (a part signed under "Don't
  know", or by a drafter this node cannot name, then reads "signed by Dr B" as if Dr B wrote it). Pins:
  `view.rs::an_attested_part_never_reads_as_the_signers_own`,
  `note_read.rs::an_unknown_writers_part_reads_writer_not_known_signed_by`.
- **A pick and the re-lock forget the held key FIRST; only a person's input resets the idle clock.**
  **Tempting wrong fixes:** forget the key only after a pick succeeds (a refused pick leaves Dr A's key for Dr B);
  count a poll or an IPC answer as activity (main.js's lock-poll lesson: the terminal never re-locks). Pins:
  `whos_here_tests.rs::pick_forgets_the_held_key`, `a_pick_of_someone_not_enrolled_names_nobody`,
  `relock_forgets_claim_and_key`; `whos_here_webview_tests.rs::idle_resets_only_on_user_input`,
  `relock_waits_for_a_sign_in_flight`.
- **The claim, the recent people and the bookmarks live in window memory only.** A bookmark holds a patient's name.
  **Tempting wrong fix:** persist them "so a restart remembers" (a patient's name on disk in the clear). Pin: the
  storage check in `whos_here_webview_tests.rs::parts_drawn_through_cell` covers the webview; nothing pins the Rust
  side (`whos_here::Here` simply does no I/O), so a reviewer must.
- **The addendum ref is read as `::uuid::text`** (`note/read.rs`). **Tempting wrong fix:** parse the ref's raw text
  in Rust — one peer note with a spelling Postgres accepts and Rust's parser does not then fails the chart's whole
  note list. Pin: `note_read.rs::an_addendum_ref_in_a_non_canonical_spelling_still_reads`.

**⇒ GUI SIGN-IN SLICE 3a — THE NOTE STREAM AND ITS DRAFT (ADR-0087, db/064, db/065, db/044 + db/037 in place) — do
not undo** (pins: `crates/cairn-event/src/note.rs`'s tests, `db/tests/{064_clinical_note,065_note_draft}_test.sql`,
`crates/cairn-node/tests/{note_floor,note_sign,note_read,window_role_grants_notes}.rs`,
`cairn-gui-tauri/src/{notes,notes_webview}_tests.rs`):
- **Two replay traps: db/044 and db/037 are edited IN PLACE.** Every `db/*.sql` replays on a schema change. The
  `note-sign` widening of `ui_gesture_timing_kind_ck` lives in db/044 itself, in BOTH its `CREATE TABLE` line and its
  drop/re-add: a widening in a later file is undone when 044 replays, and the re-add then fails on a stored
  `note-sign` row. The note's shred hook is the `to_regclass`-guarded `DELETE FROM clinical_note` inside db/037's
  `cairn_execute_shred`, beside the medication tables'. **Tempting wrong fix:** "tidy" either into db/064/065. Pins:
  db/tests/065's replay of the real db/044 over a stored row; `note_floor.rs::a_shred_scrubs_the_note_projection`.
- **A spend needs its note, in the same transaction.** db/065's deferred trigger accepts a spend only if its
  `clinical.note.asserted` of the same chart has `xmin` = this transaction's id, and `note_draft_spent.event_id` is
  `UNIQUE`. So `sign_note` submits with **no savepoint**: a nested `transaction()` around `submit_event` gives the
  row a subtransaction id and every sign is refused at COMMIT. **Tempting wrong fixes:** wrap the submit "for error
  handling"; relax the check to "any note of this chart" (an old note then wipes every draft on the chart). Pins:
  db/tests/065's committed-earlier case (and 066's, against the part-aware spend); `note_sign.rs`'s `a_signed_note_is_attested_and_spends_its_draft`,
  `a_failed_submit_leaves_the_draft_in_place`, `a_second_sign_of_the_same_draft_is_refused_spent_and_writes_one_note`.
- **Drafts have no table grant.** `note_draft` and `note_draft_spent` (and since 3b-1 `note_draft_part` and
  `note_draft_revision`) are `REVOKE ALL … FROM PUBLIC` and granted to
  nobody; the doors are definers granted to `cairn_window` only. Never `GRANT SELECT` to `cairn_agent` "for a simpler
  read", and never keep a copy in browser storage (draft prose on disk in the clear). Drafts are in no backup or
  export (ADR-0087 decision 9). Pins: db/tests/065's grant block and its `cairn_window` probe;
  `notes_js_never_touches_browser_storage`; `window_role_grants_notes.rs`.
- **Never `innerHTML` a note.** A note's text is drawn through `cell` (`textContent`), never parsed as HTML or
  markdown by this window: it is shown exactly as typed. Pin: `notes_and_drafts_are_drawn_as_text` (no `innerHTML`,
  `outerHTML`, `insertAdjacentHTML`, `document.write` in notes.js).
- **Escape once, at signing; read only the projection.** Drafts hold the RAW text; `sign_note` escapes the spent
  row's text once. Escaping in the window too would double every backslash. Notes are read only from
  `clinical_note`, never from `event_clear` or `event_log`: the projection is what keeps a peer's unchecked `/2` body
  off the screen (ruling R8). `clinical.note.asserted` stays OFF db/048's no-thread list. Pins:
  `sign.rs::the_body_is_the_spent_row_escaped_once`, db/tests/064 (R8, the no-thread assert).

**⇒ GUI SIGN-IN SLICE 2 — IN-PERSON ENROLMENT AND RE-KEY (ADR-0085, db/004, db/061, db/063) — do not undo** (pins:
`crates/cairn-node/tests/{actor_rotate_key,actor_readers_after_rotation,suppression_owner_gate,vault_enrollers,vault_rekey,vault_admin_lib,restore_actor_registry_new_ops,window_role_vault_doors,device_actor_enrolment}.rs`,
`db/tests/{004_actors,063_vault_enrollers}_test.sql`, `cairn-gui-tauri/src/{signer,signer_webview,enrol,rekey,reseal,superuser_panel}_tests.rs`;
which reader asks which question: `docs/superpowers/plans/2026-11-01-slice-2-actor-reader-table.md`):
- **"Is this key current?" has three answers; ask the one you mean.** A door that lets a key author something NEW
  here reads `actor_current`. A gate that RE-ADMITS an event signed elsewhere or earlier (`apply_remote_event`, db/043's
  deferred pass, and so a restore) or RE-GRADES a stored one (`cairn_claim_authority` R1/R2, db/048's worklist) reads
  `actor_key_vouches`. A HISTORY reader filters `op IN ('enroll','rotate-key')` — never `('enroll','supersede')`: a
  `supersede` row carries no key (#666), so that filter silently drops every rotated-in key. **Tempting wrong fixes:**
  "tidy" a re-grade back to `actor_current` (a re-keyed clinician's past attestations go unverified); move
  `submit_event` to `actor_key_vouches` (a rotated-away key authors again). Pins: `a_rotated_away_key_cannot_submit`,
  `a_rotated_attester_still_vouches_for_past_claims`, `a_rotated_away_keys_past_event_is_admitted_by_remote_apply`,
  `a_recall_selects_what_both_keys_of_a_rotated_actor_signed`, `a_remote_event_by_a_rotated_in_key_is_attributed_to_its_actor`.
- **`actor_current` picks the latest row FIRST and filters OUTSIDE the subquery.** Filtering inside lets a repudiated
  or superseded latest row fall back to the older key the rotation retired. Its column list never changes
  (`CREATE OR REPLACE VIEW` cannot). Pins: `a_repudiated_latest_key_leaves_the_actor_without_a_current_key`,
  `a_superseded_actor_is_not_current`.
- **"Unconfirmed" is defined only in `cairn_key_confirmation`.** A door approved by a superuser or the console writes
  the `confirm` row in the SAME transaction; a row with `approved_by IS NULL` is `outside` the mechanism, never
  unconfirmed. Repudiation is final for the key (a later confirm is refused) and never revokes the actor. Pins:
  `confirmation_has_one_definition`, `a_superuser_enrolment_is_confirmed_at_once`, `bootstrap_writes_a_confirm_row`,
  `a_repudiated_enrollers_unconfirmed_members_are_doubtful`, `rekey_after_repudiation_restores_the_actor`.
- **Unconfirmed never blocks signing by itself.** Confirmation status alone never stops a key; only a repudiation, or
  the key's retirement (a rotation, a revoke, a supersede), does. A site's
  own expiry stops the vault RELEASING the key, not the key signing: a seed already released, or a key the window
  holds, signs until it is dropped (`submit_event` does not read `vault_policy`). The expiry is OFF by default
  (`vault_policy` empty). **Tempting wrong fix:** refuse an unconfirmed key "to be safe" —
  it sends the newcomer back to paper or to a shared login. Pins: `no_expiry_by_default`, `a_confirmed_key_never_expires`,
  `a_doubtful_key_expires_like_an_unconfirmed_one`.
- **`cairn_actor_append` refuses a clock behind the actor's latest registry row** (every op). Readers order by
  `recorded_at` first and a restore keeps the original timestamps, so a slow replacement machine would otherwise
  write a `rotate-key` that leaves the old key current, or a withdrawal that does nothing. Equal is allowed (`seq`
  breaks the tie). Pin: `a_clock_behind_the_registry_is_refused`.
- **A re-key with no member row (after a restore) needs a superuser or the console, and a person's actor**
  (`cairn_vault_install_key`). A CLI-enrolled human has the same `{role, handle}` shape, so letting an enroller take
  that branch hands them any such actor. Pin: `rekey_without_a_member_row_needs_a_superuser_and_a_person`.
- **The approver rule lives only in `cairn_vault_approver_ok`, and an enroller's act needs a current grant AND a
  confirmed (or `outside`) key** (`cairn_vault_enroller_problem`). Dropping the confirmed-key clause re-opens the
  impostor chain. An enroller never enrols or re-keys a `superuser` or an `admin`, grants, or judges; nobody re-keys a
  superuser outside the console. Pins: `an_unconfirmed_enroller_cannot_enrol`, `an_enroller_cannot_enrol_a_superuser_or_an_admin`,
  `an_enroller_cannot_grant_or_judge`, `a_stale_grant_token_is_refused`.
- **After a repudiation only a superuser re-keys, and a superuser's key is repudiated only at the console**
  ([ADR-0086](../spec/decisions/0086-who-may-act-after-a-repudiation.md), #926). `cairn_vault_rekey` reads the target's
  confirmation UNDER the actor lock (key-then-actor, `cairn_actor_append`'s order), so it cannot race the judgement,
  and locks the member ROW before both, as a superuser's re-key does (`cairn_vault_install_key`), or the two deadlock;
  `cairn_vault_judge_key` refuses `repudiate` on any actor whose pinned role is `superuser`; the owner's
  `cairn_vault_bootstrap_repudiate` (`vault-repudiate-superuser-key`) is the only way. **Tempting wrong fixes:** read
  the status before taking the lock ("it is only a read"); let a superuser repudiate a fellow superuser from the panel
  "for convenience" (one stolen key then strips every other superuser). Pins: `rekey_after_repudiation_restores_the_actor`,
  `a_repudiation_landing_mid_rekey_is_seen` (fails on the read-before-lock fix), `an_enrollers_rekey_takes_the_row_lock_first`,
  `a_superusers_key_is_repudiated_only_at_the_console`, `an_enroller_may_rekey_a_doubtful_member`,
  `the_console_repudiates_a_superusers_key`.
- **Every administrative act is signed over its own domain-separated address**, computed in SQL and fetched by Rust
  (`cairn_vault_{enrol,rekey,grant,judge,reseal}_address`). The re-key address binds the ROLE (a restored node has no
  member row to check it against); the grant address binds the prior grant `seq`; the reseal address binds the old
  blob's digest. This stops an approval token from being replayed as another act, or as a clinical event. It does NOT
  stop a client holding a released seed from signing anything: the purpose rule binds only at the vault (#900).
- **The release door's RETURN shape never changes** (`outcome, seed, signing_key_id, retry_after_ms`): a new fact is a
  new `outcome` (`replaced`, `expired`) or a companion function. `replaced` is counted exactly like `wrong` and given on
  a FAILURE only; `expired` and `not-current` come before the count. Pins: `rekey_keeps_the_actor_and_retires_the_old_key`,
  `an_expired_unconfirmed_key_is_not_released`.
- **The rules the release and enrol doors call live in db/061, AHEAD of the doors**, not in db/063: on an upgrade's
  replay a door calling a function a later file defines raises "function does not exist" at a concurrent sign-in
  (ADR-0083's never-raises promise). **Tempting wrong fix:** move them to db/063 "where the slice lives" to shorten
  db/061.
- **A change of passphrase checks the current one through the counted release door** (purpose `reseal`), never a
  second, uncounted check. Pins: `reseal_releases_the_key_for_reseal_only`, `reseal_needs_the_members_own_proof`,
  `a_wrong_current_passphrase_changes_nothing`.
- **Every vault answer that becomes a person's sentence goes through `signer::from_release_noting`**, or a replaced
  passphrase reads "not right" again. Pin: `a_vault_answer_reaches_the_person_through_from_release_noting`.
- **The re-key form asks whether the staff ID has a vault row BEFORE any key is released**
  (`vault::staff_id_taken`, then `rekey::missing_member_details`): with none (after a restore) only a superuser may
  re-key, and a name and a contact are required; either is refused in a plain sentence, and the approver spends no
  release. Never decide it by parsing the
  door's refusal. Blank fields with a row keep the stored values (sent as NULL). Pins:
  `a_rekey_with_no_member_row_needs_a_name_and_a_contact`, `an_enroller_cannot_rekey_a_member_with_no_row`,
  `the_rekey_runs_its_steps_in_order`,
  `the_rekey_form_sends_its_name_and_contact_fields`. The success line says whether a superuser must still confirm,
  by who approved (`the_success_line_says_whether_a_superuser_must_still_confirm`).
- **Every member row of the grant panel offers *Repudiate key* on that member's CURRENT key, carried by the row**
  (`vault_admin::enrollers` reads `cairn_vault_member_key` and the key's status as companion calls; never widen
  `cairn_vault_enrollers`' `RETURNS TABLE`). The worklist alone cannot reach a confirmed key or an enroller's own key,
  so without it the cascade is unreachable from the window. A repudiated key is said so and not offered again.
  **Tempting wrong fix:** "tidy" the panel to Repudiate from the worklist only. Pins:
  `the_grant_list_names_each_members_key_so_a_confirmed_key_can_be_repudiated`,
  `a_grant_row_offers_its_members_key_to_repudiate_unless_already_repudiated`, `a_grant_rows_repudiate_uses_the_rows_own_key`.
- **A judgement acts only on the person's CURRENT key** (`cairn_vault_judge_key`, under the actor lock a rotation
  takes). The panel reads its lists once; a re-key in between leaves a row naming the old key, and judging it would
  repudiate the genuine person's rotated-away key and leave the new one live. The page names the judgement
  (`"confirm"` / `"repudiate"`, `superuser_panel::Judgement`), never a bare boolean; a floor refusal gets its own
  sentence (`REFUSED`, "the lists may have been out of date"), and a failed list re-read never says "nothing was
  recorded" (`READ_FAILED`) and drops the rows. **Tempting wrong fix:** let the floor judge any key the person ever
  held "so #922 has a path" — a prior key needs its own door. Pins: `a_rotated_away_key_cannot_be_judged`,
  `repudiate_on_a_stale_row_is_refused_and_says_so`, `a_judgement_is_named_never_a_boolean`,
  `a_failed_write_says_which_kind_of_failure`, `a_failed_list_read_never_says_nothing_was_recorded`.
- **The window finds the approver's grant row by the vault's own key** (`enrol::staff_id_key`: full Unicode lowercase,
  spaces trimmed, as `cairn_vault_staff_id`), never an ASCII-only fold. A granted enroller whose key was repudiated
  is told to get a re-key, not to wait for a confirmation; a listed member without the grant (an admin, whose role
  serves the enrol purpose) is told plainly after the release. Pins: `standing_folds_case_as_the_vault_does`,
  `a_repudiated_enroller_is_told_to_get_a_new_key`, `a_listed_member_without_the_grant_is_refused_after_the_release`.
- **A key that is no longer current is told to ask for a re-key**, never to be enrolled again (a new staff ID is a
  second actor for one person). Pin: `a_key_no_longer_current_is_told_to_ask_for_a_re_key`. Both contact fields say
  the contact must work without any hospital login (`both_contact_fields_say_it_must_work_without_a_hospital_login`).
- **Repudiate asks no question** (principle 3). Pin: `repudiate_asks_no_question`. The expiry warning names the staff
  ID and is cleared with it, so the next person never reads it as theirs. Pin:
  `the_expiry_warning_is_asked_for_the_typed_id_before_the_act`.
- **The suppression owner gate compares ACTORS, not keys**, and a suppressor key resolving to zero or several actors
  refuses. Pins: `a_rekeyed_author_may_suppress_their_own_earlier_note`, `cross_human_suppress_refused_after_author_key_rotation`,
  `a_repudiated_key_may_not_suppress_its_actors_note`, `a_rotated_in_keys_note_cannot_be_hidden_by_another_human`.

**⇒ THE WINDOW'S OWN LOGIN (ADR-0084, db/062, #898/#899) — do not undo** (pins:
`crates/cairn-node/tests/{conn_posture_db,window_role_grants,provision_window_role}.rs`,
`crates/cairn-node/src/{conn_posture,window_access}_tests.rs`, `cairn-gui-tauri/src/launch_posture_tests.rs`):
- **The over-power check runs on EVERY `release_for_act`, is never cached, and is judged over EVERY role the login is
  a MEMBER of** (`pg_has_role(…, 'MEMBER')`, so a `NOINHERIT` membership counts: a member can `SET ROLE`). An operator
  fixes a login while the window's connection stays open (#807). **Tempting wrong fixes:** read the facts once at
  launch "to save a query"; test the login's own privileges only (a member of the owner role passes). Pins:
  `a_member_of_the_owner_role_is_refused_until_the_membership_is_revoked`,
  `a_membership_that_does_not_inherit_still_counts`. The refusal comes BEFORE the passphrase is sent
  (`a_wrong_passphrase_over_an_over_powered_connection_is_never_counted`).
- **The window never calls `connect_and_load_schema`** (its login cannot replay DDL; it reads the stamp and refuses
  the LAUNCH on a mismatch). A reintroduced loader also silently restores owner-level use. Pin:
  `the_window_never_loads_the_schema` (walks every non-test source of `cairn-gui-tauri`). The window no longer runs
  the loader's deferred re-adjudication pass; that is the loader's, not a bug to patch back into the window.
- **`cairn_window` never gains a vault read, `cairn_vault_unseal`, `INSERT` on `event_log`, or a peer door**, and never
  inherits `cairn_node`. A missing grant is added narrowly (a column list, not the table) with its reason, and the
  entry point is added to `window_role_grants.rs`. Pins: `the_window_login_has_none_of_the_powers`,
  `each_power_alone_is_seen`, and the four `the_windows_*_run_under_its_own_login` tests.
- **`provision-window-role` refuses a cluster BEFORE it mutates, and never touches an existing role that is not a window
  login.** Its role DDL is one transaction, so a failed step leaves no half-made login. Order: `require_local`, the rules read, freshness, the `trust` verdict, THEN the schema load and the role
  DDL. `trust` is refused whatever its line's type or database; `peer` without a map only warns. No override flag, for
  this refusal or for `require_local`. Pins: `the_verb_refuses_before_it_mutates_in_source_order`,
  `the_verb_refuses_a_trusting_cluster_and_changes_nothing`,
  `an_existing_role_that_is_not_a_window_login_is_refused_and_untouched`; the window's own launch refuses a remote
  database first (`the_remote_refusal_comes_before_any_connect`).

**⇒ GUI SIGN-IN SLICE 1 — THE HUMAN KEY VAULT (ADR-0083, db/061) — do not undo** (pins:
`crates/cairn-node/tests/vault_floor.rs`, `vault_lib.rs`, `secret32_conversions_are_named.rs`;
`cairn-gui-tauri/src/{signer_tests,enrol_tests}.rs` and `state.rs`'s held-key tests):
- **Never grant a read of `vault_member`** (nor of `vault_attempt`/`vault_terminal`), and never add a function, view
  or door that returns `sealed`. A blob that leaves the database is an OFFLINE guessing target no back-off reaches.
  (No grant binds the owner or a PostgreSQL superuser. The window connected as the owner when this slice shipped; since ADR-0084 it runs on its own login — see that block above.)
  `cairn_vault_unseal` stays revoked from PUBLIC: for every role but the owner, `cairn_vault_release` is its only
  door, and db/061 refuses to load if that REVOKE did not take (a non-owner loader's REVOKE is a no-op; pin `the_schema_refuses_to_load_while_the_runtime_role_can_unseal`). Defence in depth (the runtime role cannot read a blob to feed it), and only on a replay: a `cairn_pgx` reinstall re-grants PUBLIC unseen (#908). Pin:
  `the_runtime_role_cannot_read_the_vault`. **Tempting wrong fix:** a "roster" view for an admin screen that
  selects `*` — select the named non-secret columns through a definer, or nothing.
- **Never make `cairn_vault_release` raise on a passphrase.** Wrong passphrase, unknown staff ID, wrong purpose and
  a wait are all ANSWERS (`outcome` rows): a raised error can carry the bind parameters, the passphrase, into the
  server log. Same for `cairn_vault_unseal` (NULL, never `pgrx::error!`). Pins:
  `a_wrong_passphrase_is_an_answer_not_an_error`, `refusal_sentences_never_echo_the_passphrase`. **Tempting wrong
  fix:** `RAISE EXCEPTION 'wrong passphrase'` "for clarity". The window builds its sentence from `KeyRefusal`.
- **Never let a superuser key out for `'clinical'`** (nor a clinician key for `'enrol'`, unless it passes slice 2's
  enroller rule — see that block; `admin` is administrative-only like `superuser`): the release door maps
  role → purpose BEFORE unsealing. Pins: `a_superuser_key_is_not_released_for_a_clinical_act`,
  `a_clinician_key_cannot_enrol`, `the_superuser_key_is_never_held` (enrolment bypasses `signer_for_act`, which may
  hold keys). The rule binds only at the vault: a client holding a released superuser key could still sign a
  clinical event (ADR-0083 decision 7, #900) — do not read the vault rule as the whole defence.
- **Never take a human `&SigningKey` in new code** — take `&dyn EventSigner` (`cairn_event::signer`). Slice 5's
  in-database signer is a new implementation of that trait; a call site typed on `SigningKey` is one more to rewrite.
  Pins: `a_refusing_human_signer_seals_and_signs_nothing` (cairn-node; builds `AuthorParams` over a signer that is
  not a `SigningKey`, so the field's type is a compile-time pin) and `a_refusing_signer_signs_nothing`
  (cairn-event); a new `&SigningKey` parameter elsewhere is still held only in review.
- **Count before unsealing, with `setval`, in the per-member sequences.** Moving the count after the unseal, into a
  table column, or to `nextval` re-opens a bypass (rollback) or a phantom-failure lock-out after a crash. A missing
  `vault_attempt` row answers `unknown`. Pins: `a_rolled_back_guess_still_counts`, `an_erroring_statement_still_counts`,
  `a_count_is_logged_exactly`, `a_missing_attempt_row_fails_closed`, `waiting_does_not_count_a_failure`.
- **The attempt row's `FOR UPDATE` lock is load-bearing.** The door counts with an absolute
  `setval(mark + failures + 1)`, so without the lock N terminals reading the state together each get a guess that
  is counted once. Pin: `a_second_terminal_waits_for_the_attempt_row_lock` (the older concurrency test passes with
  the lock deleted). **Tempting wrong fix:** dropping `FOR UPDATE` "because sequences are atomic anyway".
- **A revoked or superseded member's key is not released** (`not-current`, before counting). Pins:
  `a_revoked_member_is_told_so_and_nothing_is_unsealed_or_counted`, `a_revoked_superuser_cannot_enrol`.
- **Staff-ID normalisation lives only in SQL** (`cairn_vault_staff_id`); Rust asks for the enrolment address
  (`SELECT cairn_vault_enrol_address(…)`) and the pinned handle (`cairn_vault_staff_id`) rather than computing a
  twin; both enrol doors rebuild the pinned set `{role, handle}` and refuse any other. Pins:
  `release_normalises_the_staff_id`, `staff_id_taken_normalises_and_answers`,
  `both_doors_refuse_a_pinned_set_they_did_not_build`, `a_case_variant_of_a_staff_id_mints_no_second_actor`.
- **Never send an empty passphrase to the vault** (it is a counted wrong guess), and never enrol one sign-in would
  refuse: one rule, `cairn_node::vault::passphrase_is_usable` (spaces alone are none), at sign-in and at every
  enrolment door (`mint_sealed` itself refuses). Pins: `a_passphrase_of_spaces_is_not_usable`,
  `a_key_is_never_sealed_under_a_passphrase_of_spaces`, `a_passphrase_of_spaces_is_refused`. `signer::decide`
  yields `Release` only with a passphrase; a staff ID alone uses the holder's live key or forgets it. The strip keeps the ID on any
  terminal with a hold (`keyHoldSecs`, not `heldBy`, which the post-act poll has not refreshed yet). **Tempting wrong
  fix:** "simplify" `decide` back to "any credential means a release". Pins:
  `an_empty_passphrase_is_never_sent_to_the_vault`, `a_staff_id_alone_never_asks_the_vault`,
  `clear_signer_empties_the_passphrase_and_keeps_the_id_only_on_a_held_terminal`.
- **A future last-attempt stamp is not a wait.** `cairn_vault_release` waits only when the stamp is not after now; a
  clock stepped back would otherwise lock a member out for as long as it stepped. The guess it lets through restamps
  the clock at now, so it frees ONE guess. Pins: `a_clock_stepped_back_never_locks_out`,
  `a_clock_stepped_back_frees_one_guess_and_no_more`.
- **A late release never re-stores a key** (`Held::keep_for_next_act` / `store_if_current` and its generation;
  every wipe bumps it, an enrolment's included). Pins: `a_lock_between_the_wipe_and_the_store_drops_the_released_key`
  and its neighbours in `state.rs`, `signer_for_act_keeps_the_key_with_the_generation_of_its_own_wipe`,
  `enrolling_wipes_a_held_key`; `a_new_signer_replaces_the_held_one`, `an_expired_hold_needs_a_credential`.
- **A failed enrolment with no ordinary answer is "may have been enrolled"**, never "nothing was recorded" (a retry is
  told the ID is taken, and a new ID makes two actors for one person). `vault::enrol_member` keeps the database
  error on the chain (`LocalDbFault`) so `db_diagnosis::answered_with_error` can tell. **Tempting wrong fix:**
  `anyhow!("…: {}", db_text(&e))`, which drops the error and makes every failure look unanswered or every one
  answered. Pins: `a_failed_enrolment_says_whether_the_server_answered`,
  `an_unanswered_enrolment_failure_is_outcome_unknown`.
- **The superuser's and the terminal's passphrases never share an env variable** (`CAIRN_VAULT_PASSPHRASE` vs
  `CAIRN_TERMINAL_PASSPHRASE`). Pin: `the_two_vault_passphrases_never_share_an_env_variable`.

**⇒ #807 — THE WINDOW'S CONNECTION IS REOPENED WHEN LOST — do not undo** (pins:
`cairn-gui/cairn-gui-live/tests/reconnect.rs`, `connection.rs`'s backoff and wording unit tests and `Send` pin):
- **Reopen only when the client `is_closed()`, and only when the lock is taken** (`NodeConnection::lock`,
  `cairn-gui-live/src/connection.rs`). Never on an error: a refusal is an answer on a live connection. Never
  mid-command, and never a retried write: a commit with no answer stays unknown, and the next command just gets a
  working connection for the re-read once the node answers. A failed or unanswered attempt (5 s cap) starts a 2 s
  backoff, kept in the same lock as the client.
- **A silently dead socket is noticed by TCP keepalives, set for every connection either binary opens** (#875,
  #878; the `cairn-db-keepalive` crate: idle 30 s, 3 probes 10 s apart, `tcp_user_timeout` 30 s on Linux only).
  `cairn_node::db::connect` applies them for the window, the CLI and `cairn-node sync`; `cairn-sync` builds every
  production connection from `db_config(conn)`. A setting the connection string names wins, unless it is zero
  (tokio-postgres drops a zero, so ours applies). Pins: `db_connect_applies_the_dead_peer_settings` (db.rs, by
  source), cairn-sync's `keepalive_tests` (the config read back; every production `NoTls` connect in `src/*.rs`
  matched by a `db_config(` call).
  **Tempting wrong fixes:** setting them in the window only — the CLI and `cairn-node sync` hang on the same dead
  socket; a new `postgres::Client::connect(conn, NoTls)` in `cairn-sync` — it compiles and silently drops them.
- **A command holding the connection past `NodeConnection::ANSWER_WITHIN` (15 s) is cut off** (#875 option 2):
  every guard `lock`/`lock_if_open` hands out (`ConnectionGuard`) starts a watchdog that, at the limit, ABORTS the
  client's connection task (`cairn_node::db::connect_with_task`'s handle), so the stuck query fails `connection
  closed` and nothing downstream is new: a cut-off COMMIT is "not known" via `commit_failure`, and the next `lock`
  reopens. The whole HOLD is timed, not one statement. The watchdog marks the client cut BEFORE aborting
  (`Cutter::cut_off`; `Held::is_lost` reads the mark), because `is_closed()` turns true only once the task is
  dropped; and the watchdog and the guard's drop each CLAIM the hold first (`Watch`), so a guard dropped as the
  watchdog fires still marks the client before the lock is released. Pins: `connection/tests.rs`
  (`a_client_marked_cut_is_lost_before_its_task_has_died`, the two claim tests). **The reopen after a cut ENDS the
  old backend first** (`Backend::end_on`: `pg_terminate_backend(pid, 3 s)`, matched on pid AND `backend_start`):
  the server never hears of the cut, so a cut COMMIT may land after a re-read on the fresh connection said "not
  recorded", inviting a second entry (PR #892 review). A backend that will not end fails the press. Pin:
  `a_commit_cut_off_is_not_known_and_its_re_read_is_final`. The window's connection is opened apart from the
  schema load for that handle. The webview's waiting bar
  (`src-ui/waiting.js`) shows only after 2 s and runs out at the same 15 s (`NODE_ANSWERS_WITHIN_MS`), then says
  "Reconnecting…". Pins: `tests/answer_limit.rs` (mutation-checked: without the abort both cut-off tests fail),
  `waiting_webview_tests.rs`. **Tempting wrong fixes:** a `tokio::time::timeout` around a call site — dropping the
  query future drops the guard, which cancels its watchdog and leaves the client stuck on the dead socket, so the
  next command hangs the same way (a caller that must bound its WAIT spawns the work and times the task, as
  `LiveData::standing` does for `probe_within`); a SERVER `statement_timeout` alone — a dead link never delivers
  the server's timeout error; a reopen that reads at once after a cut — the cut-off backend may still commit; lowering 15 s — a node alive but slow is cut off
  too, and its commits read "not known"; a watchdog on a `fixed` connection — aborting it kills it for good.
- **A cut-off command's error SAYS the window cut it off** (#893): tokio-postgres hands every pending query of an
  aborted task the same bare `connection closed` as a server restart, so the marker is added afterwards by the
  one party that knows — every call site passes its error through `ConnectionGuard::explain` (or a `CutWatch`
  taken from the guard) **while the guard is still held**, and an outcome line's cause is
  `cairn_gui_live::cause_of(&e)`, never `e.root_cause()`. A failure the node already worded as TEXT inside an `Ok`
  outcome (a sign-off's stop AND failed lines — `name_the_cut`; a worklist side — `name_cut_in_sides`) gets the cut
  named by `name_cut_in`, which touches only a text ending in `connection closed`; one inside an outcome as an
  error (a "Different people" pair) is explained one by one. The sentence says the command *was not finished*
  within the limit, never that the node *did not answer*: the whole hold is timed, and a command of many answered
  statements is cut too (#895 review). The marker is a context layer (`CutOff`), so classification is unchanged: a
  cut read stays an outage, a cut COMMIT stays "not known". Pins: `answer_limit.rs`'s `a_cut_off_command_says_…`
  (which also pins the `connection closed` ending against a real cut), `a_cut_off_search_says_…` and the commit
  test's class/cause asserts; `cut_off.rs`'s unit tests; `a_cut_off_sign_off_names_the_cut`,
  `each_judged_pair_failure_is_explained`, `a_side_the_window_cut_off_names_the_cut`. **A new `NodeConnection`
  call site must do the same**: `node_lock_explains_tests.rs` fails any function in cairn-gui-tauri or
  cairn-gui-live that locks the node and never explains (exemptions in its `EXEMPT`, each with why) — but per
  FUNCTION, so a second error path forgotten beside an explained one still passes. **Tempting wrong fix:** reading
  a `CutWatch` after the guard drops (or holding one across an await): the flag is the CLIENT's, not the hold's, so
  a later hold's cut can set it and name a cut this error never had.
- **A failed reopen is an ERROR that says why** (#876): `lock()` returns `Err(ConnectionLost)` naming the cause;
  a press inside the backoff tries nothing and says when the last attempt failed. It is an outage (`Retry::Now`:
  `From` into `DataError::Unavailable` and into the window's `ErrorView`, "Nothing was done: …") unless the server
  refused the reopen (#880, below). **Tempting wrong
  fixes:** `?` inside a per-thread write loop (cease, release) — an earlier thread may be written, so the lock
  failure becomes THAT thread's `Err` and is reported with the others; and gesture timing taking `lock()` — it uses
  `lock_if_open`, so it cannot spend the reopen attempt the act's re-read needs. Pins: `reconnect.rs`'s
  `lose_then_press_five_times` assertions and `lock_if_open_never_spends_a_reopen_attempt` (the primitive);
  `a_lost_connection_fails_one_thread_and_timing_never_reopens` (write_failure_tests.rs, a source guard: both
  wrong fixes compile).
- **A reopen the server REFUSED is an operator's, not a retry** (#880): class `28`, `3D000`, `42501`,
  `55000` (`server_refused`) make `ConnectionLost` classify as `DataError::NotProvisioned` (`Retry::AfterOperator`); busy or
  starting up (`53300`, `57P03`) stays an outage. The code survives `db::connect` as `cairn_node::db::ConnectFailed`
  (text unchanged, `sqlstate()` beside it, NO `source()`); `refused` is stored WITH the cause, so a press inside the
  backoff says "operator" too; `data_error_from` downcasts `ConnectionLost` first (cease and release wrap it in
  `anyhow`); `read_chart_of` returns `anyhow::Error`, so `chart_read_failed` sees it too — that read is where the four
  write gestures (sign-off, cease, withhold, release) REOPEN, so it is the route that matters most. **Tempting wrong fixes:** `.context()` over the `tokio_postgres::Error` in `db::connect` — `main`'s
  `{e:?}` then loses the `[SQLSTATE]` (`db_diagnosis.rs`'s 3D000 test fails); classifying only the press that tried.
  Pins: `a_reopen_the_server_refuses_asks_for_an_operator_on_every_press` (reconnect.rs, a real `3D000`),
  `a_refused_reopen_names_the_operator_by_every_route`, `only_a_refusing_server_needs_an_operator`,
  `a_chart_read_lost_to_a_refused_reopen_needs_an_operator` (also a source guard: no `to_string` in `read_chart_of`).
  **A refusal tokio-postgres raises itself** ("password missing", an unsupported auth method, a SCRAM failure) has
  no code: `ConnectFailed::refused_by_client()` marks it from the error's rendered category (`Kind` is private), and
  the reopen classifies it as `Refusal::Client` beside `server_refused`'s `Refusal::Server` (#884). Its text must
  never say the node refused (`Refusal::clause`): an operator would look for a rejected login the node never logged,
  and a SCRAM failure is the CLIENT rejecting the server's proof. **Tempting wrong fix:** adding the words of `Connect`,
  `Io`, `Timeout` or `Closed` to `client_refusal` — those are outages the next press may get past
  (`a_node_that_hangs_up_is_an_outage_not_a_refusal`, no database needed). Pins: `a_refusal_the_client_raises_itself_is_marked_as_one`
  (db_diagnosis.rs, a fake node driving the real library, so an upgrade that rewords a kind fails it),
  `a_reopen_the_client_cannot_authenticate_asks_for_an_operator` (reconnect.rs), `a_client_refusal_is_not_said_to_be_the_nodes`.
  `cairn_gui_live::failed_reopen_cause` reads any reopen text back for a line that is not a press (#888). **Every
  `NotProvisioned` renderer must be true of a refused reconnect too**: the front door's search arm keeps "The search
  FAILED — this is NOT a "no match"" (`a_search_that_needs_an_operator_still_never_reads_as_nothing_found`), and
  the register arm claims nothing about the node's enrolment. Before #880 the search arm was bare text — safe only
  while no live path reached it.
- **A refused reopen says its remedy ONCE** (#885): its text ends in `OPERATOR_REMEDY` (`connection.rs`); a renderer
  wrapping a `NotProvisioned` asks `says_operator_remedy` and does not restate it (register and search arms, the
  judgement arm, which then drops its "yet"). Other causes keep their wrapper's remedy: an enrolment refusal does not
  say what pressing again does, and the judgement arm's not-held cause resolves by sync. The stale-list banners
  (`chart_stale.js`) name "Read this chart again" but never order a press: they cannot tell a refused reopen from an
  outage. Pins: `only_a_refused_reopen_says_the_operator_remedy`, `a_refused_reopen_s_remedy_is_said_once_at_the_front_door`,
  `every_data_error_arm_is_worded_for_a_link`.
- **A search clears the front door's provisioning line only by asking the node again** (#882): `funnel_status` probes the
  standing on EVERY call (`FunnelBackend::provisioning`; there is no launch-time copy), and `funnel.js`'s
  `recheckProvisioning` asks after a search that succeeded while the line shows a warning. **Tempting wrong fix:**
  clearing the line on any successful search — an unenrolled node searches fine, so that wipes a true warning. Pin:
  `a_search_clears_the_provisioning_line_only_by_asking_the_node_again`. A registration that succeeded clears it too: that is the
  node's answer as well. The probe is bounded (`probe_within`, `PROBE_WITHIN`, twice a reopen's wait) because `boot()` shows the front door only
  after it answers; what it waits on, `LiveData::standing`, runs as a task of its own, so giving up never drops a
  connection guard mid-query (PR #892 review): pin `funnel_status_probes_the_live_node_afresh_and_bounded`.
- **The reopen is `cairn_node::db::connect_with_task`, not `connect_and_load_schema`.** The schema was checked (never loaded) at launch, and a
  replay takes DDL locks a read never needs (#726). It also skips the deferred-event re-adjudication, and since
  ADR-0084 so does the window's launch: only one-off loader verbs run it (#912). Unpinned — the tempting "keep it the same as launch" fix.
- **`LiveData::new` is `NodeConnection::fixed`**: a caller-made client may carry session state (`lost_commit.rs`'s
  kill switch is a `SET`) that a fresh one would silently lack. Only the window's launch uses `reopening`.
- Not checked on reopen: that the database is still the SAME node. Writes there are expected to be refused if this
  window's key is not enrolled; reads are not checked at all. A launch against the wrong database has the same
  exposure.

**⇒ #863 + #864 — A SHRED HANDS ITS ROWS ON; A LIST IS ONE SNAPSHOT — do not undo** (pins:
`crates/cairn-node/tests/shred_refolds_overlay_winner.rs`, `tests/medication_list_one_snapshot.rs`):
- **A shred ends by re-folding the threads its event named (#863)** — `cairn_execute_shred` (db/037) deletes only
  the rows its event produced; for a one-winner projection whose winner that was, the key was left EMPTY (shred the
  newer of two stops: the drug read current). `cairn_refold_after_shred` then re-runs the heal-safe appliers of the
  other eligible events that can win the same rows: same threads (`cairn_medication_threads_named`: `medication_id`,
  `subject_a`, `subject_b`, CANONICALISED — the floor admits any uuid spelling) and a type sharing a non-`*_flag`
  projection table with the shredded type (the registry's `projection_tables` — exhaustive by CONVENTION only, which
  db/039's narrow rebuild also relies on). A NEW medication event type naming a thread under another key must be added
  to the helper. The TARGET is excluded by id: a sealed target names no thread once its clear view is deleted, but the
  sync door also shreds a peer's PLAINTEXT event, whose body still names its thread (PR #865 review). An EARLIER-shredded
  plaintext sibling is still replayed, as `cairn_reproject`'s heal does — **#866** (filtering it needs the shred log,
  whose predicate `shred_predicate_has_one_home.rs` keeps in one place). A sibling whose replay raises aborts the shred;
  a sibling's refusal is re-raised BARE (still P0001 — `USING ERRCODE` is forbidden in db/ by
  `floor_refusals_carry_no_errcode.rs`) naming the sibling; any other code propagates untouched (whether it should block the erasure at all: **#867**). It runs lenient
  (`cairn.remote_apply = on`) and puts the caller's posture back — left `on`, a later write in the strict door's
  transaction skips its raise-not-clamp checks. It WAITS on each sibling's custody lock; shreds are serialized first
  (`cairn_lock_shred_refold`, before the custody lock) so two never wait on each other; a door meeting a shred in a
  cycle gets a transient 40P01 (db/005's deadlock note). Cost: a scan of medication events' clear payloads per shred
  (`shortcut:`). **Tempting wrong fixes:** "never over-erase" read as "nothing to restore"; TRY the sibling's lock and
  skip it (a holder that rolls back or only re-receives the event writes nothing — PR #865 review); compare thread
  ids as raw text; run the whole-log `cairn_reproject` from a shred. Pins:
  `shredding_the_newest_stop_leaves_the_earlier_stop_in_force`, `shredding_the_winning_coding_restores_the_earlier_coding`,
  `shredding_a_separation_restores_the_reconciliation_it_overrode`, `the_refold_puts_the_callers_apply_posture_back`,
  `a_thread_is_matched_however_its_uuid_is_spelt`, `a_shred_waits_for_a_sibling_whose_holder_rolls_back_then_folds_it`,
  `a_peers_shred_of_a_plaintext_stop_stays_scrubbed`, `a_deferred_sibling_is_not_promoted_by_a_shred`,
  `a_peers_flagged_stop_is_replayed_lenient_so_the_shred_goes_through`,
  `a_sibling_that_fails_to_replay_is_named_and_keeps_its_class`.
- **Every production medication-list read runs in ONE snapshot (#864)** — `medication::snapshot::in_snapshot`
  (`REPEATABLE READ READ ONLY`): the CLI's `medication-list` and both cease notes, sign-off's two reads, the
  window's `read_chart_of`. The readers keep `GenericClient` so tests can still pass a bare client or an aborted
  transaction (the notes' failure seam). Sign-off's two reads are two snapshots ON PURPOSE (comparing them is the
  point, #335), and its writes stay READ COMMITTED: a door write at REPEATABLE READ reopens the custody/shred race
  (db/005's `cairn_lock_custody` note). **Tempting wrong fix:** one snapshot spanning sign-off's reads AND writes.
  Pins: `a_stop_committed_after_the_read_began_is_not_seen`, `production_list_reads_run_in_one_snapshot` (counts six call
  sites — a new production reader is added there), and `snapshot.rs` in `db_errors_stay_legible.rs`'s
  `UNMARKED_COMMITS` (a read-only commit).

**⇒ #832 — THE WINDOW'S WITHHOLD (ADR-0081 slice B) — do not undo** (pins: `cairn-gui-tauri/src/hold/plan_tests.rs`,
`hold/command_tests.rs`, `src/holds_webview_tests.rs`, `cairn-gui-tab-medications` `hold_view.rs`,
`crates/cairn-node/tests/ui_timing.rs`):
- **A Release sends only the hold ids the row DREW.** A fresh read at press time would end a hold a colleague
  wrote after the clinician last looked, unseen. **Tempting wrong fix:** have `release` re-read the holds "so it
  is never stale". Pins: `a_release_ends_only_the_holds_it_was_shown` (+ `a_hold_that_ended_meanwhile_is_named_gone`),
  and the webview guard `a_release_sends_only_the_holds_it_drew` (the only `invoke("release"` sends the ids it was handed).
  **And the report SAYS the unseen hold is still on the line** ("Released 1" alone reads as "givable"): `ReleasePlan::unseen`
  counts holds on the line not in `shown` (pending too, so the words are "still on this line", never "in force",
  #839), and the stale refusal for an all-gone Release says it too (`all_shown_gone`). Pins:
  `a_hold_not_shown_is_counted_unseen`, `a_release_says_a_hold_it_was_not_shown_is_still_on_the_line`,
  `a_release_with_nothing_left_to_end_names_an_unseen_hold`.
- **A cease or release that wrote nothing is a refusal (`Err`), not a report (#840).** When every write failed and
  none is unknown, `retry_when_nothing_written` gives ONE class — **stale wins** (maintainer's ruling), else the
  first failure's — and the text is the same outcome sentence. As a report it wiped the typed reason and lost the
  stale class. A gesture whose threads were all HELD BACK (no write tried) still returns its report. **Tempting
  wrong fix:** return the report "so the outcome line names every thread" — the `Err` text already does. Pins:
  `a_gesture_that_wrote_nothing_takes_one_class_and_stale_wins`, `a_gesture_that_wrote_nothing_is_a_refusal_with_its_class`.
- **Withhold and Stop are drawn only where their command can write (#841)** — `chart_pane` asks `withhold_target`
  and `cease_plan` with the opened chart, so a hazard line with nothing on this chart offers no press that can only
  refuse or stop nothing — and SAYS where to go instead (`NOT_ON_OPENED_CHART`): the refused press used to. Hazard
  lines only: an ordinary line `withhold_target` refuses is an anomaly (a reconcile raced the list's reads), and the
  press names it where a hidden button says nothing. A cease that can write nothing answers before the key, marked
  behind the record (as release); one that stopped nothing is no timing sample. **Tempting wrong fixes:** compute
  either from `status` in `build_row` (it has no opened chart); narrow every row on `.is_ok()`. Pins:
  `withhold_and_stop_are_offered_only_where_they_can_write`, `the_refusals_come_before_the_key`.
  **Release too (#844):** `chart_pane` asks `release_plan` with EVERY hold the row drew (the ids the window
  sends) and hides Release when it would write nothing. Its own sentence (`HOLD_NOT_ON_OPENED_CHART`), because
  Withhold and Stop may still stand on that line, where "no record on the chart you opened" would be false. Said
  only when every hold is on a MEMBER thread: a non-member's hold sits on a chart outside the record, may be another
  person's own W, and "release it from its own chart" would send the clinician to end it. Separation speaks
  instead, and it speaks ON THE ROW (the window's outside-set sentence carries `SEPARATION_INSTRUCTION`, as the
  CLI's does): the report covers only withheld lines, so a signed or ceased line would otherwise show a W with no
  Release and no way forward (PR #845 review). Pin: `release_is_offered_only_where_it_can_write`.
- **A synced event naming another chart makes its line a hazard (#836)** — `WrongChartReasons::patient_conflict`,
  read from db/031's `medication_patient_conflict_flag` (`read.rs` `read_conflict_charts`). The sync door
  converges-and-flags, the event is projected anyway, so another person's W, release, dose or stop sits on the
  line; the flag reached no screen before. A linked chart is the same person (no hazard) unless the set holds a
  doubted link, and then ANY conflict is a hazard (db/031 flags only two different charts, so "unless the named
  chart is the opened one" could never apply). Remedy by STATUS (`conflict_remedy`): a current line is
  stopped and recorded again (`CONFLICT_INSTRUCTION`) — never separation (the line may be one thread); a ceased
  line gets `CONFLICT_CEASED_INSTRUCTION` (the stop itself may be the stray event, and it may be the line the
  clinician stopped to clear it). "Cannot be signed" only where `withheld_reasons` withholds the line (the #717
  lesson), and the window puts the remedy ON THE ROW, because its report covers withheld lines only. **Trap:** `medication_group_member` has NO row for a
  thread no reconciliation touched, so the read is `LEFT JOIN` + `COALESCE(group_id, medication_id)`; the inner
  join first written missed every ordinary line, and only the DB pin caught it. Pins:
  `tests/medication_wrong_chart_hold.rs`, `hazard.rs` `a_conflict_*`.
  **Arrival order (#846):** the flag is written only when the thread's chart is ALREADY known, and a dose, dose
  correction or release never makes it known, so one that arrived first is never flagged. db/059's
  `medication_thread_stray_chart` view finds it at read time and `read_conflict_charts` unions it in. **Tempting
  wrong fixes:** "the flag table is the conflict record" (it is order-dependent; reading it alone re-opens the gap);
  "the view replaces the flag" (the view sees only doses, corrections and releases; a stray CLAIM — assert, stop,
  coding, W — is only ever in the flag); dropping the `WITH t` filter (the view then calls
  `cairn_medication_thread_patient` on every dose on the node), or narrowing it to the group ids (a stray on a
  reconciled line's non-key member is missed). The stray can come first on the LOCAL door too: the dose door does not
  check that a thread exists. Pins: the `a_stray_*` tests, `the_stray_chart_view_names_only_a_known_chart_that_differs`,
  and `the_runtime_role_reads_a_list_whose_conflict_comes_from_the_view` (tests run as the owner; only this catches a
  missing `cairn_agent` grant).
- **The chart an event was recorded FOR hears about it too (#848)** — `PatientMedicationList::elsewhere`
  (`medication/elsewhere.rs`), every renderer through `elsewhere_report` (CLI list, the window's `chart-elsewhere`
  paragraph, the compare panel's notes). The list selects lines by the thread's CURRENT chart, so a thread a later-HLC
  assert moved to another chart, or this patient's dose/W/stop on another chart's thread, had no line here and the
  conflict read (#836) never looked: the drug vanished unsaid. Read-side only (maintainer, 2026-10-23); the overlay
  still moves the thread (#192's converge contract untouched — freezing it only moves the blindness to the other
  chart). **Tempting wrong fixes:** reading only the flag's `asserted_patient` (the moved-thread case is
  `standing_patient`; a thread claimed by a stop, W or coding — anything but an assert — has no point-0 dose row, so
  the view cannot see it either); reading the stray view's `standing_patient` (that is the thread's chart NOW — a set
  chart there means the thread is this set's, listed or claimed here and on no list, never elsewhere);
  dropping the listed-THREAD or listed-GROUP exclusion (the line is then said twice, once as a hazard on the row);
  showing it as a row (`outside_set` would add the separation remedy, wrong for one moved thread, and a member-less
  row reaches Withhold/Stop/Release logic that never saw one); dropping the `EXISTS` on `medication_thread_group`
  (a thread claimed only by a stop/W/coding, this patient's or another chart's, is on NO list, and "on another
  chart" would be false — this patient's event on it is said on NEITHER chart until its assert arrives); trusting
  the earlier member read alone for "already listed" (a separate READ COMMITTED statement — a line re-homed onto a
  set chart between the reads came back "on chart <own chart>"; the query re-checks in its own snapshot, PR #851
  review). The remedy is CONDITIONED: if the drug is already on this list, the
  misfiled dose/stop/release/W is recorded again on that line, never a second line for the drug (double-dose reading). An
  empty, all-stopped or all-signed list with a report makes no absence claim (window `empty_message`, CLI "! but"). The view is filtered by
  `asserted_patient`, so db/059 indexes `patient_id` on its three source tables. The sign-off outcome carries it
  too (`SignOffOutcome::elsewhere`, #849: CLI "! " line in every outcome, the window's outcome notes; the CLI's
  empty, all-stopped and all-signed sentences make no absence claim beside it; pins
  `a_stopped_or_signed_chart_with_a_drug_elsewhere_points_at_it`, `a_sign_off_outcome_carries_the_lines_elsewhere`,
  `the_sign_off_verb_prints_the_report`, `an_empty_chart_with_a_drug_elsewhere_makes_no_absence_claim`); the
  report is permanent (**#850**, a decision). Pins: `tests/medication_rehomed_thread.rs`
  (each SQL predicate mutation-checked, the three CLI calls source-guarded, a linked set from each side), the
  semantic tree's `the_chart_warnings_are_announced_not_merely_displayed`, the webview's
  `a_drug_elsewhere_alone_shows_the_warnings_section`.
- **A release of a hold that has ENDED is a stale refusal at the local door (#843, maintainer 2026-10-23)** — a
  colleague already released it, or it ran out, by the release's `t_effective`. ONE predicate,
  `cairn_medication_hold_ended` (db/059 section 7b), with three callers: the read-time fold (section 10 — so a hold
  the floor calls ended is never still on screen, where its release would be refused on every re-read), section 8's
  local-door `RAISE` (unbypassable, any client) and `release_medication_holds`'s pre-check, which refuses BEFORE
  signing with `stale_picture_refusal` — a floor `P0001` carries no scope, so without the pre-check the window could
  not re-read (#840's "stale wins" now fires for a release). The pre-check asks only about holds on the release's
  OWN thread: a hold id on another thread is a wrong input, left to the floor's thread sentence ("release it
  there"), never "stale — re-read and retry" (PR #853 review). A PENDING hold is standing: releasing it cancels it.
  **Tempting wrong fixes:** match the floor's message text in Rust for the scope (the pre-check reuses the predicate
  instead); re-inline the fold's tests (they drift); refuse at sync apply or replay (`cairn.remote_apply = 'on'`: a
  peer's dead release is admitted, the fold makes it a no-op — a local veto forks the event set, and a strict replay
  could not start the node). **#855:** section 8 locks each named hold (sorted) before it checks, and a floor
  refusal of a release the pre-check passed re-asks the predicate, so an overtaken release is STALE too, carrying the
  floor's own sentence (it names the FIRST bad hold, which may be another fault) — never
  drop the lock to "simplify" (two local releases are admitted together again) and never re-ask on an UNKNOWN commit
  outcome (the release may be our own). Pins: `tests/medication_release_stale.rs`, `tests/medication_release_race.rs`.
- **A Stop that lands on a line already stopped is WRITTEN, and the window says so (#843)** — no refusal: its reason
  (an adverse reaction, say) may be the only record of it while no allergy stream exists. `cease_impl` appends
  `already_stopped_note` (the earlier stop's reason, or "no reason was recorded") when its press-time read shows the
  line `Ceased` AND the press landed in full — every thread stopped for certain. Beside a commit with no answer
  (#814) or a failed thread, "your stop is recorded" would contradict the outcome's own "NOT known" / "NOT stopped"
  (PR #853 review). The note names the shown stop's reason AND every other stop's (#857): "no reason was recorded"
  only when no stop gave one, never beside a reasonless latest stop (PR #860 review). It closes with "the list keeps every
  stop's reason", true of a line that reads stopped (#857) and, since #861, of a stopped thread on a line still current
  (`stopped_member_reasons`) — so the sentence takes no `list` argument: the PR #860 caller-chosen claim and both its
  constants (`LIST_KEEPS_EVERY_REASON`, `LIST_SHOWS_REASONS_ONLY_WHEN_STOPPED`) are gone. The CLI's
  `medication-cease` says it too (#854), from the same sentence (`already_stopped_sentence`) and line rule
  (`MedicationRow::already_stopped_note`, `cairn-medication-view`), asked through `medication::already_stopped_note`
  BEFORE it ceases (after, its own stop would read as "already"). The window stops every thread of a line, the CLI
  ONE: so the CLI also asks the thread's own cessation, because a thread stopped beside a later active sibling leaves
  the line current and the line rule silent (PR #856 review). That the line still reads current is said AFTER the
  stop by `still_current_note` (#859, next bullet), never by this note. That read is display-only: its failure becomes a
  sentence (`not_known_whether_stopped`), never `?` — a stop is a safety write — and a thread on no line of the
  patient's list here says it cannot tell (silence would read as "not stopped"). The note goes to stderr, beside the
  CLI's other warnings. **Tempting wrong fix:** refuse it as stale like a release. Pins:
  `a_stop_of_a_line_already_stopped_says_so`, the `cease_sorts_an_interrupted_commit_from_a_failure` source shape,
  `tests/medication_cease_already_stopped.rs`.
- **Every stop's reason stays on the list (#857, maintainer 2026-10-23: "every stop's reason")** — `medication_cessation`
  (per thread, through db/033's `medication_group_status`) still decides the status, and db/033's
  `medication_group_cessation` view picks the SHOWN stop (`cessation_reason`, the one whose date the line shows); db/060's `medication_cessation_reason`, one
  row per stop that gave a reason, keyed by content address, supplies `other_cessation_reasons` — every OTHER
  distinct reason (surrounding whitespace ignored), newest recorded first, read in the SAME statement as the shown one
  (`read_cessation_reasons`, members resolved once and aggregated — a per-line subquery re-scans
  `medication_statement` for every stopped line). A line that reads CURRENT (a sibling's later dose wins) carries every stopped member's reason in
  `stopped_member_reasons` instead (#861, maintainer 2026-10-24: "show it, worded apart"), in a stopped line's order,
  said "a stopped record of this drug gave: …" (`stopped_record_lines`, `stopped_record_detail`) — never "Stopped",
  since the drug is not. **Tempting wrong fix:** fill `cessation_reason`/`other_cessation_reasons` on a current line
  (every renderer would then say "Stopped"/"ceased" of a current drug); keep the ceased fields ceased-only. A shown stop with no reason beside another stop's reason is said first ("no reason
  given"), so the other reason is not read as the shown stop's (`ceased_reason_lines`, `cessation_detail`). db/060
  bumped the generation so the loader's heal backfills; db/037's shred scrubs the row. **Tempting wrong fixes:**
  `COALESCE` the old reason into `medication_cessation` (still loses a reasoned earlier stop and a sibling's);
  reading the two parts in two statements. (#863 and #864 are fixed: the block above.) Pins: `tests/medication_stop_reasons.rs` (incl.
  `a_line_kept_current_by_a_sibling_still_shows_the_stopped_threads_reasons`), `other_reasons_drop_the_shown_one_and_repeats_only`, the
  renderers' unit tests, `registry_row_count_is_pinned` (30) and db/tests/039 (32).
- **A CLI stop of one thread says when the line still reads current (#859)** — `medication-cease` re-reads the line
  AFTER the stop (`still_current_note`) and names its threads with no stop: under latest-effective-wins (db/033)
  only an unstopped thread's dose can keep a line current. They are read from the whole group
  (`medication_thread_group`), not `row.members` (this patient's charts only): a thread on a chart not linked to this
  patient is counted, never named, and the note then drops "stop that one too". It says "no
  stop … is readable here", never "recorded": a stop sealed without custody, or a thread's only stop shredded, leaves no
  `medication_cessation` row though a stop was recorded (PR #860 review; #863 re-folds an earlier stop). Display-only:
  a failed re-read becomes `not_known_whether_current`, never `?`, since the stop is already written. Both notes go
  to stderr after the success line. Pins: `a_stop_of_one_thread_on_a_line_kept_current_names_the_sibling`,
  `a_sibling_already_stopped_is_not_named_as_unstopped`, `a_sibling_on_an_unlinked_chart_is_counted_never_named`,
  `the_cease_verb_reads_the_note_first_and_prints_it_last`.
- **A release recorded for a LATER instant is said on its hold (#835)** — the hold still stands until then (the
  fold is unchanged), and `MedicationHold::release_pending` carries the instant from db/059 section 7c's
  `cairn_medication_hold_release_pending` (the same thread-and-name match as 7b, and only a release BEFORE a timed
  hold's own `until`: one at or after it ends nothing, so "release at <t>" would be false — PR review). 7c does NOT
  look at `starts_at`: a release at or before a pending hold's start cancels it, and both surfaces then say
  "cancelled" (`MedicationHold::release_cancels_it`), since "from 08:00, release at 16:00" reads as a W from 08:00
  to 16:00. The CLI's `hold_line` and the window's `hold_view` say it BEFORE the free-text reason, and
  `next_hold_change` counts it, so the window re-reads when it takes effect. `read_group_holds` asks the fold and 7c
  at ONE instant (`n.now` in FROM); **tempting wrong fixes:** `clock_timestamp()` in the select list (re-evaluated
  per row), a `released_at >= starts_at` "tightening" of 7c, or a new fold column (a return-type change
  `CREATE OR REPLACE` refuses, and an older binary's replay of db/059 then fails). Pins:
  `tests/medication_release_pending.rs` (incl. its one-instant source guard), `a_pending_release_is_said_on_the_hold`
  and `a_release_before_the_start_*` (both surfaces), `a_pending_release_is_a_hold_change`.
- **A stale refusal keeps the gesture's button down (PR #842 review)** — `withhold`/`release` resolve "spent" for a
  report AND for a refusal whose retry is `after_reread` (`Retry::AfterReread` promises no live retry); any other
  refusal re-enables it. Pin: `both_gestures_re_read_like_cease`.
- **The W is a BUTTON whose accessible name carries its reason** (symbol + " — " + detail), and a ceased line
  whose stops gave a reason has its status word as such a button, revealing `cessation_detail`'s text ("Stopped:
  <reason>", then "Another stop's reason: <reason>" per other stop, #857; no reason on any stop: plain text) — and so does a CURRENT
  line with a stopped member's reason ("A stopped record of this drug gave: <reason>", #861). The semantic tree (`cairn-gui-tab-medications` `lib.rs`) declares the same labels — pin
  `every_hold_and_hold_control_is_a_labelled_button`. Hover is never the only way to the reason: WebKit does not
  focus a clicked button, so click pins it; Escape or an outside click unpins. **Tempting wrong fix:** a `title`
  tooltip or a hover-only popover. Pins: `the_w_is_a_button_whose_label_is_its_reason`,
  `escape_or_a_click_elsewhere_unpins_a_detail`.
- **The revealed detail is out of flow (`position: absolute`).** In flow it re-laid the table out, the button jumped
  from under a real pointer, `mouseleave` hid it, and the W flickered. Only a real mouse shows it (found walking
  #832). Pin: `a_revealed_detail_moves_nothing`.
- **"Until reviewed" is the default, and the select and the time field never disagree silently.** Typing a time
  picks "until a time"; a partial `datetime-local` is refused by Rust; and choosing "until reviewed" back EMPTIES the
  time field — `value = ""` plus a type reset, which WebKit needs to drop a PARTLY typed time (else the press
  still sent the typed time and the W lapsed silently at that hour). **Tempting wrong
  fix:** treat an empty or unparseable time as open-ended, or let a leftover time win over the select. Pins:
  `until_reviewed_is_the_default_and_never_an_empty_time`, `typing_a_time_chooses_until_a_time`,
  `choosing_until_reviewed_clears_the_time`, `a_re_read_keeps_the_until_choice_and_time`.
- **One press, one W.** The pressed button is disabled before it sends and stays so until the re-read; a returned
  report clears that line's fields (so the next press cannot repeat the W), a THROWN failure keeps them (the
  clinician's typing is not lost) and re-enables its own button — nothing was written, and the re-read may have
  failed and kept the old line. **Tempting wrong fix:** clear on every outcome, or re-enable in `finally` (that
  makes a recorded W pressable again). Pins: `a_gesture_button_disables_itself_before_it_sends`,
  `a_returned_report_clears_the_line_before_the_re_read`.
- **`status_label` "withheld" is wording only; `current` is the fact.** `current`, `can_cease` and sign-off are
  unchanged by a hold. **Tempting wrong fix:** key behaviour off the label. Pin: `a_withheld_line_stays_current_signable_and_ceasable`.
  The row's styling (`data-withheld`, `data-ceased`) is the one reader of the words, tied to Rust's by
  `the_row_draws_the_w_and_its_controls`.
- **The list is re-read when a hold starts or ends.** A hold changes with the clock, not with an event, so a screen
  left open kept "withheld · W until 20:00" into the next morning (where the dateless time read as today's) and
  "current" past a pending W's start. Rust sends `next_hold_change`; `drawList` arms one timer at it, a failed re-read
  retries each minute. **Tempting wrong fix:** always print the date instead — the status word would still be
  stale. Pins: `the_next_hold_change_is_the_earliest_instant_still_ahead`,
  `the_list_is_read_again_when_a_hold_starts_or_ends`.
- **A Release that can write nothing from this chart says so before the key** (a hazard line's other-chart holds),
  and only a release that released something is timed. Pin: `the_refusals_come_before_the_key`, which also pins
  that a withhold's reason and end are refused before the line lookup and the key.
- **Times on screen are LOCAL; the wire stays UTC.** `local_instant` converts for display only. **Tempting wrong
  fix:** send the local string. Pins: the `hold_view` display tests (`a_hold_ending_today_shows_the_local_clock_time`
  and its siblings) for the local half; for the UTC half, the `toISOString()` conversion pinned in
  `until_reviewed_is_the_default_and_never_an_empty_time` (`holds_webview_tests.rs`).
- **An unknown commit outcome is "NOT known", never "failed"** (a dropped connection mid-commit may have recorded the
  event). Pin: `both_gestures_sort_an_interrupted_commit_from_a_failure`. Release writes one event per thread
  (`a_release_writes_one_event_per_thread`, the plan half), each in its own call — the transaction scope is not
  pinned in the window.
- **The two gestures are timed** (`withhold`, `release` in `ui_gesture_timing`, db/044's check constraint).
  Pin: `the_withhold_gestures_are_timed`.

**⇒ #803 — THE MEDICATION WITHHOLD (ADR-0081) — do not undo** (pins: `tests/medication_withhold.rs`,
`medication_withhold_fold.rs`, `medication_withhold_signoff.rs`, `medication_withhold_clock.rs`, `medication_read.rs`):
- **The start of an un-`from`'d hold and a release's instant are the authoring node's PHYSICAL clock, carried as
  `t_effective`; the authoring node never stamps them from its HLC wall.** A peer whose clock runs fast pushes this node's HLC up to 24 h ahead (db/020),
  and a "W from now" stamped with that wall starts in the future: pending, so the drug reads as givable. db/059 has
  **no HLC-wall fallback**: the floor refuses, at both doors, a withhold with neither `from` nor `t_effective` and a
  release without `t_effective` (another client's, or a hand-built body). **Tempting wrong fix:** restore the
  `COALESCE(…, to_timestamp(hlc_wall/1000))` fallback "for robustness" — it makes a third-party client's "W from now"
  read as not started. The instant is read BEFORE the HLC tick, so the bitemporal ceiling never flags it. A test that
  hand-builds such a body stamps it with `medication_kit::effective_at_wall`.
  Pins: `medication_withhold_clock.rs` (both tests, each mutation-checked against its own line);
  `a_missing_t_effective_is_refused_at_the_sync_door_too`.
- **A hold sits BESIDE `MedicationStatus`; there is no `Withheld` variant.** **Tempting wrong fix:** add one.
  `signoff.rs` filters `status == Active` and the window's `view.rs`/`row_view.rs` (`current`, `can_cease`) would
  silently drop withheld drugs out of sign-off. Pin: `a_withheld_line_stays_current_and_signable`.
- **Expiry writes no event.** Whether a line is withheld is the read-time fold `cairn_medication_group_holds(groups,
  now)` (db/059); `now` is a parameter so tests pin instants (the fold tests; the staleness-clock pin
  `expiry_does_not_stale_a_sign_off`).
- **A release names the withholds it ends by EVENT ID, and only on its own `medication_id` THREAD** (`r.medication_id =
  w.medication_id` in the fold is the only guard on the sync path; the list shows holds per LINE, so `hold_line`
  prints each hold's event id and thread, and the local door's refusal names the thread a sibling's hold is on).
  Pins: `a_synced_release_on_another_line_ends_nothing`, `a_release_naming_another_lines_withhold_is_refused`.
  A release also cancels a not-yet-started hold (`a_release_cancels_a_pending_hold`).
- **Refusals split in two.** The structural floor runs at both doors (`an_empty_span_is_refused_by_the_floor_at_sync_apply_too`);
  state refusals (ceased line, `until` <= resolved start, release naming a non-withhold or another thread's) live at the
  LOCAL door only. **Tempting wrong fix:** move a state refusal to the sync door; a peer's valid event then wedges or
  forks sync. Pins: `a_sync_applied_withhold_on_a_ceased_line_is_admitted`,
  `a_peer_hold_already_over_is_admitted_at_sync_but_never_shown`; the structural tables
  `every_structural_{withhold,release}_refusal_holds_at_the_local_door`.
- **A ceased row carries no holds** (cessation wins, ADR-0081 decision 5). Pin: `a_ceased_line_shows_no_holds`.
- **`cairn_medication_thread_patient` has ONE definition, in db/031; its withhold arm is `to_regclass`-guarded
  there and orders by `(hlc_wall, hlc_counter, origin COLLATE "C", content_address)`**: the earliest claim stands. The
  other arms hold one row per thread; this one holds many. **Tempting wrong fix:** redefine the fn in db/059 — every
  replay then briefly runs db/031's body without the arm. Pin:
  `the_earliest_orphan_withhold_claim_stands_whatever_the_arrival_order`.
- **`cairn_event_thread` (db/048) probes one table per source file — db/031, db/032 AND db/059.** Pin: cairn-sync's
  `schema_subset_alone_satisfies_every_door`, whose two crash-mid-replay windows each leave exactly one probe deciding
  (window 1 probe 2, window 2 probe 3). Deleting probe 2 or probe 3 each turns it red (mutation-checked separately).
  Probe 1 has no window of its own: a replay that lacks db/031's table lacks db/032's and db/059's too (they load
  later), so probes 2 and 3 already return NULL there. A new
  medication source file needs a probe AND a window of its own.
- **The sign-off thread index was RENAMED `event_log_medication_thread_idx2`** (db/034) and the old one dropped in
  db/059: an `IF NOT EXISTS` index never picks up an edited predicate, so a type added to the commitment lists needs a
  NEW name again, never an edited predicate. Pin: `only_the_six_type_thread_index_survives_a_schema_load`.
- **Pinned counts this slice moved:** twin registry 26 (`twin_registry.rs`, `db/tests/034`), projection registry 29/31
  (`projection_registry.rs`, `db/tests/039`), `thread_resolution_cost.rs` 7 tables / 8 types.

**⇒ #822 + #823 — THE CLI AFTER AN UNKNOWN COMMIT OUTCOME — do not undo** (pins: `db_diagnosis/refusal_scope_tests.rs`'s
`only_a_stale_refusal_or_an_unknown_outcome_gets_the_cli_remedy`, `tests/event_lookup.rs`, `auto_apply.rs`'s
`run_verdict` tests, and `tests/identity_commit_lost.rs`'s `a_lost_auto_apply_commit_stops_the_run_with_its_summary`,
mutation-checked; `a_failed_hlc_tick_stops_the_run_with_its_summary` and `event_lookup.rs`'s unit tests since the
second review):
- **The marker's "check whether it was recorded" has a command: `cairn-node event <id>`**, named by `cli_remedy`'s
  line, `--conn` BEFORE the subcommand (it is not a global flag; the binary test runs that form). It reads clear
  envelope columns only (never the body, never `safety`) and says "IS / is NOT recorded on this node". The remedy
  claims only "if it does, do not run the command again", never "if not, run it again": a command could have committed
  earlier steps, which its own sentence reports. **"NOT recorded" says "as of now" and names the check before a
  re-run** (the interrupted session has ended, `pg_stat_activity`): when only the client's side of the connection was
  cut, the server can still be finishing that commit, and a bare "not recorded" invites the re-run that doubles it. A
  lookup that cannot read `event_log` is an ERROR, never "absent" (`a_lookup_that_cannot_read_is_an_error_not_absent`).
- **Auto-apply STOPS at the first pair whose outcome is unknown** and returns its summary (`outcome_unknown`,
  `not_attempted`; never counted `errored`). **Tempting wrong fix:** `errored += 1` and carry on, as for any pair. The
  session is dead, so the next pair's first call ends the run as a plain "connection closed" with no summary.
  "Running auto-apply again is safe" holds only because the link and the proposal's `auto_applied` status commit in
  ONE transaction: split them and that sentence becomes false. That pair's stderr line renders the cause through
  `commit_outcome_unknown_cause`, WITHOUT the marker's "it may be recorded twice", which would contradict it.
- **A connection that dies anywhere else stops the run too** (`connection_lost`: a `client.is_closed()` check at the
  top of each pair, and ANY `next_hlc` failure — not only one on an already-closed client, because a FATAL answering
  the tick arrives before the close; `node_hlc_tick` raises nothing of its own, so a failed tick is the session. Both
  print their cause, so the verdict's "the cause is on stderr above" holds even with `errored == 0`; pins
  `a_connection_lost_mid_pair_stops_the_run_with_its_summary`, `a_failed_hlc_tick_stops_the_run_with_its_summary`).

**⇒ #811 — A RE-READ AFTER AN INTERRUPTED COMMIT SAYS ONLY WHAT IT KNOWS — do not undo** (pins: `funnel/view_tests.rs`'s
wire test, `judgement_view_tests.rs`, `duplicates/verdict_tests.rs`, and the source guards in
`reread_webview_tests.rs` / `read_again_webview_tests.rs`):
- **`Retry::AfterReread(Recorded)`: `No` = a stale refusal, nothing recorded; `Unknown` = an interrupted commit
  (#713), it may have been.** It words only the out-of-date notice (`LIST_OUT_OF_DATE` vs `LIST_MAY_BE_OUT_OF_DATE`).
  **Tempting wrong fix:** collapsing it back to a unit variant because the window "does the same" for both. It does
  the same, but must not SAY the same.
- **`No` wins** (`Recorded` is ordered `Unknown < No`; `combined_retry`, `behind_the_record`, and `staleAfter` in
  `chart_stale.js`, which a later `"unknown"` never clears): one stale refusal PROVES the list is out of date.
  **Tempting wrong fix:** letting `Unknown` win "because it is worse". The banner then says "whether your last act
  was recorded is not known" right after a Stop whose own line said "nothing was done".
- **…but only until a list read AFTER the stale refusal draws** (`drewRead` clears `staleAfter` on its own, before
  `behindAfter` ends). That list shows the change the refusal was over; if only an interrupted commit is still behind
  it, a failed read says "may be". **Tempting wrong fix:** one `behindRecorded` value that stays `"no"` until the
  whole state ends — it then claims "the record changed" over a list that already shows that change.
- **Only `"no"` words "is out of date"** (`markListStale`'s `proven`): anything else, an unknown future value
  included, gets the weaker "may be", and the sign-off button says what the notice says.
- **"A change the re-read names may be your own" is Rust's, in the interrupted act's OWN sentence, for Link and
  Unlink only** (`judgement_error_view(act, Charts::MayMove, e)`). A confirmation (`confirm_error_view`) and a
  "different people" judgement are `Charts::Kept`: they never move membership, so a change named after one is
  certainly another source's. **Tempting wrong fix:** a JS sentence keyed on `"unknown"`. It cannot tell the acts
  apart, so it excuses a colleague's link as the clinician's own.
- **That sentence is window state too** (`unknownLeads`, said by `withOwed` from whichever read ends the state). The
  act's own re-read can be overtaken ("Read this chart again", an own act's draw floor), and the read that draws
  would otherwise name the clinician's own link as another source's change. **Tempting wrong fix:** relying on the
  re-read's `lead` argument alone.
- **#814: every `.commit()` and every autocommit event write in the tree is inventoried** by
  `db_errors_stay_legible.rs` (autocommits are FOUND, by a door call on a receiver other than `tx`, since the #817
  review; never declare a count by hand): in `MARKED_COMMIT_SITES`
  (each through `db_diagnosis::commit_failure`, counted per file: judge, each sign-off line, both `seal_sign_submit`
  arms, the attested reconciliation, the CLI's attestation, a registration since #815, and since #817 every identity
  write: identify, a John-Doe chart, evidence, a proposal's link, an auto-applied link, and the autocommit submits of a
  sensitivity grade/withdrawal, a shred and a text evidence) or in `UNMARKED_COMMITS` with its reason (a
  restore's `finalize_identity`, whose landed commit fences the door; the node-plane events and actor enrolments,
  which a re-run cannot double; a read-only snapshot; the sync harness's load generator). A marked file's
  commit that writes NO event (auto-apply's two send-to-review commits) is counted in `EVENTLESS_COMMITS` with its
  reason. An unattested submit's autocommit statement IS its commit, so it is marked too.
  Only an ordinary ERROR answer proves a rollback; no answer, or a FATAL/PANIC (which can follow a landed commit), is
  marked. `data_error_from` classifies a marked error as `DataError::OutcomeUnknown` (#815), so `node_failure`
  re-reads it. **Tempting wrong fix:** a bare `tx.commit().await?` "because
  the statements before it already succeeded". It compiles, reads as an outage, and offers the #713 retry. **Also:** a
  cease's unknown threads are neither stopped nor "NOT stopped", and `write_failure::cease_outcome` drops "Stopped 0"
  when one is unknown.
- **#815: an interrupted REGISTRATION spends its search, exactly as a success does** (`TokenStore::settle` →
  `Restored::Consumed`; pins: `token.rs`'s `an_unknown_outcome_consumes_the_form_as_a_success_does`,
  `commands_register_tests.rs`'s unknown-outcome walk, mutation-checked; end to end through a real lost COMMIT,
  `cairn-gui-live/tests/lost_commit.rs` (#820), and in the root tree `tests/identity_commit_lost.rs`, which pins that
  the marker names the REGISTRATION event and the error text the chart, John Doe's headline included).
  `data_error_from` classifies the marker as `DataError::OutcomeUnknown` FIRST; `settle` itself asks `RegistrationFailure::may_have_written`, so the short call
  stays the safe one (#659) and no caller special-cases it; `register_error_view` says "NOT known" with
  `Retry::AfterReread(Unknown)`, which funnel.js reads as "every search is spent": it forgets the token it holds even
  when a newer search replaced the one sent, so the next Register press searches first, and that search finds the
  chart if it landed. **Tempting wrong fix:** restoring the search "so the clerk can press
  again" — `register_patient` mints a fresh `patient_id` per call, so that press registers the same person twice.
  Merely dropping the attestation is not enough either: a background search minted mid-flight would still license a
  second chart, which is why it is `commit()`'s invalidation.
- **#818: once a sign-off line may have committed, `sign_off_medication_list` never returns `Err`.** A lost commit,
  or a transaction that will not open, ends the loop with `SignOffOutcome::interrupted` (the line it stopped at, a
  `StoppedLine` — `MayBeSigned { event }` or `NotSigned`, never an `Option` whose `None` reads as "not known" — and the
  untried rest); a failed ROLLBACK keeps the line's own reason. Its `cause` is the root cause, not the marker's text.
  The CLI's empty-`attested` sentence is the pure `nothing_attested_sentence`: never "nothing was signed off" beside a
  line that may be signed. The window's
  whole sentence is Rust's `write_failure::sign_off_outcome` (truth-table tested; "Signed 0" is dropped beside an
  unknown line, as `cease_outcome` drops "Stopped 0") and `signOff` says `report.text` and re-reads with
  `report.behind_the_record` (returned by `sign_off_outcome` with the text, so they cannot disagree). Pins:
  `tests/signoff_interrupted.rs` — a test-only trigger on one named line answers its COMMIT FATAL, kills the backend
  inside the line (its own reason survives the dead ROLLBACK; mutation-checked), or answers the COMMIT with an ERROR
  (that line alone fails; the rest are signed). **Tempting wrong fix:** a `?` on `client.transaction()` or
  `tx.rollback()` in the loop — it compiles, and discards every line already signed and every reason.
  **And (#820):** `SignOffOutcome`'s three production readers (the CLI's `sign-off` arm and its
  `nothing_attested_sentence`, `sign_off_impl`) destructure it naming every field, no `..`, so a new field breaks the
  build at each reader. **Tempting wrong fix:** a `..` "to shorten it" — Rust never warns on an unread `pub` field,
  which is how `failed` was once dropped. Pinned by source guards in both trees:
  `tests/signoff_readers_name_every_field.rs` and `commands_tests.rs`'s `sign_off_impl_names_every_outcome_field`.
- **No script spells the string `"after_reread"`**, not even in a comment: the retry is an object, and
  `failure.retry === "after_reread"` is always false (no re-read, a live button). Guarded over every `src-ui/*.js`.
- **`forgetBehind` is called only by `drewRead` and `holdCharts`** (counted). One more call in an own-act path would
  end the stale state silently.
- **Any re-read that names a change says `READ_AGAIN` ("review it before you act on it")**, including after the
  clinician's own successful act (M5), and once per line: "Read this chart again" keeps the line as its lead. A
  colleague's change is news to review either way.

**⇒ #797 — A CHANGED RECORD RE-READS ITSELF AND SAYS WHAT CHANGED (PR #805) — do not undo any of these** (pins: the Rust
halves by `crates/cairn-node` tests of `stale_picture_refusal`'s callers, `cairn-gui-live`'s error mapping tests,
`cairn-gui-tauri`'s `chart_set/change.rs`, `write_failure.rs` and `link/judgement_view.rs` tests; the JS halves by SOURCE
guards in `cairn-gui-tauri/src/reread_webview_tests.rs` and `read_again_webview_tests.rs`, each mutation-checked, and
a headless walk on the PR). The
maintainer's governing rule, which every item below serves: *"The chart I hold in my hand when I make changes is my
responsibility — my obligation to know its contents. I need only to be informed of what changed between me taking the
chart and finishing amends that I cannot know (that is, anything that wasn't in the chart when I took it and I didn't
change myself)."*
- **A "the record changed under you" refusal is routed by TYPE, never by text.** `RefusalScope::Stale` (cairn-node,
  raised by the record check in `chart_link.rs`, `admit.rs`'s `LacksPair`, `shown.rs`'s `stale_record`/`stale_pair`,
  `medication/signoff/rules.rs`'s `ensure_same_charts` and `ensure_same_targets`) → `DataError::Stale` → `Retry::AfterReread` (the funnel maps it to
  `Never`: it has no chart to re-read); on the Rust side `ChartSetError { Unreadable, Changed }`. **Tempting wrong
  fix:** matching `e == CHANGED` (or any sentence) again — a reworded node sentence then silently stops re-reading. The
  neither-record arm of `shown.rs` stays `Input` (nothing is stale; the input is wrong).
- **Node sentences name no window control.** They say only what happened ("— nothing was done" / "nothing was
  signed"); the CLI shows them too. The window's `chart_stale.js` adds what to do; the CLI prints
  `db_diagnosis::cli_remedy`'s line after a `Stale` refusal (`main` wraps `run()`). Pinned by
  `crates/cairn-node/tests/node_sentences_name_no_window_act.rs` (#806). **Tempting wrong fix:** putting
  "press Read this chart again" into the node sentence — it is nonsense at a terminal.
- **`READ_AGAIN` ("The chart has been read again — review it before you act on it.") is said only after an ACCEPTED
  answer whose list actually DREW** (`renderedPatient === patient` after `render`). **Tempting wrong fix:** saying it
  when the re-read is sent — a failed or dropped read then claims a chart the screen does not show.
- **"The list is behind the record" is WINDOW state (`behindAfter`, `chart_stale.js`), not one read's flag.** A
  stale refusal records the newest read revision sent before it (`knowListBehind`, in `refresh` before its revision is
  taken); WHICHEVER read fails last then marks the list (`listKnownBehind`); only a list drawn from a read sent AFTER
  the refusal ends it (`drewRead`), and taking the list down ends it (`holdCharts(null)`). A success report can carry
  it too: a partly-recorded "Different people" whose failed pair was `AfterReread` sets
  `LinkReportView::behind_the_record`. An OLDER read landing after a newer one already failed and marked the list
  re-marks it and leaves the outcome line (`sayAfterDraw`; whether it was marked is read BEFORE `render`). Pins:
  `a_known_stale_list_outlives_the_read_that_learnt_it`, `an_older_read_landing_late_keeps_a_stale_mark` (walked
  headlessly: an overtaking "Read this chart again", an older read landing late before and after the failure). **Tempting wrong fixes:** a `staleIfFails`
  argument — the re-read it rides is overtaken by "Read this chart again" or an own act's draw floor, its failure is
  never said, and the overtaking read fails unmarked; or a plain boolean cleared by ANY draw — `mayDraw` lets an older
  read, sent before the refusal, draw after it.
- **`markListStale` keeps the rows and disables sign-off** (`#chart-stale` says `LIST_OUT_OF_DATE`, or since #811
  `LIST_MAY_BE_OUT_OF_DATE`: see that block). **Tempting wrong
  fix:** hiding the rows — the clinician then has no chart at all during a partition (availability over consistency),
  and the node's set check refuses the write regardless, so the rows were never the danger; the danger is acting on
  them unwarned.
- **`set_change` (the sentence "Now part of it: B.") is `None` for an equal seen set, an unreadable one, or one that
  does not hold the opened chart** (`chart_set/change.rs`: `seen_set` drops an unreadable or foreign set,
  `set_change_sentence` says nothing for an equal one; both pure). **Tempting wrong fix:** a
  sentence whenever the sets "differ" in any way — an unreadable or foreign baseline would announce every member as new.
- **The window HOLDS the chart set it drew (`heldCharts`), moved only by being shown or by the clinician's own act**
  (`applyOwnAct`, `holdOwnAct`; every `refresh` sends it as `seenCharts`). Only what the clinician did not do and could
  not see is announced. **Tempting wrong fix:** the "set formula" `(held − (sent − after)) ∪ (after − sent)` — in the
  table's fourth row (a Link answered on the OTHER chart, drawn before its commit) it announces the clinician's OWN link
  back to them; the anchored `applyOwnAct` passes every case in the table above it in `chart_stale.js` (pinned only
  by source guard — its helpers `without`/`ordered`/`sameCharts` by body — and the headless walk; there is no JS
  runner, #332) and is the only spelling to use. Do not pass the baseline as a one-call argument either: a second
  re-read starting within one round trip announces the own Link.
- **`heldCharts` is deliberately SEPARATE from `renderedCharts`.** `renderedCharts` keeps feeding every write's set
  check; `heldCharts` is only the baseline for naming changes. **Tempting wrong fix:** merging them, or sending
  `heldCharts` to writes.
- **The draw floor is raised (`drawnRevision = ++chartReadRevision`) only when the own act MOVED the held set.**
  **Tempting wrong fix:** raising it unconditionally on every own act — that drops a colleague's in-flight read that
  would have been the first news of a real change.
- **`charts_after` (the Link / Unlink / Confirm reports' record as the act left it) is read INSIDE the act's
  transaction** (`chart_link/judge.rs`, after the apply, before the commit). **Tempting wrong fix:** reading after the
  commit — a colleague's link landing in that gap is then folded into the clinician's "own" baseline and never
  announced. The guard `the_set_left_is_read_after_the_apply_and_before_the_commit` (`chart_link/judge_tests.rs`)
  pins that order. **And it is attached only when the act changed the record** (`reload` true;
  `LinkReportView::left`, pinned by the `*_reports_no_set` tests): an Outranked act's set would move the held set
  and raise the draw floor with no re-read following, dropping a colleague's read already in flight.
- **"Read this chart again" (`#reread`) is always visible in the chart view, and its label is pinned to every sentence
  that quotes it** (`LIST_OUT_OF_DATE`, `REVIEW_WAITS_FOR_LIST`, `NO_LIST_TO_COMPARE`, `chart_set`'s `UNREADABLE`, the
  banner's and the record-links' read-failure sentences; `every_sentence_that_names_the_control_quotes_its_label`).
  `LINK_GONE`'s `Never` arms deliberately name no button: a re-read redraws the same lines, so it cannot mend them.
  **Tempting wrong fix:** showing it only after a failure (the first failed read then has no way out), or relabelling it in one place.
- **"Read this chart again" re-reads with the outcome line as its lead (`rereadLead`), less a read failure it is
  retrying** (`sayReadFailed` remembers the failure's line and lead; a drawn re-read with nothing to add takes a
  standing failure back to its lead). Pins: `read_again_keeps_the_outcome_line_but_not_a_failure_it_retries`,
  `the_only_lead_less_re_read_is_a_chart_s_first`. **Tempting wrong fixes:** a lead-less `refresh()` — a change it
  names REPLACES a partial sign-off report (ADR-0060: partial completion is reported, never implied); or the raw
  outcome text as the lead — each failed press then piles another "Could not read the chart" onto the line, and a
  recovered read leaves the failure up. The one lead-less re-read is a chart's first (`enterChart`, after `clearChart`
  empties the held set, so it can name no change).
  Sign-off and cease return an `ErrorView` through `write_failure.rs`: a locked key is `Now`; no node key or no DB is
  `AfterOperator`; a failed chart read before the write is `Now` (`chart_read_failed` — an outage, never `refused`);
  never a bare string (it is worded "could not reach its backend").
- **Accepted limitations (honesty, not safety — do not "discover" them later):** a stale refusal's out-of-date mark is
  lost if an own act's re-read outdates it and then fails (the node's set check refuses writes regardless); a foreign
  change whose re-read hits a DRAWING fault is never named, because `showUnshownList` nulls `heldCharts`.
- **No window sentence says "reload the chart" or "refresh and try again"** (guard
  `no_window_sentence_says_reload_the_chart`, over every `.rs`, `.js` and `.html` source but tests): the window has a
  named control, so name it.

**⇒ THE CHART RE-READ (#799 · #800 · #801, PR #802) — do not undo any of these** (pins: the JS halves by SOURCE
guards in `cairn-gui-tauri/src/chart_view_webview_tests.rs`, each mutation-checked; walked headlessly on the PR):
- **Two questions, two predicates, each pinned as its exact expression.** An ANSWER draws while
  `mayDraw(patient, revision)` holds — its chart is on screen and nothing NEWER has been DRAWN (`drawnRevision`,
  recorded just before `render`). A FAILURE is said only while `isLatestRead(patient, revision)` holds — no newer
  read has STARTED (`chartReadRevision`, taken before the send). **Tempting wrong fix:** one "newest started" rule
  for both — the older of two overlapping re-reads usually lands first and was dropped, so if the newer failed the
  screen kept a list older than both (PR #802 review). Or back to `patient !== displayedPatient`, which cannot tell
  two reads of one chart apart.
- **`clearChart` sets a floor: `drawnRevision = ++chartReadRevision`.** It is LOAD-BEARING: `mayDraw` asks only
  about what is drawn, so without the floor a first-visit read could land on the second visit (A → front door → A).
- **`redrawBannerAlone` notes the revision and never bumps either counter** (a bump would make an in-flight chart
  read fail `isLatestRead`, so its failure would go unsaid).
- **A re-read keeps the typed stop reasons** (`stop_reasons.js`: `typedReasons` before the rows are emptied,
  `restoreReasons` after, focus and caret included). The banner-only re-read after a decided refusal (below) still
  stands, on its own reason.
- **`render` = the list (`drawList`, which sets `renderedPatient`/`renderedCharts` and draws EVERYTHING but the
  banner) under one guard, THEN the banner.** A drawing fault takes the whole list down (`clearDrawnList`, shared
  with `clearChart`), NULLS `renderedPatient`/`renderedCharts` — so the banner drawn next offers no Review and
  `compare` refuses, the rule a failed or in-flight read already keeps — disables sign-off with a worded reason, and
  says `LIST_NOT_SHOWN` in `#chart-unshown`, inside the warnings ABOVE the table, **never on the outcome line** and
  never as "Could not read the chart". A new drawer goes inside `drawList`; the banner must stay after it.

**⇒ THE BANNER'S AND TRAY'S FAILURE STATES (#745 · #752 · #740) — do not undo any of these** (pins:
`cairn-gui-tauri/src/duplicates/{tests.rs,view_tests.rs,verdict_tests.rs}`, `worklist/tests.rs`,
`commands_tests.rs::every_command_the_webview_calls_is_registered`; the JS halves by SOURCE guards —
`duplicates/webview_tests.rs` and `worklist/tests.rs`, each mutation-checked — since there is no JS runner):
- **The banner hides ONLY on Rust's `silent`** (`DuplicateSection::is_silent`, sent by a hand-written `Serialize`).
  Both destructure EXHAUSTIVELY: a new field must not compile until someone decides whether it un-silences the banner.
  **Tempting wrong fix:** `#[derive(Serialize)]` back, or a JS-side conjunction over the fields.
- **A failed `med_list` read redraws the banner ALONE** (`duplicate_banner` → `banner_alone_impl`; `main.js`'s `refresh`
  catch → `redrawBannerAlone`): the warning needs only the chart set. An unread set is a worded banner, never a hidden
  one; unread member names are said (`note_unread_names`), never dropped; a refusal of the read is an `ErrorView`
  (a bare string would be worded "could not reach its backend"). **`render` draws the banner through
  `drawDuplicates`** — a drawing fault is said ON the banner, keeping its own error and check lines, never thrown into
  `refresh`'s "Could not read the chart". **duplicates.js builds ONE section itself (`emptyBanner`)**, its keys pinned
  to what Rust sends — a second hand-built literal is a second copy of the shape to drift.
- **A decided (`"never"`) refusal of a comparison or a judgement re-reads the BANNER** (`redrawBannerAlone` from
  `link.js`'s `compare` and `sendJudgement`; #713's `after_reread` still re-reads the whole chart). **Tempting wrong
  fix:** `refresh()` — a heavier read that rebuilds every row over a refusal that changed nothing but the banner
  (a typed stop reason survives it since #799; the set-changed refusals, which DO stale the list, re-read the whole
  chart since #797 — see that block). `NOTHING_OPEN` / `NOT_SHOWN_OR_RESOLVED` never say "reload": no sentence says "reload the chart" or "refresh and try again" (`no_window_sentence_says_reload_the_chart`); the
  way out is the "Read this chart again" button (#797).
- **No comparison without a drawn list:** `compare` refuses while `renderedPatient` is null (`NO_LIST_TO_COMPARE`),
  and a banner drawn while no list is on screen (after a failed FIRST read, or a list that could not be DRAWN — #801)
  shows `REVIEW_WAITS_FOR_LIST` in place of Review. Both sentences must be true of all three cases (failed, in flight,
  not drawable — "could not be read or shown, or is still being read") and name the way out ("Read this chart again", the
  always-visible button #797 added — its label is pinned to the sentences that quote it).
- **The tray's count and list each carry a revision** (`trayCountRevision`, `trayListRevision`): an older answer is
  dropped. **The list is emptied when a re-read starts** — never the old entries, whose Review `close_chart` made
  unopenable. **Wherever the list is emptied, `trayListRevision` is bumped too** (`refreshTray`'s clear): an older
  list answer still in flight would otherwise land during the count's await and draw the entries straight back.

**⇒ R6'S RESIDUALS (PR #792: #787 · #786 · #783 · #782) — do not undo any of these** (pins:
`tests/standing_link_legacy_address.rs`, `cairn-patient-search/src/list_wire.rs`, `cairn-medication-view`'s
`the_doubted_link_remedy_names_the_bridge_lines_door_too`, `cairn-gui-tauri/src/link/{lock_hint_tests.rs,confirm_view.rs}`):
- **`standing_link` reads `content_address` as an `Option`** and refuses a NULL as a NODE-STATE refusal (a pre-#115
  row; db/018's `ADD COLUMN IF NOT EXISTS` leaves it nullable). **Tempting wrong fix:** back to `r.get(0)` "because
  the column is NOT NULL" — it is only on a fresh database. The remedy names the HEAL first (`cairn-node reproject
  --prefix identity.`, no deletes: replaying the winner beats a NULL address, db/002's `COALESCE(cur_addr, '\x')`)
  and a whole `--rebuild` only as the fallback (it truncates every projection and holds chart reads). Never name
  `--prefix identity. --rebuild` — `cairn_reproject` refuses it (it would truncate `patient_chart`). The test runs both.
- **A row's doubt mark crosses the wire only as `CandidateList`'s additive `doubted_rows`** (`list_wire.rs`). The
  row's own flag is the one in-memory truth; `doubted_rows` exists only on the wire, absent when empty. Never put
  the mark into `PersonRow`'s serde (its bare member array is what the attested list is built from), and never give
  `CandidateList` a second in-memory doubt field.
- **Both bridge remedies name the unlink's door**: `DOUBTED_LINK_INSTRUCTION` and the review panel's `Separated` note
  (`confirm_view.rs`) say to open "Review this link…" on a link still joining the pair, "marked in doubt or not" —
  never "unmarked" (a joining link can also clash).
- **The first lock poll starts on `DOMContentLoaded`**, never at `main.js`'s top level, and **no script tag may be
  `defer`, `async` or a module**: that ordering is what makes `updateLinkLock` exist when the answer lands.

**⇒ R7'S DURABLE RULES (PR #788) — do not undo any of these** (pins: `tests/{unverified_link_note,
duplicate_check_status}.rs`, `cairn-gui-tauri/src/duplicates/{read_order_tests,view_catchup_tests}.rs`,
`cairn-gui-tauri/src/link/record_links_unverified_tests.rs`, `unlink_tests.rs::an_unread_note_list_never_blocks_a_verdict`):
- **A soft veto stays OUT of `cairn_chart_set_doubts`.** `cairn_chart_set_unverified_links` is a separate function so
  the R1b hold, `EXISTS` over the doubts, can never see a `degrade_hold` (db/016: it must never demote a link). Folding
  it in as a third kind would freeze sign-off on formatting differences.
- **The banner and the tray read the check state BEFORE the open pairs** (`duplicate_section`, `tray_count_impl`). The
  worker never deletes a notice before the proposals it caused are committed (`queue_db.clear_notices`); pairs read
  first can miss one that
  lands between the reads, then "nothing waiting" — a silent "checked, none open". **Tempting wrong fix:** "tidy" the
  reads back into display order.
- **A failed unverified-notes read never costs the links list** — `RecordLinks.unverified` is a `Result`, the lines
  and their doors draw, and the section says the notes are unknown. A line without a note must never be read as
  "verified", so an unread note is said, never shown as none.
- **`read_snapshot` reads every status column with `try_get`** (`status_column`): `cairn-node duplicate-check` never
  replays the schema (#726), so a binary newer than its database meets an older function shape and must say so, not
  panic.
- **db/056's heal guard is the NEWEST OUT column's absence** (`charts_changing`), so one guard covers every older shape.
  A future column change moves the guard to the new newest column.

**⇒ R6'S DURABLE RULES (PR #777) — do not undo any of these** (pins: `tests/{link_doubts,confirm_link,unlink_as_shown,
search_doubted_row}.rs`, `cairn-gui-tauri/src/link/{confirm_view.rs,record_links.rs,unlink_tests.rs,
unlink_view_tests.rs,unlink_offer_tests.rs}`, the `unlink.js` field-name drift guards):
- **ONE SQL spelling of "doubted": db/054's `cairn_chart_set_doubts`** (`clash` = a `link_veto_flag` row or an
  un-attested standing link that trips the hard veto now; `separated` = an attested unlink between two charts the set
  still joins). R1b's hold, `cairn_chart_set_has_doubted_link`, is `EXISTS` over it, so the hold and the window's
  per-line marks cannot disagree. Never write a second predicate for either.
- **Never sign a verdict without the standing address the panel read.** `RecordsShown::pair` is re-checked under
  CARNLK for BOTH the confirm (`chart_link::confirm_link_as_shown`) and the panel's Unlink
  (`chart_link::unlink_charts_as_shown_from`): a colleague's judgement of the SAME pair landing mid-review, while a
  third chart keeps the record joined, leaves the set unchanged — only the pair's address shows it. A changed pair is
  refused (`stale_pair`); an empty address is refused (`require_standing`). The CLI's `unlink-charts` (no panel)
  keeps `unlink_charts`. **When you guard one verdict, ask the same question of every verdict on that panel.**
- **`standing_agrees`: the list's read and the panel's read must agree, or NEITHER verdict is offered** (a standing
  read of nothing included — `PAIR_CHANGED_MID_READ`). It checks that two reads agree; it is NOT a second doubt rule.
- **The doubt never enters the signed list or a row's JSON**: `displayed_charts()`, `SearchAttestation`, the cap and
  the ranking ignore it (`search_doubted_row.rs` compares the attestation's bytes with and without it).
- **Both verdicts are disabled in flight and hidden once one lands — Outranked included** (`unlink.js`, and R5a's
  `link.js`). `same_verdict` offers nothing for an attested link and needs both charts of the pair known held.
- **"Review this link…" / "Review this judgement…" is the ONE door per line**; the panel, not the list, decides which
  verdicts apply. The clash mark says "while", never "until" (two doubted links: judging one leaves the hold), names
  what CAN be signed ("only lines recorded on this chart alone" — hazard.rs withholds shared lines too), and
  `DOUBTED_LINK_INSTRUCTION` names both doors.
- **A door carries the attestation its line showed** (`RecordLinkView::attested` → `compare_linked`): a pair judged
  since the list was drawn is refused (`line_still_shows`, `LINE_STALE`) before any panel is drawn — otherwise an
  attested link opens a silent panel offering only Unlink. The list's two reads (`read_record_links`) share ONE
  read-only REPEATABLE READ snapshot, and a clash naming no standing link is said (naming "Read this chart again" since #797), never dropped.
- **Any recorded verdict disarms a newer comparison of the same pair**, in both panels (`disarmNewerReview` /
  `disarmNewerComparison` — the scripts share one global scope, so never one name for both). Never key it on
  `reload`: "Different people" reloads even when every pair was outranked.
- **One Rust spelling of the doubt boolean: `patient::edges::record_is_doubted`** — the hold (via the medication
  read) and the duplicate-review tray's doubted side (#723 on the tray) both ask it; an unread doubt is a worded
  side error, never "not doubted". An empty standing address is refused by the node itself (`judge()`), with the
  true reason.

**⇒ R5b'S DURABLE RULES (PR #749) — do not undo any of these** (pins: `matcher/tests/test_{judged,proposal_retraction}.py`,
`tests/{auto_apply,duplicate_worklist,duplicate_review_flags}.rs`, `duplicate_review/{worklist.rs,tests.rs}`,
`cairn-gui-tauri/src/worklist/`):
- **The matcher's skip rule IS db/057's** (save ADR-0078 decision 1's clamp-and-flag case; `judged.py`, the six-case
  drift test): one record, or an ATTESTED `patient_link` row. "Judged" is not "a human judged it" — only an UNLINK needs
  attestation to settle a pair (ADR-0078). `AWAITING_HUMAN` is exactly `pending` + `review`: the matcher never
  retracts a human's verdict.
- **`DISPUTED_SQL` is the ONE spelling of "disputed"** (a standing un-attested unlink): the banner, the worklist's flags
  and `auto_apply` step 2b format it in — never write `state = 'unlink' AND NOT attested` again. Auto-apply never links
  over a dispute: the proposal goes to `review`, with no event and no `patient_link` row.
- **The count and the list share one grouping** (`RECORDS_FROM`, `LOW_RECORD`/`HIGH_RECORD`), so they cannot
  disagree. **The count never evaluates the veto or the dispute; flags are read only for the shown pairs**
  (`read_pair_flags`, one statement); **`candidate_read`'s `read_display_facts` is the one display read** — ONE batched
  call per list read. An open tray re-reads on every return, over the connection the search shares.
- **The tray re-reads on every return because `close_chart` clears `shown`** (drop it and Review is refused).
  **"Different people" refuses an `accepted` pair IN THE BACKEND** (`DifferentPeople::AcceptedAsSame`, #736). **A
  failed record read is worded per side and withholds Review** when the newer side failed; it never fails the list.
- **The tray's Review buttons are `data-candidate`** (disabled while a registration saves); **the status line sits
  OUTSIDE the collapsed `<details>`** ("(0)" must never read as "checked, none"); never `role="alert"` or focus-taking.

**⇒ R5a'S DURABLE RULES (PR #735)** (pins: `tests/{match_proposal_open,duplicate_review}.rs`,
`cairn-gui-tauri/src/duplicates/{view_tests.rs,verdict_tests.rs,tests.rs}`):
- **`match_proposal_open` (db/057) is the ONE "still needs a human" predicate**: a pair leaves it only when it reads as
  one record or an **ATTESTED** unlink stands — **never count an un-attested unlink** (any unreviewed writer could
  clear a banner). Its statuses are pinned to `OPEN_PROPOSAL_STATUSES`.
- **An absent banner means only "checked, none open"**: every failed read is a worded line (`duplicates/view.rs`); a
  Current node never appends "up to date" after an unheld member's "not yet run" line.
- **`AppState::shown` is NOT widened**: Review's other chart is admitted by `duplicates::admit_other` re-reading the view
  (after `ALREADY_IN_RECORD`). **`duplicate_section` reads in two phases** — never hold `state.db` (a non-re-entrant
  tokio Mutex) across `read_chart_of` / `chart_set_of`.
- **"Different people" = an attested unlink on every pair STILL open between the two records, read fresh**; a pair's
  failure is carried, never `?`; a failed pair reads "For charts A and B, <its own sentence continued>" (#740 — never
  "not recorded", never a stutter); all-failed → one ErrorView naming every pair. **The veto note is computed fresh**
  (`cairn_match_veto`), never from stored `veto_findings`.
- **Webview:** `duplicates.js` loads after `main.js` and BEFORE `funnel.js` (whose last line boots) — never a `typeof`
  guard on `render`; "Different people" shows only on the banner's own comparison token.

**⇒ #725'S DURABLE RULES** (pins: `matcher/tests/test_{blocking_sql_shape,targeted_blocking,pure_modules_import_without_psycopg}.py`):
- **No range arm joins a sex scan.** A chart's blocking sexes ride on its birth-window row (`sexed_window`, one join by
  `patient_id`); the `+sex` arm is a FILTER (`anchor_sexes && member_sexes`). A CTE scan has no statistics, so an
  equi-join of two sex scans on a ~2-valued column is a planner cross-product (14.7 M rows).
- **The anchored per-chart range statement is the sweep's plus `relevant_anchor` and `_ANCHOR_CLAUSE`, nothing
  else**; the clause restricts the ANCHOR, never the member. Add a range arm to `_RANGE_ARMS` and both statements get it.
- **`pipeline/db.py` is the ONLY module that imports psycopg at import time** (plus `eval/measure_check`); blocking SQL
  lives in the pure `pipeline/blocking_sql.py`; a psycopg-needing test imports inside or `importorskip`s; a guard
  collects the suite without psycopg. **The canary runs capped too** (10, 6). Reproduce CI's pure job with
  `CAIRN_ALLOW_DB_SKIP=1 uv run --isolated pytest`.

**⇒ R4'S DURABLE RULES (PR #724)** (pins: `tests/{match_pending,duplicate_check}.rs`,
`matcher/tests/test_{check_chart,worker_drain,watch_backoff,targeted_blocking,judged}.py`):
- **The hook never raises and never waits** (`the_hook_has_no_raising_path`, the `lock_timeout` test): a plain INSERT,
  a null guard, no RAISE/EXCEPTION/ON CONFLICT. **Never "fix" it with `EXCEPTION WHEN OTHERS`** — a swallowed failure
  is a silently skipped check. `patient_chart`'s hook is INSERT-only; the catalogue test pins `tgtype` and the `uuid`
  arguments.
- **Delete exactly the notice ids READ, captured before the projection reads — never `id <= max`** (a bigserial is
  assigned at INSERT, not commit); bulk mode captures before the sweep. **No read transaction across slow work**
  (`check_chart` rolls back after its read-prep and after every assess); a no-op DDL replay waits behind any reader (#726).
- **"Behind" = charts waiting AND no completed work AND the OLDEST notice older than 5 min** (`quiet_age_s`).
  **Progress is stamped ONLY by completed work** — never before work, never for a failed pair: either masks a
  crash-loop as "running" (`test_a_sweep_whose_blocking_raises_stamps_no_progress`, `…every_pair_fails…`).
- **`cairn_chart_check_pending` is TRUE until a worker has run, while a notice waits, and for a chart not held here**
  (never a false "checked"). **The worker never links** (it writes only via `persist`) and **never proposes a judged
  pair**, in either mode or in reconciliation. **`me` is canonicalised once** (`str(uuid.UUID(...))`).
- **`cairn-node duplicate-check` uses `db::connect`, never the schema replay** (it is cron-able). Status wording lives
  only in `duplicate_check.rs` (goldens); the runbook quotes it verbatim.

**⇒ R3'S DURABLE RULES (ADR-0076 decision 6, PR #721)** (pins: `person.rs`, `tests/search_by_person.rs`,
`attestation.rs`, `candidate_text.rs`, `prompt.rs`, `search_person.rs`, `commands.rs`, the byte-identical goldens):
- **Rows are built ONLY by `group_by_person`** (pure): a row sits at its best-ranked member; matched members first in
  rank order, then unmatched oldest first; every chart exactly once; the seven ADR-0075 keys rank CHARTS.
- **The signed list is ONLY `CandidateList::displayed_charts()`**; **the cap counts ROWS and never splits one**
  (`withheld` counts people; `shown_charts` is signed).
- **A chart not held here reads `Unknown` and never sets `incomplete`; a HELD chart with no readable name does. A
  missing component fails the search — never a row of one.**
- **Every member of every shown row can be opened, and nothing else** (`AppState::shown`); the window never picks the
  chart for the clerk. Single-chart rows are byte-identical to before R3; people-vs-charts wording lives in
  `funnel/rows.rs`. The row label reaches a screen reader via `aria-describedby` — unpinned (#332).

**⇒ R1b'S DURABLE RULES (#697 (b), #701, PR #717):**
- **A doubted set withholds every line not recorded ONLY on the opened chart** (`medication::hazard::wrong_chart_reasons`,
  pure; an EMPTY chart list is not "only on the opened chart"); the opened chart's own lines stay signable and every
  line stays on screen. Pins: `hazard.rs`, `tests/doubted_link_withholds.rs` (its mirror test, opened from the OTHER
  chart, is the only one proving `opened` is used).
- **`cross_patient` is KEPT, set from the same reasons map as `wrong_chart`**; readers go through
  `is_wrong_chart_hazard()` or `hazard_reasons()`, never either field alone (#720). **`hazard_reasons()` is
  STATUS-BLIND; `withheld_reasons()` / `withheld_rows()` say what THIS gesture withholds** — anything saying a line
  "cannot be signed until …" asks them.
- **The rule judges every chart a group touches, INCLUDING each member thread's own chart** (`read.rs`
  `touched_charts`); a row missing from the reasons map is a hazard (`row_reasons`).
- **"Doubted" has three cases** — a `link_veto_flag` row; an UN-attested standing link tripping the hard veto now
  (#220's path); an ATTESTED unlink between two charts still in the set (the A–C–X bridge) — spelled once, since R6, in
  `cairn_chart_set_doubts`. It reads the STORED `pl.attested` (#701), never re-derives it, and takes one record's
  charts (`person_charts`). **An UN-attested unlink inside the set is deliberately NOT a doubt** (any unreviewed writer
  could freeze sign-off).
- **The remedy is a judgement of the LINKS, never thread separation, and never promises "either judgement lifts this
  hold"** (`DOUBTED_LINK_INSTRUCTION`); each sentence must stay true in every case shown. The CLI prints it ONCE below
  the list (`list_text::doubted_link_note`); the outside-set row lines stay byte-identical.

**⇒ THE LINK PRECEDENCE FLOOR'S DURABLE RULES (R2a, ADR-0076 decision 5):**
- **`patient_link`'s winner order is ATTESTED FIRST, then HLC** (`cairn_link_overlay_wins`, db/018). "Attested" is ONE
  definition (`attester_key IS NOT NULL AND cairn_attestation_vouched(event_id)`), evaluated once per applied event and
  STORED with the winner; a new reader reads `pl.attested`, never a second spelling.
- **`db/055`'s EXISTENCE is load-bearing** (generation 55; the loader's heal re-decides winners the old order chose).
  **Never fold it into db/018, and never delete its re-fold block** — cairn-sync heals with the OLD applier (#703). A
  peer on an older binary ranks the old way (identity.md §5.2).
- **Recorded is not took effect**: every judgement path reads back, in its own transaction, what now stands
  (`chart_link::standing_link`, `LinkEffect`); auto-apply rolls back a matcher link that is not itself the winner.
- **One lock order: the `match_proposal` row, then db/018's CARNLK** (reversed, a same-pair race deadlocks, 40P01;
  pinned in `tests/chart_link_machine.rs`).
- **A judgement is a human's** (`Reviewer`: the human key signs AND attests; no node-key fallback). `link` needs BOTH
  charts held; `unlink` also admits a displayed member not held here, filed under the HELD chart (db/005 step 8b).
  Only OPEN proposals move.

**⇒ R2b-1'S DURABLE RULES ("Same person as…", PR #707)** (pins: `chart_compare.rs`, `link/same_person_tests.rs`,
`link/{view_tests,judgement_view_tests}.rs`, `chart_link.rs`):
- **The comparison is SET against SET** (`cross_vetoes` over every left×right pair, or a B–C clash goes unseen).
  **Link names BOTH COMPARED sets**, sent back verbatim; a changed set is `THIS_CHANGED` / `OTHER_CHANGED`.
- **Every panel message goes through `setMessage`, never a bare `.textContent`** (an empty status line is `hidden`).
  No JS harness pins it (#332): a headless walk asserts `getComputedStyle(el).display !== "none"`.
- **A partially-read comparison offers no Link. Absence and precision wording lives in `link/view.rs`** (an absent fact
  on a chart not held here reads "unknown — registration not yet received here"; struck names are listed; a coarse DOB
  names its precision). **An identifier finding is never "verified"**; only a dob/sex-at-birth hard veto (both
  provenance-rank ≥ 60) is "Verified facts differ". **A locked key is not a verdict** (`Retry::Now`).
- **Code selects by a typed field, never a display label** (`MedListRowView::current`); the panel's search counts what
  it shows. **`chart_link`'s pre-check refusals are marked verdicts — never a bare `anyhow::bail!`** (same chart →
  `RefusalScope::Input`; not held or a non-human key → `NodeState`, #702).

**⇒ R2b-2'S DURABLE RULES ("Not the same person…", ADR-0077, PR #711)** (pins: `admit_tests.rs`,
`tests/{unlink_from_record,chart_link,record_edges}.rs`, `unlink_view_tests.rs`):
- **`FiledUnder::RecordOf` is UNLINK-ONLY, re-checked UNDER CARNLK in `assert_link_in_tx`** (after the proposal-row
  lock), then `person_charts(opened)` is re-read; it lives in the signing core because `assert_link_in_tx` is `pub`
  (`a_peer_unlink_landing_mid_judgement_is_seen_before_anything_is_signed`).
- **"Still joined?" asks the SUBJECTS** (`high ∈ person_charts(low)`), never the filed-under or open chart; the record
  REPORTED is the open chart's. **An opened chart / `--from` unrelated to the pair is REFUSED.**
- **The list is per LINK, never per member**; `record_edges` keeps `state = 'link'`. **The unlink panel is its OWN
  `<section>`, mutually exclusive with the link panel**; after an unlink focus goes to the patient heading.
  **`Outranked` never says a retry "changes nothing"** (the merge is bounded at 24 h of drift). A locked key names its
  own button; `standing_edge`'s unread list is `Retry::Now`.
- **An open chart's refusal scope follows the FACT** (`AdmitRefusal`): not held → `NodeState`; held but its record
  lacks the pair → `Stale` since #797 (routed by type, see that block; it was `Input` and said "reload the chart").
- **The webview-field guards scan `main.js`, `funnel.js` and `unlink.js`**; `link.js` has only its search read guarded
  (the rest is #715). `renderLinks` stays in `main.js`. Never say "by the matcher" for an un-attested link ("without a
  clinician's confirmation on record here").

**⇒ THE COMBINED READ'S DURABLE RULES (R1, ADR-0076):**
- **A combined list's duplicate flag is db/054's `cairn_medication_duplicate_groups` over the SET**, never the
  per-patient `patient_medication_reconciliation_flag` (one drug on two LINKED charts would go unflagged — a
  double-dose reading hazard). Its dup_key stays db/033's (`medication_dup_key_drift.rs`).
- **The read selects groups by MEMBERSHIP over the set** (filtering by `patient_id` is #334 returning);
  `list_chart_set_medications` is PRIVATE. **A cease on a wrong-chart-hazard line stops only the opened chart's
  threads** and names the rest (`chart_set::cease_plan`).
- **`MedicationRow::display_chart`** (serialized `patient_id`) is where the group DISPLAYS, never a write target — use
  `source_charts` / `MemberVouch::patient_id`. **Sign-off and cease act on each thread's OWN chart** (the strict door
  refuses a misfiled attestation since #689's fix).
- **A linked chart not held here reads trust `unknown`** (`person::trust_of`). **A missing group** uses
  `MISSING_GROUP_INSTRUCTION`, never the separation remedy.
- **EVERY CHART COMMAND NAMES THE DISPLAYED CHART SET** (decision 3): `displayed_patient` FIRST, then the set
  (`chart_set.rs`); `sign_off_medication_list(…, displayed: Option<&ChartSet>)` refuses a changed set; `None` is the
  CLI. A failed member-identity read keeps the list and warns (`ChartPane::members_error`).
- **A never-linked chart gains no header, label or line** (golden
  `combined_read.rs::a_never_linked_chart_reads_exactly_as_before`). Sign-off over a non-empty
  `groups_missing_from_chart` has no DB coverage (same class as #333).

**⇒ THE FUNNEL'S DURABLE RULES — do not undo any of these** (full text: the funnel design page's dated notes):
- **`search_patients` RANKS BY SEVEN KEYS** (`rank_candidates`, inputs in `patient/search_rank.rs`): passes →
  identifier → a §5.4 callsign typed WHOLE → name tokens (exact or a ≥3-byte prefix, as `db/046`; callsigns never
  split) → DOB near-miss → EXACT tokens → chart age. It only REORDERS; each key has a victim if dropped
  (`patient_search_ranking.rs`). Tokens count over the RETAINED names (`patient_name`, #349).
- **`incomplete` is the SEARCH's partiality only; truncation is `withheld`** (ADR-0075; folding them re-creates #671;
  `attestation_through_the_port.rs`). **Never raise `PROMPT_CAP` to make a number look better.**
- **The raw typed name travels WITH its token** (`FunnelSession`); an older revision's search is DROPPED; the
  webview forgets its token on every edit. **`require_provisioned` runs in the register command, BEFORE `take`.**
- **Only a chart some list on screen showed can be opened** (`AppState::shown`; since R3 every member of a row).
  **EVERY CHART COMMAND NAMES THE CHART ON SCREEN** (`AppState::displayed_patient`; 2c's Critical — a sign-off once
  signed patient B on a review of A's list): each has a `*_impl` pinned by a not-on-screen test; `open_patient` is
  private to `funnel` — **never let a new chart command resolve `open_patient()` alone.** A registration never
  writes, or switches charts, behind an open chart (`register_impl` + `open_after_registering`).
- **A click within 800 ms of a prompt landing is "show me", never "register"** (`PROMPT_READ_GUARD_MS`, SOFT POLICY
  in `funnel.js`, #677). **The standing probe matches all four `ActorStanding` arms** (`provisioning_sentence` → `standing_sentence`) (a `Retired` key meets db/004's
  resurrection refusal, #152), paired with the key it probed (#670).
- **Every sentence and its retry advice lives in `funnel/view.rs`** (`Retry::{Now, AfterOperator, Never}`; person-row
  wording in `funnel/rows.rs`). A refusal (`P0001`, `DeliberateRefusal`) is not an outage; the remainder is #655.
- **`TokenStore::settle` ends a `take`**; a success INVALIDATES; `discard` does NOT clear `in_flight`. **A dropped
  `register` future still latches the store (#669) and `register` is cancellation-unsafe (#649): never race it
  against a timeout or `select!`.** The step-3 trigger is advisory, never a gate.
- **Nothing provisions an actor on a write path** (`enrolment_is_never_a_write_side_effect.rs` — **when it goes red,
  do not add your call site to `ALLOWED`**). `resolve_matcher_actor` still enrols (#663); a SUPERSEDED key's class is
  undecided (#666, then #664). **`init` must not `?` its enrolment.**
- **`cairn-gui-live` holds the DB-backed ports.** The P0001 rule has three homes (#652); the `gui` job's step-level
  `CAIRN_ALLOW_DB_SKIP=1` is load-bearing and its deletion invisible (#656); derived truncate lists miss identity
  tables (#658). **`--mock` holds ONE `MockData`** (half of #668); its matching is NOT db/046's and it has no link
  concept — every mock row is a person of one (#722).

## The eighteen traps

**Eighteen traps. Each is a step a next session takes in good faith.** The full argument for each is in its ADR,
its test's header or ROADMAP; this keeps the rule, the pin and the tempting wrong fix.

1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY** (`keystore::adopt_derived_unwrap_secret`); anywhere
   else re-creates the #495 coupling. `unwrap_secret_is_not_derived.rs` sweeps every shipping tree and asserts each
   allow-list entry is live: **when it fails, delete the entry; never add one.** It and `is_a_test_gate_attribute`
   (pgrx's `#[cfg(any(test, feature = "pg_test"))]`) move **together**.
2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6): a database
   recreated under an existing key file needs `cairn-node establish-unwrap-key` before its first sealed write. Never
   make a red fixture green by weakening `ensure_unwrap_key`.
3. **`cairn-sync`'s ONE derived fallback** (no `<key>.unwrap` AND the derived key equals the registered one → start,
   warn every startup). An absent file may fall back; a present-but-unusable one never may — **never merge those two
   arms.** Retiring the fallback is **#514**.
4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It registers a secret
   from the NEW seed and the singleton registrar then refuses the real key for good; recover the export, or restore
   again into a fresh database.
5. **⇒ `Secret32` DOES NOT SEPARATE ONE SECRET ROLE FROM ANOTHER (#511)** — `Secret32::from_bytes(sk.to_bytes())`
   compiles. `secret32_conversions_are_named.rs` pins exactly **two** seed→unwrap-secret sites (the ADR-0066
   migration, trap 3's fallback); a third is the defect returning. `unwrap_secret_is_the_signing_seed` and
   `secret_opens_the_carried_custody` are not made redundant by the types.
6. **`init` refuses a database that already has custody registered** (it reads `node_unwrap_key` first) and names
   `establish-unwrap-key` as the remedy — on a restored node read trap 4 first.
7. **⇒ A BODY SHREDDED AFTER A CAPTURE KEEPS ITS DEK ON THAT MEDIUM. NOT A LEAK — DO NOT "FIX" IT.** A backup
   restores the state at capture time; invalidating old backups is rotation policy (principle 9; ADR-0067 decision
   2). **Never filter old segments** (`medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`).
   Operator half owed: **#589**.
8. **⇒ A PEN ROW WHOSE DEK "BELONGS TO ANOTHER NODE" IS STILL RETAINED (#578).** *"Did not open with the key we
   have now"* is not *"not ours"* (the right `<key>.unwrap` may be on a USB stick); the escape is db/021's human
   `acked`. Pinned by `requeue_releases_custody.rs::a_penned_dek_from_another_node_is_kept_until_a_human_decides_otherwise`
   and `requeue_retains_unlanded_custody.rs`. **Look-alike fix:** keying the guard on the opened `dek`, not the
   row's `dek_wrapped`, passes the headline test and skips the check whenever the key did not open. Keep the Rust
   check although the floor also catches it — it says WHY.
9. **RETIRED (#584, ADR-0070); live residue: never remove or move the `cairn_project_late_custody` calls** — in both
   doors AFTER the substitution guard, in db/020 BEFORE the `cairn.remote_apply` clear (wrapping db/005's call in
   `cairn.remote_apply = 'on'` turns a strict refusal into a flag). Pinned in `late_custody_reaches_the_chart.rs`,
   each placement with its own positive control. (#597's notice is still misleading.)
10. **⇒ WHEN `late_custody_guards.rs` FIRES, THE GUARD IS RIGHT.** Every `event_clear` writer (`apply_remote_event`,
    `submit_event`) calls `cairn_project_late_custody`; every applier reading custody is `heal_safe = TRUE`. **Wrong
    fixes:** a custody reader `heal_safe = false` (a silent owed rebuild — **#610**), flipping a non-idempotent one
    to `TRUE`, exempting a writer. A third writer is a DECISION; a custody read in a helper, `MERGE` or dynamic
    `EXECUTE` is invisible to the guard — review by hand.
11. **⇒ `restore` EXITS 3 FOR AN INCOMPLETE RECOVERY; A TEST EXPECTING 1 IS THE BUG (#594, ADR-0071).** Exit 1 =
    BLOCKED, checked FIRST. **Wrong fixes:** `success()` for a torn medium; a wrong recovery code exiting 1 (its end
    state is the no-registry state, 3); "simplifying" `past_chain_break` to the medium's clinical total (M8). The
    precedence's ONE test is `restore_cli_surface.rs::without_the_flag_a_piped_restore_still_inherits_no_custody`
    — never weaken it to `!success()`; `restore_exit_vocabulary.rs` pins the value,
    `exit_incomplete_matches_cairn_nodes_restore` the agreement. ⚠️ **Write `Some(n)`, never `!status.success()`.**
    ⚠️ An ACKED pen row counts in `penned` and `requeue` will not clear it; do not subtract it. Residuals **#611**,
    **#616**.
12. **⇒ THE SUBSTITUTION REFUSAL HAS ONE HOME (`cairn_refuse_substitution`, db/053), AND A DOOR CALLS IT.** `IS
    DISTINCT FROM` ("cannot tell" is a refusal); a fourth inline copy is #608's `<>` fail-open returning
    (`substitution_guard_is_single_source.rs`). db/005/db/020 read under `GET DIAGNOSTICS`; **db/009 reads
    unconditionally — do NOT "tidy" it into a `ROW_COUNT` check**, its guard AFTER the `IF/ELSE` (M7); a db/009
    refusal aborts the WHOLE restore, correctly. A DEFERRED clinical record is REPORTED, never an exit-3 cause. All
    five event-log doors: `substitution_guard_covers_every_writer.rs` over `pg_proc`. Residuals #608, #605, #569, #622.
13. **⇒ THE NODE PLANE'S SUBSTITUTION REFUSAL LIVES IN EACH DOOR'S TAIL; THE PULLER ASKS THE TABLE, NOT THE ERROR
    (#619, ADR-0073).** An early `RETURN` bypasses the guard (the genesis arm, safe only for want of an `ON
    CONFLICT`) and the catalogue guard will NOT notice a new one. **Wrong fixes:** a dedicated SQLSTATE (P0001 is a
    contract, #228); matching the sentence; penning only when the GUARD raised
    (`a_rival_refused_by_an_earlier_check_is_still_penned`); a skip for the failed-lookup freeze
    (`node_substitution_lookup_freezes.rs`, M10); `offered` over the whole frame (#268's alarm fatigue, M11). A
    sixth writer gets the call. Residuals #620, #622, #605.
14. **⇒ A DOOR THAT LETS POSTGRES RAISE ON CALLER-SUPPLIED BYTES BREAKS THE P0001 CONTRACT (#621, ADR-0074).** Every
    signed-bytes field goes through a P0001 helper (`cairn_uuid_or_raise` on `pg_input_is_valid`,
    `cairn_hlc_nonneg_or_raise`, `cairn_node_role_or_raise`); `node_door_input_guards.rs` fails on a bare `::uuid`.
    **Wrong moves:** a REGEX narrower than the cast (M9); re-inlining the role list into the CHECK (M10); `USING
    ERRCODE`; deleting the CHECKs (the raw-SQL floor). ⚠️ **The puller's default for an UNKNOWN SQLSTATE is pen;
    "tightening" it to freeze reinstates #621.** The local classes (`08 40 42 53 55 57 58`, no SQLSTATE) equal
    `cairn-sync`'s (`sqlstate_classes_agree.rs`; merging the copies is #626); **`XX001`/`XX002` are LOCAL**. Such a
    pen row leaves only by applying or an ack. **The role CHECK stays `NOT VALID`** (a validating pair re-scans on
    every connect). Known exception: `cairn_body`'s `22P05` on a NUL (**#628**). Residuals #626, #629, #605, #268,
    #625, #631, #632, #633, #634.
15. **⇒ THREE THINGS IN `db/046`'s PASS 3 LOOK LIKE NOISE AND ARE EACH WORTH HUNDREDS OF MS ON A PI (#639).** (a)
    **`OFFSET 0` is an optimisation fence** — removing it changes no result, so nothing fails. (b) **The lateral is
    `UNION ALL` on purpose.** (c) **The parts-branch skip must test the same `lower(normalize(pn.value, NFC) COLLATE
    pg_unicode_fast)` it splits** — U+0130 `İ` lowercases to `i` + U+0307; dropping `lower` LOSES A TOKEN.
    `the_subset_argument_holds_for_every_unicode_code_point` checks all 1,114,111 code points;
    `the_subset_probe_still_describes_the_query_db046_runs` pins the composed expression. **A probe list is a sample,
    not an argument; re-measure with `scripts/measure_patient_search.py`, do not reason.** Residuals #641, #640,
    #643, #637.
16. **⇒ A GUARD AND THE THING IT GUARDS MUST BE ASKED ABOUT THE SAME STRING** — pin the COMPOSED expression as a
    literal (`include_str!` + `contains`), not its pieces.
17. **⇒ EVERY NAME `lower()` NAMES ITS COLLATION: `COLLATE pg_unicode_fast` (#775).** A bare `lower()` follows the
    cluster (ICU locally: full mapping; CI's libc `C.UTF-8`: simple — `İ`→`i`, `Σ`→`σ`), so a stored token and a
    typed one could case-map differently and a chart went unfindable. The COLLATE also moves the regex classes
    (`\s` = Unicode White_Space: NBSP now splits). Search side and sweep side (matcher `blocking_sql.py`) must move
    TOGETHER — the db/046 DRIFT invariant (sweep-paired ⊆ search-found) — and a matcher-side change bumps
    `__version__` (blocking SQL is not in the digest). Pins: `db046_names_the_collation_at_exactly_its_four_lowercasing_sites`,
    the drift fixtures, `test_eval_generator_sync.py`. A libc test DB is still the way to reproduce CI for anything
    else locale-shaped: `CREATE DATABASE cairn_test_libc TEMPLATE template0 LOCALE_PROVIDER libc LOCALE
    'en_US.UTF-8' ENCODING UTF8;` + `CREATE EXTENSION cairn_pgx;`.
18. **⇒ A TARGETED `cargo test --test X` REPORTING `ok` IS NOT PROOF — ONLY THE SWEEP IS (#661).** A stale test
    binary (shared `target/`, a running rust-analyzer) reported `ok` twice against source it contradicts — silent
    and green. **Gate on the sweep**; `CARGO_TARGET_DIR=/tmp/…` when an IDE is open; re-run every CI gate in CI's
    order AFTER the final edit.
