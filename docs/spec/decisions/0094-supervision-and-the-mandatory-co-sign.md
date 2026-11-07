# ADR-0094 — Supervision and the mandatory co-sign

- **Status:** Accepted (supersedes three parts of [ADR-0092](0092-the-vouch-and-the-sign-off.md): decision 1's offer
  rule, decision 11's `needs_sign_off`, and decision 12's rule for when " — not co-signed" clears and its limit to a
  student's part. Decisions 13, 14 and 15 below say what replaces each. Amends
  [ADR-0093](0093-a-repudiated-signature-counts-as-unsigned.md) (d), as (b) below says. Builds the slice 3c-3 design's
  §3.1–§3.3)
- **Date:** 2026-11-07
- **Spec version at acceptance:** 0.96
- **Issues:** draft PR [#1013](https://github.com/cairn-ehr/cairn-ehr/pull/1013) (GUI sign-in slice 3c-3a). Design:
  `docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md` (§3.1–§3.3);
  plan: `docs/superpowers/plans/2026-11-07-sign-in-slice-3c-3a-supervision.md`. Touches
  [#1010](https://github.com/cairn-ehr/cairn-ehr/issues/1010) (not guaranteed (b)) and settles the later-vouch mark
  ADR-0092's Consequences left to [#1003](https://github.com/cairn-ehr/cairn-ehr/issues/1003).
- **Builds on:** [ADR-0092](0092-the-vouch-and-the-sign-off.md) (the vouch, the sign-off, the pure rules and the
  words) · [ADR-0093](0093-a-repudiated-signature-counts-as-unsigned.md) (a repudiated signature counts as unsigned) ·
  [ADR-0085](0085-in-person-enrolment-and-re-key.md) (the enroller grant, *unconfirmed*, `rotate-key` keeps the actor) ·
  [ADR-0083](0083-the-human-key-vault.md) (the vault, the strip) · [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)
  (additive-only evolution) · [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (logic below the UI)

## Context

On paper, a probationary doctor's entries are countersigned by a supervisor until the supervisors end the
probation. Ending it does not countersign yesterday's entries. Who supervises whom is a list or a note in a staff
file. A supervisor may also initial a registrar's entry as reviewed, though nothing obliged them to.

Until this slice only a student's part waited for a co-signature, because the `student` role is pinned on the actor
(ADR-0092 decision 10). A co-sign by any non-student other than the writer cleared it, and nothing said who
supervised whom. This slice, **3c-3a**, records supervision, makes "— not co-signed" follow that record as it stood
when the part was signed, lets only a supervisor clear a mandatory co-sign while the writer has one, and adds
"Co-sign as reviewed". No event type, twin, floor rule or projection changes. The record lives in the actor registry
(db/004, db/052 in place), the doors in db/072 (`SCHEMA_GENERATION` 72), and the rules in pure Rust over `NoteRow`.

## Decision

### The maintainer's rulings

1. **The record is ops on `actor_event`** (2026-11-07, plan review), the table where `grant-enroller` already lives.
   It is carried by the registry's existing backup slot and restore door (db/052). A separate table would have had to
   duplicate both: `vault_*` tables are in no backup (ADR-0083).
2. **Nobody acts on their own supervision as the supervisee** (2026-11-07). This is wider than design §3.1's "no
   self-dealing". Otherwise a supervisee holding the assigner right could unlink their supervisors and fall into the
   no-supervisor fallback (decision 13), where any clinician's co-sign clears theirs. Without the assigner right, stepping
   away is the one act a link's own party may make; an assigner may also link themselves as someone else's
   supervisor (decision 9).
3. **A re-sign under an UNCONFIRMED key counts** (2026-11-07; this closes the question 3c-3b left open). It reads
   "(signer unconfirmed)", as built. `vouch::re_signs` asks only that the vouch's key is not repudiated and is of the
   same actor, never whether it is confirmed. ADR-0085: unconfirmed never blocks signing by itself.

### The record (db/004, db/052)

4. **Six new ops, each a row with NO key and `approved_by NOT NULL`:**
   - `link-supervisor` / `unlink-supervisor` sit on the SUPERVISEE's `actor_id` and name the supervisor in a new
     column, `actor_event.related_actor` (additive, ADR-0012). The rows sit on the supervisee because every later
     question is about them ("does this writer's note need a co-sign, and whose clears it?");
   - `require-cosign` / `lift-cosign` sit on the person;
   - `grant-assigner` / `withdraw-assigner` sit on the holder of the "assigns supervision" right.

   Only a link row carries `related_actor`. A second guarded `DO` block widens `actor_event_op_check` and
   `actor_event_op_shape`, and names any existing row that would break the new shape before it changes the
   constraints. `cairn_actor_append` gains `p_related BYTEA DEFAULT NULL`; its 5-argument form is dropped first, so
   existing calls resolve to the default. For a link op `p_related` must be an enrolled `human` actor other than the
   row's own actor; every other op refuses one. The existing clock-behind refusal applies to all six. `approved_by` is
   the approver's hex key, or `'console'`; `approval` is the approver's token, NULL for the console. Pins:
   `db/tests/004_actors_test.sql`; `restore_actor_registry_supervision.rs`'s `the_supervision_ops_survive_a_restore`,
   `a_tampered_related_actor_is_refused`.

5. **"As of t".** The state of each family (link/unlink for one supervisor, require/lift, grant/withdraw) at time
   *t* is its latest row with `recorded_at <= t`, latest by `(recorded_at, seq)`, as `actor_current` orders.
   - **A part's t** is its `event_log.hlc_wall`, never `t_effective`, which is freely backdatable.
   - **A vouch's t** is its own `hlc_wall`.
   - **The comparison is at the millisecond.** A registry row's `recorded_at` has microseconds; an HLC wall has
     milliseconds. `Supervision::from_rows` floors each row's time to the millisecond, so a row in the same
     millisecond as a part counts as in effect at it.

   Pins: `note/supervision.rs`'s `the_requirement_is_read_as_it_stood_at_t`, `a_link_is_read_as_it_stood_at_t`,
   `a_row_in_the_same_millisecond_as_the_part_is_in_effect`, `a_tie_on_recorded_at_takes_the_later_row`.

6. **A student needs co-signing by role.** Nothing is stored for it, and `require-cosign` / `lift-cosign` on a
   student are refused. A graduate re-enrols as a new actor, so the requirement falls away by itself (design §3.1).

7. **Restore carries the new ops, and the export is a one-way door.** `restore_actor_registry` (db/052) accepts the six
   op words, decodes `related_actor` from hex, inserts it, and refuses a present row that differs only in it.
   `ActorRegistryRow.related_actor` is `#[serde(default, skip_serializing_if = "Option::is_none")]`, so an export
   whose rows carry no `related_actor` has today's shape (`localstate_wire_pins.rs` is unchanged). An export that
   carries a link row does not decode on an older build (`deny_unknown_fields`). One that carries any of the six ops
   decodes there, but the older build's restore door refuses the op word. So a backup taken after the first
   supervision act restores only on a 3c-3a build.

### The doors (db/072)

8. **One approver rule, one address.** `cairn_supervision_approver` is the approver rule's one home. The approver must
   be a vault member whose key is their actor's current key. Anyone but a superuser also needs that key `confirmed` or
   `outside`, as an enroller does (ADR-0085 decision 3). The token must be the approver's own attestation over the
   act's address. The address is `'\x1220' || sha256('cairn-supervision' \n op \n staff(subject) \n
   COALESCE(staff(supervisor), '') \n prior)`, where `staff()` is `cairn_vault_staff_id` and `prior` is the `seq` of
   the family's latest row (0 for none). A token is therefore good for one change of one state. The door's own refusals
   start `cairn_supervision_act: ` (the console door's, `cairn_supervision_console_act: `), name their check, and
   never echo the token; the final `cairn_actor_append` can still refuse in its own words (`cairn_actor_append: this
   machine's clock reads …`). Nothing parses the prefix: the window classifies a refusal by its SQLSTATE. An
   **assigner** is a member whose latest assigner row is a grant. Pins: `supervision_doors.rs::an_unconfirmed_member_cannot_approve`;
   `supervision_doors_approver.rs`'s `a_stale_token_is_refused`,
   `a_token_by_another_key_than_the_approvers_is_refused`, `an_approver_whose_key_is_not_current_is_refused`,
   `a_non_member_cannot_approve`; `db/tests/072_supervision_test.sql` (the address preimage).

9. **The matrix lives in one function, `cairn_supervision_apply`**, which the window's door
   (`cairn_supervision_act`) and the console's (`cairn_supervision_console_act`) both run. It locks the subject's
   member row (`FOR UPDATE`), finds the supervisor's actor, reads the prior, checks the approver (the console approves
   itself, as `'console'`, with no token), applies the refusals below, then calls `cairn_actor_append`.

   | op | approved by | refused when |
   |---|---|---|
   | `link-supervisor` | superuser, assigner | supervisee = supervisor; either party a `superuser` or `admin`; the supervisor a `student`; approver = supervisee; the link already stands |
   | `unlink-supervisor` | superuser, assigner, the supervisor themself | approver = supervisee; no link stands |
   | `require-cosign` | superuser, assigner | a student; a `superuser`/`admin`; approver = the person; already required |
   | `lift-cosign` | superuser, assigner, a current supervisor of the person | a student; approver = the person; not required |
   | `grant-assigner` / `withdraw-assigner` | superuser only | a `superuser` holder; a no-op |

   A holder of the assigner right may link themselves as someone ELSE's supervisor (design §3.1). Who may hold the
   right is site policy: a superuser may grant it to any non-superuser member, a student included (controller
   ruling; principle 9). The console door applies the party and state refusals but none of the approver checks.
   Pins: `supervision_doors.rs`'s `link_supervisor_by_a_superuser_or_an_assigner_with_every_refusal`,
   `unlink_supervisor_by_a_superuser_an_assigner_or_the_supervisor`,
   `require_cosign_by_a_superuser_or_an_assigner_with_every_refusal`,
   `lift_cosign_by_a_superuser_an_assigner_or_a_current_supervisor`, `grant_and_withdraw_assigner_by_a_superuser_only`,
   `a_supervisee_cannot_act_on_their_own_supervision_even_as_assigner`, `a_supervisor_may_step_away_and_lift`,
   `the_console_door_shares_the_matrix_without_an_approver`.

10. **Two readers, and the grants.** `cairn_supervision_record()` returns every row of the six ops, actors as hex,
    `ORDER BY recorded_at, seq`; it is granted to `cairn_window`. `cairn_supervision_staff()` returns every
    non-superuser member as of now: `needs_cosigning` (true for a student), `assigner`, and the staff IDs of the
    supervisors whose link stands (the enroll row's pinned handle when the member row is gone). It is granted to
    `cairn_agent`, with `cairn_supervision_act` and `cairn_supervision_prior`. Those four and the console door are
    `SECURITY DEFINER SET search_path = public, pg_temp`; the helpers are invoker. Only the pure address keeps PUBLIC
    EXECUTE: every other function is revoked from PUBLIC, and the console door and the helpers get no grant. Pins:
    `supervision_doors.rs`'s
    `the_record_lists_every_act_in_order`, `the_staff_list_says_who_supervises_whom_now`;
    `window_role_vault_doors.rs::the_windows_supervision_doors_run_under_its_own_login`;
    `floor_execute_grants.rs::public_cannot_execute_the_supervision_doors`.

11. **The node library and the console verbs** (`cairn_node::supervision_admin`). `act` reads the prior and the address
    from the database, signs the address with `vault_admin::approve`, and runs the window's door. `console_act` runs
    the console door; `staff` reads the panel rows. The CLI verbs are `cairn-node supervision-set --op <word>
    --staff-id <id> [--supervisor <id>]` and `cairn-node supervision-list`. Both refuse a non-local connection
    (`conn_posture::require_local`). `supervision-set` checks `--supervisor` (required for exactly the two link ops)
    before it connects, as a runtime error (controller ruling); clap refuses an unknown op word. Pins:
    `supervision_admin_lib.rs`'s `act_links_and_lifts_through_the_door`, `console_act_needs_no_token`,
    `staff_reads_the_panel_rows`; `cli_vault_refuses_remote.rs`'s `supervision_set_refuses_a_remote_database`,
    `supervision_set_parses_strictly`.

### The read (`note::read`, `note::supervision`)

12. **The note read loads the record once and annotates each part.** `list_patient_notes` reads
    `cairn_supervision_record()` once, on the caller's client. It shares the notes' snapshot only when the caller
    gives one: the window's read runs in `in_snapshot` (REPEATABLE READ), while the re-read inside
    `vouch::vouch_as_shown` runs under READ COMMITTED, so there the record and the notes are two statements. `NOTES_SQL` gains `e.hlc_wall`
    as its last column (`NoteRow.recorded_at`), and `parse_vouches` keeps each vouch's `hlc_wall` as `PartVouch.at`.
    `supervision::annotate` then stamps four plain facts for the part's writer:
    - `NoteRow.needs_cosigning`: the record required the writer to be co-signed at the part's time;
    - `NoteRow.writer_supervisors`: the writer's supervisors NOW;
    - `PartVouch.by_supervisor`: the vouch's signer supervised the writer at the vouch's time;
    - `PartVouch.writer_supervised`: the writer had any supervisor at the vouch's time.

    The writer is `supervision::writer_actor`: `signed_by`'s actor, else `drafted_actor`. It is `None` for a signer
    this node cannot place (an empty actor), and a part with no writer keeps every default. With every fact at its
    default, the rules below are the pre-slice rule with two differences: a part its student writer signed only by
    a later vouch now carries the mark (decision 15), and a non-student's attested sign-off on a part whose envelope
    key is not a human's clears it, where the old rule needed `co-signed`, which the floor refuses there (decision
    13). An op word the build does not know fails the read rather
    than being ignored. Actors are compared through `vouch::same_actor`, so an empty actor is nobody's supervisor.
    Pins: `note/supervision.rs`'s `annotate_judges_the_requirement_at_the_parts_time`,
    `annotate_judges_each_vouchs_supervisor_at_its_own_time`, `an_unplaceable_signer_is_no_writer`,
    `annotate_leaves_a_part_with_no_writer_alone`, `an_unknown_op_word_is_an_error`,
    `an_empty_actor_is_nobodys_supervisor`.

### The co-sign rules (`note::cosign`, `note::vouch`)

13. **When a co-sign is owed, and whose vouch clears it.**
    - `writer_signature(p)` is the signature a co-sign countersigns: `signed_by`, else the first vouch by the part's
      `drafted_actor` under a key that is not repudiated (a part that went in unsigned, signed later by its writer).
      It is `None` for a `signed_by` with an empty actor (controller ruling: an unplaceable signer judges nothing).
    - `cosign_required(p)`: the part `part_is_signed`, does not `counts_as_unsigned` (ADR-0093), and its writer's
      signature is a student's or `p.needs_cosigning` holds. A part that went in unsigned is judged at its own
      `hlc_wall`, the commit, not at the writer's later signature.
    - `clears_cosign(p, v)`: the vouch is by a key that is not repudiated, of a known actor other than the writer's,
      AND either its signer supervised the writer at the vouch's time (any role, attested or co-signed: design §3.2
      Q4), or, **the fallback**, the writer had no supervisor at the vouch's time and a non-student's vouch is the
      strongest countersignature the floor allows ON THAT PART: a `co-signed` one, or, only where the part's envelope
      key is not a human's (`signer_human_actor` is `None`, where db/071 refuses `co-signed`), an attested
      `sign-off` (controller ruling). The floor also admits an attested sign-off on a human-signed part; it does not
      clear the mark (not guaranteed (b)).
    - A vouch made BEFORE the writer's own later signature still counts (controller ruling): it named the same bytes.
    - `awaits_cosign(p)`: required, and no vouch clears it. Supervisors share the load: whichever co-signs first
      clears it for all of them.

    **This supersedes ADR-0092 decision 12's clearing rule** ("clears only for a `CoSigned` vouch by a non-student of
    a different actor") **and its limit to a student's part.** Pins: `cosign_tests.rs`'s
    `a_probationers_part_reads_not_co_signed`, `a_non_supervisors_co_sign_does_not_clear_it`,
    `a_supervisors_attested_sign_off_clears_it`, `with_no_supervisor_any_clinicians_co_sign_clears_it`,
    `a_part_signed_later_by_its_writer_still_needs_its_co_sign`,
    `a_sign_off_before_the_writers_own_signature_still_counts`, `an_unplaceable_signer_owes_no_co_sign`;
    `note_supervision.rs`'s `a_lifted_requirement_leaves_the_earlier_part_waiting`,
    `only_a_supervisor_clears_a_mandatory_co_sign`, `a_pre_slice_students_part_is_cleared_by_any_clinician`,
    `a_part_gone_in_unsigned_is_cleared_by_a_clinicians_sign_off`.

14. **"Sign off this note" stays offered while a part awaits its co-sign.** `needs_sign_off(parts)` is
    `signed_off_by(parts).is_none() || parts.iter().any(awaits_cosign)` (design §3.2 Q2). A probationer who signed
    every part is named by `signed_off_by`, yet the note still waits for a supervisor (or, where she had none at
    the vouch's time, the fallback's countersignature). The offer still never depends
    on the presser. While the writer has a supervisor, a non-supervisor's sign-off is recorded and read, but the part keeps its
    mark and the offer stays. **This supersedes ADR-0092 decision 1's offer rule and decision 11's `needs_sign_off`.** Pins:
    `cosign_tests.rs::the_offer_stays_while_a_mandatory_co_sign_is_missing`;
    `note_supervision.rs::only_a_supervisor_clears_a_mandatory_co_sign`.

15. **The words** (`view::part_byline`, still the one producer of part bylines):
    - the mark is " — not co-signed" while the part `awaits_cosign`, and " — not co-signed (no supervisor recorded)"
      when the writer has no supervisor NOW (`writer_supervisors` is empty), so nobody in particular is waited for;
    - it sits straight after `signed_by`'s words or, for a part with no `signed_by`, straight after the words of the
      vouch `writer_signature` picked;
    - a repudiated, not re-signed signature still reads " — counts as unsigned" in its place, never both
      (`cosign_required` is false for such a part).

    E.g. "signed by Dr Bea — not co-signed · co-signed by Dr Zed" (Zed is not her supervisor); then "signed by Dr Bea
    · co-signed by Dr Zed · co-signed by Dr Sue" once her supervisor co-signs. A student with no supervisor recorded:
    "signed by Sam (student) — not co-signed (no supervisor recorded)". Every pre-slice expectation of " — not
    co-signed" changed only by that suffix. Pins: `cosign_tests.rs::a_repudiated_part_reads_counts_as_unsigned_not_both`;
    the byline assertions of decision 13's `note_supervision.rs` tests; `view_tests.rs`.

### "Co-sign as reviewed" (`note::review`, `notes_review.rs`, `notes_compound.js`)

16. **The node gesture vouches only the presser's supervisees' parts.** `review_vouches(parts, presser)` plans each
    part that:
    - `part_is_signed`, does not `counts_as_unsigned`, and is not `cosign_required` (clearing an owed co-sign is
      Sign off's job);
    - has a writer (`writer_actor`) other than the presser, with the presser among the writer's supervisors NOW;
    - has no vouch by a supervisor of the writer (at that vouch's time) under a key that is not repudiated, and has
      not been signed by the presser.

    Once any supervisor has reviewed a part, nobody is offered it. The role is `vouch::planned_role`, the arm Sign
    off's planner already used, extracted unchanged: `co-signed` exactly where db/071 admits it, `attested`
    otherwise. `review_note` runs `vouch::vouch_as_shown`, the transaction `sign_off_note` now also runs: the presser
    is the signing KEY's actor, never the who's-here claim (ADR-0092 decision 16); the note is re-read; `Changed`
    unless it holds exactly the parts shown; `NothingToSign` for an empty plan. The vouch IS a `sign-off` vouch, so,
    where `planned_role` picks `co-signed`, it reads " · co-signed by …" (the attested case is (l)). Pins: `review.rs`'s `review_vouches_only_the_pressers_supervisees_parts`,
    `review_skips_a_part_that_needs_a_co_sign`, `once_any_supervisor_reviewed_it_no_one_is_offered_it`;
    `note_review_cosign.rs`'s `a_supervisor_co_signs_a_reviewed_note`, `review_never_vouches_another_writers_part`,
    `the_key_not_the_claim_decides`, `a_part_added_after_it_was_shown_refuses_changed`.

17. **The window command `note_review_cosign` keeps `note_sign_off_impl`'s order** (ADR-0092 decision 14): the
    chart on screen, fixture mode, the ids, the key, one node call, an unknown outcome (#814), then a timing sample
    under `note-sign-off` for a success only (a refusal or a failure returns first, and an unknown outcome is not
    timed). `CompoundView.can_review` is `can_review` for the
    who's-here claim's actor, false with no claim; it only decides the offer. A review settles with its OWN words,
    not Sign off's (controller ruling: Sign
    off's "every part of the note now carries your signature" is false of a review). They are "Co-signed as reviewed
    — the note now carries your co-signature.", "This note changed since it was shown — read the chart again, then
    co-sign it. Nothing was signed." and "There is nothing here for you to co-sign as reviewed. Nothing was
    signed.". Pins: `notes_review_tests.rs`'s `can_review_follows_the_claims_supervision`, `review_words_are_exact`,
    `review_follows_sign_offs_order`; `notes_review_db_tests.rs`'s `a_supervisor_reviews_from_the_window`,
    `the_key_not_the_claim_decides_in_the_window`.

18. **The webview's "Co-sign as reviewed"** is drawn when `compound.can_review && noteDraft === null`, beside "Sign
    off this note" when both apply. Its accessible name is "Co-sign as reviewed: " + the note's time. Its press is
    `pressSignOff`'s: the who's-here gate (`claimAsked()`, else `askWhosHere`), nothing while the editor holds a draft
    or `signInFlight()`, the button down and the pane's in-flight flags held until it settles. It settles with
    "reviewed", "not-reviewed" or "review-unknown", which the who's-here picker words in its own lines: "Your review
    co-signature went through — open the chart to see which parts it covers.", "Your co-signature signed nothing —
    open the chart to see where the note stands." and "Whether your co-signature went through is not known — check
    the chart." The first names no parts (controller ruling): a review can skip some of the presser's supervisees'
    parts. No dialog. Pins: `notes_sign_off_webview_tests.rs`'s `review_is_offered_only_when_the_view_says_so`,
    `review_sits_behind_the_whos_here_gate_and_refuses_in_flight`,
    `its_accessible_name_is_co_sign_as_reviewed_plus_the_time`, `a_switch_during_a_review_says_what_the_review_did`,
    `no_dialog`.

### The panels (`supervision_panel.rs`, `supervision.js`)

19. **Two panels over the same doors.** The **assigner's panel** (`#supervision-panel`) lists every non-superuser
    member and draws buttons from each row's state: require or lift co-signing (not for a student), grant or
    withdraw the assigner right, unlink each supervisor, and link a supervisor by staff ID. **"My supervisees"**
    (`#my-supervisees`) lists the members whose supervisors include the who's-here claim's staff ID. It offers only
    "Lift co-signing" (when required, not for a student) and "Step away". With nobody named, it says so before the
    database is asked. A member who needs co-signing with no supervisor reads "needs co-signing · no supervisor
    recorded" (a student: "needs co-signing (student) · no supervisor recorded"), so the gap has an owner (design
    §3.4). The lists are read with no credential. Pins: `supervision_panel_tests.rs`'s
    `the_gap_row_reads_no_supervisor_recorded`, `my_supervisees_needs_a_named_claim`,
    `an_assigner_links_and_a_supervisor_lifts_from_the_window`.

20. **The panel's key is released for one act and never held.** `supervision_key` keeps `superuser_key`'s order:
    refuse a blank passphrase, `forget_held_key`, lock, then `release_for_act` with `Purpose::Clinical` and, on
    `Release::RefusedPurpose`, once more with `Purpose::Enrol`. A superuser's or an admin's key serves only `enrol`,
    and the vault refuses the purpose before it counts a try, so the retry costs nothing. Every other answer goes
    through `from_release_noting`; it never calls `signer_for_act`. The door decides who may act. The window says a
    deliberate refusal in one sentence of its own and never shows the door's words. Every act clears the strip and
    re-reads the pressing panel. A failed re-read never says "nothing was recorded". A later good read clears an
    earlier failure's sentence, in both panels and in the superuser panel, while an act's own outcome stays. The
    lift's sentence is "{id} no longer needs notes co-signed. Notes signed while it was required still need a
    co-signature if they have none." (controller ruling: true when no notes wait). Pins:
    `supervision_panel_tests.rs`'s `a_supervision_act_wipes_then_releases_and_holds_nothing`,
    `a_superuser_key_is_released_for_enrol_after_a_refused_clinical`,
    `a_failed_supervision_read_never_says_nothing_was_recorded`, `supervision_act_clears_the_strip_both_ways`,
    `a_good_read_clears_a_stale_status`, `the_lift_sentence_claims_no_waiting_notes`, `no_dialog`;
    `superuser_panel_tests.rs::a_good_read_clears_a_stale_status`.

## What is not guaranteed

- **(a) The record is node-local, so the same note may read differently on another node until slice 5** (design §3.1
  Q6). The local record applies to every note this node reads, its own and its peers'. `actor_event` rows replicate
  nowhere and travel only in this node's local-state export (ADR-0093 (i)). A peer whose record differs may read a
  part "— not co-signed" that this node reads cleared, or the reverse.
- **(b) #1010's residual, under the fallback.** A non-supervisor's ATTESTED sign-off of a part whose envelope key is
  a human's does not clear the mark (decision 13). This is ADR-0093 (d)'s case: a sign-off before a student's
  re-sign was planned attested. The part now `awaits_cosign`, so, unlike ADR-0093 (d)'s "the button is gone for
  everyone", "Sign off this note" stays offered. The earlier signer's press answers `NothingToSign`, because they
  have signed the part. A press by any other non-student co-signs it and clears the mark
  ([#1010](https://github.com/cairn-ehr/cairn-ehr/issues/1010)).
- **(c) A raw client can still file a `co-signed` vouch by a non-supervisor.** db/071 checks only that the envelope
  key is another human's; no floor rule reads the record. While the writer had a supervisor at that vouch's time, it
  is read (" · co-signed by Dr Zed") and clears nothing. Where the writer had none, the fallback counts it, as before
  this slice.
- **(d) The clocks are not one clock.** A registry row's `recorded_at` is this database's arrival clock
  (`clock_timestamp()`); a part's and a vouch's `hlc_wall` is the signer's HLC, a peer's included. They are compared
  at millisecond granularity, flooring the registry time. A row in the same millisecond as a signing counts as in
  effect, and a peer's HLC running ahead of this database's clock shifts which side of a row its parts fall. A
  restore keeps the original `recorded_at`.
- **(e) A part that went in unsigned is judged at its commit**, its own `hlc_wall`, not at the time its writer later
  signed it. A requirement recorded between the commit and that signature does not apply to the part.
- **(f) Review timings share the `note-sign-off` timing kind**, so the timing aggregates (RUNBOOK §5) cannot tell a
  review from a sign-off (no db/044 widening).
- **(g) The panels do not refresh each other**, and "My supervisees" is not re-read on a who's-here change or an
  idle re-lock. A list drawn for one person can stay on screen for the next. The key still decides every act.
- **(h) The panels draw buttons the door will refuse**, such as "Grant assign-supervision right" for a presser who
  is not a superuser, or "Link a supervisor" on an admin's row. The refusal is said plainly and the list is re-read.
  The link field is not in a form, so Enter does nothing, and a press with a blank supervisor clears the strip.
- **(i) A probationer sees "Sign off this note" on a note they wrote alone**, and pressing it answers
  `NothingToSign`, as a student does (ADR-0092 (e)). The offer never depends on the presser.
- **(j) A note of two or more parts can read "signed off by Dr Zed" while a part still reads "— not co-signed".**
  `compound_state` does not ask `awaits_cosign`, as design §3.2 allows: a non-supervisor's sign-off is recorded and
  read.
- **(k) A supervisor who re-enrolled as a new actor cannot step away** from a link made to their old actor: the door
  compares actors. A subject with no member row cannot be acted on, by the window or the console (as db/063's
  `cairn_vault_set_enroller`).
- **(l) An attested review reads " · signed off by", not " · co-signed by".** On a supervisee's part whose envelope
  key is not a human's, `planned_role` plans `attested`. No test pins those words.
- **(m) No test isolates a re-sign under an unconfirmed key** (decision 3): it holds because `re_signs` never reads
  `unconfirmed`.
- **(n) Finding what to co-sign is chart by chart** until 3c-3c's person menu (design §3.4).
- **(o) The window's commands against a database are pinned only by DB-gated tests** (`notes_review_db_tests.rs`;
  `supervision_panel_tests.rs`'s `an_assigner_links_and_a_supervisor_lifts_from_the_window` and
  `a_superuser_key_is_released_for_enrol_after_a_refused_clinical`), which CI does not run
  ([#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)).

## Consequences

- **Easier.** A probationary doctor's part reads "— not co-signed" until one of their supervisors co-signs it, and
  lifting the probation leaves the earlier parts waiting, as on paper. A supervisor can initial a supervisee's note as
  reviewed. A student's part signed only by their own later vouch now carries the mark too, which ADR-0092's
  Consequences left open ([#1003](https://github.com/cairn-ehr/cairn-ehr/issues/1003)). Steps (paper *N* → built *M* → target *K*):
  - record "B supervises C": 1 → 1 (one act, the strip's passphrase counted with it as slice 1 counts it) → 1;
  - set or lift `needs_cosigning`: 1 → 1 → 1;
  - co-sign a reviewed note from the opened chart: 2 (open the folder, sign) → 2 ("Co-sign as reviewed", the
    passphrase) → 2;
  - see that a part still needs its supervisor: 1 (look for the countersignature) → 0 (the byline) → 0.

  No `M > N`. Time budgets, owed by a human in RUNBOOK §25: a supervision act ≤ 3 s; co-sign as reviewed ≤ 3 s
  (Sign's, RUNBOOK §16/§23). No decision is forced: the byline names what is pending, and "(no supervisor
  recorded)" names the gap.
- **Harder.** Every reader that asks "does this part still wait for a co-sign?" must go through
  `cosign::awaits_cosign` over an annotated `NoteRow`, never `signed_by.student`. Every judgement of the record must be
  "as of" the part's or the vouch's own `hlc_wall`, never now; among the note rules only the "(no supervisor
  recorded)" suffix and the review offer read the record as it stands now. A seventh supervision op must be added to
  db/004's two CHECKs, `cairn_actor_append`, db/052's
  op list, db/072, `supervision::family` (which fails the read on an unknown word), `SupervisionOp` and the CLI's
  word list at once.
- **The bet.** That a byline naming what is pending, and the gap row in the assigner's panel, are enough for
  supervisors to find and clear their supervisees' parts, chart by chart, until 3c-3c's menu. We would know it is
  wrong if a probationer's parts stay "— not co-signed" for weeks, or if "(no supervisor recorded)" rows stay in
  the panel with nobody acting on them.
- **Pinned by** `db/tests/{004_actors,072_supervision}_test.sql`;
  `crates/cairn-node/src/note/{supervision,cosign_tests,review,view_tests}.rs`'s tests;
  `crates/cairn-node/tests/{supervision_doors,supervision_doors_approver,supervision_admin_lib,restore_actor_registry_supervision,note_supervision,note_review_cosign,window_role_vault_doors,floor_execute_grants,cli_vault_refuses_remote}.rs`;
  `cairn-gui/cairn-gui-tauri/src/{notes_review,notes_review_db,supervision_panel,notes_sign_off_webview}_tests.rs`.
  RUNBOOK §25 carries the human measurements.
