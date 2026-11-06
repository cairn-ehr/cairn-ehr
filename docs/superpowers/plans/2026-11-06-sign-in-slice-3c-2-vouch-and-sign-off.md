# GUI sign-in, slice 3c-2 — the vouch, "Sign off this note", students and unconfirmed signers — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician can **vouch later** for a part of a note that went in without their signature, and the
chart says so. Two acts make a vouch. (1) **Signing an addendum** also vouches for the signer's own earlier
unsigned parts of that note, and the signer strip lists those parts before the press. (2) **"Sign off this
note"** vouches for every part of a compound. It co-signs each part someone else signed and attests each
part nobody vouched for. Students enrol with a `student` role. A student's signed part reads **"signed by Sam
(student) — not co-signed"** until someone co-signs it. A part signed by a key no superuser has confirmed reads
**"(signer unconfirmed)"**.

**Architecture:** One new event type, `clinical.note-attestation.asserted` (profile
`clinical.note-attestation/1`): **one event per vouched part**, signed by the human, carrying a
`{human, attested | co-signed, responsibility}` contributor and a human attestation token. That is chart_link's
shape, sealed as medication attestations are. Its payload names the target event, the target's content
address and the act. The floor (`db/071`) is offline-first at the remote door: structure only, the target
need not be held. The strict door checks the target: a note of the same chart, the content address matching,
the act's own rule. A projection `clinical_note_attestation` feeds the note read. Which parts a gesture
vouches, and how the compound reads, are pure functions in `cairn-node`. The window draws the result, sends
back the part ids it showed, and the node vouches only those ("as shown").

**Tech Stack:** PostgreSQL 18 (new `db/071`; in-place edits to `db/037`, `db/044`), Rust (`cairn-event`,
`cairn-node`), Tauri 2 + plain JS (`cairn-gui-tauri`, no npm).

**Spec:** [`docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](../specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
§3.4 (vouching later, sign-off, students), §3.7 (grades in words), §4.1, §4.3 (the overlay is a wire decision),
read with [ADR-0091](../../spec/decisions/0091-the-compound-note-and-the-addendum.md) **decision 3** (the vouch
shape, decided 2026-11-05) and its "(d) an honest under-claim until 3c-2". Read ADR-0087…0091 and the sign-in
blocks of `docs/developers/durable-rules.md` before Task 1.

**Maintainer, 2026-11-06 (this plan's question):** "Sign off this note" is offered **unless one non-student
clinician has signed or vouched for every part**. So an ordinary note signed by its author has no button. A
student's note, a note with an unsigned part, and a note two clinicians each signed all show it, and a later
addendum by someone else brings it back.

## Global Constraints

- **Wire (`cairn_event::note_attestation`, new):** `NOTE_ATTESTATION_EVENT_TYPE = "clinical.note-attestation.asserted"`,
  `NOTE_ATTESTATION_SCHEMA_VERSION = "clinical.note-attestation/1"`. Payload exactly
  `{"target": "<uuid>", "target_address": "<lower hex of the target's event_log.content_address, multihash prefix included>", "act": "addendum" | "sign-off"}`.
  Contributors exactly `[{"actor_id": K, "role": R, "responsibility": {"held_by": K}}]`. Here `K` is the
  human's key id, the envelope's `signer_key_id` is `K`, and `R` ∈ `"attested" | "co-signed"`. The token is
  `sign_attestation(ca, K, R, human_sk)`. `act = "addendum"` ⇒ `R = "attested"`. `patient_id` is the
  **target's** chart. `t_effective: None`, as `medication::attestation::build_attestation_body`. Twin (fixed
  copy, `render_note_attestation_twin`): `"Vouches for note part {target}: {act words} ({role})."`, where the
  act words are `"signed later by its writer"` (addendum) or `"the note signed off"` (sign-off).
- **Submit:** `prepare_sealed_as(tx, body, human_sk)` (`medication::sealed_submit`, so the safety rung,
  seal-then-sign and the unwrap-key check stay one path). Then `sign_attestation` over `event_address(&signed_bytes)`
  and `SELECT submit_event($1, $2, $3, $4)` with (bytes, token, attester verifying key bytes, DEK). One
  `next_hlc` per event, as `note::sign` allocates them.
- **Floor `cairn_check_note_attestation(text, jsonb)` (db/071).** Messages are prefixed
  `note attestation: `. Rules:
  - **R8**, as db/068: at the remote door a non-`/1` body returns unchecked.
  - Payload present; schema `/1`.
  - Exactly one contributor carries `responsibility`. Its role is `attested` or `co-signed`, and its
    `actor_id` = `signer_key_id`.
  - `target` is a uuid string. `target_address` is non-empty, even-length hex. `act` ∈ {`addendum`, `sign-off`}.
    An `addendum` act carries `attested`.
  - **Strict door only** (`cairn.remote_apply` not `on`):
    - The target is an `event_log` row of type `clinical.note.asserted`, with `patient_id` = the envelope's and
      `content_address = decode(target_address, 'hex')`.
    - `addendum` ⇒ the target has a `drafted` contributor whose key's actor is the signer key's actor.
    - `co-signed` ⇒ the target's `signer_key_id` is a human's key (`cairn_note_key_is_human`), and it
      belongs to a different actor than the vouch's signer. Only a part someone else signed is co-signed;
      an unsigned part is attested.

  The remote door checks structure only (ADR-0091 decision 3: offline-first; a vouch may arrive before its
  note). Registered in `cairn_event_twin_check` with the message
  `note attestation requires a non-empty authored twin (§3.13/§3.19)`. Off db/048's no-thread list, as notes are.
- **"A key's actor"** is `actor_event.actor_id` of the row `WHERE signing_key_id = k AND op IN ('enroll','rotate-key')`,
  so a re-keyed member is still the same person.
- **Projection `clinical_note_attestation`** (event_id PK, patient_id, target uuid, target_address bytea, act
  text, role text, signer_key_id text, t_effective, hlc_wall, hlc_counter, origin, content_address bytea NOT
  NULL; indexes on `target` and `content_address`). The applier is `clinical_note_attestation_apply` (clear
  payload, `/1` only, insert-or-nothing), registered `heal_safe`, `run_order` 20. A shred deletes its rows by
  `content_address` (db/037, in place, as `clinical_note_discard`).
- **Reader doors (db/071).** Each is `STABLE SECURITY DEFINER SET search_path = public, pg_temp`, revoked from
  `PUBLIC` and granted to `cairn_window`:
  - `cairn_note_key_actor(p_key text) RETURNS text` — that key's actor, hex; NULL if unknown.
  - `cairn_note_key_role(p_key text) RETURNS text` — the actor's pinned `role` (`actor_event.pinned ->> 'role'`;
    immutable per actor, so a student who re-enrols as a doctor is a new actor and old parts keep "(student)").
  - `cairn_note_staff_actor(p_staff_id text) RETURNS text` — the actor of the member a who's-here claim names
    (`vault_member.actor_id`, staff id normalised as the vault's doors do).
  - Reused as they are: `cairn_note_vouched_attester(event)` (a vouch counts only when it returns the vouch's
    own `signer_key_id`) and `cairn_key_confirmation(key)` (`'unconfirmed'` or `'doubtful'` ⇒ "signer
    unconfirmed").
- **`SCHEMA_GENERATION` 70 → 71**; `db/071` is appended to `crates/cairn-node/src/db.rs`'s `SCHEMA`. db/044's
  `gesture_kind` CHECK (both lists, in place) gains `'note-sign-off'`.
- **Read model (`cairn-node`).**
  - `NoteRow` gains `signed_by: Option<Signer>` (Some exactly when `grade` is `Signed`), `drafted_actor:
    Option<String>` and `vouches: Vec<PartVouch>`.
  - `pub struct Signer { actor: String, name: String, student: bool, unconfirmed: bool }`.
  - `pub struct PartVouch { event_id: Uuid, act: VouchAct, role: VouchRole, by: Signer }`.
  - `VouchAct { Addendum, SignOff }` and `VouchRole { Attested, CoSigned }` live in `cairn_event::note_attestation`,
    with `as_wire()`.
  - Only **verified** vouches whose `target_address` equals the part's own `content_address` are read, in
    `(hlc_wall, hlc_counter, event_id)` order.
- **Pure rules (`crates/cairn-node/src/note/vouch.rs`, new).**
  - `signed_for(p)` is the part's `signed_by` plus every vouch's `by`.
  - `part_is_signed(p)` is `signed_by.is_some() || !vouches.is_empty()`.
  - `signed_off_by(parts) -> Option<&Signer>` is the first non-student actor, in the root's `signed_for` order,
    who is in `signed_for` of **every** part.
  - `needs_sign_off(parts) = signed_off_by(parts).is_none()`.
  - `sign_off_vouches(parts, presser_actor) -> Vec<(Uuid, VouchRole)>` decides each part in turn: skip it if
    the presser is in its `signed_for`; else `CoSigned` if it has `signed_by`; else `Attested`.
  - `addendum_covers(parts, actor) -> Vec<Uuid>` is every part with `!part_is_signed` and
    `drafted_actor == Some(actor)`.
- **Words (exact; `view.rs`).**
  - A signer's name is `name` + `" (student)"` if a student + `" (signer unconfirmed)"` if unconfirmed.
  - A part's byline is today's `byline_words` with that name in place of the bare one.
  - A student signer with no `CoSigned` vouch gets `" — not co-signed"` straight after the name.
  - Then each vouch in order: `" · signed later by {V}"` (addendum), `" · signed off by {V}"` (sign-off,
    attested), `" · co-signed by {V}"` (co-signed).
  - The spoken form keeps today's `?` → `unverified ` rule.
  - Compound state (≥ 2 parts, exhaustive, no `_` arm):
    1. `"signed off by {S}"` when `signed_off_by` is `S` and the compound holds a `SignOff` vouch by `S`.
    2. Otherwise, if any `!part_is_signed` part grades `WentInUnsigned`/`WriterNotKnown`: `"includes an unsigned part"`.
    3. Otherwise, if any unsigned part grades `Claimed`: `"includes a part whose signature this node cannot verify"`.
    4. Otherwise `"every part signed"`.
- **Sign-off (`note::vouch::sign_off_note`).**
  - The window sends the compound's root and the part ids it showed.
  - The node re-reads the chart set's notes in the transaction, groups them (`compounds`), and finds the
    compound with that root. If the compound's parts are not exactly the ids sent, it answers
    `SignOffRefusal::Changed` and writes nothing.
  - It vouches `sign_off_vouches(parts, signer's actor)`, one event each, filed under each part's chart. An
    empty plan answers `SignOffRefusal::NothingToSign`.
  - Refusal words:
    - Changed: `"This note changed since it was shown — read the chart again, then sign it off. Nothing was signed."`
    - NothingToSign: `"You have already signed every part of this note. Nothing more was signed."`
  - Success: `"Signed off — every part of this note now carries your signature."`
- **The addendum's vouch (`note::sign::sign_note`).**
  - `SignNoteInput` gains `also_vouches: &'a [Uuid]`: the ids the strip listed, empty for a new note.
  - In Sign's transaction, after the parts are signed, the node vouches (`Addendum`, `Attested`) every id in
    `also_vouches ∩ addendum_covers(the target's compound, signer's actor)`.
  - `sign_note` returns `Committed<SignedNote>` in place of `Committed<Vec<Uuid>>`. It is
    `pub struct SignedNote { pub parts: Vec<Uuid>, pub vouched: Vec<Uuid> }`, with `parts` the old vector.
    Update every reader of `.value` (grep `sign_note(`, both trees).
  - The window's success line appends `" Your earlier unsigned part now reads as signed."` (n = 1) or
    `" Your {n} earlier unsigned parts now read as signed."`. When fewer than the strip listed were vouched, it
    further appends `" {k} listed part(s) were not yours by key, so they were not signed."`.
- **The strip's line.** For an addendum draft, the editor's own line below "Adding to …" reads
  `"Your signature also covers your earlier unsigned part{s} of this note: {time words, '; '-joined}"`. It is
  computed for the **who's-here claim's** actor, and shown only when that list is non-empty. It is information,
  never a dialog.
- **Student label:** `index.html`'s role `<select>` gains `<option value="student">student</option>` after
  `nurse`. `signer_webview_tests.rs` (role list) and `enrol.rs`'s doc line are updated. No vault change:
  `cairn_vault_purpose_allowed` already grants `clinical` to every role except `superuser`/`admin`. Pin that
  in db/tests/071.
- **Pinned guards a change here must satisfy** (memory "pinned-count guards"):
  - `twin_registry.rs` + `db/tests/034` (28 → 29), and **both** `cairn_projection_apply` counts.
  - `floor_execute_grants.rs` (the check fn, the applier and the three doors revoked from `PUBLIC`).
  - `window_role_grants_notes.rs` (the doors and `submit_event` of a vouch, driven under `cairn_window`).
  - `db_errors_stay_legible.rs` per-file counts, for each new `LocalDbFault` read.
  - `tests/common` helper lists (`identity_scaffolding_shared.rs`).
  - The **struct-literal sites** of `NoteRow` and `SignNoteInput` (grep them; include `cairn-sync/tests` and
    the window). Run the **full-workspace** build (memory "cross-crate call-site signature changes").
- **House rules:**
  - TDD.
  - Every non-trivial fn is documented for a junior reader.
  - Files stay under 500 lines: `notes_webview_tests.rs` (494) and `notes_tests.rs` (497) take no new
    tests — new pins go in new files.
  - No `salt`/`nonce`/`iv` names.
  - Run `scripts/check_closing_keywords.py` before every commit.
  - DB-gated tests show no `skipped:` line (`--nocapture`); a DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`.
  - A task touching `cairn-gui/*/src` runs every root source guard DB-free, with its own `CARGO_TARGET_DIR`.

## Review Focus

1. **Nurse B signs an addendum while who's here still says Nurse A.** The strip listed A's unsigned parts.
   B's key is not A's, so nothing of A's is vouched, and the success line says so. Pinned in Task 4.
2. **The note changes between showing and signing off** (a colleague's addendum lands, or a part is
   shredded). The sign-off is refused `Changed`, and it never vouches a part the clinician did not see.
   Pinned in Task 3.
3. **A raw-SQL client forges a vouch.** It tries an `addendum` act for someone else's part, a `co-signed` on
   its own signature, a target on another chart, or a stale `target_address`. The strict door refuses all four.
   The remote door admits each one structurally sound, and the read never counts one whose `target_address`
   is not the part's own or whose token is not verified. Pinned in Tasks 2 and 3.
4. **A vouch reaches a peer before its note.** It is admitted, shows nothing, and counts once the note lands,
   without a pen row. Pinned in Task 2.
5. **Two clinicians sign off the same note at once.** Both sets of vouches are kept, the state names the first
   signer in root order, and neither press is refused. Pinned in Task 3.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the progress sheet. A supervisor reads a student's entry and countersigns beneath it.
  A nurse who left a line unsigned comes back, adds a line and signs, which covers her own earlier line. A
  consultant countersigns an entry a registrar and a nurse wrote.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Countersigning a note: 2 (read, sign) → 2 ("Sign off this note", then the strip's passphrase, one act as
    slice 1 counts it) → 2.
  - Finishing one's own unsigned note: 3 → 3 (Add, type, Sign; the earlier part is covered by the same press)
    → 3. ADR-0091's under-claim (d) ends here.
  - Seeing that a student's part is not yet countersigned: 0 (no second signature on the page) → 0 → 0 (the
    byline says "— not co-signed").
- **Time + cognitive load:** sign-off ≤ 3 s (RUNBOOK §16's signing budget; the new `note-sign-off` timing
  kind). The strip's "also covers" line is read before the press: no new decision is forced, and it never
  blocks. A human owes the measurement in **RUNBOOK §23** (Task 7).

---

### Task 1: The vouch body and its twin (pure, `cairn-event`)

**Files:**
- Create: `crates/cairn-event/src/note_attestation.rs`
- Modify: `crates/cairn-event/src/lib.rs` (the `pub mod` line)

**Interfaces:**
- Produces: the constants above; `VouchAct`, `VouchRole` (each `as_wire() -> &'static str`, `Debug, Clone,
  Copy, PartialEq, Eq`); `pub struct NoteVouch<'a> { pub target: &'a str, pub target_address_hex: &'a str,
  pub act: VouchAct, pub role: VouchRole }`; `pub fn note_attestation_payload(v: &NoteVouch) -> Value`;
  `pub fn render_note_attestation_twin(v: &NoteVouch) -> String`; `pub fn vouch_contributors(kid: &str, role:
  VouchRole) -> Value`.

- [ ] **Step 1: Write the failing tests** in the module:
  - `the_payload_is_exactly_three_keys`
  - `the_contributor_bears_responsibility_held_by_itself`
  - `the_twin_words_are_exact`: `"Vouches for note part 0190…: signed later by its writer (attested)."`
    and the sign-off/co-signed form
  - `an_addendum_act_is_never_co_signed`: a `debug_assert` in the payload builder; `#[should_panic]` in debug
- [ ] **Step 2:** run `cargo test -p cairn-event note_attestation`. Expected: FAIL (no module).
- [ ] **Step 3: Implement** the module, mirroring `note_discard.rs`'s layout.
- [ ] **Step 4:** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): the vouch body and its twin (clinical.note-attestation/1)`.

### Task 2: The vouch stream's floor, projection and reader doors (`db/071`; `db/037`, `db/044` in place)

**Files:**
- Create: `db/071_clinical_note_attestation.sql`, `db/tests/071_clinical_note_attestation_test.sql` (with
  `\ir _scratch_database_guard.sql`), `crates/cairn-node/tests/note_attestation_floor.rs`
- Modify: `db/037_born_sealed.sql` (shred hook), `db/044_ui_gesture_timing.sql` (both CHECK lists),
  `crates/cairn-node/src/db.rs` (`SCHEMA`), `crates/cairn-event/src/schema_generation.rs` (71),
  `crates/cairn-node/tests/{twin_registry,floor_execute_grants,window_role_grants_notes}.rs`,
  `db/tests/034_twin_registry_test.sql`, and the two projection-count pins

**Interfaces:**
- Consumes: Task 1's body; `chart_link`'s test pattern for submitting a human-attested event; the test kit's
  vault enrolment helpers (`tests/common/vault_enroller_kit.rs`).
- Produces: the floor, the table, the applier and the three doors exactly as in Global Constraints.

- [ ] **Step 1: Write the failing tests** (`note_attestation_floor.rs`, DB-gated, `test_serial_guard`):
  - `a_sign_off_vouch_is_admitted_and_projected`
  - `an_addendum_vouch_by_the_parts_own_writer_is_admitted`: the part was committed by the timer with
    `{writer, drafted}`
  - `an_addendum_vouch_for_someone_elses_part_is_refused`
  - `a_co_sign_of_ones_own_signature_is_refused`
  - `a_co_sign_of_a_part_no_human_signed_is_refused`: the target was committed by the timer
  - `a_target_on_another_chart_is_refused`
  - `a_stale_target_address_is_refused`
  - `a_target_that_is_not_a_note_is_refused`
  - `a_vouch_without_a_token_is_refused`: the db/005 gate's message
  - `the_remote_door_admits_a_vouch_whose_note_is_not_held`: no pen, no error; it projects; Review Focus 4
  - `a_re_keyed_writer_vouches_for_a_part_drafted_under_the_old_key`
  - `a_shred_scrubs_the_projection`
  - `the_doors_answer_actor_role_and_staff_actor`: role `student` for a student enrolment; NULL for an
    unknown key

  db/tests/071 mirrors the structural rules. It also pins `cairn_vault_purpose_allowed('student', false,
  'clinical')` = true. Bump the pinned counts.
- [ ] **Step 2:** run `cargo test -p cairn-node --test note_attestation_floor -- --nocapture`. Expected: FAIL.
- [ ] **Step 3: Implement** `db/071` (mirroring `db/068`'s sections 1–6), the shred line, the timing kind,
  `SCHEMA` and generation 71.
- [ ] **Step 4:** run that command plus `twin_registry`, `floor_execute_grants`, `window_role_grants_notes`,
  `note_sign_parts` and `note_discard`, and the SQL mirror (via `scripts/run-db-gated-tests.sh`'s SQL step,
  `PGHOST=127.0.0.1 PGPORT=5532`, #862). Expected: PASS, no `skipped:`. Mutate to check: drop the strict
  `addendum` arm; `an_addendum_vouch_for_someone_elses_part_is_refused` must FAIL. Revert, then run
  `git status --short`.
- [ ] **Step 5: Commit** `feat(note): the vouch stream's floor, projection and readers (db/071)`.

### Task 3: Reading vouches, the pure rules, and "Sign off this note" (`cairn-node`)

**Files:**
- Create: `crates/cairn-node/src/note/vouch.rs` (the pure rules plus `sign_off_note` and a shared
  `vouch_in_tx`), `crates/cairn-node/src/note/vouch_read.rs` (`VOUCHES_SQL` and the pure attach),
  `crates/cairn-node/tests/note_sign_off.rs`
- Modify: `note/read.rs` (`NoteRow`'s three fields; `NOTES_SQL` gains the signer's actor, role and
  confirmation and the drafted key's actor, through the doors), `note/compound.rs` (state over
  `part_is_signed`, `SignedOff`), `note/view.rs` (the words), `note/mod.rs`; every `NoteRow` literal

**Interfaces:**
- Consumes: Task 2's doors; `compounds`; `prepare_sealed_as`; `AuthorParams`.
- Produces:
  - The pure functions and types in Global Constraints.
  - `pub(crate) async fn vouch_in_tx(tx, author: &AuthorParams<'_>, part: Uuid, chart: Uuid, act: VouchAct, role: VouchRole, node_origin: &str) -> anyhow::Result<Uuid>`.
    It reads the part's `content_address` from `event_log` and submits as in Global Constraints.
  - `pub struct SignOffInput<'a> { pub patient: Uuid, pub root: Uuid, pub shown_parts: &'a [Uuid], pub node_origin: &'a str }`.
  - `pub enum SignOffRefusal { Changed, NothingToSign }`.
  - `pub async fn sign_off_note(client: &mut Client, input: &SignOffInput<'_>, author: &AuthorParams<'_>) -> anyhow::Result<Result<Vec<Uuid>, SignOffRefusal>>`.
  - `CompoundState::SignedOff(String)`; `state_words(&CompoundState) -> Option<String>`.

- [ ] **Step 1: Write the failing tests.**
  - Pure, in `vouch.rs`:
    - `a_note_signed_wholly_by_its_author_needs_no_sign_off`
    - `a_students_note_needs_sign_off_until_co_signed`
    - `two_clinicians_parts_need_sign_off`
    - `sign_off_co_signs_others_signed_parts_attests_unsigned_ones_and_skips_ones_own`
    - `addendum_covers_only_the_actors_unsigned_parts`
    - `a_later_addendum_by_another_brings_the_offer_back`
  - Pure, in `view.rs`/`compound.rs`:
    - `a_student_part_reads_not_co_signed`: `"signed by Sam (student) — not co-signed"`
    - `a_co_signed_student_part`: `"signed by Sam (student) · co-signed by Dr Zed"`
    - `an_unconfirmed_signer_is_marked`
    - `a_vouched_unsigned_part_reads_signed_later`
    - `the_state_reads_signed_off_by`
    - `a_vouched_part_no_longer_makes_the_note_unsigned`
  - DB, in `note_sign_off.rs`:
    - `signing_off_a_students_note_co_signs_it`
    - `signing_off_attests_a_part_that_went_in_unsigned`
    - `a_part_added_after_the_note_was_shown_refuses_changed`: Review Focus 2
    - `signing_off_a_note_one_signed_wholly_is_nothing_to_sign`
    - `two_sign_offs_at_once_both_stand`: Review Focus 5
    - `a_forged_vouch_is_not_read`: a peer-admitted vouch with a wrong address, and one with an unverified
      token, change no byline; Review Focus 3
    - `a_linked_charts_part_is_vouched_on_its_own_chart`
- [ ] **Step 2:** run them. Expected: FAIL.
- [ ] **Step 3: Implement.** The vouch read runs in the same snapshot as `list_patient_notes`'s note
  statement. `signed_by` comes from the `Signed` grade's key; a `Claimed` part has none.
- [ ] **Step 4:** run them, then `cargo build --workspace --all-targets`. Expected: PASS, no `skipped:`.
  Mutate to check: let `sign_off_vouches` include the presser's own signed part. The pure test must FAIL, and
  so must the floor's co-sign rule. Revert.
- [ ] **Step 5: Commit** `feat(note): read vouches; the sign-off rules and "Sign off this note" (cairn-node)`.

### Task 4: An addendum's signature covers its writer's earlier unsigned parts (`note::sign`)

**Files:**
- Modify: `crates/cairn-node/src/note/sign.rs` (`SignNoteInput.also_vouches`, the vouch step, `vouched` on
  the success value); every `SignNoteInput` literal (`cairn-node/tests`, `cairn-sync/tests`, the window)
- Test: `crates/cairn-node/tests/note_addendum_vouch.rs` (new)

**Interfaces:**
- Consumes: `vouch_in_tx`, `addendum_covers`, `compounds`, Task 3's read in-transaction.
- Produces: `SignNoteInput.also_vouches: &'a [Uuid]`; `SignedNote` (Global Constraints).

- [ ] **Step 1: Write the failing tests:**
  - `signing_an_addendum_vouches_the_writers_own_unsigned_part`: the compound's state is no longer
    "includes an unsigned part"
  - `a_listed_part_that_is_not_the_signers_is_not_vouched`: Review Focus 1
  - `an_unlisted_own_part_is_not_vouched`: as shown
  - `a_new_note_vouches_nothing`
  - `the_timer_commit_vouches_nothing`: no key, no vouch
- [ ] **Step 2:** run them. Expected: FAIL.
- [ ] **Step 3: Implement.** The vouches go in Sign's transaction, after the parts. A refused vouch rolls the
  Sign back. The target's compound is read in that transaction from the header's target (ADR-0091 decision
  7), never from the window.
- [ ] **Step 4:** run them plus `note_addendum_sign`, `note_sign_parts`, `note_sign`, `authority_lockstep`,
  then `cargo build --workspace --all-targets`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): an addendum's signature covers its writer's earlier unsigned parts`.

### Task 5: The window's views and commands (`cairn-gui-tauri`, Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/notes_sign_off.rs`, `notes_sign_off_tests.rs`
- Modify: `notes_compound.rs` (`CompoundView.can_sign_off: bool`, `shown_parts: Vec<String>`, `covers_line:
  Option<String>`, `covers: Vec<String>`), `notes.rs` (`note_sign`'s `also_vouches: Vec<String>`, the success
  words), `main.rs` (register `note_sign_off`), the pane read (the claim's actor, via
  `cairn_note_staff_actor`, read once per pane)

**Interfaces:**
- Consumes: Task 3/4's node API.
- Produces: `#[tauri::command] note_sign_off(patient_id: String, root_event_id: String, shown_parts:
  Vec<String>, credential: Option<Credential>) -> Result<NoteSignReport, ErrorView>`. It follows
  `note_sign_impl`'s order: chart on screen, fixture, ids parsed, then the key; it records the
  `"note-sign-off"` timing; an unknown outcome uses Sign's #814 shape in its own words, starting
  `"Whether this note was signed off is NOT known"`.

- [ ] **Step 1: Write the failing tests** (`notes_sign_off_tests.rs`, pure + DB):
  - `can_sign_off_follows_needs_sign_off`, including a linked chart's compound
  - `shown_parts_are_the_compounds_part_ids_in_order`
  - `the_covers_line_names_the_claims_own_unsigned_parts`
  - `no_covers_line_without_a_claim_or_without_parts`
  - `sign_off_refusals_are_worded_exactly`
  - `sign_off_asks_for_no_key_on_a_malformed_id`
  - `the_sign_success_line_counts_vouched_and_unvouched`
- [ ] **Step 2:** run them (`cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri notes_sign_off`,
  own `CARGO_TARGET_DIR`). Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** run them plus the existing `notes_*` tests, and the root source guards DB-free. Expected: PASS.
- [ ] **Step 5: Commit** `feat(window): sign off a note; the strip's covers line; Sign reports what it vouched`.

### Task 6: The webview (`notes_compound.js`, `notes.js`, `index.html`)

**Files:**
- Modify: `src-ui/notes_compound.js` (the button), `src-ui/notes.js` (Sign sends `alsoVouches`, the draft's
  covers line), `src-ui/index.html` (the `student` option)
- Create: `src/notes_sign_off_webview_tests.rs`; modify `signer_webview_tests.rs` (the role list)

**Interfaces:**
- Consumes: Task 5's views and command.

- [ ] **Step 1: Write the failing tests** (source-guard style, `source_guard::js_fn`, as
  `notes_compound_webview_tests.rs`):
  - `sign_off_is_offered_only_when_the_view_says_so`
  - `sign_off_sends_the_shown_parts_and_the_strips_credential`: `signerCredential()`, then
    `clearSigner(signed)` as `notes_discard.js` does
  - `sign_off_sits_behind_the_whos_here_gate`
  - `its_accessible_name_is_sign_off_this_note_plus_the_time`
  - `the_covers_line_is_text_and_sign_sends_its_ids`
  - `no_dialog`: no `confirm(` and no `<dialog`
  - `the_student_role_is_offered`
- [ ] **Step 2:** run them. Expected: FAIL.
- [ ] **Step 3: Implement.** Every word is Rust's (ADR-0091 decision 11), except the button label
  `"Sign off this note"` and its accessible-name prefix.
- [ ] **Step 4:** run them; do a mock walk per memory "webview mock walk recipe", asserting VISIBILITY.
  Expected: PASS.
- [ ] **Step 5: Commit** `feat(window): "Sign off this note"; the strip's covers line; the student role`.

### Task 7: ADR-0092, spec, RUNBOOK §23, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0092-the-vouch-and-the-sign-off.md` (and its `mkdocs.yml` nav line)
- Modify: `docs/spec/index.md` (v0.94), the ADR index, `data-model.md` §3.3 (memory "medication spec-section
  mislabel": note prose sits beside medication's in §3.3), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`
  (§23: sign off ≤ 3 s; the student byline; the covers line, by keyboard and VoiceOver; WebKit),
  `docs/developers/durable-rules.md` (a 3c-2 block), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1:** write ADR-0092. It records the wire shape (ADR-0091 decision 3, in full), the floor's rules
  by door, the as-shown sign-off, the offer rule (maintainer, 2026-11-06), the words, and what is not
  guaranteed:
  - a vouch on a peer that cannot read its sealed body counts nothing;
  - #918's repudiated-key display is 3c-3's;
  - the pending-sign-offs menu is 3c-3's;
  - whether a student may sign off is site policy, not mechanism (a student's sign-off reads with
    "(student)").

  Fact-check it sentence by sentence against the SQL (memory "SDD evidence + ADR fact-check").
- [ ] **Step 2:** spec, RUNBOOK §23, the durable-rules block, HANDOVER/ROADMAP; file issues for every review
  residual. Run `scripts/check_closing_keywords.py`, then the docs build
  (`uv run --with-requirements docs/requirements.txt -- mkdocs build`) and
  `cargo test -p cairn-node --test paper_parity_plan_section`.
- [ ] **Step 3: Commit** `docs(note): ADR-0092 — the vouch and the sign-off; spec v0.94, RUNBOOK §23`.
