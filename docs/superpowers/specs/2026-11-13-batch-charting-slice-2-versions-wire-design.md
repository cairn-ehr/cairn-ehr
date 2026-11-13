# Batch charting slice 2 — versions, set aside, "From:" and the "why" (wire design, #1072)

- **Date:** 2026-11-13
- **Status:** brainstormed with the maintainer 2026-11-13 (the playback approved: "nothing wrong — proceed");
  builder-checked and its two questions answered the same day (§9); **approved by the maintainer 2026-11-13**
- **Extends:** [`2026-11-11-batch-charting-1072-design.md`](2026-11-11-batch-charting-1072-design.md) §3.4 and §4
  (approved 2026-11-11). Everything there stands; this document fixes the wire shapes that design left to the
  builder and adds one act (§3.4 below) the parent did not have.
- **Builds on:** ADR-0099 (the charted line and its five keys), ADR-0100 (the per-writer list, the tick rules, one
  Sign with per-line transactions), ADR-0056 (an older node admits an unknown type uninterpreted), ADR-0052
  (born-sealed), ADR-0053 (per-write human authorship), ADR-0060 (per-line transactions).
- **Comes before:** batch-charting slice 3 (the warning threshold as administrator policy) — unchanged.

Text marked **[maintainer]** was decided by the maintainer. **[my call]** is the designer's, open to correction.
Unmarked text restates the parent design or a standing project rule.

## 1. Why — the next morning

The parent design's case: at night the clinician reconciles three disagreeing sources and sets one version of a drug
aside; next day someone finds out which was right, and quite often it is the one set aside.

**[maintainer, 2026-11-13]** The record does not settle the question. Corroboration happens **outside Cairn** —
ringing the carer, the pharmacist, the GP. So the record's job is to tell whoever reviews it **who to ring and what
each source said**, and then to let them record **what the ringing found**. "Whoever reviews it" is **any reviewing
clinician** — the ward junior, the day team, the ED doctor, a pharmacist — not one role.

**[maintainer]** A version exists only **beside a current line of the same drug**. "Listed but not taken" (the GP
letter lists atorvastatin; the patient stopped it months ago) is **not** a medication line: the clinician writes it in
the admission note, and if the drug is already charted, stops it with the existing stop act.

## 2. Goals, non-goals, anti-goals

**Goals**
1. A set-aside version carries enough to act on next morning: what the source said, which source exactly, who set
   it aside and when, and why.
2. A reviewing clinician can settle a version either way in one signed act: **restore it** ("Use this version
   instead") or **mark it checked, not current**. Each act records what was checked (its own "From:" and "why").
3. Nothing is erased: the replaced line and a checked version stay readable.

**Non-goals**
- A version with no companion line (§1: that is a note, not a version).
- Grouping uncoded lines or combinations as versions (parent §3.4: exact coded moiety only).
- A version on a linked chart's line (the doubted-link and linked-set rules of R1/R1b apply unchanged; **[my call]**
  "Set aside" is offered only on lines of the opened chart).
- Attachments (packet photos) — their own slice.

**Anti-goals** — failure even if it works:
- A set-aside or checked version counted, shown or checked as a **current** medication anywhere — including on an
  older node, which must see an unknown type and show nothing.
- A version whose companion did not go in, sitting in the record pointing at nothing.
- A merged "source + why" text whose two meanings can never be told apart again (parent §4.2).

## 3. The design, as decided

### 3.1 "From:" naming and the "why" — two keys on a charted line

- Two optional free-text keys on `clinical.medication.asserted` **[my call: names]**: `source_detail` (the naming
  of the actual document or person — "GP letter, dated Aug 2025", "GP surgery, by phone") and `why` (the one-line
  reason — "packet 25 mg, confirmed by daughter; GP letter is old"). Never one merged key (parent §4.2). Both
  inserted only when present, never as null (principle 11).
- `info_source` stays required and keeps its vocabulary. **"From:" is built in this slice** (slice 1 had none,
  ADR-0100): set once at the top of the list, every line typed after takes it until changed, each line can change
  its own (parent §3.4, including "From:" overriding the press's default; unset, the press's default stands).
- **[maintainer, 2026-11-13]** "From:" is a **short preset list, each item tied to one `info_source` category**, plus
  a free-text box naming the actual document (→ `source_detail`):

  | Preset | `info_source` |
  |---|---|
  | Patient | `patient-reported` |
  | Carer / family | `patient-reported` |
  | GP letter | `external-record` |
  | Pharmacy | `external-record` |
  | Packets / Webster pack seen | `clinician-observed` |
  | Other | `unknown` **[my call]**: the category is never guessed from the free text |

  The preset's own label is not a wire key **[my call]**: the category goes to `info_source`, and the preset label
  prefixes `source_detail` when the free text is empty ("GP letter") so the twin still names it.
- Both are in the twin. An older node loses only their display.

### 3.2 The set-aside version — its own event type

- **Type** **[my call: name]**: `clinical.medication-set-aside.asserted`. Born-sealed, human-signed, its own twin,
  floor, projection and shred hook (parent §4.1). Its own pinned-count registrations (the four places, two totals).
- **Payload** **[my call: shape]**:
  - `version_id` — an immortal id minted for the version (UUIDv7), as `medication_id` is for a line, so the two later
    acts can name it.
  - `alternative_to` — **mandatory**: the `medication_id` of the companion line it lost to **[maintainer: a version
    exists only beside a companion]**. An explicit reference, not "same moiety", so it collapses under the right line
    even when two current lines share a drug (regular + PRN oxycodone).
  - The drug fields **exactly as a charted line spells them** (`substance` with `term`, `coding`, `formulation`,
    `strength`, `components`; `dose`; `sig`; `route`; `started` / precision), so one reader parses both.
  - `info_source` (required) + optional `source_detail` + optional `why`.
  - **Absent by construction:** `safety` (no §5.9 claim), `warning_override`, `charted_as`. A version is never checked
    against allergies, never counted on the active list, never feeds decision support (parent §3.4). The floor
    refuses the three keys if present.
- **Twin** **[my call: wording]**: *"{source} reported {drug line}. Set aside, not current — {why}"*. The signature
  vouches for what the source said, not that the patient takes it (parent §3.4).
- **Floor** **[my call, after the builder check]**: the medication stream keeps orphans and flags, never pens
  (`db/031`: an orphan cessation is admitted; `cairn_guard_medication_patient` raises at the local door, flags on
  remote apply). The version follows it: at the **authoring door** `alternative_to` must name a medication thread
  already projected for **the same chart** (the window always has one: the companion commits first in the same
  transaction, or is already charted); at the **remote door** a version whose companion is absent is admitted and
  **shown nowhere** until the companion arrives (it is never current, so an orphan costs only display), and a
  companion on another chart is flagged as the cessation's is. `alternative_to` is **not** a chart claim (no fifth
  arm in `cairn_medication_thread_patient`): the version's own envelope names its chart.

### 3.3 "Use this version instead" — a restore

- **[maintainer, 2026-11-13] The reviewer may edit it.** The morning call often finds "right drug, but the dose has
  changed since". So the press **opens a list line pre-filled from the version** (the slice-1b de-label pre-fill
  pattern), carrying a pending `restores`. The reviewer edits it if needed, sets "From:" and "why" (what the
  corroboration found — "GP surgery, by phone"; "dispensing history confirms 50 mg bd since March"
  **[maintainer: the act records what was checked]**), and signs it like any list line: checked, ticked by the
  ordinary rules, re-checked at Sign. The floor does **not** compare its drug fields with the version's.
- **At Sign, one transaction, two events** **[parent §3.4, shapes my call]**:
  1. the existing `clinical.medication-cessation.asserted` on the companion line, `reason` set by the window
     ("replaced by the {source} version") — **only if the companion is still current** (a stopped companion needs no
     second stop; `medication_cessation` keeps one row per line);
  2. a new `clinical.medication.asserted` — a new thread — with a new optional key **`restores`** = the version's
     `version_id`, and its own `info_source` / `source_detail` / `why`.
- **The floor enforces the pairing** **[my call, after the builder check]**: at the authoring door a deferred check
  holds a `restores` line to (a) naming a set-aside version of the same chart and (b) that version's companion being
  **not current at commit** — stopped in the same transaction or before. A raw-SQL client cannot write a restore that
  leaves two current lines. The remote door keeps structure only (orphan-tolerant, as §3.2).
- **`charted_as`** **[my call]**: the restored line inherits the companion's (`pre-existing` or `started-here`), so
  ADR-0099 decision 17.5's reason gate and the default source behave as the companion's did; the reviewer can change
  it on the line.
- **A companion no longer current** (stopped for another reason, or already replaced by another version's restore)
  **[my call]**: its versions stay readable under the stopped line and **no longer count as unresolved**; restore and
  "checked" stay offered; a restore then emits no cessation.
- An older node sees a stop and a new current line — correct (parent §4.3).
- **Display** **[my call]**: the replaced line shows collapsed under the new one as "replaced — {who}, {when}", derived
  from `restores` → version → `alternative_to`; no extra event. The restored version itself shows as "restored".
- Who may press it: role policy, by default anyone who may chart (parent §3.4).

### 3.4 "Checked: not current" — new, from this brainstorm

- **[maintainer]** Any reviewing clinician may record that a version was checked and is not current, so it stops
  looking like an open question.
- **Type** **[my call: name, shape]**: `clinical.medication-set-aside-checked.asserted`, born-sealed, human-signed,
  own twin / floor / projection / shred hook. Payload: `version_id` (mandatory, a set-aside version of the same
  chart at the authoring door), `info_source` (required), optional `source_detail`, optional `why`. No verdict field:
  the event type is the verdict; the opposite outcome is the restore (§3.3).
- **Display** **[my call]**: the version stays readable, collapsed under its line, reading "checked by {who},
  {when}: not current — {why}"; it leaves any count of unresolved versions.
- **It locks nothing** **[my call]**: a checked version can still be restored later; the later act wins, as every
  overlay. A second "checked" on the same version is admitted and shown (two reviewers); it changes nothing.

### 3.5 Set aside in the list, and at Sign

- **[maintainer]** "Set aside" is offered only on an unsigned list line that has a **companion**: another list line,
  or a current charted line, of the same coded moiety (the parent's "2 versions" group). No companion → no button;
  the clinician removes the line and writes the note.
- **[my call]** Pressing it marks the line set-aside in the draft. db/078's row gains: the set-aside mark and the
  chosen companion; per-line `info_source` / `source_detail` / `why`; a pending `restores`. The list-level "From:"
  is stored per (writer, chart) so it survives a crash (parent §3.1). Ticks and the default tick no longer apply to
  a set-aside line. With two or more candidate companions (both oxycodone
  lines), the press asks which line it is a version of — one pick, no dialog; a single candidate is taken.
- **At Sign** **[parent §3.3 "a line's own set-aside versions go in with it"]**: a set-aside version whose companion is
  a **ticked list line** goes in **in that line's transaction**, after it; one whose companion is **already charted**
  goes in in its own transaction. A version whose companion is unticked, or fails to commit, **stays in the list**,
  reported per line ("1 version kept: its line was not charted"). Never a version pointing at nothing (anti-goal).
- A set-aside line is not ticked and not counted as "charted" in the outcome line; the outcome names versions
  separately ("Charted 11; 2 versions kept as set aside").

## 4. Constraints and facts for the builder

- **Wire is forever (principle 11).** Additions only: two keys + `restores` on `clinical.medication.asserted`, two new
  event types. No existing key changes meaning.
- `restores` and `alternative_to` are ids inside a sealed body; projections read them after unseal
  (`cairn_clear_payload`, db/031), as the cessation's `medication_id`.
- `sealed_submit.rs` already has a two-line, one-transaction path (the restore's stop + line, a line + its version).
- The four pinned-count guards per new type (memory: `twin_registry.rs` + `db/tests/034`, both
  `cairn_projection_apply` counts). Two new types here.
- Cross-crate call sites (`cairn-sync/tests/clinical_pull.rs`) follow any `seal_sign_submit` / orchestrator
  signature change.
- The webview's answer is never trusted: companion, set-aside mark and ticks are re-derived in Rust at Sign (ADR-0099,
  ADR-0100).
- `SCHEMA_GENERATION` bumps; one new `db/0NN` for the two types' floor, projections and shred hooks, plus db/078's
  draft-row columns (in place or in the new file — db/078's replay rules decide).

## 5. Paper-parity benchmark (§1.2) — for the plan to carry

- **Paper counterpart:** the admission medication history with crossed-out alternatives; next day, the reviewer
  writes "confirmed with GP: 50 mg bd" and rewrites or initials the line.
- **Steps:**
  - Set aside a version: paper N = 1 line written + 1 cross-out → M = 1 press (inside the list's one Sign) → K = 1
    press.
  - Restore next day: paper N = 1 rewrite + 1 note of the call → M = 1 signed act → K = 1 press + "From:" + "why" +
    Sign (+ the edits, when the dose changed — as paper's rewrite would carry them).
  - Checked, not current: paper N = 1 initial/annotation → M = 1 signed act → K = 1 press + "why" + Sign.
- **Time + cognitive load:** owed by the slice exposing the window surface; RUNBOOK times a restore and a check from
  the collapsed version.

## 6. Risks named

1. **A version mistaken for current** — mitigated on the wire (own types, no `safety`, floor refuses current-line
   keys) and on screen (collapsed, "not current").
2. **A restore that signs a stop without its new line** — mitigated by one transaction for both events.
3. **A stale version nobody settles** — the "checked" act gives the reviewer the way to close it; unresolved versions
   are countable for a later menu group **[my call: no menu group in this slice]**.

## 7. Left to the builder

The twin wording beyond §3.2's sense, the window's layout of the collapsed version and its two buttons, the draft
columns, the slice split (likely: 2a wire + floor + projections + list "Set aside" and Sign; 2b restore + checked
acts and their display).

## 8. ADR owed

ADR-0102: the two keys; the set-aside version as its own type with a mandatory companion; "Use this version instead"
as a stop + a `restores` line in one transaction; "checked, not current" as its own act; a version only beside a
companion.

## 9. Builder check

Run 2026-11-13. **Corrected against the code:** the medication stream keeps orphans and flags, never pens (§3.2 now
follows it); "From:" did not exist yet and is built here (§3.1). **Answered by the maintainer:** the restore is
editable (§3.3); "From:" is a preset list tied to categories, packets seen = `clinician-observed` (§3.1). **Settled
as my calls:** an orphan version is admitted and shown nowhere; `alternative_to` is not a chart claim; the floor
enforces a restore's pairing with a deferred check; a stopped companion's versions stop counting as unresolved and a
restore of one emits no cessation; the restored line inherits the companion's `charted_as`; the draft columns (§3.5).
