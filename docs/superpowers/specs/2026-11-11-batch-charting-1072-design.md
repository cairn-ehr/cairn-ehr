# GUI milestone — charting a list of medications under one signature (design, #1072)

- **Date:** 2026-11-11
- **Status:** brainstormed with the maintainer 2026-11-11 (the rulings in #1072's comments, then this conversation);
  builder-checked 2026-11-11; the maintainer answered its three questions the same day (§9); approved by the maintainer 2026-11-11
- **Milestone step:** "chart a medication" (HANDOVER ⇒ NEXT), for a patient with many. Slice 1a (PR #1066,
  [ADR-0099](../../spec/decisions/0099-charting-a-medication.md)) charts one line per Sign; that costs one signature
  per line where paper costs one for the list. **[maintainer]** ruled that an `M > N` defect (house rule 7).
- **Builds on:** ADR-0099 (the one line, the check at charting, the re-check at Sign, `warning_override`),
  ADR-0098 (the allergy check), ADR-0087/0088 (the note draft store and per-writer parts — the pattern the medication
  draft follows), ADR-0095 (the person menu), ADR-0089's `note_policy` (a console-only site setting), ADR-0056
  (a node admits an event type it cannot interpret and does not project it), ADR-0021 (soft policy in the window).
- **Comes before:** charting slice 1b (#1053/#1054's de-label and group comment), which must then fit the list.
- **Closes, when built:** #1072.

Text marked **[maintainer]** was decided by the maintainer. Text marked **[my call]** is the designer's, open to
correction. Unmarked text restates a standing project rule or a fact about the code.

## 1. Why

**The real case [maintainer, 2026-11-10 in ED]:** an elderly patient brings a handwritten list, an old GP letter,
and his daughter has photos of the packets on his bedside table. The three disagree. The clinician reconciles
them with the patient and daughter, is called away several times with the list half written, and in practice
worked it out **on paper first and then typed it in** — the duplicated effort this design removes. Late at night
the GP, prescriber and pharmacy cannot be reached; the next day someone (most often a junior ward doctor, sometimes
the ED doctor, the day team or a pharmacist) finds out which version was right, and **quite often it is the version
first set aside**.

Paper counterpart: the admission medication history — lines written under each other, one signature at the
bottom, the scratch sheet beside it holding the crossed-out alternatives.

## 2. Goals, non-goals, anti-goals

**Goals**
1. One Sign charts every ticked line of the list: one key release and one review, each line its own transaction
   (ADR-0060).
2. The unsigned list is the scratch sheet: durable, per writer, back exactly as left after any interruption.
3. Conflicting versions of a drug are kept, in the record, and one is restorable later without retyping.
4. A line with a warning can never go in on a box nobody ticked.

**Non-goals (this design)**
- Stopping a medication inside the list. Stopping stays its own act, as today **[my call; maintainer did not object]**.
- Attaching the packet photos to a line (attachments exist, ADR-0013; their own slice).
- Interaction, drug–disease, renal/hepatic or safe-dose checks. Only the allergy check exists; the mechanism must
  admit the others without change (§3.5).
- A co-sign on a junior's charting (supervision exists for notes, ADR-0094; its own slice if wanted).
- Drafts crossing to another node (slice 5, #963).

**Anti-goals** — failure even if it works:
- A warned line signed because a box was pre-ticked, or ticked before the warning existed.
- A set-aside version shown anywhere as a **current** medication — including on an older node.
- An unsigned list that goes into the record by itself.
- A screen cluttered with reconciliation modes and options (the maintainer's worry): every addition below is one
  control for the whole list, or appears only when opened.

## 3. The design, as decided

### 3.1 The list is a durable per-writer draft

- **[maintainer]** Unsigned lines are kept **durably** and come back **exactly as they were** — text, picks,
  ticks, reasons, set-aside marks — after a crash, a restart, the terminal lock, or a who's-here switch.
- **[maintainer]** **Per writer:** the next person on the terminal never sees or signs them. The writer is the person
  named in who's here when the line is typed, as for note parts (ADR-0088).
- **[maintainer]** **Never committed by a timer.** Unlike a note draft (ADR-0089), an untouched list stays a draft
  until its writer signs or removes its lines. The safe default is inaction: a medication must be actively charted
  and signed before anyone may be tempted to give it.
- **[maintainer]** The person menu's **"Waiting for you"** (ADR-0095) counts unsigned medication lines beside
  unsigned notes, so a writer who forgot Sign is told — the guard against "I thought it went in".
- **[my call]** It saves as typed, on the note editor's one-second autosave pattern, in its own draft store on the
  node (definer doors for `cairn_window` only, as db/065). One list per (writer, chart). It is bound to the terminal
  where it was started and **"Continue here"** moves it, exactly as a note draft (ADR-0088, #958).
- **[my call]** In no backup and no export, as note drafts (ADR-0087's ruling).
- **[my call]** When the list comes back, every line is re-checked before it is shown: an allergy may have been
  entered meanwhile.
- **[maintainer, §9 Q3]** Removing an unsigned line leaves no record — a plain delete. *"We only need to audit what
  has potential clinical consequences"*: a note draft can go in by itself (so its discard is audited, ADR-0090); a
  medication draft cannot.

### 3.2 Ticks: the default tick and the hand tick

- **[maintainer]** A line gets the **default tick** only when it is **complete** — at least **drug, formulation,
  dose and dosing instructions** — **and has been through every check that exists** at that moment, with no
  warning. Future checks (safe dose, recommended timing) need all four parts, which is why the bar is that high.
- **[my call]** "Complete" is judged on the line's *reading*, not on text being present: the drug is picked (coded),
  the formulation known, the dose read with certainty (ADR-0099 decision 11's `read_directions`), the frequency read.
  "Checked" means the check ran and answered — a "not checked" line is not checked. The default tick is given when
  the line is finished (Enter, or leaving the line), never while it is being typed.
- **[maintainer, §9 Q1]** **No default tick while the chart's allergies are "not yet asked"** (ADR-0096). A chart with
  no allergy records checks clean, but clean against nothing is not clean. The default tick needs an allergy list or a
  "No known allergies" statement. Charting is never blocked by this: the unconscious patient with no ID and no
  records still gets medication, by hand ticks, and the allergy status stays "not yet asked".
- **[maintainer]** **A hand tick is possible at any stage**, complete or not: the prescriber takes responsibility.
  There is no default convenience for it — it must be actively ticked, then signed. This keeps principle 4: a
  dose nobody knows is charted as unknown, never invented.
- **[maintainer]** A line **with a warning** is **unticked** and stays visibly unsigned until someone acts — tick it
  (with a reason where a reason is required, §3.5) or remove it.
- **[my call]** Editing a line removes a hand tick (the responsibility covered the old text) and re-derives the
  default tick from the edited line.
- **[maintainer]** If a **new warning arrives** between the tick and the Sign, the tick is removed **for the lines
  the warning affects**; the rest stay ticked. **[my call]** This applies to *any* tick given before that warning
  existed — default or hand, and a hand tick whose reason answered a different warning. A prescriber's
  responsibility covers what they saw.

### 3.3 Sign

- **[maintainer]** One Sign signs every ticked line, one event per line, under **one** signature act (one key release,
  one passphrase where the terminal asks). Precedent: the note Sign already signs one event per part in one
  transaction (ADR-0088); the medication sign-off attests many threads in one press.
- **[my call, maintainer agreed]** Sign re-runs every check on a fresh read before the key is released (ADR-0099
  decision 17, per line). **If any ticked line now carries a warning it did not carry when it was ticked, nothing is
  signed**: those lines lose their tick and show the warning, the window says so in one line naming how many, and a
  second press signs the rest. Nothing has been written at that point, so this is no rollback (ADR-0060 decision 5:
  the picture on screen moved). The same holds for a changed chart set.
- **[my call]** A check that **cannot run** at Sign: a *default*-ticked line loses its tick (its tick was conditioned
  on "checked") and the Sign is refused as above; a *hand*-ticked line proceeds "not checked", except one carrying a
  reason, which is refused `CHECK_DID_NOT_RUN` as in ADR-0099.
- **Each line commits in its own transaction (ADR-0060 decision 7)** — what makes the Sign one human act is the
  single key release and the single review, never a shared transaction (the medication sign-off had exactly that
  defect once, `signoff.rs:10-12`). A line's own set-aside versions (§3.4) go in with it **[my call]**: one clinical
  decision.
- **A per-line defect never refuses the others (ADR-0060 decision 1)** **[builder check]**: a line failing an ADR-0099
  decision-17 per-line refusal (press, pick, blank part) or failing to commit stays unsigned in the list with its
  reason, and the rest go in. The window reports per line: "Charted 11 of 12; 1 not charted — {reason}", and an
  unknown commit outcome for a line is worded as in ADR-0099.
- Signed lines leave the draft; unticked lines stay in it.

### 3.4 Sources and versions

- **[maintainer, approved suggestion]** **"From:" sticks at the top of the list.** Set once ("From: GP letter"),
  every line typed after takes it until it is changed ("From: packet photos"). One control for the list, nothing per
  line; each line shows its source and can be changed. It maps to the existing `info_source` vocabulary
  (`patient-reported` | `clinician-observed` | `external-record` | `unknown`) plus a short free-text naming of the
  actual document (§4). **[my call, builder check]** "From:" overrides the press's `info_source` default (ADR-0099
  decision 14): a GP letter is `external-record` on a Start line too. With "From:" unset the press's default stands.
- **[maintainer, approved suggestion]** **Conflicting versions are just two lines.** Two lines for the same
  drug are marked **"2 versions"** and neither gets the default tick, even when both are complete.
  **[my call]** "Same drug" = the same coded moiety (a single-drug pick). Uncoded lines and combinations are never
  grouped automatically.
- **[my call, builder check]** A **current, already-signed line** of the same drug joins the group too: a list line
  duplicating a drug already charted gets no default tick and shows "already on the list: {line}". Set aside applies
  only to unsigned lines; changing a signed one is the existing stop act or §3.4's "Use this version instead".
- **[maintainer, §9 Q2]** **Two lines of one drug may both be current** (regular + PRN oxycodone; metoprolol 50 mg mane
  + 25 mg nocte). So ticking one version never sets the other aside. The clinician ticks both, or presses **"Set
  aside"** on the one not believed — one deliberate press, as crossing out on paper. An unticked, not-set-aside version
  stays in the draft.
- **[maintainer, approved suggestion]** An optional one-line **"why"** on a line ("packet 25 mg, confirmed by
  daughter; GP letter 50 mg is old"), signed into the event, shown to the next reader. Appears only when opened.
- **[maintainer]** **Every version is kept** — it often turns out later that the set-aside one was right.
- **[my call, maintainer agreed]** **Kept in the signed record, not the private draft**, because the next-day
  catch-up is usually someone else (a junior ward doctor, the day team, a pharmacist). At Sign, each set-aside
  version is signed too, as what it truly is: *"{source} reported {drug line}. Set aside, not current."* The
  signature vouches for what the source said, not that the patient takes it.
- **[my call]** On the medication list a set-aside version shows **collapsed under the current line** of the same
  drug ("1 other version: GP letter, metoprolol 50 mg bd"), with its source and "why".
- **[my call]** **"Use this version instead"** is one signed act: it stops the current line and charts the
  set-aside version as a new current line referencing it, in one transaction. The new line is checked like any
  new line; a warning is handled as in §3.2/§3.5 (it opens as an unticked list line rather than signing blind).
- **[my call]** Who may press it is role policy, not the window's: by default anyone who may chart, a pharmacist
  included where the hospital allows.
- **Set-aside versions never trigger the allergy check, never count toward the active list, never feed decision
  support** — they are not what the patient is believed to take.

### 3.5 Warnings and the threshold

- **[maintainer]** "Warning" is **general, not allergy-only**: the mechanism keys on "a finding at or above the
  warning threshold", never on allergy by name. The signed override is already general
  (`warning_override {reason, against: [{source, ref}]}`, ADR-0099).
- **[maintainer]** The threshold is a **policy setting changed only by an administrator**, never a per-user
  preference.
- **[my call]** It follows `note_policy`'s pattern: one console-only row on the node, a `cairn-node` verb to set it,
  read by the window at each check. Today's scale has three levels from ADR-0098 — **allergy hit**, **adverse-effect
  note**, **nothing** — and the default threshold makes only an allergy hit a warning. Later checks add their own
  findings at a level on the same scale; nothing below it changes.
- **[my call, builder check]** The threshold can only be **lowered** (to make adverse-effect notes warnings too), never
  raised past an allergy hit: no setting turns allergy warnings off. The Start line's reason gate (ADR-0099 decision
  17.5, keyed today on "a mark") follows the threshold; an adverse-effect note's `against` entry is
  `{source: "allergy", ref}` naming its allergy record, as a hit's is.
- **[maintainer, ADR-0099]** A *Start* line with a warning needs a typed reason before it can be ticked; *Already
  taking* does not. Both presses can sit in one list, each line carrying its own **[my call]**.

## 4. Constraints and facts for the builder

- **Wire is forever (principle 11, additive only).** Three additions, shapes the builder's, constraints here:
  1. **A set-aside version must be its own event type**, never a flag on `clinical.medication.asserted`: an older
     node ignores an unknown key and would project the version as a **current medication** — the anti-goal. A new
     type is admitted uninterpreted by an older node (ADR-0056) and shown nowhere as current. Born-sealed (ADR-0052),
     human-signed, its own twin, floor, projection and shred hook.
  2. **The source naming and the "why"** — **two** optional free-text keys on the assertion (and on the set-aside
     event), never one: a merged key would lose forever which text is which **[my call, builder check]**. An older
     node losing them loses display only; the twin carries them.
  3. **"Use this version instead"** — the new assertion references the set-aside event (an optional key). Safe on
     an older node, which shows a current line, correctly.
- The pinned-count guards: a new event type +1s both `twin_registry.rs` and `db/tests/034`; a projection also both
  `cairn_projection_apply` counts (memory: four places, two totals).
- `seal_sign_submit` opens a transaction only when `attest` is `Some` (`sealed_submit.rs:373-399`); the window passes
  `None`, so per-line commits need no new transaction variant. What batching needs is the signer released once and
  used for N lines (as the sign-off's `attest_thread_in_tx` takes an unsealed key by reference), and a line + its
  set-aside versions in one transaction. Cross-crate call sites (`cairn-sync/tests/clinical_pull.rs`) follow any
  signature change.
- The window keeps ADR-0099's rule that the webview's answer is never trusted: ticks, completeness and warnings are
  re-derived in Rust at Sign.
- The person menu's reads are budgeted (#1021): its new group must be read by keys like the others.
- `SCHEMA_GENERATION` bumps; one new `db/0NN` for the draft store, the set-aside event and the policy row.

## 5. Paper-parity benchmark (§1.2) — for the plan to carry

- **Paper counterpart:** the admission medication history, a scratch sheet beside it.
- **Steps:** a 12-line home list: paper N = 12 lines written + 1 signature → architecture M = 1 human signature act
  (≥ 12 events) → UI target K = 12 lines + 1 Sign, plus 1 more Sign only when a warning arrived meanwhile. A
  conflicting version: paper = one more line crossed out → K = one more line, its tick auto-set-aside. Next-day
  restore: paper = rewrite the line → K = 1 press + Sign.
- **Time + cognitive load:** owed by the slice exposing it; RUNBOOK times a 10-line list as one Sign vs ten (#1072's
  ask), and an interrupted list's resume.

## 6. Risks named

1. **The default tick is a convenience that can be trusted too far.** A tired clinician signs eleven pre-ticked lines
   unread. Mitigation: the bar is completeness *and* every check; the parsed reading of each line stays visible.
2. **A set-aside version mistaken for current** — on screen (collapsed, labelled "not current") and on the wire (own
   event type).
3. **A forgotten list** — counted in "Waiting for you"; it never goes in by itself, so the risk is omission, which the
   count addresses.
4. **Versions grouped wrongly** — only exact moiety matches group; a missed grouping leaves two lines each needing
   its own decision, the safe failure.

## 7. Left to the builder

Wire key names and shapes (within §4), the draft tables, the twin wording, the window's layout of the tick column,
"From:" and "2 versions", the console verb's name, the slice split (likely: draft store + one-Sign list; versions +
set-aside event; the threshold row).

## 8. ADR owed

ADR-0100: the list as a durable, never-auto-committed per-writer draft, deleted lines unaudited; the default-tick bar
(complete, checked, allergies asked); untick on a later warning; one Sign over ticked lines, per-line transactions
under ADR-0060; set-aside versions as their own event type; the warning threshold as administrator policy, lowerable
only.

## 9. Answered by the maintainer (from the builder check, 2026-11-11)

1. **Allergies not yet asked** → no default ticks at all; charting proceeds by hand ticks (the unconscious patient
   without ID or records still gets medication), and the status stays "not yet asked". §3.2.
2. **Two current lines of one drug** → ticking one never sets the other aside; "Set aside" is its own press. §3.4.
3. **Removing an unsigned line** → a plain delete: only what has potential clinical consequences is audited. §3.1.

Also corrected from the builder check, against the project's own rule: the design first had one transaction for the
whole Sign, contradicting ADR-0060 decision 7; it now commits per line (§3.3).
