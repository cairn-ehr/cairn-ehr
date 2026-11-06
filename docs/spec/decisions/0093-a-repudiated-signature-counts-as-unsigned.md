# ADR-0093 — A repudiated signature counts as unsigned

- **Status:** Accepted (supersedes [ADR-0092](0092-the-vouch-and-the-sign-off.md) decision 10's clause "A
  `repudiated` key is NOT marked"; amends ADR-0092 decision 11's rules and decision 12's words, as decisions 3–10
  below say; ends ADR-0092's "not guaranteed" (c); builds the slice 3c-3 design's §3.5)
- **Date:** 2026-11-07
- **Spec version at acceptance:** 0.95
- **Issues:** draft PR [#1006](https://github.com/cairn-ehr/cairn-ehr/pull/1006) (GUI sign-in slice 3c-3b). Design:
  `docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md` (§3.5); plan:
  `docs/superpowers/plans/2026-11-07-sign-in-slice-3c-3b-repudiated-signature.md`. Builds the display half of
  [#918](https://github.com/cairn-ehr/cairn-ehr/issues/918), which stays open for its link half. Filed by the build:
  [#1007](https://github.com/cairn-ehr/cairn-ehr/issues/1007) (a co-sign over a repudiated envelope key) ·
  [#1008](https://github.com/cairn-ehr/cairn-ehr/issues/1008) (the offer outlives a colleague's sign-off) ·
  [#1009](https://github.com/cairn-ehr/cairn-ehr/issues/1009) (a re-signed student part was never co-signed; fixed by
  this slice, decision 6) · [#1010](https://github.com/cairn-ehr/cairn-ehr/issues/1010) (a sign-off before the
  re-sign leaves a student's part uncountersigned).
- **Builds on:** [ADR-0092](0092-the-vouch-and-the-sign-off.md) (the vouch, the sign-off, the pure rules and the
  words) · [ADR-0085](0085-in-person-enrolment-and-re-key.md) (repudiation; `rotate-key` keeps the actor) ·
  [ADR-0007](0007-authorship-and-accountability.md) (signature proves origin, attestation confers responsibility) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (logic below the UI)

## Context

On paper, an entry found to have been signed by an impostor is not torn out. Someone writes beside it that the
signature is not Dr A's, and the real Dr A reads the entry and signs it again if the words are theirs. Nobody else's
signature can stand in for hers: a colleague can say "I read this", but not "Dr A wrote this".

Cairn can already repudiate a key (ADR-0085): a superuser marks it, after which it signs nothing new, and
`cairn_key_confirmation` answers `repudiated` for it. What it signed before is still on the chart, and its signature
still verifies, because a signature proves which key signed, not who held the key. Until this slice the chart read
such a part exactly like any other signed part (ADR-0092, not guaranteed (c)). This slice, **3c-3b**, makes the
chart say what happened and stop counting the signature. No event, floor rule or projection changes: the note read
already fetched each signer's confirmation word (`NOTES_SQL`, `vouch_read::vouches_column!`), and the work is in the
pure rules and the words.

## Decision

### The maintainer's rulings (2026-11-06 and 2026-11-07)

1. **A part signed by a since-repudiated key counts as unsigned**, everywhere the chart asks whether it is signed.
   Its old signature is overlaid, never erased: the byline still names the person the key was enrolled for.
2. **Only the real person's own re-signature clears it**: a vouch by the same actor under a key that is not
   repudiated. A colleague's sign-off, or a supervisor's co-sign, is recorded and read, but the part stays unsigned,
   because the words may be the impostor's. **If the real person has left, the part stays unsigned for good**,
   annotated "key had been repudiated". That annotation is the maintainer's wording (2026-11-07); the byline built
   from it, "(key had been repudiated) — counts as unsigned", replaces the design §3.5 example "signed with a key since
   repudiated".

### The read (`note::vouch_read`)

3. **`Signer` gains `repudiated`**, set by `signer_of` exactly when `cairn_key_confirmation` answers `repudiated`
   (db/004). It is exclusive with `unconfirmed` (`unconfirmed` or `doubtful`), because the function returns one word.
   It is read for the key the part's `Signed` grade names and for every vouch's signer, from the columns ADR-0092
   decisions 9 and 10 already read; no SQL changes. **This supersedes ADR-0092 decision 10's "A `repudiated` key is
   NOT marked".** Pin: `vouch_read.rs::a_key_the_node_does_not_know_matches_no_actor_and_is_named_so`.

### The pure rules (`note::vouch`), amending ADR-0092 decision 11

4. **"Re-signed" is one predicate, `vouch::re_signed(p)`**: the part's `signed_by` is repudiated AND some vouch on it
   is by a key that is not repudiated, of the same actor (`same_actor`, so an empty actor matches no one). It is false
   for a part with no `signed_by` or a standing one. The rules and the words both use it. Pin:
   `vouch_tests.rs::only_the_real_persons_re_sign_clears_it`.

5. **A repudiated key's signature counts for nothing, and neither does its vouch:**
   - `signed_for(p)` lists every signer of the part whose key still stands: `signed_by`, then each vouch's signer,
     skipping a repudiated one. A colleague's vouch on a repudiated, not re-signed part is listed (`has_signed`
     needs it); whether the part is signed is `part_is_signed`'s answer, not this list's;
   - `part_is_signed(p)` is `re_signed(p)` for a repudiated `signed_by`, true for any other `signed_by`, and, with no
     `signed_by`, whether some vouch's signer is not repudiated;
   - `full_signers(parts)` yields nobody unless every part `part_is_signed`, so while a repudiated part is not
     re-signed, a colleague who vouched it never reads as having signed the note off, and `needs_sign_off` stays true.

   Pins: `vouch_tests.rs`'s `a_repudiated_signature_counts_as_unsigned`,
   `a_colleagues_sign_off_does_not_clear_a_repudiated_part`, `a_repudiated_keys_vouch_counts_nothing`.

   **A colleague's sign-off made before the re-sign starts counting after it** (controller ruling). The vouch named
   the part's exact bytes (its content address, ADR-0092 decision 9), and once the real person says those words were
   theirs, the colleague's vouch is an ordinary sign-off of them: the colleague is in `signed_for`, the part is
   `part_is_signed`, and `signed_off_by` names that colleague (on a note of two or more parts the state can read
   "signed off by" them). Pin: `vouch_tests.rs::a_colleagues_earlier_sign_off_counts_once_re_signed`.

6. **"Sign off this note" offers a repudiated part to its own re-keyed writer, and co-signs it only once re-signed.**
   `sign_off_vouches` still skips a part the presser `has_signed`; because `signed_for` no longer lists the
   repudiated key, which shares the re-keyed writer's actor (`cairn_note_key_actor` does not filter repudiation), the
   real person is planned a vouch rather than answered `NothingToSign`. The plan is `Attested` for every part whose
   `signed_by` is repudiated and not `re_signed`, even though the registry knows its human: a `co-signed` vouch would
   read "co-signed by Dr Zed" over a possibly forged signature, as if Zed had put a second signature under the real
   person's. Once the part is re-signed, its signature stands again and is co-signed as any other (#1009), which the
   floor admits (a human's envelope key, another actor), so a supervisor can clear a re-signed student part's "— not
   co-signed". **Only a REPUDIATED, not re-signed `signed_by` refuses `CoSigned`** (controller ruling): a part with no
   `signed_by` whose envelope key is a human's keeps `CoSigned`, the floor's own rule (ADR-0092 decision 11). The
   re-sign needs no floor change: db/071's strict door asks who the signer is only of an `addendum` act and a
   `co-signed` role, so it admits a `sign-off` vouch with role `attested` by the part's own actor. Pins: `vouch_tests.rs`'s `the_re_keyed_writer_is_offered_their_repudiated_part`,
   `a_repudiated_part_is_never_planned_as_a_co_sign`, `a_re_signed_part_is_co_signed_again`,
   `sign_off_co_signs_only_what_the_floor_admits`; `note_repudiated_signer.rs`'s
   `a_repudiated_signers_part_reads_unsigned_until_they_re_sign`, `a_colleagues_vouch_on_a_repudiated_part_is_attested`,
   `a_re_signed_students_part_is_co_signed_by_a_supervisor` (through the planner and the floor).

7. **An addendum never covers a part that has a `signed_by`**, so never a repudiated part: `addendum_covers` takes
   only parts with no `signed_by` that are not `part_is_signed`. A repudiated part is re-signed with "Sign off this
   note". The addendum's floor rule (the signer drafted the part) need not hold for a part the writer signed as its
   author. Pin: `vouch_tests.rs::an_addendum_does_not_cover_a_repudiated_part`.

### The words (`note::view`, `note::compound`), amending ADR-0092 decision 12

8. **The signer's name** (`signer_words`) is followed, after any " (student)", by **" (key had been repudiated)"**
   for a repudiated key, for a part's signer and for a vouch's signer alike. " (signer unconfirmed)" and it never
   both apply.

9. **A part's byline** (`part_byline`, shown and spoken alike):
   - a repudiated `signed_by` that is not re-signed reads **" — counts as unsigned"** after the name, in the place a
     student's " — not co-signed" goes, and instead of it (never both);
   - the vouch that re-signs it (decision 4's predicate, applied to that vouch) reads **" · re-signed by {name}"**,
     whatever its act or role; every other vouch keeps ADR-0092's words;
   - once re-signed, a student's part reads " — not co-signed" again, as any student part, until a supervisor
     co-signs it;
   - a `CoSigned` vouch by a repudiated key does not clear a student's " — not co-signed".

   E.g., with the note fixture's clinician: "signed by Dr Ada Notewriter (key had been repudiated) — counts as
   unsigned"; after Dr Zed's sign-off, "… — counts as unsigned · signed off by Dr Zed"; after her re-sign, "signed by
   Dr Ada Notewriter (key had been repudiated) · signed off by Dr Zed · re-signed by Dr Ada Notewriter". A re-sign by
   a key that is itself repudiated later stops counting, and the part reads "— counts as unsigned" again. A student:
   "signed by Sam (student) (key had been repudiated) — not co-signed · re-signed by Sam (student)", then, after a
   supervisor's co-sign, "signed by Sam (student) (key had been repudiated) · re-signed by Sam (student) · co-signed by
   Dr Zed". Pins: `view_tests.rs`'s `a_repudiated_signers_byline_says_it_counts_as_unsigned`,
   `a_re_signed_students_part_reads_not_co_signed_until_a_supervisor_co_signs`,
   `a_repudiated_students_part_counts_as_unsigned_not_uncosigned`, `a_re_sign_by_a_key_since_repudiated_counts_nothing`,
   `a_repudiated_co_signer_does_not_clear_not_co_signed`.

10. **The compound state** (a note of two or more parts):
    - "signed off by {S}" needs S to have pressed sign-off under a key that is not repudiated: a sign-off vouch by a
      repudiated key never makes the note read "signed off by";
    - a part signed by a repudiated key and not re-signed makes the note read "includes an unsigned part". The
      `Signed` arm of the unsigned scan, unreachable before, is reached for exactly that part.

    The window offers "Sign off this note" whenever `needs_sign_off` holds and the editor is free, unchanged (ADR-0092
    decision 17), so a repudiated part brings the button. Pins: `compound.rs`'s `a_repudiated_part_makes_the_note_include_an_unsigned_part`,
    `a_repudiated_keys_sign_off_does_not_sign_the_note_off`; `notes_sign_off_tests.rs::a_repudiated_part_offers_sign_off`.

## What is not guaranteed

- **(a) A raw client can still file a `co-signed` vouch of a repudiated signature.** db/071's strict door reads
  `cairn_note_key_is_human` (db/067), which ignores repudiation, so it admits one by another actor. It is read
  (" · co-signed by Dr Zed"); while the part is not re-signed it clears nothing, because decisions 5 and 9 ask
  `re_signed` first. Once the real person re-signs, it counts as an ordinary co-sign of their words (it would clear a
  student's "— not co-signed"). The floor was left unchanged (the plan's ruling): the node never plans one while the
  part is not re-signed.
- **(b) A co-sign can still be planned over a repudiated ENVELOPE key** when the part has no `signed_by`: the read
  carries no repudiation word for the envelope's `signer_key_id`, and `signer_human_actor` comes from
  `cairn_note_key_is_human` ([#1007](https://github.com/cairn-ehr/cairn-ehr/issues/1007)).
- **(c) After a colleague signs off a repudiated part, the button stays offered to them** (`needs_sign_off` is still
  true) and pressing it answers `NothingToSign`, the pattern a student meets today (ADR-0092 (e);
  [#1008](https://github.com/cairn-ehr/cairn-ehr/issues/1008)).
- **(d) A supervisor's sign-off made before a student's re-sign leaves the part uncountersigned.** It was planned
  `Attested` (decision 6), so after the re-sign the part reads " — not co-signed". The supervisor is now in
  `signed_for`, so a second press answers `NothingToSign`; and once the supervisor has signed every part,
  `needs_sign_off` is false and the button is gone for everyone
  ([#1010](https://github.com/cairn-ehr/cairn-ehr/issues/1010)). A sign-off made after the re-sign co-signs
  (#1009, fixed).
- **(e) The parts to re-sign are found chart by chart.** Nothing lists a person's repudiated parts across charts
  until 3c-3c's person menu (design §3.4, group 1).
- **(f) An addendum never covers a repudiated part** (decision 7). A writer who adds to the note and signs is not
  told the repudiated part is left; its byline still says "— counts as unsigned".
- **(g) Links and other attested acts are untouched.** A `patient_link` attested by a since-repudiated key keeps its
  standing; that is #918's other half, which stays open
  ([#918](https://github.com/cairn-ehr/cairn-ehr/issues/918)).

## Consequences

- **Easier.** A forged signature no longer passes for a real one: the chart says the key had been repudiated and
  counts the part as unsigned, with no act beyond the repudiation itself. Steps (paper *N* → built *M* → target *K*): seeing that a
  part's signature no longer counts 1 (find and read the annotation) → 0 (the byline says it) → 0; the real person
  re-signing their part 2 (read, sign) → 2 ("Sign off this note", then the strip's passphrase, one act as slice 1
  counts it) → 2. No `M > N`. Time budget, owed by a human in RUNBOOK §24: re-sign ≤ 3 s (Sign's, RUNBOOK §16; the
  `note-sign-off` timing kind). No decision is forced: the byline names what happened.
- **Harder.** Every reader that asks "is this part signed?" or "who signed it?" must go through `part_is_signed`,
  `signed_for` and `re_signed`, never `signed_by.is_some()`: a repudiated `signed_by` is present and counts for
  nothing. Every future plan that co-signs must ask whether the signature it co-signs still stands.
- **The bet.** That "— counts as unsigned" on the byline, "includes an unsigned part" on the note and the sign-off
  button coming back are enough for the real person to find and re-sign their parts, chart by chart, until 3c-3c's
  menu. We would know it is wrong if repudiated parts stay unsigned for people who are still on staff, or if readers
  take "signed by Dr A (key had been repudiated)" to mean Dr A signed it.
- **Pinned by** `crates/cairn-node/src/note/{vouch_tests,view_tests,compound,vouch_read}.rs`'s tests,
  `crates/cairn-node/tests/note_repudiated_signer.rs` and
  `cairn-gui/cairn-gui-tauri/src/notes_sign_off_tests.rs`. RUNBOOK §24 carries the human measurements.
