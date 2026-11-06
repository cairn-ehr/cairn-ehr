# Sign-in slice 3c-3 — supervision, the person menu, and a repudiated signature — design

- **Date:** 2026-11-06 (brainstorm with the maintainer)
- **Builds:** the slice 3 design's §3.6 (pending sign-offs: the person menu) and §3.7's display half of
  [#918](https://github.com/cairn-ehr/cairn-ehr/issues/918)
  ([`2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md));
  ends [ADR-0092](../../spec/decisions/0092-the-vouch-and-the-sign-off.md)'s under-claims (c) and (d).
- **Widens §3.6** with a thing the slice 3 design did not have: **supervision** as a relation between staff.
- **Mockup:** `.superpowers/brainstorm/74157-1793973205/content/person-menu.html` (gitignored; the maintainer:
  "exactly as I envisioned it"). It is a sketch, not a spec: layout and words are the builder's within §3.

## 1. Intent

Today a supervisor finds the notes that wait for their countersignature chart by chart, and checks their
supervisees' work by pulling files at random. The maintainer, an ED consultant who at times supervises up to twenty
doctors up to consultant level (overseas trained, on probationary registration), sharing that load with colleagues of
the same seniority: random file-pulling "is tedious and failure prone", and "my supervisees' recent notes" is a
long-wished feature at senior medical officers' meetings. No record system they have used supports supervision at
all: it lives on paper or in unstructured annotations that are neither searchable nor discoverable.

This slice makes three things true:

1. **Supervision is recorded**: who supervises whom, and who must have every signed note co-signed.
2. **Each person sees what waits for their signature** in one place: their own, and, for a supervisor, their
   supervisees' notes that need a co-signature, plus a list of supervisees' recent notes to review.
3. **A signature by a since-repudiated key counts as unsigned** (#918's display half), so it lands back on those
   lists for a real person to sign or correct.

## 2. Goals, non-goals, anti-goals

**Goals**
- A supervision relation, many-to-many: any staff member may have one or more supervisors; a supervisor may have
  many supervisees of any seniority (maintainer).
- A per-staff attribute **`needs_cosigning`** that makes co-signing **mandatory** for that person's signed notes;
  without it, co-signing is at the supervisor's discretion (maintainer).
- The person menu on the who's-here bar (mockup), with three groups (§3.4).
- An optional **"Co-sign as reviewed"** on a supervisee's note that did not need a co-sign, shown only to that
  writer's supervisors (maintainer).
- A part signed by a since-repudiated key reads so, and counts as unsigned (maintainer).

**Non-goals**
- Re-grading a repudiated attester's **links** (`patient_link`) — #918's other half stays open.
- Dates on supervision (a rotation's end) — a link lasts until someone ends it (maintainer).
- Syncing supervision between nodes — it lives where enrolment lives; slice 5 moves both.
- A lighter "seen, no concerns" mark distinct from a co-signature — the review mark IS a co-signature (maintainer).
- Who may be a supervisor (seniority, credentials) — site policy (principle 9); Cairn ships the mechanism.

**Anti-goals (a failure even if it works)**
- A supervision change that can be made, or undone, without a signed, append-only record of who did it and when.
- A note written under mandatory supervision that silently stops needing its co-sign because the requirement was
  lifted afterwards.
- A repudiated signature erased or rewritten; it is overlaid, and the part keeps showing what happened.
- The discretionary review list nagging: it is never counted in the bar's number.
- A confirmation dialog anywhere (principle 3).
- Reading a chart, or seeing the menu, requiring a passphrase (slice 3 design §3.6: the count is shown to whoever is
  at the terminal; after the shared unlock any staff member can open those charts anyway).

## 3. The design, as decided

### 3.1 Supervision and `needs_cosigning` (maintainer, 2026-11-06)

- **A link** "B supervises C" is made by a **superuser**, or by a member a superuser has granted a new
  **"assigns supervision"** right, the same pattern as slice 2's enroller grant (ADR-0085).
- **`needs_cosigning`** on C is set by the same people.
- **Any one of C's supervisors may lift `needs_cosigning` on C** alone ("end of mandatory supervision is usually at
  the discretion of the supervisors"). A link is ended by the same people who may make one, or by the supervisor
  themselves (*my call*: a supervisor may step away from a supervisee; nobody else is forced to).
- **A student has `needs_cosigning` for as long as they are a student.** The `student` role is pinned per actor
  (ADR-0092 decision 10), and a graduate re-enrols as a new actor, so the requirement falls away by itself. *My
  call:* a student's `needs_cosigning` is derived from the role and cannot be lifted, so nothing new is stored for
  it; the "(student)" label stays as built.
- **No dates.** A link and the attribute last until ended.
- *My call (builder check Q7):* **no self-dealing** — nobody may be linked as their own supervisor, and nobody may
  lift their own `needs_cosigning`; every supervision act is signed by a **confirmed** key, as enroller acts are
  (ADR-0085 decision 3). A holder of the "assigns supervision" right may link themselves as someone ELSE's
  supervisor.
- *My call (builder check Q7):* 3c-3a carries **two window surfaces**: the assigner's panel (superuser or holder of
  the right: make/end links, set `needs_cosigning`), and a supervisor's **"My supervisees"** panel (lift a
  supervisee's `needs_cosigning`, step away from a link). Console verbs mirror both.
- *My call (builder check Q6):* **the local record applies to every note this node reads**, its own and its peers',
  and the divergence until slice 5 is accepted and stated in the ADR: the same note may read "— not co-signed" on one
  node and cleared on another whose supervision record differs. Today a facility has one usable terminal anyway
  (#909). "As of signing" compares the part's `t_recorded` (HLC wall) with the local record's times.
- **Every change is a signed, append-only record** (who, when, which link or attribute), searchable and auditable,
  never an edit. *Builder's:* whether this is new ops in the closed actor-event algebra (db/004, where
  `grant-enroller` lives) or its own registry table with its own signed doors. Either way it must be carried by
  backup/restore (db/052's restore doors enumerate the actor ops today), read by the floor where it gates anything,
  and stay node-local until slice 5 (as the vault does).

### 3.2 What a co-signature means now (maintainer, 2026-11-06)

- **`needs_cosigning` is judged as it stood when the part was signed** (maintainer agreed): a note signed on the last
  day of probation still waits for its co-sign after the requirement is lifted, as a paper entry made under
  supervision still needs its countersignature. So the attribute's history must be readable as of a part's signing
  time (*builder's*: the part's `t_recorded`, never its stated `t_effective`, which is freely backdatable).
- **"— not co-signed" comes from `needs_cosigning`**, not from the `student` role alone. A probationary doctor's part
  reads the same as a student's. (Today: ADR-0092 decision 10/11, `Signer.student`.)
- **A mandatory co-signature is cleared only by one of the writer's supervisors** (maintainer agreed). A
  non-supervisor's sign-off is still recorded and still read ("signed off by …"), but does not clear "— not
  co-signed". *My call:* "a supervisor" means a supervisor of the writer **at the time of the co-signature** (a
  supervisor who has since stepped away still co-signed while responsible); a co-signer who was never the writer's
  supervisor never clears it.
- **Supervisors share the load:** whichever of a writer's supervisors co-signs first clears the item for all of them
  (maintainer).
- **A writer with `needs_cosigning` and NO supervisor recorded falls back to today's rule** (maintainer, 2026-11-07,
  builder check Q1): any non-student clinician's co-sign clears it, and the part and its menu row read **"no
  supervisor recorded"** so the gap is visible. This keeps every student part signed before this slice clearable.
- *My call (builder check Q4):* a part that went in unsigned and was signed later by its writer's own vouch still
  needs its co-sign when the writer had `needs_cosigning` at the part's `t_recorded`. Because the floor admits
  `co-signed` only of a human-signed envelope (db/071), **a supervisor's vouch of either role** (`attested` or
  `co-signed`) clears a mandatory requirement.
- *My call (builder check Q2; corrects this document's first draft, which the code contradicted):* **"Sign off this
  note" is offered while any part still lacks a mandatory co-sign** — so `needs_sign_off` gains that clause; today a
  non-student who signed every part ends the offer (`vouch.rs:60-78`), which would leave a probationary doctor's note
  with no button. It stays offered to whoever is named (the offer never depends on the presser, as now). A
  non-supervisor's sign-off is recorded and the state line may read "signed off by …", but the part keeps "— not
  co-signed" and the offer stays. The new ADR names the ADR-0092 decisions (1, 11, 12 at least) it supersedes.

### 3.3 "Co-sign as reviewed" (maintainer, 2026-11-06)

- On a supervisee's signed note that does **not** need a co-signature, the writer's supervisors (and only they) see
  an optional **"Co-sign as reviewed"**. It is the same `co-signed` vouch as a mandatory co-sign (ADR-0092's
  `sign-off` act, role `co-signed`) — the note then reads "co-signed by Dr M. Okafor" — with Sign's strip, staff ID +
  passphrase, no dialog.
- Once any of the writer's supervisors has co-signed it, the note leaves every co-supervisor's review list.
- *My call (builder check Q5):* it vouches **only the parts signed by the presser's supervisees**, never another
  writer's part or an unsigned part of the same compound — so it is its own node gesture, not `sign_off_note` under a
  second label. If the compound also needs a sign-off, both buttons may show; each says what it covers.

### 3.4 The person menu (mockup; maintainer: "exactly as I envisioned it")

- **Whose:** the person named in who's here (a claim, never a sign-in; ADR-0088). Nobody named → no menu (*my call*).
  The picker shows each person's count too (slice 3 design §3.6).
- **The bar** shows "Waiting for you: N". **N counts only what MUST be signed**: groups 1 and 2 below. Group 3 is
  never counted (maintainer agreed: it must not nag).
- **Group 1 — Your signature.** The person's unsigned drafts (with when each goes in by itself, ADR-0089); their
  parts that went in unsigned (`Asserted`, their `drafted` key's actor); and their parts **signed with a key since
  repudiated** (§3.5) — "counts as unsigned · check it is yours, then sign or correct it".
- **Group 2 — Needs your co-signature.** Signed parts, by any of the person's supervisees, whose writer had
  `needs_cosigning` at signing time and that no supervisor of the writer has co-signed. Each row names the
  co-supervisors who could also clear it ("you or Dr P. Shah"). A writer with no supervisor recorded is in nobody's group 2 (*my call*); the assigner's panel lists every
  staff member with `needs_cosigning` and no supervisor, so the gap has an owner.
- **Group 3 — Your supervisees' recent notes.** Collapsed by default. Supervisees' signed notes that did not need a
  co-sign and that none of the writer's supervisors has co-signed, newest first. *My call* (maintainer did not
  object): the last **7 days**. A reviewed note drops off for all co-supervisors.
- **Each row:** patient, what (progress note), when, and its state in words; **"Open note"** opens that chart on that
  note (slice 3 design §3.6). For a group-3 note, the opened note offers "Co-sign as reviewed" (§3.3).
- *My call:* the menu reads no sealed body. Patient, kind, time, writer and signatures come from what the chart
  list already reads without opening text (envelope, projection, registry); if a builder finds a field that is only
  in a body, it is dropped from the row, never decrypted for a list.

### 3.5 A repudiated signature (#918's display half; maintainer, 2026-11-06)

- A part whose `Signed` grade's key (ADR-0092 decision 10: the verified attester, else the signer) has since been
  **repudiated** reads, e.g., **"signed with a key since repudiated — counts as unsigned"**, and is treated as
  unsigned everywhere: `part_is_signed`, `needs_sign_off`, the person menu.
- The old signature is **overlaid, never erased**: the byline still names the key's actor and the original time.
- It appears in **group 1 of the key's actor** (with re-key, a repudiated key still belongs to the real person's
  actor, ADR-0085) and, where mandatory co-signing applied, in their supervisors' group 2.
- A **vouch** made by a since-repudiated key counts nothing either (*my call*, same reason).
- **Only the real person's own re-signature clears it** (maintainer, 2026-11-07, builder check Q3): a vouch by the
  key's actor under a key that is NOT repudiated. A colleague's sign-off or a supervisor's co-sign of it is recorded
  and read, but the part stays unsigned and stays in the real person's group 1, because the words may be the
  impostor's. **If the real person has left, it stays unsigned for good, annotated "key had been repudiated"**
  (maintainer). *My call:* until re-signed it is not in anyone's group 2 — there is no valid writer's signature to
  co-sign.
- *Corrects the first draft (the code contradicted it):* the floor needs **no widening** for the re-sign — db/071's
  strict door already admits a `sign-off` act with role `attested` by the part's own actor (`db/071:150-168`). The
  work is in Rust and the readers: `signed_for` (`vouch.rs`) must not count a repudiated `signed_by`, or
  `sign_off_vouches` answers `NothingToSign` to the real person (the repudiated key shares their actor,
  `cairn_note_key_actor`); and the planner must not plan a `co-signed` vouch of a repudiated-key signature
  (`cairn_note_key_is_human`, db/067, ignores repudiation). *Builder's:* whether the floor should also refuse such a
  `co-signed` vouch; the words of the re-sign ("re-signed by Dr A" or similar); and that `compound_state` does not read
  "every part signed" while a repudiated part counts as unsigned.
- The words "(signer unconfirmed)" keep covering `unconfirmed` and `doubtful` (ADR-0092 decision 10).

## 4. Constraints and facts for the builder

- **Read vouches only through `vouch_read`'s two checks** and compare people by actor (ADR-0092 "Harder"). A new
  role/act word must be added everywhere at once (wire enums, floor, applier skip, `act_of`/`role_of`,
  `vouch_words`).
- **Grades and signers live in** `crates/cairn-node/src/note/{vouch,vouch_read,view,compound}.rs`; `Signer` is
  `{actor, name, student, unconfirmed}` today (`vouch_read.rs:31`). `cairn_key_confirmation` (db/004) already returns
  `repudiated`.
- **The grant pattern** to copy is db/063's `cairn_vault_set_enroller` (superuser-signed over the member's current
  state; a no-op refused; `cairn_actor_append`).
- **The window** is plain JS + Tauri, no npm (`cairn-gui/cairn-gui-tauri/src-ui`); the who's-here bar is
  `#whos-here-bar` in `index.html`. Every note act that holds the strip checks `signInFlight()` (ADR-0092).
- **Cross-chart read:** the menu is the first window read that spans charts. `cairn_window` already reads
  `clinical_note` and `clinical_note_attestation` on every chart (granted to `cairn_agent`, db/064:313, db/071:202,
  inherited per db/062:22); a definer door is needed only for the supervision record and the actor registry (db/062's
  posture: the window never holds the owner's keys). It carries a §1.2 time budget (§6).
- Durable rules: read the sign-in 3c-2, 3c-1, 3b-2a, 3b-1 and slice-2 blocks in
  `docs/developers/durable-rules.md` before touching their code; every pinned-count guard a new event type or door
  moves (memory of past slices: twin registry + db/tests/034, `floor_execute_grants`, `window_role_grants_*`).

## 5. Build order — *my call*, one PR each

| PR | Contents |
|---|---|
| **3c-3a** | Supervision + `needs_cosigning`: the record, the doors, console verbs and a superuser panel in the window; "— not co-signed" from the attribute as of signing; a mandatory co-sign cleared only by a supervisor; "Co-sign as reviewed". The ADR. |
| **3c-3b** | A repudiated signature counts as unsigned (§3.5): the readers and the planner; only the real person's re-sign clears it. Independent of 3c-3a; may go first. |
| **3c-3c** | The person menu: the cross-chart door, the bar's count, the three groups, "Open note" onto the note, the picker's counts. Needs 3c-3a (groups 2–3) and 3c-3b (group 1's repudiated rows). |

## 6. Paper-parity benchmark (§1.2) — for each plan to carry

- **Paper counterpart:** the supervisor's countersignature beneath an entry in the bedside folder; finding what to
  countersign = walking the ward or a stack of folders; random audit = pulling files at random; supervision itself =
  a paper list or a note in a staff file.
- **Steps:** find what waits for my signature — paper: walk to each folder (*N* = one per folder) → built *M* = 2
  (open the menu, read it) → *K* = 1 (the count on the bar already says whether anything waits). Countersign one
  note from the menu — paper 2 (open folder, sign) → 3 ("Open note", "Sign off"/"Co-sign as reviewed", passphrase) →
  *K* 3, with the passphrase as slice 1 counts it. **Ruled not an `M > N` defect** (maintainer, 2026-11-07): paper's
  "open folder" includes finding and walking to it, the menu's "Open note" does not — "much better than paper,
  because now we do not have to waste a lot of time searching for the notes that require supervision". Record a supervision link — paper 1 (write a list) → 1 act + passphrase.
- **Time + cognitive load:** the menu opens ≤ 1 s with 20 supervisees and a week of notes (*my call*; measured by
  3c-3c's RUNBOOK section); co-sign ≤ 3 s (Sign's budget, RUNBOOK §16/§23). Cognitive load: the count names only
  obligations; the review list is opt-in.

## 7. Left to the builder

The record's shape (§3.1), the floor widening's shape (§3.5), the door's query, the words beyond those quoted here,
the menu's layout within the mockup, the superuser panel's layout, the order of 3c-3a and 3c-3b, and how the new ADR
(or ADRs) splits.
