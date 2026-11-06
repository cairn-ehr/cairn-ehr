# ADR-0092 — The vouch and the sign-off

- **Status:** Accepted (builds [ADR-0091](0091-the-compound-note-and-the-addendum.md) decision 3, the vouch shape
  decided 2026-11-05, and ends its under-claim (d); builds the slice 3 design's §3.4 vouching later, sign-off and
  students, and §3.7's grades in words; supersedes no ADR decision)
- **Date:** 2026-11-06
- **Spec version at acceptance:** 0.94
- **Issues:** draft PR [#1000](https://github.com/cairn-ehr/cairn-ehr/pull/1000) (GUI sign-in slice 3c-2). Design:
  `docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md` (§3.4, §3.7, §4.1, §4.3);
  plan: `docs/superpowers/plans/2026-11-06-sign-in-slice-3c-2-vouch-and-sign-off.md`. Filed by the build:
  [#1001](https://github.com/cairn-ehr/cairn-ehr/issues/1001) (a forked draft keeps a covers line that may be false) ·
  [#1002](https://github.com/cairn-ehr/cairn-ehr/issues/1002) (the medication pane reads the strip while a note act
  is in flight) · [#1003](https://github.com/cairn-ehr/cairn-ehr/issues/1003) (the reviews' minor residuals);
  extended [#995](https://github.com/cairn-ehr/cairn-ehr/issues/995) (a sealed vouch admitted without its key, then
  healed). Left to 3c-3: [#918](https://github.com/cairn-ehr/cairn-ehr/issues/918)'s display half.
- **Builds on:** [ADR-0091](0091-the-compound-note-and-the-addendum.md) (the compound note, the addendum, decision 3)
  · [ADR-0089](0089-the-quarantine-commit-and-the-asserted-grade.md) (a part that went in unsigned) ·
  [ADR-0088](0088-whos-here-and-a-drafts-parts.md) (who's here is a claim; parts) ·
  [ADR-0087](0087-the-first-note-stream-and-its-draft.md) (the note, R8) ·
  [ADR-0085](0085-in-person-enrolment-and-re-key.md) (*unconfirmed*; `rotate-key` keeps the actor) ·
  [ADR-0083](0083-the-human-key-vault.md) (the vault, the strip) ·
  [ADR-0053](0053-per-write-human-authorship.md) / [ADR-0052](0052-born-sealed-clinical-bodies.md) (human
  authorship, born-sealed bodies) · [ADR-0007](0007-authorship-and-accountability.md) (signature proves origin,
  attestation confers responsibility) · [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (logic below the UI)

## Context

On paper, a supervisor reads a student's entry and countersigns beneath it. A nurse who left a line unsigned comes
back, writes another line and signs, and the one signature covers both. A consultant countersigns an entry that a
registrar and a nurse wrote. Nothing on the page is edited: the second signature is new ink below the old.

Until this slice Cairn could not say any of this. A part that went in unsigned (ADR-0089) stayed unsigned for ever,
and signing an addendum vouched for the addendum only (ADR-0091 (d)). A student's signature read like anyone else's.
This slice, **3c-2**, adds the **vouch**: a separate, human-signed event that says "I stand behind that part". Two
gestures make one. **Signing an addendum** also vouches for the signer's own earlier unsigned parts of that note,
and the strip lists them before the press. **"Sign off this note"** vouches for every part of a compound the presser
has not signed. A `student` role is offered at enrolment, a student's signed part reads "— not co-signed" until a
supervisor co-signs it, and a signer whose key is not yet confirmed (or was approved by a key since repudiated)
reads "(signer unconfirmed)".

## Decision

### The maintainer's ruling (2026-11-06)

1. **"Sign off this note" is offered unless one non-student clinician has signed or vouched for every part.** So a
   note signed by its author alone has no button. A student's note, a note with an unsigned part, and a note two
   clinicians each signed all show it, and a later addendum by someone else brings it back. The rule is
   `vouch::needs_sign_off` = `signed_off_by(parts).is_none()` (decision 11); the window offers the button exactly
   when it holds (`CompoundView.can_sign_off`, decision 17). Pins: `vouch.rs`'s
   `a_note_signed_wholly_by_its_author_needs_no_sign_off`, `a_students_note_needs_sign_off_until_co_signed`,
   `two_clinicians_parts_need_sign_off`, `a_later_addendum_by_another_brings_the_offer_back`;
   `notes_sign_off_tests.rs::can_sign_off_follows_needs_sign_off`.

### The wire (`cairn_event::note_attestation`, `db/071`)

2. **A vouch is one `clinical.note-attestation.asserted` event per vouched part**, profile
   `clinical.note-attestation/1` (ADR-0091 decision 3, built):
   - **payload** exactly `{"target": <the part's event id>, "target_address": <lower hex of the part's
     event_log.content_address, multihash prefix included>, "act": "addendum" | "sign-off"}`;
   - **contributors** exactly `[{"actor_id": K, "role": "attested" | "co-signed", "responsibility": {"held_by":
     K}}]`, where `K` is the human's key id and the envelope's `signer_key_id`. The human signs AND attests;
   - **the token** is `sign_attestation(event_address(signed_bytes), K, role, human_sk)`, submitted with the
     attester's verifying key and the DEK through `submit_event($1, $2, $3, $4)`;
   - **sealed**, through `medication::sealed_submit::prepare_sealed_as`, so the safety rung, seal-then-sign and the
     unwrap-key check stay one path;
   - **`patient_id`** is the TARGET part's chart, never the chart on screen; **`t_effective`** is absent, as the
     medication attestation's; one `next_hlc` per vouch; no attachments;
   - **the twin** is fixed copy: "Vouches for note part {target}: signed later by its writer (attested)." for an
     addendum, "Vouches for note part {target}: the note signed off ({role})." for a sign-off.

   An `addendum` act is the writer's own first signature of a part, so it always carries `attested`: the payload
   builder asserts it in debug builds and the floor refuses it at both doors (decision 4). Pins:
   `note_attestation.rs`'s `the_payload_is_exactly_three_keys`, `the_contributor_bears_responsibility_held_by_itself`,
   `the_twin_words_are_exact`, `an_addendum_act_is_never_co_signed`;
   `note_attestation_floor.rs::a_sign_off_vouch_is_admitted_and_projected`,
   `a_vouch_without_a_token_is_refused` (db/005's gate: "requires attestation (no token presented)").

### The floor (`cairn_check_note_attestation`, db/071)

3. **Every message starts `note attestation: `**, and each rule raises its own, so the mirror can tell which fired.
   The check is registered in `cairn_event_twin_check` with "note attestation requires a non-empty authored twin
   (§3.13/§3.19)". The type is `additive`, targets no other author, and stays off db/048's no-thread list, as notes
   do.

4. **Both doors** (on a `/1` body): a payload; schema `clinical.note-attestation/1`; **exactly one** contributor with
   `? 'responsibility'`, whose role is `attested` or `co-signed` and whose `actor_id` is `signer_key_id`; `target` a
   uuid string; `target_address` non-empty, even-length hex (either case, compared as decoded bytes); `act`
   `addendum` or `sign-off`; an `addendum` never `co-signed`. **Ruling R8**, as db/064 and db/068: at the remote door
   a body that is not `/1` returns unchecked (it is carried, never projected). Pins: db/tests/071.

5. **The strict door only** (`cairn.remote_apply` not `on`; db/020 raises it before running a floor, and
   `submit_event` pins it off, #602):
   - the contributors array has **exactly one** entry;
   - the target is an `event_log` row of type `clinical.note.asserted` ("… is not a note held on this node"), whose
     `patient_id` is the vouch's own (a vouch is filed under its part's chart, the #689 failure otherwise), and whose
     `content_address` equals `decode(target_address, 'hex')` (a vouch names the exact version it vouches for);
   - an **`addendum`** needs a `drafted` contributor on the target whose key shares an actor with the signer's;
   - a **`co-signed`** needs a target whose `signer_key_id` is a human's (`cairn_note_key_is_human`) and does NOT
     share an actor with the signer's: a co-signature is a second signature, by someone else. An unsigned part is
     attested, never co-signed.

   "Shares an actor" is `cairn_note_keys_share_actor(a, b)`: both keys carry an `enroll` or `rotate-key` row of one
   `actor_id`, so a re-keyed member is still the same person (ADR-0085). It is owner-only, revoked from PUBLIC, and
   called only from inside the floor. It is not the window's text door `cairn_note_key_actor`, which answers NULL
   for a key held by several actors and so could not refuse a co-sign on it. Pins:
   `note_attestation_floor.rs::an_addendum_vouch_by_the_parts_own_writer_is_admitted`,
   `an_addendum_vouch_for_someone_elses_part_is_refused`, `a_co_sign_of_ones_own_signature_is_refused`,
   `a_co_sign_of_a_part_no_human_signed_is_refused`, `a_target_on_another_chart_is_refused`,
   `a_stale_target_address_is_refused`, `a_target_that_is_not_a_note_is_refused`,
   `a_re_keyed_writer_vouches_for_a_part_drafted_under_the_old_key`.

6. **The remote door checks structure only** (ADR-0091 decision 3: offline-first). A vouch may arrive before its note,
   and where a note lives is node-local knowledge, so judging it there would fork the event set. It also tolerates
   extra contributory entries beside the one bearing contributor (ADR-0051: refusing a verifiable peer event for an
   addition this node does not need would break set-union losslessness); the read looks only at the bearing one. A
   vouch whose note is not held is admitted without error, so the puller pens nothing, and is projected. The read
   counts it only once its note is projected here with the very bytes it names (decision 9). Pin:
   `note_attestation_floor.rs::the_remote_door_admits_a_vouch_whose_note_is_not_held`.

### The projection and the doors (`db/071`; `db/037`, `db/044` in place)

7. **The projection `clinical_note_attestation`** holds one row per vouch: `event_id` (PK), `patient_id`, `target`,
   `target_address`, `act`, `role`, `signer_key_id`, `t_effective`, `hlc_wall`, `hlc_counter`, `origin`,
   `content_address`, indexed on `target` and `content_address`, readable by `cairn_agent`. Its applier
   `clinical_note_attestation_apply` reads the CLEAR payload (NULL: no custody, nothing to project), projects `/1`
   only, inserts or does nothing, and is registered `heal_safe`, `run_order` 20.

   **The applier re-imposes the floor's closed vocabularies**, because the heal path skips the floor: a sealed vouch
   admitted remotely without its DEK passes db/020 step 7 on structure alone, and is projected when a key lands. It
   SKIPS (never raises, and there is no table CHECK, either of which would fail the heal) a body whose bearing role is
   missing or not `attested`/`co-signed`, whose act is not `addendum`/`sign-off`, whose addendum carries
   `co-signed`, whose target is not a uuid, or whose `target_address` is not even-length hex. So the table holds only
   what the read's `VouchRole`/`VouchAct` can name. A shred deletes its rows by `content_address` (db/037, in place,
   beside `clinical_note_discard`). Pins: db/tests/071 (rows 0011–0016, through the real dispatcher);
   `note_attestation_floor.rs::a_shred_scrubs_the_projection`; the registry counts (`twin_registry.rs` and
   db/tests/034 28 → 29; `projection_registry.rs` 32 → 33 and db/tests/039 34 → 35).

8. **Three reader doors**, each `STABLE SECURITY DEFINER SET search_path = public, pg_temp`, revoked from PUBLIC and
   granted to `cairn_window` (the window reads neither `actor_event`'s actor and pinned columns nor `vault_member`,
   db/062); each answers NULL for what it does not know:
   - `cairn_note_key_actor(key)`: the actor whose `enroll`/`rotate-key` row carries the key, lower hex; NULL for an
     unknown key and for a key registered to several actors;
   - `cairn_note_key_role(key)`: that actor's first `enroll` row's `pinned ->> 'role'`. Pinned determinants are
     immutable per actor, so a student who re-enrols as a doctor is a NEW actor and the parts they signed as a
     student keep reading "(student)";
   - `cairn_note_staff_actor(staff_id)`: the actor of the vault member a who's-here claim names, the staff id
     normalised by db/061's `cairn_vault_staff_id`.

   Reused as they are: `cairn_note_vouched_attester(event)` (db/065) and `cairn_key_confirmation(key)` (db/004).
   db/044's `gesture_kind` CHECK (both lists, in place) gains `'note-sign-off'`. `SCHEMA_GENERATION` is **71**. Pins:
   `note_attestation_floor.rs::the_doors_answer_actor_role_and_staff_actor`;
   `floor_execute_grants.rs::public_cannot_execute_the_vouch_doors`;
   `window_role_grants_notes.rs::the_windows_vouch_and_its_doors_run_under_its_own_login`.

### The read (`cairn_node::note::read`, `note::vouch_read`)

9. **A vouch counts only when its token is verified AND it names the part's own bytes.** The vouch column reads a
   `clinical_note_attestation` row for part `n` only where `a.target = n.event_id`, `a.target_address =
   e.content_address` (the part's `event_log` row), and `cairn_note_vouched_attester(a.event_id) = a.signer_key_id`
   (db/020 may store a peer's token unverified; such a vouch vouches for nothing). It is one column of the note
   statement itself, so the vouches come from the same snapshot as their notes. The words are parsed strictly: an act
   or role word this build does not know is an error, never a guess. Vouches are ordered by `(hlc_wall,
   hlc_counter, event_id)`. Pins: `note_sign_off.rs::a_forged_vouch_is_not_read` (a vouch for stale bytes and one
   with an unvouched token are projected but not read; an honest remote vouch is, the positive control);
   `vouch_read.rs`'s `only_a_verified_vouch_for_the_parts_own_bytes_is_read`, `vouches_are_read_in_hlc_order`,
   `a_word_this_build_does_not_know_is_an_error`.

10. **Who signed a part is read from the key the `Signed` grade authenticated.** `NoteRow` gains `signed_by:
    Option<Signer>` (`Some` exactly when the grade is `Signed`), `drafted_actor` (the actor of the part's `drafted`
    key) and `vouches`. The grade's key is `COALESCE(cairn_note_vouched_attester(e.event_id), e.signer_key_id)`: the
    verified attester when there is one, else the signer. The grade's name is now read for that same key, so
    `signed_by` and the grade name one person. A `Signer` is `{actor, name, student, unconfirmed}`:
    - `actor` is the key's actor, EMPTY when the node cannot place the key; an empty actor matches no one,
      itself included (`vouch::same_actor`), so an unknown key is never taken for the presser, a writer, or a
      signer-off;
    - `student` is the pinned role `student`;
    - `unconfirmed` is `cairn_key_confirmation` = `unconfirmed` or `doubtful`. A `repudiated` key is NOT
      marked (#918's display half is 3c-3's), nor an `outside` one, to which the confirmation mechanism never
      applied.

    Pin: `vouch_read.rs::a_key_the_node_does_not_know_matches_no_actor_and_is_named_so`.

### The pure rules (`cairn_node::note::vouch`)

11. **People are compared by ACTOR, never by key.** For a part `p`:
    - `signed_for(p)` is `p.signed_by` then each vouch's `by`;
    - `part_is_signed(p)` is `signed_by.is_some() || !vouches.is_empty()`;
    - `signed_off_by(parts)` is the first non-student, in the ROOT's `signed_for` order, who is in `signed_for` of
      every part;
    - `needs_sign_off` is its absence;
    - `sign_off_vouches(parts, presser)` skips a part the presser has signed, plans `CoSigned` for a part with
      `signed_by` whose actor is known, and `Attested` otherwise;
    - `addendum_covers(parts, actor)` is every part with `!part_is_signed` whose `drafted_actor` is that actor.

    **A part signed by a key this node cannot place is planned `Attested`, not `CoSigned`** (controller ruling): the
    floor co-signs only a part whose signer it knows as a human's, so a co-sign would be refused and take the whole
    sign-off with it. An attested sign-off on a signed part is admitted (what is not guaranteed, (g)). Pins: the
    `vouch.rs` tests of decision 1, `sign_off_co_signs_others_signed_parts_attests_unsigned_ones_and_skips_ones_own`,
    `addendum_covers_only_the_actors_unsigned_parts`, `a_key_this_node_does_not_know_matches_no_one`,
    `a_part_signed_by_a_key_this_node_cannot_place_is_attested_not_co_signed`;
    `note_sign_off.rs::signing_off_a_part_signed_by_an_unknown_key_attests_it`.

### The words (`cairn_node::note::view`, `note::compound`)

12. **A part's byline is `view::part_byline`, the one producer of part bylines.** The window words the root's
    heading, each part in "{n} parts" and each seam with it:
    - **The signer's name** is `name`, then " (student)" for a student, then " (signer unconfirmed)" while their key
      is unconfirmed or doubtful (`signer_words`). This holds for a vouch's signer too.
    - **" — not co-signed"** follows a student signer's name straight away. It clears **only** for a `CoSigned`
      vouch by a non-student of a different actor from the signer's (controller ruling; design §3.4: the
      SUPERVISOR's co-sign). A student's own co-sign, or another student's, does not clear it, though a peer's remote
      door may admit one.
    - **Each vouch** then follows in order: " · signed later by {V}" (an addendum), " · signed off by {V}" (a
      sign-off, attested), " · co-signed by {V}" (a sign-off, co-signed).
    - **The spoken form** keeps the `?` → "unverified " rule (#973).

    E.g. "signed by Sam (student) — not co-signed"; "signed by Sam (student) · co-signed by Dr Zed".

    **The compound state** of a note of two or more parts reads, in this order:
    1. "signed off by {S}", when `signed_off_by` is S AND the compound holds a `sign-off` vouch by S;
    2. "includes an unsigned part", when a part that is not `part_is_signed` grades `WentInUnsigned` or
       `WriterNotKnown`;
    3. "includes a part whose signature this node cannot verify", for such a part that grades `Claimed`;
    4. "every part signed".

    A note one doctor signed throughout, which nobody pressed sign-off on, reads "every part signed". The match over
    grades has no `_` arm. Pins: `view_tests.rs`'s `a_student_part_reads_not_co_signed`, `a_co_signed_student_part`,
    `a_students_own_co_sign_does_not_clear_not_co_signed`, `another_students_co_sign_does_not_clear_not_co_signed`,
    `an_unconfirmed_signer_is_marked`, `a_vouched_unsigned_part_reads_signed_later`; `compound.rs`'s
    `the_state_reads_signed_off_by`, `a_vouched_part_no_longer_makes_the_note_unsigned`,
    `the_state_words_are_exact`.

### "Sign off this note", as shown (`note::vouch::sign_off_note`, `notes_sign_off.rs`)

13. **The node signs off only the parts the window showed.** The window sends the compound's root and its part ids
    (`CompoundView.shown_parts`, in part order). In ONE transaction the node:
    - reads the presser's actor (`cairn_note_key_actor`; NULL is an error, "the signing key … belongs to no enrolled
      person on this node; nothing was signed");
    - re-reads the chart set's notes as the window reads them (`list_patient_notes`: linked charts, the doubted-link
      rule) and groups them (`compounds`);
    - answers **`Changed`** and writes nothing when no compound has that root, or when its parts are not exactly the
      ids sent (the same set, and as many);
    - plans `sign_off_vouches`, answering **`NothingToSign`** for an empty plan;
    - otherwise writes one `sign-off` vouch per planned part, each under that part's own chart, and commits through
      `commit_failure` (#814).

    The check is on part ids: a vouch that landed after the note was shown changes the plan, never the parts that
    get signed. Two clinicians signing off one note at once both stand (each vouch's `next_hlc` row lock serialises
    the writes; neither press is refused), and the state names the first in the root's order. Pins:
    `note_sign_off.rs::signing_off_a_students_note_co_signs_it`, `signing_off_attests_a_part_that_went_in_unsigned`,
    `a_part_added_after_the_note_was_shown_refuses_changed`,
    `a_part_shredded_after_the_note_was_shown_refuses_changed`,
    `signing_off_a_note_one_signed_wholly_is_nothing_to_sign`, `two_sign_offs_at_once_both_stand`,
    `a_linked_charts_part_is_vouched_on_its_own_chart`; `vouch.rs::the_shown_parts_must_be_exactly_the_notes`.

14. **The window's command `note_sign_off` keeps `note_sign_impl`'s order.** The checks run in this order: the chart
    on screen, then fixture mode, then the ids read ("not a note id"), then the key. A passphrase that signs nothing
    is a step paper never has. Its words are exact:
    - success: "Signed off — every part of this note now carries your signature.";
    - `Changed`: "This note changed since it was shown — read the chart again, then sign it off. Nothing was
      signed.";
    - `NothingToSign`: "You have already signed every part of this note. Nothing more was signed.";
    - an unknown outcome: "Whether this note was signed off is NOT known — no ordinary answer came back while signing
      it off (event {id}: {cause}). Once the window can reach the node again, the chart shows whether it was signed
      off.". It never says "the note is there": the note exists either way.

    Both refusals ask for a re-read (`stale`). Only a sign-off that went through is timed, as `gesture_kind =
    'note-sign-off'`; a refusal, a failure or an unknown outcome is no sample. Pins:
    `notes_sign_off_tests.rs::sign_off_refusals_are_worded_exactly`, `sign_off_asks_for_no_key_on_a_malformed_id`;
    `notes_sign_off_db_tests.rs::note_sign_off_impl_signs_off_under_the_window_login`.

### The addendum's vouch (`note::sign`, `note::sign_vouch`)

15. **An addendum's Sign vouches `also_vouches ∩ addendum_covers`, for the signing KEY's actor.**
    `SignNoteInput.also_vouches` names the ids the strip listed. Inside Sign's transaction, after the new parts are
    submitted, the node:
    - reads the signing key's actor (NULL matches no one);
    - re-reads the chart set's notes and takes the compound whose root is the draft HEADER's target (ADR-0091
      decision 7), never one the window names;
    - vouches (`addendum`, `attested`) each listed id that is also covered, in listed order and once each, under its
      part's own chart.

    A new note, an empty list, a header target that roots no compound, or a key this node cannot place vouches
    nothing: an under-claim, never a refusal of a Sign that is otherwise good. A refused vouch rolls the whole Sign
    back. `sign_note` returns `Committed<SignedNote { parts, vouched }>`. **Review Focus 1:** when Nurse B signs while
    who's here still names Nurse A, the strip listed A's parts and B's key covers none of them, so nothing is vouched
    and B's addendum still signs. Pins: `note_addendum_vouch.rs::signing_an_addendum_vouches_the_writers_own_unsigned_part`,
    `a_listed_part_that_is_not_the_signers_is_not_vouched` (key and claim name different people),
    `an_unlisted_own_part_is_not_vouched`, `a_new_note_vouches_nothing`, `the_timer_commit_vouches_nothing`;
    `sign_vouch.rs::only_listed_parts_that_are_covered_are_vouched_in_listed_order_once`.

16. **The strip's "also covers" line is computed for the who's-here CLAIM; the vouch is decided by the KEY.** The pane
    reads the claim's actor (`cairn_note_staff_actor`, a named claim only) in the same snapshot as the notes. Each
    compound gets the line "Your signature also covers your earlier unsigned part{s} of this note: {…}" with its
    ids (`CompoundView.covers_line`, `covers`), shown only when that list is non-empty. It is information, never a
    question. Each covered part is named by its stated time words, or "begun {when its writing began}" when its time
    was not stated (controller ruling: "no stated time · writing began …" repeated per part reads as clutter; the
    part's own byline still carries it). An addendum draft carries the same fields, cloned from its target's compound
    (`DraftView.covers_line`, `covers`), so a reopened draft shows the line and Sign sends its ids. `note_sign`'s
    `also_vouches` is OPTIONAL (controller ruling): a Sign path that sends no list under-claims, never fails. After
    "Signed — the note is on the chart.", the success line says:
    - " Your earlier unsigned part now reads as signed." (one vouched);
    - " Your {n} earlier unsigned parts now read as signed." (several);
    - and, when fewer were vouched than listed, " {k} listed part(s) were not covered: not yours by key, or already
      signed." (controller ruling: the plan's "were not yours by key" is false for a part signed meanwhile).

    Pins: `notes_sign_off_tests.rs::the_covers_line_names_the_claims_own_unsigned_parts`,
    `no_covers_line_without_a_claim_or_without_parts`, `the_sign_success_line_counts_vouched_and_unvouched`.

### The webview (`src-ui/notes_compound.js`, `notes.js`, `index.html`)

17. **"Sign off this note" is offered only while the editor is free, and no act starts while a press is in flight.**
    The button is drawn when `compound.can_sign_off && noteDraft === null`, on a linked chart's note too (each vouch
    files under its part's own chart, and no draft is involved; unlike Add, #993). Its accessible name is "Sign off
    this note: " + the note's time. It sits behind the who's-here gate (`claimAsked()`, else `askWhosHere`). It sends
    `shownParts` and the strip's `signerCredential()`, then clears the strip (its staff ID joins the type-ahead
    list only on success), says Rust's words and re-reads the chart on every path.

    One strip credential serves one act at a time (controller ruling). A press holds its button down, and does
    nothing while the editor holds a draft or another press is in flight. It rides the note pane's `noteSigning` /
    `noteSignInFlight`, so the idle re-lock and a who's-here switch wait for it; it settles as `null`, and the re-read
    chart says whether the note was signed off. A sign-off is the first note act in flight with the editor FREE, so
    `newNote`, `addToNote`, `continueHere` and the editor's Sign and Discard all refuse to start while
    `signInFlight()`. An act in flight used to imply an open editor; without these guards a Sign opened mid-press
    would reuse the passphrase still in the strip, and the two presses would clear each other's flag. Sign sends
    `alsoVouches: held.covers` (`[]` for a new note). Every word is Rust's, except the label "Sign off this note" and
    its accessible-name prefix. Pins: `notes_sign_off_webview_tests.rs`'s
    `sign_off_is_offered_only_when_the_view_says_so`, `sign_off_sends_the_shown_parts_and_the_strips_credential`,
    `sign_off_sits_behind_the_whos_here_gate`, `its_accessible_name_is_sign_off_this_note_plus_the_time`,
    `the_covers_line_is_text_and_sign_sends_its_ids`, `no_dialog`,
    `nothing_opens_the_editor_while_a_press_is_in_flight`, `sign_and_discard_refuse_to_start_while_a_press_is_in_flight`.

18. **`student` is offered in the enrolment AND re-key role lists**, after `nurse`. Re-keying is in the list too
    (controller ruling): a re-key never changes a role, so without the option a student could not be re-keyed at
    this window. No vault change: `cairn_vault_purpose_allowed` already grants `clinical` to every role but
    `superuser` and `admin`. Pins: `notes_sign_off_webview_tests.rs::the_student_role_is_offered`,
    `signer_webview_tests.rs`'s role list; db/tests/071's `a_student_may_release_a_key_for_clinical_signing`.

## What is not guaranteed

- **(a) A vouch on a peer that cannot read its sealed body counts nothing.** The vouch is sealed; a node without
  custody projects nothing (`cairn_clear_payload` is NULL), so there the part reads as it did before the vouch: an
  under-claim, never an over-claim.
- **(b) A sealed vouch admitted remotely without its DEK, then healed, is untested.** Decision 7 says what the
  applier does; the SQL mirror pins the skip on unsealed rows only. Same class as #948's sealed cases
  ([#995](https://github.com/cairn-ehr/cairn-ehr/issues/995), extended).
- **(c) A repudiated signer is not marked.** "(signer unconfirmed)" covers `unconfirmed` and `doubtful` only, so a
  part signed by a since-repudiated key reads plainly signed until 3c-3 builds #918's display half
  ([#918](https://github.com/cairn-ehr/cairn-ehr/issues/918)).
- **(d) No pending-sign-offs menu.** A supervisor finds a note to sign off by opening its chart; the menu is 3c-3's.
- **(e) Whether a student may sign off is site policy, not mechanism** (principle 9;
  [ADR-0028](0028-finalized-closed-contributor-role-enum.md): `co-signed` has a slot because deployments gate on it).
  The button is offered to any claim
  (controller ruling). A student's sign-off vouches read "(student)", and `signed_off_by` skips students, so the note
  never reads "signed off by" a student and keeps offering the button. A student who has signed every part already is
  answered `NothingToSign`.
- **(f) Concurrent addenda can double-vouch a part.** Two addenda by the same writer, signed at once and both listing
  the same unsigned part, file two vouches for it; the byline then reads "signed later by Ada" twice. Harmless: both
  are true.
- **(g) An `attested` sign-off of a part another human signed is admitted.** The floor constrains only `co-signed`
  (controller ruling); a raw client can make such a part read "signed off by V" instead of "co-signed by V", which
  does not clear a student's "— not co-signed". It is still V's real signature, never a forgery. The node plans it
  only for a part whose signer it cannot place (decision 11).
- **(h) A forked draft can keep a stale covers line** ([#1001](https://github.com/cairn-ehr/cairn-ehr/issues/1001)).
  The fork copies `covers`, and the line may be false after the draft was signed elsewhere. Nothing over-claims: the
  node re-reads and covers only what is still the signer's and unsigned, and the success line says how many were not
  covered.
- **(i) The medication pane's acts can read the strip while a note act is in flight**
  ([#1002](https://github.com/cairn-ehr/cairn-ehr/issues/1002); window-wide, older than this slice). Decision 17's
  guard covers the note pane only.
- **(j) Buttons drawn before the editor opens stay up and do nothing** — now "Sign off this note" too
  ([#994](https://github.com/cairn-ehr/cairn-ehr/issues/994)).
- **(k) The window's use of `part_byline` is pinned only by a DB-gated test** (`notes_sign_off_db_tests.rs`, which
  CI does not run, [#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)).
- **(l) Minor residuals of the reviews** are [#1003](https://github.com/cairn-ehr/cairn-ehr/issues/1003).

## Consequences

- **Easier.** A part that went in unsigned can be signed later by its writer, and a supervisor can countersign a
  whole note, both by new events that never touch the part. Steps (paper *N* → built *M* → target *K*):
  countersigning a note 2 (read, sign) → 2 ("Sign off this note", then the strip's passphrase, one act as slice 1
  counts it) → 2; finishing one's own unsigned note 3 → 3 (Add, type, Sign; the earlier part is covered by the same
  press) → 3, which **ends ADR-0091's under-claim (d)**; seeing that a student's part is not yet countersigned 0 (no
  second signature on the page) → 0 → 0 (the byline says "— not co-signed"). No `M > N`. Time budget, owed by a
  human in RUNBOOK §23: sign off ≤ 3 s (Sign's, RUNBOOK §16; the `note-sign-off` timing kind). The "also covers" line
  is read before the press; it forces no decision and never blocks.
- **Harder.** Every reader of a note part's signatures must go through `vouch_read`'s two checks (a verified token,
  the part's own address) and compare people by actor. Every future vouch act or role must be added everywhere
  at once: the wire enums, the floor, the applier's skip, `act_of`/`role_of`, and `vouch_words`'s exhaustive match. Every
  future note act that holds the strip must check `signInFlight()`.
- **The bet.** That a byline reading "— not co-signed", a state line reading "signed off by …", and a button offered
  only while a note still needs one are enough for a supervisor to see what is left to countersign, chart by chart,
  until 3c-3's menu. We would know it is wrong if supervisors miss a student's uncountersigned entry, or press "Sign
  off this note" on notes they meant to leave.
- **Pinned by** `db/tests/071_clinical_note_attestation_test.sql`, `crates/cairn-event/src/note_attestation.rs`'s
  tests, `crates/cairn-node/src/note/{vouch,vouch_read,view_tests,compound,sign_vouch}.rs`'s tests,
  `crates/cairn-node/tests/{note_attestation_floor,note_sign_off,note_addendum_vouch,window_role_grants_notes,floor_execute_grants}.rs`
  and `cairn-gui/cairn-gui-tauri/src/{notes_sign_off,notes_sign_off_db,notes_sign_off_webview}_tests.rs`. RUNBOOK §23
  carries the human measurements.
