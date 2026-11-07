# GUI sign-in, slice 3c-3b — a since-repudiated key's signature counts as unsigned — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A note part signed by a key a superuser has since **repudiated** (the impostor case, ADR-0085/0086)
reads **"signed by Dr A (key had been repudiated) — counts as unsigned"**, counts as unsigned everywhere the
chart asks, and is cleared **only** by the real person's own re-signature under a key that is not repudiated. A
colleague's or supervisor's sign-off of it is recorded and read but clears nothing. If the real person has left,
the part stays unsigned for good, annotated. A vouch made by a repudiated key counts nothing either. This is
#918's display half.

**Architecture:** No wire change, no SQL change, no `SCHEMA_GENERATION` bump. The note read already fetches
`cairn_key_confirmation` for a part's signer (`read.rs`, `NOTES_SQL` column 17) and for every vouch's signer
(`vouch_read::vouches_column!`). That function already answers `repudiated` (db/004). A repudiated key still
grades `Signed`, because the grade checks only the signature. So the slice is (1) one new `Signer.repudiated`
flag, set by `signer_of`; (2) the pure rules in `note::vouch` counting a repudiated signature as no signature;
(3) the words in `note::view` and the state in `note::compound`. The window draws `part_byline` and
`CompoundView.can_sign_off` (`needs_sign_off`) unchanged; only its test fixtures gain the field.

**Tech Stack:** Rust (`cairn-node`; `cairn-gui-tauri`'s tests only), PostgreSQL 18 for the DB-gated test.

**Spec:** [`docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md`](../specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md)
§3.5 (approved by the maintainer 2026-11-07), read with
[ADR-0092](../../spec/decisions/0092-the-vouch-and-the-sign-off.md) decisions 10–12 and "What is not
guaranteed" (c). Read the sign-in 3c-2 block of `docs/developers/durable-rules.md` before Task 1.

**Maintainer, 2026-11-07:** "go back to unsigned"; only the real person's own re-signature clears it ("1a");
"if Dr A has left, it stays marked as unsigned and annotated with 'key had been repudiated'".

## Global Constraints

- **Repudiated means exactly** `cairn_key_confirmation(key) = 'repudiated'`. It is mutually exclusive with
  `unconfirmed` (`unconfirmed`/`doubtful`), since the function returns one word.
- **Copy, exact:**
  - `signer_words` appends, after any `" (student)"`, **`" (key had been repudiated)"`** for a repudiated
    signer (the existing `" (signer unconfirmed)"` cannot also apply).
  - `part_byline`, for a part whose `signed_by` is repudiated and **not re-signed**: the signer's words are
    followed by **`" — counts as unsigned"`**, in the place where a student's `" — not co-signed"` goes. That
    replaces `" — not co-signed"`: never both.
  - A vouch that **re-signs** (it is not repudiated, and is by the same actor as a repudiated `signed_by`) reads
    **`" · re-signed by {signer_words}"`**, whatever its act or role. Every other vouch keeps `vouch_words`'
    words, with `signer_words` (so a repudiated vouch-signer is annotated too).
  - Examples, with the note fixture's clinician `Dr Ada Notewriter` (use the fixture's actual display name):
    - `"signed by Dr Ada Notewriter (key had been repudiated) — counts as unsigned"`;
    - after Dr Zed's sign-off:
      `"signed by Dr Ada Notewriter (key had been repudiated) — counts as unsigned · signed off by Dr Zed"`;
    - after Ada's re-sign:
      `"signed by Dr Ada Notewriter (key had been repudiated) · signed off by Dr Zed · re-signed by Dr Ada Notewriter"`.
- **"Re-signed"** (the one predicate the rules and the words share), for part `p` whose `signed_by = Some(s)`
  with `s.repudiated`: some vouch `v` in `p.vouches` with `!v.by.repudiated && same_actor(&v.by.actor,
  &s.actor)`.
- **No floor change.** db/071's strict door already admits a `sign-off`/`attested` vouch by the part's own actor
  (`db/071:150-168`). The floor still admits a `co-signed` vouch of a repudiated-key signature, because
  `cairn_note_key_is_human` ignores repudiation. The node must never PLAN one (Task 1), and the read must
  never let one clear anything (Tasks 1–2). The ADR states that a raw client can still file one.
- Compare people by **actor**, never by key (ADR-0092 decision 11). An empty actor matches no one.
- House rules: TDD, inline docs for a junior, files under 500 lines (`vouch.rs` is 357, `view.rs` 240,
  `compound.rs` 386: tests go in their existing `*_tests.rs`/`mod tests`; a new DB suite gets its own file),
  test key material minted at runtime.
- **Pure-test command:** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib note::` (memory: a DB-free run
  fails without it). If an IDE holds `target/`, prefix `CARGO_TARGET_DIR=/tmp/cairn-3c3b`.

## Review Focus

1. **A colleague's sign-off of a repudiated part clears nothing.** The byline still says "— counts as
   unsigned", `needs_sign_off` stays true, and the real person is still offered the press. Pinned in Task 1
   (rules) and Task 3 (through the database).
2. **The re-keyed real person must not be answered `NothingToSign`.** Their new key shares the repudiated key's
   actor (`cairn_note_key_actor` does not filter repudiation), so a `signed_for` that still counted the
   repudiated signer would skip the part. Pinned in Tasks 1 and 3.
3. **A planned vouch on a repudiated part is `Attested`, never `CoSigned`.** Otherwise it would read "co-signed
   by Dr Zed" over a forged signature and clear a student's "— not co-signed". Pinned in Task 1.
4. **A re-sign by a key that is later repudiated itself stops counting:** the part reads "— counts as unsigned"
   again. Pinned in Task 2.
5. **A vouch by a repudiated key counts nothing, on any part**, including the only vouch on a part that went in
   unsigned, and as the co-signer that would clear a student's mark. Pinned in Tasks 1 and 2.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** an entry in the bedside folder found to be signed by an impostor. It is annotated
  ("signature not Dr A's"), never torn out, and the real Dr A reads it and countersigns it if it is theirs, or
  writes a correction.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Seeing that a part's signature no longer counts: 1 (find and read the annotation) → 0 (the byline says it)
    → 0.
  - The real person re-signing their part: 2 (read, sign) → 2 ("Sign off this note", then the strip's
    passphrase, one act as slice 1 counts it) → 2.
  - Finding the parts to re-sign: chart by chart until 3c-3c's person menu (design §3.4 group 1). The same
    under-claim as ADR-0092 (d), stated in the ADR, not an `M > N` defect.
- **Time + cognitive load:** re-sign ≤ 3 s (Sign's budget, RUNBOOK §16/§23; the existing `note-sign-off`
  timing kind). No new decision is forced: the byline names what happened. A human owes the measurement in
  **RUNBOOK §24** (Task 4).

---

### Task 1: The flag and the rules (`note::vouch_read`, `note::vouch`)

**Files:**
- Modify: `crates/cairn-node/src/note/vouch_read.rs` (`Signer`, `signer_of`, its tests)
- Modify: `crates/cairn-node/src/note/vouch.rs` (`signed_for`, `part_is_signed`, `full_signers`,
  `sign_off_vouches`, `addendum_covers`, the `fixture` module)
- Test: `crates/cairn-node/src/note/vouch_tests.rs`
- Modify (compile only): the `Signer { … }` literals in `cairn-gui/cairn-gui-tauri/src/notes_sign_off_tests.rs:33`,
  `notes_parts_tests.rs:203`, `notes_compound_tests.rs:45` gain `repudiated: false`

**Interfaces:**
- Produces: `Signer.repudiated: bool` (doc: "the key was repudiated by a superuser since; its signature counts
  for nothing, ADR-0093"); `signer_of` sets it from `confirmation == Some("repudiated")`;
  `pub fn re_signed(p: &NoteRow) -> bool` in `vouch.rs` (the Global Constraints predicate; false unless
  `signed_by` is repudiated); `fixture::repudiated(actor, name) -> Signer`.
- Rule changes (signatures unchanged):
  - `signed_for(p)` lists only signers whose signature counts: `signed_by` and each vouch's `by`, skipping any
    `repudiated`.
  - `part_is_signed(p)`: a repudiated `signed_by` ⇒ `re_signed(p)`; a counting `signed_by` ⇒ true; `None` ⇒
    some vouch whose `by` is not repudiated.
  - `full_signers(parts)` yields nobody unless every part `part_is_signed`, so a colleague who vouched a
    repudiated part never "signed it off".
  - `sign_off_vouches`: `CoSigned` only when the part's `signed_by` is `Some` and not repudiated (as well as the
    existing `signer_human_actor` rule); otherwise `Attested`.
  - `addendum_covers`: also requires `p.signed_by.is_none()`. A repudiated part is re-signed by "Sign off this
    note", never covered by an addendum, whose floor rule (own *drafted* part) need not hold for it.

- [ ] **Step 1: Write the failing tests** in `vouch_tests.rs`, using `fixture::repudiated`:
  - `a_repudiated_signature_counts_as_unsigned`: `part(1, Some(repudiated("a","Dr A")))` ⇒
    `!part_is_signed`, `signed_for` empty, `needs_sign_off(&[p])`.
  - `a_colleagues_sign_off_does_not_clear_a_repudiated_part` (Review Focus 1): add a `SignOff`/`Attested` vouch
    by `who("z","Dr Zed")` ⇒ still `!part_is_signed`, `needs_sign_off`, `signed_off_by(..).is_none()`.
  - `only_the_real_persons_re_sign_clears_it`: a `SignOff`/`Attested` vouch by `who("a","Dr A")` (the same actor,
    a counting key) ⇒ `re_signed`, `part_is_signed`, `!needs_sign_off`.
  - `the_re_keyed_writer_is_offered_their_repudiated_part` (Review Focus 2):
    `sign_off_vouches(&[p], "a") == [(p.event_id, VouchRole::Attested)]`.
  - `a_repudiated_part_is_never_planned_as_a_co_sign` (Review Focus 3):
    `sign_off_vouches(&[p], "z") == [(p.event_id, VouchRole::Attested)]`, although `signer_human_actor` is
    `Some("a")`.
  - `a_repudiated_keys_vouch_counts_nothing` (Review Focus 5): `part(2, None)` with a vouch by
    `repudiated("z","Dr Zed")` ⇒ `!part_is_signed`.
  - `an_addendum_does_not_cover_a_repudiated_part`: the part has `drafted_actor = Some("a")` and a repudiated
    `signed_by` ⇒ `addendum_covers(&[p], "a")` is empty.
  - In `vouch_read.rs`'s `a_key_the_node_does_not_know_matches_no_actor_and_is_named_so`: add `repudiated:
    false` to the expected literal, and assert `signer_of(None, None, None, Some("repudiated")).repudiated &&
    !….unconfirmed`.
- [ ] **Step 2: Run** the pure-test command. Expected: FAIL (no field `repudiated`, no `re_signed`).
- [ ] **Step 3: Implement** the Interfaces above. Each rule's doc comment says why a repudiated signature is
  skipped (the words may be the impostor's) and names ADR-0093.
- [ ] **Step 4: Run** the pure-test command, then the GUI tree's
  `(cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri --lib notes_)`. Expected: PASS, and every
  pre-existing `note::` and `notes_` test still passes unchanged.
- [ ] **Step 5: Commit** `feat(note): a since-repudiated key's signature counts for nothing (#918, display half)`.

### Task 2: The words and the note's state (`note::view`, `note::compound`)

**Files:**
- Modify: `crates/cairn-node/src/note/view.rs` (`signer_words`, `vouch_words`'s caller in
  `part_byline_marked`, the student co-sign check)
- Modify: `crates/cairn-node/src/note/compound.rs` (`compound_state`)
- Test: `crates/cairn-node/src/note/view_tests.rs`, `compound.rs`'s `mod tests`

**Interfaces:**
- Consumes: `Signer.repudiated`, `vouch::re_signed` (Task 1).
- Produces: the Global Constraints copy. Signatures unchanged.
  - `part_byline_marked`: for a repudiated `signed_by`, the suffix is `" — counts as unsigned"` unless
    `re_signed(p)`, and the student `" — not co-signed"` suffix is not added. A vouch that re-signs reads
    `" · re-signed by …"`. The student co-sign check also requires `!v.by.repudiated`.
  - `compound_state`: the `NoteGrade::Signed { .. }` arm of the unsigned scan, commented "Not reached" today,
    is now reached for a repudiated, un-re-signed part. It sets `unsigned = true`, so the note reads "includes
    an unsigned part". Rewrite its comment to say why.

- [ ] **Step 1: Write the failing tests:**
  - In `view_tests.rs`, `a_repudiated_signers_byline_says_it_counts_as_unsigned`: the three Global Constraints
    examples, exact, with `who`/`repudiated` fixtures named `Dr Ada Notewriter` and `Dr Zed`.
  - In `view_tests.rs`, `a_repudiated_students_part_counts_as_unsigned_not_uncosigned`:
    `"signed by Sam (student) (key had been repudiated) — counts as unsigned"`.
  - In `view_tests.rs`, `a_re_sign_by_a_key_since_repudiated_counts_nothing` (Review Focus 4): the re-signing
    vouch's `by` is `repudiated("a", …)`. The byline ends `" — counts as unsigned · signed off by Dr Ada
    Notewriter (key had been repudiated)"`; `!part_is_signed`.
  - In `view_tests.rs`, `a_repudiated_co_signer_does_not_clear_not_co_signed` (Review Focus 5): a student part
    with a `SignOff`/`CoSigned` vouch by `repudiated("z","Dr Zed")` reads
    `"signed by Sam (student) — not co-signed · co-signed by Dr Zed (key had been repudiated)"`.
  - In `compound.rs`'s tests, `a_repudiated_part_makes_the_note_include_an_unsigned_part`: two parts, the
    second signed by a repudiated key ⇒ `state_words(..) == Some("includes an unsigned part")`. With the real
    person's re-sign added ⇒ `Some("every part signed")`.
- [ ] **Step 2: Run** the pure-test command. Expected: FAIL on the new tests only.
- [ ] **Step 3: Implement** as Interfaces says.
- [ ] **Step 4: Run** the pure-test command and the GUI tree's `notes_` tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): "key had been repudiated — counts as unsigned"; "re-signed by"`.

### Task 3: Through the database: repudiate, sign off, re-sign (`tests/note_repudiated_signer.rs`)

**Files:**
- Create: `crates/cairn-node/tests/note_repudiated_signer.rs`. Reuse `note_sign_off.rs`'s pattern: `mod common;`,
  `#[path = "common/note_vouch_kit.rs"] mod note_vouch_kit;`, `common::note_setup`, `note_vouch_kit::member`,
  and the `db_test!` macro shape. Copy `ward`/`sign_as`/`sign_off`/`note` locally, since that file is 560
  lines and must not grow.
- Test: same file. Also `cairn-gui/cairn-gui-tauri/src/notes_sign_off_tests.rs` (pure).

**Interfaces:**
- Consumes: `sign_off_note`, `SignOffRefusal`, `list_patient_notes`, `compounds`, `part_byline`,
  `needs_sign_off` (all existing).
- A local helper `async fn repudiate_and_rekey(c: &Client, m: &Member) -> Member`. It reads the actor
  (`SELECT actor_id FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'`), mints a new key at runtime,
  runs `SELECT cairn_actor_append($1, 'rotate-key', $2, NULL, NULL)`, then `SELECT cairn_actor_append($1,
  'repudiate', $2, 'console', NULL)` on the OLD key (the pattern of `actor_readers_after_rotation.rs:52-63`),
  asserts `cairn_key_confirmation(old) = 'repudiated'` (the fixture's premise), and returns the Member under the
  new key with the same `staff`.

- [ ] **Step 1: Write the failing DB tests:**
  - `a_repudiated_signers_part_reads_unsigned_until_they_re_sign` (Review Focus 1 + 2):
    1. Ada signs a one-part note.
    2. `repudiate_and_rekey(ada)`.
    3. The byline is the first Global Constraints example, and `needs_sign_off` holds.
    4. Zed signs it off, which succeeds. The byline is the second example, and `needs_sign_off` still holds.
    5. Ada (new key) signs it off, which succeeds, not `NothingToSign`. The byline is the third example, and
       `!needs_sign_off`.
  - `a_colleagues_vouch_on_a_repudiated_part_is_attested`: after step 4 above, `SELECT role FROM
    clinical_note_attestation` for Zed's vouch is `attested` (Review Focus 3, through the planner and the
    floor).
- [ ] **Step 2: Run** `CAIRN_TEST_PG=<conn> cargo test -p cairn-node --test note_repudiated_signer -- --nocapture`
  (memory "PG test substrate": `cairn_test` on :5532, `PGHOST=127.0.0.1 PGPORT=5532`). Expected after Tasks
  1–2: PASS, with no `skipped:` line (memory "SDD evidence": a self-skip is not a pass). Before them: FAIL. If
  Tasks 1–2 already landed, show the test failing by stashing Task 1's `part_is_signed` change.
- [ ] **Step 3: GUI pin** in `notes_sign_off_tests.rs`, `a_repudiated_part_offers_sign_off`: a compound whose
  only part has a repudiated `signed_by` has `can_sign_off == true`. Run the GUI tree's `notes_` tests. Expected:
  PASS.
- [ ] **Step 4: Commit** `test(note): a repudiated signer re-signs through the database; a colleague's vouch is attested`.

### Task 4: ADR-0093, spec v0.95, RUNBOOK §24, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0093-a-repudiated-signature-counts-as-unsigned.md` (and its `mkdocs.yml` nav
  line)
- Modify: `docs/spec/index.md` (v0.95), the ADR index (`docs/spec/decisions/README.md`), `data-model.md` §3.3
  (memory "medication spec-section mislabel"), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (§24),
  `docs/developers/durable-rules.md` (a 3c-3b block), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0093.** It supersedes ADR-0092 decision 10's "A `repudiated` key is NOT marked" and
  amends decision 11's rules, as Task 1 changed them. It ends ADR-0092 (c). It records the maintainer's rulings
  (Spec header). What is not guaranteed:
  - a raw client can still file a `co-signed` vouch of a repudiated signature; it is admitted and read, and
    clears nothing;
  - the parts to re-sign are found chart by chart until 3c-3c's menu;
  - #918's link re-grade half stays open;
  - an addendum never covers a repudiated part.

  Fact-check every sentence against the code (memory "SDD evidence + ADR fact-check").
- [ ] **Step 2:** RUNBOOK §24, a human pass:
  - repudiate a test member's key at the window, then open a note they signed: the byline reads as
    constrained, and VoiceOver says it;
  - a colleague's sign-off leaves the part "counts as unsigned";
  - the member's re-sign ≤ 3 s;
  - WebKit.

  Then the durable-rules block (the "re-signed" predicate is THE one test; never count a repudiated signer in
  `signed_for`), and HANDOVER/ROADMAP. Comment on #918 with what is now built (the issue stays open). File
  issues for review residuals.
- [ ] **Step 3: Gates:**
  - `python3 scripts/check_closing_keywords.py <commit message file>`;
  - `uv run --with-requirements docs/requirements.txt -- mkdocs build`;
  - `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test paper_parity_plan_section`;
  - `cargo fmt --all -- --check`;
  - `cargo clippy -p cairn-node --all-targets -- -D warnings`;
  - `RUSTDOCFLAGS=-D warnings cargo doc -p cairn-node --no-deps`.
- [ ] **Step 4: Commit** `docs(note): ADR-0093 — a repudiated signature counts as unsigned; spec v0.95, RUNBOOK §24`.
