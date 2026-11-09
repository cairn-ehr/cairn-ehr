# ADR-0097 — Allergy overlays and differing accounts

- **Status:** Accepted (supersedes [ADR-0096](0096-the-allergy-list.md) decision 5's reserved-empty `refs` and
  decision 6's "`refs` empty" authoring-door rule; amends ADR-0096 decisions 10 and 16 as decisions 10 and 20 below
  say; ends ADR-0096 decision 23's deferral of corrections, de-labelling, entered in error and the differing-accounts
  flag, its "not guaranteed" (b), and (c)'s missing unconfirmed-key marker; builds slice 2 of the allergy design's §5)
- **Date:** 2026-11-09
- **Spec version at acceptance:** 0.99
- **Issues:** draft PR [#1038](https://github.com/cairn-ehr/cairn-ehr/pull/1038) (allergy slice 2). Design:
  `docs/superpowers/specs/2026-11-08-allergy-stream-design.md` §3.5, §3.6, §3.8, §7 risk 4; plan:
  `docs/superpowers/plans/2026-11-09-allergy-slice-2-overlays-and-differing-accounts.md`. Builds
  [#1027](https://github.com/cairn-ehr/cairn-ehr/issues/1027) (decision 18). Touches
  [#905](https://github.com/cairn-ehr/cairn-ehr/issues/905) (not guaranteed (h)) and
  [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) / [#401](https://github.com/cairn-ehr/cairn-ehr/issues/401)
  (not guaranteed (l)). Filed by this slice: #TBD-1 (withdraw a de-label on an older version, (c)), #TBD-2 (strike a
  de-labelled line in one act, (d)), #TBD-3 (a historical line does not name who de-labelled it, (e)), #TBD-4 (no
  trace of an overlay with no effect, (f)), #TBD-5 (the byline markers, (g)), #TBD-6 (window polish, (i)), #TBD-7
  (test gaps, (k)), #TBD-8 (files over 500 lines).
- **Builds on:** [ADR-0096](0096-the-allergy-list.md) (the allergy list) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) (born sealed) ·
  [ADR-0053](0053-per-write-human-authorship.md) (the human signs, the node seals) ·
  [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) (strict submit, lenient apply) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (a newer peer never wedges sync) ·
  [ADR-0092](0092-the-vouch-and-the-sign-off.md) (a vouch is filed under its part's chart) ·
  [ADR-0093](0093-a-repudiated-signature-counts-as-unsigned.md) (a repudiated key takes no warning away; the mark
  is only where this node's registry holds the repudiation) ·
  [ADR-0094](0094-supervision-and-the-mandatory-co-sign.md) (an unconfirmed key's act counts)

## Context

Slice 1 (ADR-0096) put the allergy list on the patient header, but nothing recorded on it could be taken back: an
entry signed on the wrong chart, or an intolerance signed as an allergy, stayed on the strip for good (ADR-0096 (b)).
On paper the clinician strikes the line in the allergy box and writes the corrected one beside it, initialled;
writes "tolerated amoxil 03/22" against a struck penicillin line; or strikes a slip and initials it. Nothing is torn
out.

This slice builds those three acts as signed overlays, never edits (principle 1): **Correct**, **De-label**
("challenged: tolerated …", design §3.5's maintainer wording) and **Entered in error**. It also flags two accounts of
one allergen that disagree (design §3.6) and marks a byline whose key is unconfirmed or since repudiated (#1027,
design §3.8). The maintainer decided four things before the plan (M1–M4); the plan's designer made six calls
(D1–D6); the build made rulings recorded in the slice's ledger, two of which replace or refine a call: **R-4a**
replaces D1, and **R-4b** refines D4 and the plan review's R-P4. Each decision below says which it builds. The first
signed event of each new type freezes its shape (principle 11), so the wire is written down exactly.

## Decision

### The wire (`crates/cairn-event/src/allergy.rs`, `crates/cairn-event/src/allergy_overlay.rs`, frozen)

1. **A correction is no new type** (M2). It is an ordinary `clinical.allergy.asserted`, `schema_version`
   `clinical.allergy/1`, with ADR-0096's payload unchanged, whose `refs` is exactly
   `[{"rel": "correction-of", "event": "<uuid>"}]`, naming **the line it replaces**, not the first line of the
   chain. A fresh entry keeps `refs: []` (`AllergyEntry::corrects` is `None`). Its twin is
   `"Corrects an earlier entry: "` followed by slice 1's words with the ISO onset (`render_allergy_twin`, R17). This
   supersedes ADR-0096 decision 5's reserved-empty `refs`; `substance.coding` stays reserved (slice 3).
2. **De-label and entered in error are two new types, one per act** (M1, the db/042 precedent):
   - `clinical.allergy-challenge.asserted`, `schema_version` `clinical.allergy-challenge/1`, payload exactly
     `{target, tolerated: {term, coding}, date: {precision, value}, text}`. `target` is a uuid string, `coding` is
     written `null` (slice 3), and `date` has the shape of ADR-0096's `onset`.
   - `clinical.allergy-retraction.asserted`, `schema_version` `clinical.allergy-retraction/1`, payload exactly
     `{target, text}`.

   Both are registered `additive` with `targets_other_author = FALSE` (db/075 step 1), so ADR-0043's owner gate,
   which fires only on a suppressing type that targets another author, never applies: a nurse may strike a doctor's
   line, as on paper. Both carry no `t_effective` and `safety: None` (ADR-0096 decision 21 stands), and are signed by
   `cairn_node::allergy::sign::{sign_allergy_challenge, sign_allergy_retraction}` through `seal_sign_submit` with
   the contributor set `{human, authored}` + `{node, recorded}` (ADR-0053).
3. **Blank text is null.** In both builders a `text` that is empty or whitespace-only is written as JSON `null` in
   the payload and adds nothing to the twin, so a sealed body never carries a value its own twin calls absent
   (`allergy_overlay::present`). Non-blank text is kept verbatim in the payload and trimmed in the twin. The window
   trims every overlay text before it reaches the builder (`allergy_overlays.rs`'s `note`). The floor does not
   enforce this: a raw-SQL client's blank string is admitted (`db/tests/075_allergy_overlays_test.sql` section 1).
4. **The twins**, exact: a de-label reads `"De-labelled by challenge: allergy record {target} — tolerated {term}"`,
   then `" · {ISO date}"` when the date is known, then `" — {text}"` when there is text; a withdrawal reads
   `"Entered in error: withdraws allergy record {target}"`, then `" — {text}"`. Both are registered in
   `cairn_event_twin_check` with a hard non-empty-twin requirement (db/075 step 6).

### The floor (`db/075_allergy_overlays.sql`; db/074, db/037 and db/044 changed in place)

5. **Two tiers, the db/041 idiom** (ADR-0051), each rule with its own message.
   - **Structural, at both doors.** A de-label: the payload is an object; `target` is a uuid string
     (`cairn_allergy_target_problem`); `tolerated` is an object whose `term` is a string not blank by
     `cairn_twin_is_present` and whose `coding` key is present and null or a coding object `cairn_check_coding_object`
     accepts; `date` is an object with a string `precision` and a `value` that is null exactly when the precision is
     `not-known`, else a string; `text` is present, a string or null. A withdrawal: the payload is an object, `target`
     is a uuid string, `text` is present, a string or null. A correction's `refs` is an array (slice 1's rule).
   - **Closed vocabularies and references, at the authoring door only** (`cairn.remote_apply` is not `'on'`): the
     `schema_version`; the date's precision words (`day`, `month`, `year`, `not-known`); its shape, year `0000` and
     a date after the recording day judged against UTC+14 (`cairn_allergy_onset_problem`, which now returns its
     reason without a field name so the entry says `onset.value` and the de-label `date.value`); `coding` null;
     and what the target is and which chart it is on (decisions 6 and 7). At the remote door a peer may overlay a
     target this node has not received yet, and a newer peer's word must never wedge sync (ADR-0056).
6. **The correction ref, at the authoring door** (db/074 step 3, in place): `refs` is `[]` or exactly one object
   with exactly the keys `rel` and `event`, `rel` = `correction-of`, `event` a uuid string naming a
   `clinical.allergy.asserted` that `event_log` holds on this chart; else "refs must be [] or one {rel:
   correction-of, event}" or "correction-of target … is not an allergy entry on this chart". This supersedes ADR-0096
   decision 6's "`refs` empty" at the authoring door. **The remote door stays lenient** (D6): it checks only that
   `refs` is an array.
7. **What each overlay may name, at the authoring door.** A de-label names an allergy entry on this chart. A
   withdrawal names an entry, a "none known" statement or a de-label on this chart (M3: withdrawing a de-label puts
   the allergy back in force), and **never another withdrawal** (D3), with its own message: "a withdrawal is never
   itself withdrawn (record the entry again instead)". "On this chart" means an `event_log` row whose `patient_id`
   is the overlay's own (`cairn_allergy_target_type`), which is clear in the envelope even for a sealed target.
8. **An overlay is filed under its target's chart** (D2, the vouch precedent of ADR-0092). Because the authoring
   door refuses a target on another chart, a correction, de-label or withdrawal of a linked chart's line must be
   signed on that chart, and the window does so (decision 20): the event's `patient_id` is the target row's chart,
   and the list read again afterwards is the chart on screen's.
9. **Projections and the in-place changes.** Two tables, one row per overlay event keyed by `event_id`:
   `clinical_allergy_challenge` (`patient_id`, `target`, `tolerated_term`, `tolerated_coding`, `date_precision`,
   `date_value`, `text`, the HLC, `origin`, `content_address`) and `clinical_allergy_retraction` (`patient_id`,
   `target`, `text`, the HLC, `origin`, `content_address`), read by `cairn_agent`. Their appliers read the clear
   payload, project nothing without custody, skip a body whose `target` is not a uuid rather than raise (a raise
   would fail the late-custody heal, the db/071 idiom), filter no `schema_version` (ADR-0096 decision 8's reasoning),
   and are registered `heal_safe` at run order 20 with `ON CONFLICT DO NOTHING`. db/037's shred hook deletes both
   tables' rows by content address; db/044 gains the timing kinds `allergy-correct`, `allergy-challenge` and
   `allergy-retract` in place; both types stay off db/048's no-thread list. `SCHEMA_GENERATION` is 75.

### Reading and the fold (`cairn_node::allergy::{read, thread, view}`)

10. **The read** (`read::list_patient_allergies`) adds the two overlay projections to slice 1's, over the same member
    charts in the window's one snapshot, and every row of all four tables now carries its signer key's standing:
    `cairn_key_confirmation` folded to `KeyState` — `unconfirmed` or `doubtful` is **Unconfirmed**, `repudiated` is
    **Repudiated**, `confirmed`, `outside`, `unknown` or no answer is **Standing**, and any other word fails the read
    (reading an unknown word as standing would fail open). An entry's `corrects` is the `event` of the first element
    of `refs` whose `rel` is `correction-of` and whose `event` parses as a uuid (`corrects_of`); anything else in
    `refs` is ignored, never an error (D6), so the entry reads as uncorrected, which over-warns. `unreadable`
    (ADR-0096 decision 10) now counts events of all four allergy types with no projected row.
11. **A suspect overlay counts only when it adds a warning** (ruling R-4b, refining D4 and R-P4;
    `thread::overlay_counts`). An overlay is *suspect* when its signer key is repudiated, or when it names an entry,
    a "none known" statement or a de-label this node reads on another chart than its own (only a lenient remote door
    admits that). A suspect withdrawal of a de-label (the allergy comes back) or of a "none known" statement (the
    reassurance goes) counts; a suspect de-label, and a suspect withdrawal of anything else, count for nothing and are
    listed as "Not counted: de-label (tolerated {term}){by}" or "Not counted: entered in error{by}", followed by
    " — names a record on another chart" when that is why. This supersedes the plan's D4 sentence that a repudiated
    key's de-label and withdrawal count for nothing. An unconfirmed signer's overlay is not suspect: it counts, marked
    (the ADR-0094 precedent).
12. **Standing de-labels** are the counted de-labels no counted withdrawal names (M3). For each target the newest
    by (HLC, event id) is the one a line shows.
13. **Threads** (`thread::heads`). An entry and every correction of it, transitively, are one thread. A `corrects`
    link counts unless the correcting entry's key is repudiated (D4: it is an account of its own and supersedes
    nothing), it names itself, or it names no entry this node reads on the correcting entry's own chart (D2). The
    threads are a union-find whose root is the smaller event id, so a thread is identified by its smallest event id
    on every node. A thread's **heads** are the members no counted link names: two corrections of one line are two
    heads, two current accounts (M2), and neither is lost. A forged cycle leaves no such member, so then every
    member is a head (over-warning). Each head carries `earlier`, the versions it replaced, newest first.
14. **Status is per head, and an overlay covers what it names, never a later correction of it** (ruling R-4a,
    replacing the plan's D1 "overlays are thread-level"). A head is *withdrawn* if a counted withdrawal names it,
    else *historical* if a standing de-label names it, else *live*. An overlay naming an older version strikes that
    version only; the versions under a struck head go with it. So a withdrawal or de-label of A leaves a correction B
    of A live, whether B was signed concurrently across a partition or afterwards, and a correction made after a
    de-label reads live again. Every concurrent case over-warns rather than under-warns; and the window offers Correct
    only on live lines, so it cannot itself correct a struck line.
15. **An overlay on an older version is said on its line** (ruling R-4c). An "Earlier:" line whose version a counted
    withdrawal or a standing de-label names ends " — entered in error {day}{by}" or " — de-labelled {day}{by}:
    tolerated {term}{ date}", so a clinician's signed act never vanishes under a later correction.
16. **Differing accounts** (D5, design §3.6). Two terms name the same allergen when they match after Unicode
    lowercasing, trimming and collapsing whitespace (`thread::same_allergen`); spelling is not forgiven. Two accounts
    of one allergen differ when their kinds differ (allergy vs adverse effect), or when both reactions are known
    words, neither is `not-known`, and they differ; a row carrying a kind or reaction word this build does not know
    never differs, and the same account twice does not (`accounts_differ`). The test runs over every live head of
    both lists and every member chart, a doubted chart's included. Every head in a differing pair has
    " — differing accounts" appended to its line's text, after any doubt mark, so the mark is on every such
    head's line in the expanded list, and on the strip for an allergy, the only kind the strip lists. Making both
    heads current departs from the design's §7 risk 4 ("the newer by HLC reads as current"), by M2.
17. **The view runs slice 1's logic over the live heads only** (`view::build_view`). A historical (de-labelled) line
    is listed under `historical`, never prominent, and overrides no "none known" statement. A withdrawn entry is off
    the strip and every list but "Entered in error". A withdrawn "none known" statement stands for no list and is not
    "Previously recorded" either. A since-repudiated key's "none known" statement stands for no list (D4) and is
    listed as "Previously recorded", marked; its entry is a live head that warns, and its
    allergy overrides a "none known" statement in its list as any allergy does. The strip adds "+N historical" after
    the adverse-effect count and before the unrecognised count.
18. **The words** (`view.rs`, `history_words.rs`; the markers are the note view's, #1027):
    - a de-labelled line: "Historically reported: {term} {allergy|adverse effect|<kind>}{ (newer allergen kind)}.
      Challenged: tolerated {tolerated}", then " {date as read}" ("14 Mar 2022", "Mar 2022", "2022") when known, then
      " — {text}";
    - a withdrawn entry: "Entered in error: {line words}{doubt mark} (withdrawn {day}{by})", with its older versions
      under it;
    - a withdrawn de-label: "Entered in error: de-label — tolerated {term} (withdrawn {day}{by})";
    - a withdrawn statement: "Entered in error: no known allergies (asked {day}{by})", or "no known drug allergies";
    - an older version: "Earlier: {line words} (recorded {day}{by})", newest first, plus decision 15's tail;
    - `{by}` is " by {name}" plus " (signer unconfirmed)" or " (key had been repudiated)", and the marker alone when
      this node cannot name the key (`thread::by_words`). An expanded line's own byline is its signer's name plus the
      marker (`EntryLine::signer`), and nothing when this node cannot name the key (not guaranteed (g)). Allergies
      carry no "(student)" mark (ADR-0096 decision 22).

### The window (`cairn-gui-tauri`'s `allergy_overlays.rs`, `allergies.rs`, `allergy_pane.rs`, `allergy_overlays.js`)

19. **The acts on a line.** A live line of the drug, other or adverse-effect list carries *Correct*, *De-label* and
    *Entered in error*; a historical line carries only *Withdraw de-label*, an entered-in-error act aimed at the
    de-label it rests on; a standing or previous "none known" line carries *Entered in error*. Entered-in-error,
    "Earlier:" and "Not counted" lines carry none. Each button's accessible name adds ": " and its line's text. De-label
    and Entered in error open one inline row under their line, cloned from the page's `<template>`, with its own *Sign*;
    one row is open at a time, and *Cancel* or Escape closes it signing nothing once no press is in flight. There is
    no dialog and no "are you sure?" (principle 3). *Correct* fills the entry form from the line's words as recorded
    (`allergy_pane::LineWords`), pressing only a button whose word is the recorded word, so a newer peer's word this
    window has no button for stays unpressed; the page says "Correcting: {the line}", and the form's *Sign* sends
    `corrects`.
20. **The refusal order**, each before the key: not the chart on screen; not the chart set the list was read over;
    fixture mode; then the act's own verdicts — for Correct the form's ("Name the substance.", "Choose Drug or
    Other.", "Choose Allergy or Adverse effect.", "Choose what happened, or Not known.", an onset it cannot read or
    after today, "Choose where this came from.", then a wire word it should never send), for De-label "Name what was
    tolerated." and a date it cannot read or after today, for Entered in error none; then `target_before_the_key`: a
    fresh read in one snapshot, its chart set checked against the one sent, and the target found on it — a live line
    for Correct and De-label, also a "none known" line or a historical line's de-label for Entered in error — else
    "That allergy line is no longer on this chart — read it again." with a re-read. Only then the key, one signed
    event filed under the target row's chart (decision 8), its timing sample, and the list of the chart on screen
    read again. `EntryInput`'s `allergen_kind` and `source` are now optional, because Correct can leave them
    unpressed, so ADR-0096 decision 16's four form refusals are six, each under its own field.
21. **Outcomes, as slice 1** (R11, #814): a sign that landed is "Recorded." with the list read again, or the
    "Signed — but …" sentence; an outcome not known is said so with a lead naming the act ("Whether this correction
    was recorded", "Whether this de-label was recorded", "Whether this was marked entered in error"), and re-read,
    never retried live. Any signed act empties a correction open in the form: the line it names may be gone from the
    list read back, and clearing only the correction would leave its words to be signed as a new entry.
22. **A correction whose line has left the list is suspended** (ruling R-6a). On any draw where the line the form is
    correcting is not a live line, the typed words stay, *Sign* is disabled, and the page says "The line you were
    correcting has changed on the record — press Correct on its current line, or Cancel correction." Re-sending it
    could only be refused, read again and refused, forever. Correct on a line, or Cancel correction, ends the
    suspension.
23. **Each history list has its own heading**, hidden with its list: "Previously recorded:", "Historically reported
    (de-labelled):", "Entered in error:" and "Not counted:", so a struck line never reads as a current Other allergy.
24. **Timing kinds.** A correction is timed as `allergy-correct` (never as `allergy-record`), a de-label as
    `allergy-challenge`, a withdrawal as `allergy-retract` (`Act::timing_kind`). As in slice 1 the sample is the
    write half only: from the key in hand to the sign's return, the re-read excluded.

## What is not guaranteed

- **(a) A withdrawal is never withdrawn** (D3). The authoring door refuses it; a peer's withdrawal of a withdrawal,
  admitted at the remote door, changes nothing in the fold, because the resolver never looks a withdrawal up as a
  target (`thread.rs`'s `resolve`; no test pins this). A wrong withdrawal is repaired as on paper, by recording the
  entry again: a fresh entry, its own thread, while the struck line stays struck.
- **(b) "The same allergen" is spelling, not meaning.** "amoxycillin" is not "amoxicillin", and "Amoxil" is not
  "amoxicillin", so two such accounts are never flagged as differing; a de-label's tolerated term is not compared to
  the line it de-labels at all. Coded equality is slice 3's.
- **(c) A de-label on an older version cannot be withdrawn from the window** (#TBD-1). It is said on its "Earlier:"
  line (decision 15), which carries no button; only a historical line carries *Withdraw de-label*. It changes no
  head's status, since it names a version no head is.
- **(d) A de-labelled line cannot be struck as entered in error in one act from the window** (#TBD-2). Its only button
  withdraws its de-label, which puts the allergy back on the strip; the clinician then strikes the live line: two acts
  where paper has one. The floor admits a direct withdrawal of the entry.
- **(e) A historical line does not say who de-labelled it, or when** (#TBD-3). Its text carries the challenge's date,
  and its byline is the entry's signer and day; the de-label's signer and signing day are shown only on an "Earlier:"
  line (decision 15).
- **(f) An overlay that counts but has no effect leaves no trace** (#TBD-4). A withdrawal of a de-label that itself
  does not count; and, from a key that is not repudiated, a de-label naming something other than an entry (only a
  remote door admits it), decision (a)'s withdrawal of a withdrawal, and an overlay naming an event this node does not
  hold or cannot read (until that event arrives or becomes readable): none is listed, not even under "Not counted".
- **(g) No marker for a signer this node cannot name, on a line's own byline** (#TBD-5). An expanded line whose key
  this node cannot name ends " · recorded {day}" with no "(key had been repudiated)"; `by_words` shows the marker
  alone everywhere else. A marker after a name already in brackets nests them: "(withdrawn 10 Nov 2026 by Dr Ada (key
  had been repudiated))".
- **(h) The window's commands against a database are pinned only by DB-gated tests**
  (`allergy_overlays_tests.rs`'s walks), which CI does not run (#905). The webview was checked by source guards
  (`allergy_overlays_webview_tests.rs`) and walked headlessly in Chromium and Playwright's WebKit with a stubbed
  `invoke`; the real Tauri argument casing, the real window's WebKit, VoiceOver and the time budgets are owed to
  RUNBOOK §28.
- **(i) The check-then-sign gap is deliberate, and safe.** `target_before_the_key` finds the line on a fresh read, but
  that snapshot ends before the key is asked for, so another terminal or a peer can correct, de-label or withdraw
  the line before the sign lands; the floor checks only that the target is of a type the act may name, on its chart,
  never that it is live. The result is the same pair of acts the fold already meets across a partition, where R-4a
  makes every order over-warn: the new act covers only what it names. Closing the gap would hold a lock across the
  clinician's key entry. `target_before_the_key` does not yet say so in its doc (#TBD-6, with the window's smaller
  residuals: a row's *Cancel* is enabled but inert during a press; a corrected line that comes back live stays
  suspended until Correct; a target that is not a uuid gets the stale-list sentence rather than a window-defect one).
- **(j) A line struck as wrong-patient can survive as its concurrent correction.** By R-4a a correction signed while
  its target was being withdrawn elsewhere stays live after the withdrawal syncs. This is the over-warning direction,
  and the clinician can strike the correction too.
- **(k) Key standing is this node's.** `KeyState` comes from this node's registry, and a repudiation does not
  replicate (ADR-0093 (i)), so two nodes can fold the same rows differently: one ignores a repudiated key's de-label,
  another counts it. The fold is deterministic given the same rows and key states. Not pinned by a test: an
  `Unconfirmed` key read from a database, and overlays across a doubted link (#TBD-7).
- **(l) Still no clear safety signal** (ADR-0096 decision 21; #406/#401). The overlays write `safety: None` too
  (`allergy_overlays.rs::signing_a_challenge_and_a_retraction_records_human_authorship` asserts it). A reader without
  custody of an overlay counts it in `unreadable`, which leads the strip, and reads its target as live
  (`an_unreadable_overlay_is_counted_and_its_target_stays_live`, `an_unreadable_overlay_leaves_its_target_live`).

## Consequences

- **Easier.** A wrong line is struck, corrected or de-labelled where it is read, in one signed act, with the old
  line legible under the new. The metformin intolerance signed as an allergy is corrected
  to an adverse effect and leaves the strip, and a "none known" it had overridden stands again. Steps (paper *N* →
  built *M* → target *K*): correct 1 (strike and rewrite) → 1 (one entry carrying `correction-of`) → 1 (Correct, edit
  the form, Sign); de-label 1 → 1 (one challenge event) → 1 (De-label, name what was tolerated, Sign); entered in
  error 1 → 1 (one withdrawal event) → 1 (Entered in error, Sign). No `M > N`; (d) is a `K > M` the window owes.
  Budgets (the plan's §1.2), owed by a human in RUNBOOK §28, with the key held: correct ≤ 15 s, de-label ≤ 15 s,
  entered in error ≤ 5 s; differing accounts read on the strip with no click.
- **Harder.** Every reader of the allergy record now folds overlays: a list built from `clinical_allergy` alone
  shows withdrawn and de-labelled lines as live (over-warning, but wrong). A slice that emits a safety signal must
  carry these overlays' supersession onto it first.
- **The bet.** That an overlay covering exactly what it names, plus a suspect overlay that can only add a warning,
  keeps every concurrent or untrusted case on the over-warning side without hiding a clinician's act. We would know
  it is wrong if clinicians meet two current accounts after a partition and cannot tell which to strike, or if
  "differing accounts" fires so often on accounts that differ only in detail (a rash and urticaria after one dose)
  that it stops being read.
- **Pinned by** `db/tests/075_allergy_overlays_test.sql` (and `074_clinical_allergy_test.sql` for the in-place
  messages); `crates/cairn-event/src/{allergy,allergy_overlay}.rs`'s tests;
  `crates/cairn-node/src/allergy/{thread_tests,view_overlay_tests}.rs` and `read.rs`'s tests;
  `crates/cairn-node/tests/{allergy_overlay_floor,allergy_overlays,window_role_grants,twin_registry,projection_registry}.rs`;
  `cairn-gui/cairn-gui-tauri/src/{allergy_overlays,allergy_overlays_webview,allergies}_tests.rs`. RUNBOOK §28 carries
  the human measurements.
