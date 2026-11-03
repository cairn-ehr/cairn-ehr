# GUI milestone, sign-in slice 3 — the progress note, its draft, and who is at the terminal (design)

**Date:** 2026-11-03 · **Status:** approved in conversation with the maintainer, awaiting a plan ·
**Refines:** slice 3 of [`2026-10-31-gui-sign-in-and-authorship-design.md`](2026-10-31-gui-sign-in-and-authorship-design.md)
(§5.7, §5.8, §6) · **Builds:** [ADR-0041](../../spec/decisions/0041-progress-note-narrative-format.md) (the note
event, text-only), [data-model §3.10](../../spec/data-model.md#310-session-identity-event-authorship-and-draft-durability)
(durable drafts), [ADR-0008](../../spec/decisions/0008-point-of-care-identity-possession-and-salvage.md) (the
grades), [ADR-0020](../../spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
(delete is never erase) · **Needs a new ADR:** see §9.

Text marked *My call* is the designer's, not the maintainer's; everything else was decided in conversation.

## 1. Why

The GUI milestone ends with a clinician writing a progress note and signing it. The slice-3 case the maintainer
named first is the **progress note written so far**: an ED nurse at a shared terminal is called away mid-note at
3 a.m. Today nothing can hold that note — the window's writes are all one-press acts (the list sign-off,
cease, withhold/release, register, link/unlink, the vault acts), and **no note stream exists**: ADR-0041 fixed the wire shape of
`clinical.note.asserted`, nothing builds it. So this slice brings the note with it (maintainer: "inside slice
3"), text-only.

A second, daily case the maintainer brought: **final-year medical students** write the history and examination;
today they borrow a doctor's login (the incumbent system has no seat for them) and the doctor marks in the
text who wrote what. This design serves it with no new mechanism: each writer's part is theirs, each
signs their own part, and the supervisor **co-signs** (ADR-0028's `co-signed`, the bearing role for a supervisory
countersignature) — their addendum and signature carry the supervisor's responsibility.

## 2. Goals, non-goals, anti-goals

**Goals**
1. A clinician writes a text progress note in the window and signs it (per-act staff ID + passphrase, as
   slice 1's signer strip already does). It is attested.
2. Everything typed is durable from the keystroke (a power cut loses nothing) and is **not** in the record
   until signed or abandoned past the quarantine timer.
3. An abandoned draft goes into the record by itself, honestly graded: **asserted** (a named writer,
   nobody vouched) or **unattributed** (writer left unknown) — never attested.
4. Whoever wrote it finds what is waiting for them and finishes / vouches for it later, by overlay.
5. One gesture says who is at a shared terminal; the person named is shown loudly.
6. A note finished in parts reads as **one note**, with its parts one press away for audit.

**Non-goals (this slice)**
- Media, drawings, the event-anchor grammar in notes (ADR-0041 decisions 2–4, 6, 7 and decision 9's second
  bullet) — text only.
- Several live hidden window contexts per terminal (parent design non-goal). "Return to my last screen" is a
  **bookmark** (maintainer).
- Commit on the dot with every terminal off (maintainer: a running window or the next chart open suffices).
- Notes replicating as drafts to other nodes — a draft is never an event and never syncs.
- A site policy that demands a passphrase to say who's here (parent design §5.8's chart-open option).
  *My call:* deferred; the default was "no" anyway.
- Measuring #797's "since they took the chart" per person. *My call:* the baseline stays what the window last
  showed; a per-person baseline is a later refinement the who's-here claim makes possible.
- Medication writes change in no way except that the signer strip's staff ID is prefilled (§3.6).

**Anti-goals — a build that does any of these has failed even if it works**
- An auto-committed note shown as signed, or any path that signs as a human without their passphrase.
- **A draft lost** to a power cut, a window crash, a terminal lock or a user switch.
- **Reading a chart requires saying who you are** (paper-parity: reading paper needs no name). The pick is
  required only to *begin writing*, where it replaces typing the staff ID at the signature.
- A draft of Dr X's discarded by anyone but Dr X (§3.5).
- The compound note hiding that a part went in unsigned, or when.
- A confirmation dialog as the safety mechanism (principle 3).
- Draft prose stored at rest more readable than the sealed event it becomes (§4.2).

## 3. The design, as decided

### 3.1 "Who's here" — one switch for the whole terminal

- A **hot key**, and a visible button for mouse/touch/VoiceOver users, **blanks the screen** (the current chart
  leaves the screen, like closing a folder) and asks **"Who's at this terminal?"**: a type-ahead over enrolled
  staff (name or staff ID) plus this terminal's recent people, each showing their pending-sign-off count, plus
  **"Don't know / just reading"**. **No passphrase.**
- The person named is shown **very loudly**: a full-width bar at the top of every screen, the name in large
  type (*my call:* never colour alone — the name is text, announced on change).
- Picking a person returns them to **their last screen**: a bookmark of (chart, tab) per (terminal, person),
  reopened fresh (a new read), never a hidden live copy. Their drafts survive regardless (§3.2).
- **The terminal lock (the screensaver) clears "who's here"**, so after it the next person is always asked
  (maintainer). The picker never blocks reading: "just reading" or going straight to search is fine.
- This one claim replaces the parent design's two passphrase-free prompts ("who's writing", "who's opening").
- It is a **claim, never authentication**: it names who drafts; it never signs. Every signature is still staff
  ID + passphrase.
- **Switching and the re-lock forget any held key** (review finding): on a key-hold terminal an act with no
  staff ID signs with the held key (`signer.js`), so without this Dr B, switched in, could sign as Dr A. The
  superuser panel already wipes the held key for the same reason ("someone new may be at the keyboard").
- *Builder's:* the key itself, the bookmark store (window-local or node — either is fine; it holds no clinical
  content beyond a chart id), the picker's layout (mockup `switch-user.html` in the brainstorm session is a sketch,
  not a spec).

### 3.2 The draft

- Starting a note needs a named writer; "unknown" is allowed (anti-goal of the parent design: no field
  satisfiable only by fabrication). If nobody is named, starting a note asks who's here first.
- Every keystroke is saved to a **durable draft store on the facility node**. Debounce interval is the
  builder's; a power cut may lose at most that interval.
- **A draft is edited only at the terminal it was started on** (maintainer, review finding I1): nothing can
  reliably tell whether someone is still writing in it on another machine — and Cairn must keep working with
  the network down — so two terminals editing one draft would silently overwrite each other. At any other
  terminal the draft is read-only, and **"Continue" there starts an addendum draft** of that terminal's own.
  The window needs a stable terminal id for this (builder's: where it is kept).
- A draft belongs to its chart and lists **every writer who typed into it**: when a different "who's here"
  at the same terminal picks a draft up and types, that person is added (maintainer: naming every writer is enough — "whoever finally
  signs has to take responsibility for the content anyway"). This refines §3.10's "keyed by `(author,
  patient)`": a draft has a subject and a writer *set*, never a session. It also departs from §3.10's "a
  'switch' hides … the previous user's draft": here drafts are visible to all (next bullet).
- **Another writer's text can be added to but never deleted — you can't rub out a colleague's ink**
  (maintainer). A draft is a sequence of parts, each typed by one writer; a writer edits only their own parts,
  and anyone else continues *below*. Enforced at the floor: the save door refuses a save that changes or removes
  another writer's part. So Dr Z can never empty Dr X's draft and type "seen" over it.
- **Drafts are visible to everyone who opens the chart** (maintainer: "it may contain important information for
  the next writer"), plainly marked *unsigned draft*, with its writers, start time and terminal. Another person
  may read it, continue it (at its terminal by typing into it; anywhere else as an addendum draft), or start
  their own note.

### 3.3 Walking away: the quarantine clock

- **The clock runs from the draft's last keystroke** (maintainer, after the builder check showed that the
  first rule — start on a new note on the same chart, a closed chart, or the terminal lock — leaves an open,
  idle draft, a crashed window and a power cut with no clock at all). The three walk-away moments the maintainer
  named are all covered by it: none of them adds keystrokes. Typing again restarts it.
- Its length is **site policy, default 3 hours**, set by the owner at the node's console (*my call:* a
  `cairn-node` verb in the shape of `vault-set-unconfirmed-expiry`, stored node-side).
- **The idle screensaver is built in this slice** (the terminal lock today is shown only at launch): after a
  per-terminal idle interval, **default 15 minutes** (maintainer; the same figure as the window's
  `IDLE_TIMEOUT`, which today caps the key hold — a separate setting, not that constant reused), the terminal
  re-locks, which also clears who's here (§3.1). *My call:* a launch setting beside `--key-hold-seconds`.
- **Who commits it:** any running window commits overdue drafts on a regular tick, and opening a chart commits
  that chart's overdue drafts first, so its reader sees what is now in the record. If no window runs, the draft
  waits — safe, still shown, marked **overdue** (maintainer accepted). The commit is signed by the **node's** key
  (today every window holds it, `--key`), contributors `[{writer, "drafted"} …, {node, "recorded"}]`, or
  `[{node, "recorded"}]` alone for an unknown writer.
- **The commit is one door in the database that decides "overdue" itself** (review finding): it checks the
  last keystroke against the policy on the node's clock and consumes the draft row in the same statement —
  idempotent, so two windows ticking at once, or a signature racing the timer, can never put two notes in the
  log, and a client on raw SQL cannot commit someone's draft early. A save or a signature on a consumed draft
  is refused.
- Until it commits, a draft is only on the facility node: a peer site does not see the note yet.

### 3.4 Signing, finishing, vouching later

- **Signing a draft** is an ordinary per-act signature; the note goes in **attested**. The signer is
  `authored` if they wrote the whole text, otherwise `attested` with every writer (themselves included, if they
  typed a part) `drafted` (review finding: ADR-0028's `authored` owns "the exact words as written", which a
  signer who typed nothing does not). Either way the signer carries responsibility for the whole text.
- **A note that went in unsigned is finished by an addendum** (`refs: [{rel: "addendum-to", event}]`, already in
  ADR-0041 decision 8's closed enum — no wire change for the link). The original is never edited.
- **Adding stays simple and vouches only for the adder's own words** (maintainer, after the builder check:
  "whoever adds can be assumed to have read what came before, but cannot be expected to verify it"). Signing
  an addendum vouches for the addendum — *my call:* and for any earlier unsigned part **named as the same
  person** (Nurse A finishing her own half-note vouches for her whole text with one press), never for anyone
  else's part. Because that name was a who's-here claim with no proof (it may be Nurse B, who forgot to switch),
  the signer strip **lists every `?`-named part the signature will also cover** (§3.7), so the signer sees what
  she is vouching for before she presses — information in the strip, not a confirmation dialog.
- **Signing off the whole note is a separate act** — "Sign off this note" on the compound — and it vouches for
  **every** part, whoever wrote it (maintainer: "the person who signs off the whole … can be expected to have
  verified it"). The supervisor of a student does this. It is a **co-signature** (maintainer, review finding
  C1): the signer `co-signed` (ADR-0028) on every part someone else signed, and an attestation on every part
  nobody vouched for. *My call:* one gesture, one overlay per part (or one over the set — the ADR's wire choice,
  §4.3). **Offered whenever the note has no sign-off of the whole yet** — a note whose parts are all signed
  still lacks its countersignature (the earlier "never on a wholly vouched note" made a student's signed history
  impossible to countersign).
- **Anyone may add an addendum to anyone's note**, named on their own part (*my call*, consistent with ADR-0041:
  cross-author `refs` are legal additive claims) — a doctor's correction to a student's history, say.
- **Students** are ordinary enrolled members who **sign their own parts** — a student's signature attests their
  authorship; the supervisor's addendum carries theirs, and the supervisor's **sign-off of the whole** co-signs
  the student's part (maintainer). Until then the part reads **signed by X (student) — not co-signed** (§3.7);
  whether a site treats it as *pending until co-signed* is policy (ADR-0028). *My call:* the enrolment form
  offers a `student` role label with the same powers as `clinician`/`nurse` (the vault's role is open text and
  `cairn_vault_purpose_allowed` gives every role except `superuser` and `admin` the `clinical` purpose, so this
  is a UI list change, plus the pinned option list in `signer_webview_tests.rs`).

### 3.5 Discarding a draft

- A draft may be **discarded until it commits (signed, or by the timer), leaving no trace in the record**
  (maintainer). After it commits, nothing removes it: only an ADR-0020 suppression overlay.
- **Only its author may discard it, and must sign the discard** with staff ID + passphrase (the who's-here pick
  carries no proof). Dr Z may not discard Dr X's draft (maintainer).
- *My call:* a draft with **several writers** needs **every writer's** signature to discard — the "sole author"
  rule generalised; in practice, once two people have typed into it, it goes into the record.
- *My call:* a draft whose writer is **unknown** cannot be discarded by anyone (nobody can prove authorship); it
  commits as unattributed. Its content can be overlaid later like any note.
- Hard policy: the discard rule is **enforced at the floor** (a client on raw SQL cannot delete a draft), via a
  signature the database verifies, the same pattern as slice 1's signature-gated enrol.

### 3.6 Pending sign-offs: the person menu

- On the who's-here bar (and in the picker), the named person's **count of things waiting for their signature**:
  their unsigned drafts and their notes that went in unsigned. Pressing it opens a list — patient, what, when,
  and when a draft will go in by itself — each with **"Open note"**, which opens that chart on the note.
- The signer strip's staff ID is **prefilled** from who's here (the passphrase never is).
- *My call:* the count is shown to whoever is at the terminal without a passphrase — after the shared terminal
  unlock any staff member can open those charts anyway, so it reveals nothing new.

### 3.7 The compound note and the author display

- A note and its addenda display as **one note** in one continuous text, headed by the first writer, the start
  time, and its state; a **"N parts ▸"** flag opens each part with who typed it, when, and whether it went in
  signed or unsigned (and when and why: "recorded by this node after 3 h unattended") (maintainer: "displayed as
  the final compound … with a flag that allows to drill down to individual contents at the individual times for
  audits").
- The compound reads **signed** only when every part is vouched for; otherwise **"includes an unsigned part"**.
- **A name that came only from who's here is shown with a `?` in front — `?Nurse A`** (maintainer): it cannot
  be verified that it was that person, the equivalent of guessing an author from the handwriting. A signed part
  shows the signer's name plainly; the `?` goes wherever an unsigned part names its writer (the parts view, the
  compound's heading, the pending-sign-offs menu, the signer strip). *My call:* the `?` is text, read out by
  VoiceOver as "unverified", never a glyph alone.
- Grades shown in words: signed (attested) · **signed by X (student) — not co-signed** (attested, no co-sign
  yet) · **went in unsigned — written by ?X** (asserted) · **writer not known**
  (unattributed / `Device`) · *signer unconfirmed* for a part signed by a key an enroller issued and no superuser
  has confirmed (the per-note marker slice 2 deferred; ADR-0085) · and, for a later-repudiated key, the display
  half of **#918**.
- Mockups: `pending-signoffs.html`, `switch-user.html`, `compound-note.html` in the brainstorm session
  (`.superpowers/brainstorm/`, gitignored) — sketches, not specs.

## 4. Constraints and facts for the builder

### 4.1 Read-side grade (`Asserted`)
`classify_authorship_confidence` (`crates/cairn-event/src/contributor.rs`) has `Attested` / `Unverified` /
`Device`; a node-signed note with `{writer, "drafted"}` grades `Device` today and loses the name. Add
`Asserted` per the parent design §5.7 (no bearing contributor, a named **human** contributory one — "human" a
registry fact passed in as resolved key ids, so the classifier stays pure). There is no exact SQL twin, but
`crates/cairn-node/tests/authority_lockstep.rs` pins how the Rust grades map onto `cairn_claim_authority` for
its three pinned shapes (a vouched human attester, device-only, a claimed author; `contributor.rs` also names
`cairn_authorship_bound` as a counterpart): **`Asserted` extends that lockstep test** (the parent design's "no SQL twin"
was too strong — builder check, 2026-11-03). *An attestation act is never asserted* (parent §5.7).

**What "signed" means on the wire** (*my call*, builder check): a directly signed note is the medication shape
— `{human, "authored"}` signed by the human (ADR-0053), which grades `Attested`; a later vouch for an unsigned
part is a verified attestation overlay on that part (§4.3). The compound reads *signed* when every part either
grades `Attested` or carries a verified vouch — two inputs, one rule, written once as a pure function.

### 4.2 The draft store
- Mutable, durable, on the facility node, reached only through `SECURITY DEFINER` doors granted to
  `cairn_window` (ADR-0084: the window runs on `cairn_window`, never the owner).
- **Draft prose at rest is no more readable than the sealed event it becomes.** On the custody node a sealed
  event's clear payload and twin live in `event_clear` (db/005, ADR-0052's operational twin); *my call*: drafts
  sit in a plain table with **no grant to `cairn_window` at all** — read and written only through definer
  doors — and no per-save encryption. That is stricter than `event_clear`, which `cairn_window` *can* read
  directly through its `cairn_agent` membership (`db/005_submit.sql:1147`, `db/062_window_role.sql:23`; review
  finding — an earlier line here wrongly called that posture door-only). Whether drafts ride in `backup` is the
  builder's to decide and state.
- **The writer set is append-only at the floor, and every save is a revision row** (*my call*, builder check):
  edits need no passphrase, so a `cairn_window` client on raw SQL — or a person who misclaims at who's here — can
  write under another's name. The parts rule (§3.2) stops anyone erasing another writer's part, but a misclaim
  can still edit the parts *named as* the person claimed. That residual is accepted (the result commits
  *asserted*, shown `?`-named, never attested), and the floor keeps it findable: no door removes a writer, and
  earlier text is recoverable from the revision rows while the draft lives. Revisions go when the draft does
  (commit or discard): a draft is not the record.
- A shred/erasure of a patient (ADR-0005) must reach their drafts too.

### 4.3 The note stream and vouching-later
- `clinical.note.asserted`, profile `clinical.note/1`: `narrative` (markdown, the pinned austere profile),
  `media: []`, `refs`, `encounter: null` (ADR-0041 decision 9, first bullet — `null` is honest, never
  manufactured). Born sealed (ADR-0052), per-write human authorship (ADR-0053), legibility twin from the
  narrative, floor-validated `refs` (unknown `rel` refused; target must exist locally — both doors), **and the
  markdown-profile check at both doors** (ADR-0041 decisions 5 and 10: no raw HTML, no load-bearing external
  URL) — safety-critical, in 3a. In this text-only subset any media anchor is refused for want of a manifest
  entry (`media: []`, decision 3), and event anchors are not valid in `clinical.note/1` (decision 9, second
  bullet).
- **The parts of a multi-writer draft** (§3.2): whether they ride into the committed body as spans or commit as
  separate parts is a wire decision for the ADR. Either way it bears on ADR-0041 decision 1's bet that sub-note
  span-level provenance is not needed — the ADR names that falsifier as now in play.
- **Times — both are captured, because they attest different things** (maintainer; ADR-0003): the writer
  **may state the time** the note describes (`t_effective` — the author attests when it happened, as a nurse
  heads a paper entry "03:00"), and `t_recorded` is the commit's HLC (the record attests when it was recorded).
  Both are true at once; a clash is flagged, never auto-resolved. *My call:* when the writer states no time,
  `t_effective` defaults to the draft's **first keystroke** as the node observed it (an observed time, not a
  clinical claim the node invents). The body also carries the draft's **typed-from / typed-until** times so the
  parts view still shows them once the draft row is gone. A directly signed note and an auto-committed one use
  the same rule; a time the writer stated in the draft survives the auto-commit.
- **`note.added` is NOT retired in this slice** (*my call*, builder check): ADR-0041 decision 1 says the
  migration registering `clinical.note.asserted` removes the walking-skeleton `note.added` row, but that event is
  the general test fixture in ~50 files and drives `note_count`; retiring it in 3a would double 3a. The new ADR
  records the deferral and an issue tracks the retirement (no production fleet has ever admitted one, so nothing
  in the field depends on either order).
- **Vouching for an earlier unsigned part is an overlay the floor verifies** (a human signature over the
  original's content address), never a mutation and never an unsigned claim. Whether that is a new
  note-attestation event type or reuse of an existing attestation shape (`sign_attestation`, db/034's medication
  attestation) is the builder's call — **it is a wire decision** and goes in the ADR (§9). The same overlay
  carries the role: `attested` for an unsigned part, `co-signed` for a part someone else signed (§3.4).
- New event types are pinned-count registrations (twin registry + `db/tests/034` + both projection counts —
  repo memory "pinned-count guards").

### 4.4 Already true, do not rebuild
Per-act signing, key hold, the signer strip (slice 1); enrollers, unconfirmed keys, `rotate-key` (slice 2); the
window holds the node's key (`--key`) in the interim (`cairn-gui/cairn-gui-tauri/src/main.rs`, "The NODE's
signing key").

**A placeholder exists and is not the note stream:** `cairn-gui-tab-note`, `NoteRef` and
`ClinicalData::note_refs` (`cairn-gui-data/src/port.rs`) are mock-only (the one `impl ClinicalData` is the mock),
used by `cairn-gui-shell`, not the Tauri window. Reusing or replacing them is the builder's call; state which.

**Not true yet — the builder check found it:** the terminal lock is shown **once at launch** and never comes back
(`signer.js`: "it does not re-lock when idle"; the Lock button only forgets the held key). §3.3 depends on it.

## 5. Build order — *my call*, one PR each

| PR | Contents |
|---|---|
| **3a** | The note stream: `clinical.note.asserted` text-only (floor, twin, projection, born-sealed), "New note" → type → Sign in the window, notes listed on the chart. The ADR (§9). |
| **3b** | Who's here (hot key, picker, loud bar, bookmark; switching forgets the held key); the idle screensaver (re-lock, clears who's here); the durable draft store (revisions, append-only writers, per-writer parts); the last-keystroke clock + the atomic commit door + the commit runner + the console knob; discard; `Asserted` + its display, with the `?` on claimed names. |
| **3c** | Addendum (vouches its own writer's parts); "Sign off this note" (co-signs / attests every part); the compound note + "N parts"; the pending-sign-offs menu; the unconfirmed / #918 markers; the `student` role label and "not co-signed". |

3a alone is a usable milestone step (write and sign a note). 3b is the draft half the maintainer asked for.

## 6. Paper-parity benchmark (§1.2) — for the plan to carry per PR

- **Paper counterpart:** the nursing/medical progress sheet in the bedside folder — pick it up, write, sign;
  a colleague finishes below in their own hand and signs; a student writes and the doctor countersigns.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Reading a note: 0 → 0 → 0 (no name asked).
  - Writing and signing a note: 3 (pick up the sheet, write, sign) → 3 (pick who's here once per stint, type,
    sign — the pick replaces typing the staff ID, which is then prefilled) → 3.
  - Finishing a note that went in unsigned: 3 (find the page, write, sign) → 3 (open from the menu, type,
    sign) → 3.
  - Countersigning a student's note: 2 (read, sign) → 2 (open, "Sign off this note" + passphrase) → 2.
  - Discarding one's own draft: 1 (tear it up) → 1 (signed discard) → 1. A draft several people wrote needs
    each writer, as a page bearing a colleague's ink would.
  - Resuming work after walking away: 2 (an open record is put away, not left lying in a public space — fetch
    it, find the page) → 2 (the shared-terminal unlock + the who's-here pick, after which the bookmark reopens
    the chart at the tab) → 2 (maintainer). The window already knows where the record is and where it was open,
    so resuming is mostly quicker than paper.
- **Time + cognitive load:** budgets and their measurement go in each PR's plan section (CLAUDE.md rule 7);
  signing stays within RUNBOOK §16's ≤ 3 s; the first runnable surface owes the human RUNBOOK timing.

## 7. Risks named

- **Forgetting to switch:** Nurse B types under Nurse A's name; the note names `?Nurse A`. Guards: the loud bar;
  the lock clears the claim; such a note only ever goes in *asserted*, shown with the `?`; signing needs B's own
  passphrase; and if A later signs an addendum, the strip shows her the `?`-named parts it would cover (§3.4).
- **No running window for days:** drafts wait (shown overdue) and are not on peer nodes until committed.
- **The commit runner is the window** — when slice 5 makes terminals nodes, or #912 gives the node a
  long-running process, the runner should move there.

## 8. What is left to the builder

Debounce interval; hot key; picker and bar layout; bookmark storage; the commit tick's period; how drafts are
sealed at rest and whether they are in backups (state it); the vouching-later wire shape (in the ADR); table and
door names; how the note list sits beside the medication list on the chart.

## 9. ADR owed

One ADR (in PR 3a, extended in 3b/3c or split as the builder sees fit) recording: the first note stream as
built (ADR-0041's text-only subset, and the deferred `note.added` retirement); the draft store, its
last-keystroke quarantine rule, its commit runner and its at-rest posture; the discard rule (author-signed, no
trace); the `Asserted` grade and the `?` on a claimed name; the two vouching acts (an addendum vouches its own
writer's parts; "sign off this note" co-signs or attests every part) and their overlay shape; the per-writer
parts of a draft and how they commit (§4.3, ADR-0041's falsifier); the note's times; the who's-here claim (a
claim, never authentication). Because ADRs are immutable, the deferred `note.added` retirement and the new
typed-from / typed-until body fields are recorded as a partial supersession of ADR-0041 decisions 1 and 2, not
an edit.

## 10. Review findings left open (PR #930 review, 2026-11-03)

Decide these before or in the plan:

- **When a continuation is signed before its original commits.** An `addendum-to` ref must name an event that
  already exists (ADR-0041 decision 8), and a draft is not one yet. Does signing the continuation first commit
  the original as it stands (asserted, `?`-named — the page taken out of the folder), or wait for it?
- **A discard leaves no trace even if someone acted on the draft** (a doctor orders levels after reading it).
  Keep an audit row (who discarded which draft on which chart, when) outside the clinical record?
- **3a ships a note editor with no draft store**, so a crash loses an unsigned note until 3b lands — against
  this slice's own anti-goal. Keep 3a off any pilot, or pull a minimal autosave into 3a?
- Smaller:
  - Show the "return to my last screen" bookmark as an offer, not an auto-open (a wrong-chart risk, ADR-0008).
  - A draft with an unknown writer can never be discarded, even one typed into the wrong chart — let a signer
    claim it by signing, then discard?
  - Record which terminal an unattributed commit came from.
  - The menu and visible drafts will need visibility gating once sealed sensitive episodes land (§5.9).
  - 3a's note list must show the existing grades, so a peer's asserted note never looks signed before 3b.
  - "Unknown is allowed" is a deployment policy setting (identity §5.11), default on.
