# ADR-0105 — A stat dose, ordered then given

- **Status:** Accepted (builds charting slice 2: the `stat` key, the given dose and its "entered in error" on the
  wire, their floor and projections, the node's two acts, and the stat line in the window)
- **Date:** 2026-11-14 (built), recorded 2026-11-14
- **Spec version at acceptance:** 0.107
- **Issues:** PR [#1106](https://github.com/cairn-ehr/cairn-ehr/pull/1106). Design:
  `docs/superpowers/specs/2026-11-10-charting-a-medication-design.md` §3.6 and §10 Q3; plan
  `docs/superpowers/plans/2026-11-14-charting-slice-2-stat-ordered-then-given.md` (the maintainer's rulings Q1–Q4 of
  2026-11-14, recorded below, and the build's rulings R6–R9). Ends ADR-0099's limit (f)
  ([#1069](https://github.com/cairn-ehr/cairn-ehr/issues/1069)). Refs
  [#1107](https://github.com/cairn-ehr/cairn-ehr/issues/1107), [#1108](https://github.com/cairn-ehr/cairn-ehr/issues/1108),
  [#1109](https://github.com/cairn-ehr/cairn-ehr/issues/1109), [#1110](https://github.com/cairn-ehr/cairn-ehr/issues/1110),
  [#1111](https://github.com/cairn-ehr/cairn-ehr/issues/1111), [#1112](https://github.com/cairn-ehr/cairn-ehr/issues/1112).
- **Builds on:** [ADR-0099](0099-charting-a-medication.md) (the charted line, `charted_as`, the directions parse;
  limit (f), "A stat line stays current") · [ADR-0103](0103-settling-a-set-aside-version.md) (the one-act idiom: the
  window's steps before the key, `{who}` as a human key's name or "signer not known") ·
  [ADR-0097](0097-allergy-overlays-and-differing-accounts.md) (the entered-in-error shape; blank text is null) ·
  [ADR-0081](0081-medication-withhold-is-a-time-bounded-hold.md) (no administration record) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) and [ADR-0053](0053-per-write-human-authorship.md) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (a reference is an authoring-door rule) ·
  [ADR-0058](0058-grade-gated-teffective-ceiling.md) (the `t_effective` ceiling) ·
  [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) (clashes flagged, never auto-resolved)

## Context

In resus the registrar writes "ceftriaxone 1 g IV stat" in the stat box of the drug chart and signs it. The nurse who
gives it initials the box and writes the time; the order is then done. A dose initialled in error is struck through,
initialled and annotated, never erased. Until this slice Cairn charted `ceftriaxone 1g iv stat` as an ordinary line:
the window read "stat" from the directions, but nothing on the wire said it was a one-off, nothing could record that
it was given, and the line stayed on the current list for good (ADR-0099 limit (f)). A current-list reader then saw a
regular ceftriaxone order where the patient had had one dose.

The maintainer ruled on four questions (2026-11-14):

- **Q1:** a given is a stop ("stat dose given") and a given, under one signature, in one transaction, the stop first.
- **Q2:** `stat` is signed only on a pure one-off *Start* line.
- **Q3:** no new section on the list; the row reads "given".
- **Q4:** "entered in error" is built in this slice, not later.

## Decision

### The order: one optional key

1. **`stat` is a new optional top-level key on `clinical.medication.asserted`:** JSON `true` on a stat order, absent
   otherwise, never `false` (`crates/cairn-event/src/medication/charting.rs`, `Charting::stat`). The builder writes the
   key only when it is true, so a line that is not a stat order serialises byte-identically to one built before the
   key existed (pinned by `an_assertion_without_stat_is_byte_identical`). The twin does not mention it: the `sig`
   already reads "stat".
2. **Its floor checks shape only, at both doors** (`db/077`, in place): absent, JSON null (which reads as absent) or
   JSON `true`; anything else is refused. The projection is a new column, `medication_charting.stat` (boolean, NOT
   NULL, default false, so every older row reads "not stat"). The floor does not compare the key with the `sig`.
3. **A set-aside version never carries it.** `set_aside_body` strips it, and `db/079` refuses it at both doors even as
   JSON null, as it refuses `charted_as`, `warning_override` and `restores`.
4. **The window signs it only on a pure one-off *Start* line (maintainer, Q2).** `med_stat::is_stat_order` is true
   when the press is *Start medication*, the directions read `stat`, and they carry no frequency and no `prn`.
   "1g iv stat then daily" is a regimen whose first dose is given now, and "prn stat" means "when needed"; neither may
   leave the list at its first give. An *Already taking* line orders nothing. The rule is called from
   `med_line_sign::charting`, so the one-line Sign and the list's Sign cannot disagree.

### The give: a stop and a given, under one signature (maintainer, Q1)

5. **Giving a stat dose is two events in ONE transaction, the stop first** (`stat_give::give_stat` in
   `cairn-node`). The first is an ordinary cessation of the line, reason `"stat dose given"`, with no stop date. The
   second is the given, whose `completes` names that cessation's `event_id`. Both are prepared and signed by the
   same human under one key release (contributors `{human, authored}` and `{node, recorded}`, ADR-0053), then
   submitted together. A failed given rolls the stop back with it. **Why a stop at all:** "current" keeps meaning "no
   `medication_cessation` row", so no reader of current lines was edited: the allergy check, sign-off,
   reconciliation, the restore lineage and the CLI all see a given stat dose as stopped. A node built before this
   slice reads the line as stopped, reason "stat dose given", and holds the given uninterpreted (ADR-0056).
6. **The given type is general:** `clinical.medication-given.asserted`, schema `clinical.medication-given/1`, payload
   exactly `{medication_id, completes}` (`crates/cairn-event/src/medication/given.rs`). The envelope's `t_effective`
   is the time given. `completes` is the stop it completes, or JSON null for an ordinary give, which stops nothing:
   a future administration record reuses the type unchanged. Twin: `Dose given: medication {medication_id}`, then
   ` — completes its stat order` when `completes` is set. The type is `additive`, with
   `targets_other_author = FALSE`, born-sealed (a `clinical.*` body, ADR-0052), with a required twin.
7. **The given's floor** (`db/082` §1, `cairn_check_medication_given`). At BOTH doors: the payload's shape;
   `charted_as`, `warning_override`, `restores` and `stat` absent, refused even as JSON null (a given dose is not a
   medication line); no §5.9 safety claim. At the authoring door only: the schema tag; a `t_effective`; the line on
   THIS chart (`medication_statement` on `medication_id` and `patient_id`); and, when `completes` is set, that it
   names a cessation of this line on this chart **inserted by this same transaction**
   (`xmin = pg_current_xact_id()`, read from the log), and that the line is a stat order.
8. **Why the same transaction (found in the build).** A stop from an earlier act is someone's real decision
   ("patient refused"). A given allowed to name it would let a later "entered in error" void that real stop and show
   the line current again. So nothing may wrap the stop's submit in a savepoint: a subtransaction stamps its own
   id, and the given is then refused (it fails closed; `give_stat`'s doc says so).
9. **The floor does not judge whether the line is current or already given.** The window does, before the key
   (decision 20). A stat line given at once from two terminals is two stops and two gives; striking one leaves the
   other's stop standing, so the line stays stopped (pinned by
   `striking_one_of_two_gives_does_not_say_waiting_again`).
10. **The projection is `medication_given`**: one row per given event, keyed by its content address, never overlaid,
    with `given_at` the envelope's `t_effective` and `hlc_wall` its `t_recorded`. Its applier is insert-or-nothing
    and `heal_safe`. It skips, never raises on, a shape it cannot represent or a given with no `t_effective`, which
    a peer's sealed body admitted without custody may be. A peer's given that arrives before its line is held and
    shown nowhere, because readers join it on `medication_id` AND `patient_id`. A shred scrubs the row by content
    address (`db/037`), and its sensitivity resolves to its line's thread (`db/048`).

### The time given

11. **A later time is refused in the window, because the floor admits it.** The strict door's `t_effective` ceiling
    (`db/005` step 1b', ADR-0058) does not refuse a time after `t_recorded` at the clock grades a node authors today:
    it admits the event and flags it (`cairn_record_ceiling_flag`). So `med_stat::given_time` refuses a time later
    than now before the key (`GIVEN_LATER_THAN_NOW`, "The time given cannot be later than now"), and the field's
    `max` is now. A raw-SQL client's forward-dated given lands with a ceiling flag.
12. **An earlier time is signed, even one before the order was charted (spec §10 Q3).** A verbal order in resus,
    given 12:30 and charted 12:34, reads "given 12:30 …, before it was charted 12:34 …": shown, never refused and
    never reordered (principle 4). "Before" means earlier than the minute the order was charted: the time given is
    stated to the minute, so a dose given untouched in the order's own minute reads after it, never before.
13. **The field.** "Time given" is a `datetime-local` pre-filled with now when Given is pressed, and an empty field
    means now. A partly typed time is never signed as midnight or as now. Rust's parser refuses one in words. The
    webview refuses one before invoking, because it reaches the script as an empty value (`validity.badInput`), and
    it keeps refusing after a redraw has rebuilt the field empty: "The time given was lost while it was being typed
    — type it again, or press Cancel and Given for now" (ruling R9). A partial the clinician clears key by key means
    now.

### Entered in error (maintainer, Q4)

14. **A strike is its own type,** `clinical.medication-given-retraction.asserted`, schema
    `clinical.medication-given-retraction/1`, payload exactly `{medication_id, target, voids, text}`. `target` is the
    struck given's `event_id`. `voids` is the stop that given completed, or null when it stopped nothing. `text` is
    the optional why, and blank is written null (ADR-0097 decision 3). Twin:
    `Entered in error: withdraws given dose {target}`, then ` — {text}`, trimmed. The type is `additive`, with
    `targets_other_author = FALSE`: a nurse may strike a doctor's entry, as on paper. It is born-sealed, with a
    required twin.
15. **Its floor** (`db/082` §2, `cairn_check_medication_given_retraction`). At BOTH doors: the shape, the four
    charting keys absent, no safety claim. At the authoring door only: the schema tag; the target is a projected
    given of this line on this chart; and `voids` IS NOT DISTINCT FROM that given's `completes`, so a strike voids
    exactly what its give stopped, never a real stop. A second strike of one given is admitted and harmless; the
    window refuses it before the key (`GIVE_GONE`).
16. **The void keys on the retraction's own `voids`, never on the given row.** Shredding the given therefore cannot
    bring its stop back. Shredding the strike can, and should: the strike is gone, so "given" stands again (`db/037`'s
    re-fold re-applies the stop). Shredding the voided stop changes nothing visible. The void reaches only the
    strike's own thread (`medication_id`), so it can never void another line's stop.
17. **Both arrival orders end in the same rows.** A stop that arrives after its strike is never projected:
    `db/031`'s `medication_cessation_apply` and `db/060`'s reason applier (both edited in place) ask
    `cairn_medication_cessation_voided` before writing the stop's row. A stop that arrived first is removed by the
    strike's applier, which deletes the stop's reason rows and its `medication_cessation` row. **Only when that
    delete removed the thread's winning row** does it re-fold the thread through `cairn_refold_after_shred`
    (`db/037`, #863), so a real stop of the same line wins again, or none does and the line reads current. The plan
    had the re-fold run unconditionally; the build found that it recursed without end, because the re-fold replays
    the thread's strikes, this one included.
18. **A stop and its strike applied at once serialize per stop.** Pulls may run at once, one per peer. Without a
    lock, a hub applying a peer's stop while another transaction applies the strike voiding it would have each miss
    the other's uncommitted rows, and the struck stat dose would read given. Both appliers take
    `cairn_lock_medication_stop(stop)`: the stop applier before its voided check, the strike after
    `cairn_lock_shred_refold()`. Whichever comes second waits, then reads the first's rows. The lock is a no-op
    while `cairn.reprojecting` is `'on'`, which `cairn_reproject` (`db/039`, in place) sets for its own run, so a
    rebuild never holds one lock per stop (`max_locks_per_transaction`). Both doors pin that setting off for their
    own call with a function-level `SET cairn.reprojecting = ''` (`submit_event` in `db/005`, `apply_remote_event`
    in `db/020`, both in place), as #602 pinned `cairn.remote_apply`, so a caller who raises it still takes the lock;
    `cairn_reproject` calls the appliers directly, never through a door. Pinned by
    `medication_given_retraction_race.rs`, both orders and both doors.
19. **A narrow rebuild that would truncate the stops is refused.** The strike's applier is registered against
    `medication_given_retraction`, `medication_cessation` and `medication_cessation_reason`, so `db/039` refuses a
    rebuild of `clinical.medication-given-retraction`, of `clinical.medication-given` (which matches both new types)
    and of `clinical.medication-cessation`: none of them can truncate the stops and replay only one side. The last was
    already refused, because `medication_patient_conflict_flag` is fed from outside it too; the strike is now a second
    reason. `clinical.medication-given.` (with the dot) rebuilds the given alone, and `clinical.medication` or `''`
    rebuilds stops and strikes together, to the same rows.

### In the window

20. **Given** is offered only on a stat order of the OPENED chart (never a linked chart's) that is waiting and whose
    line is current. Its press opens an inline row under the line: "Time given", *Sign* and *Cancel*; the row signs
    nothing until its own Sign. Sign runs ADR-0103's steps in order: the chart on screen and writable, the time
    parsed, then a FRESH read of the opened chart (`waiting_here`: a stat order there, waiting, and still in
    `patient_medication_current`). A press made stale by a stop or a give since the pane was drawn is refused
    before the key, with no event written: `STAT_NOT_OPEN`, "That stat dose is no longer waiting to be
    given; read the chart again". Only then is the key released, once. The outcome reads "Given.". No dialog.
21. **Entered in error** is offered on each standing give of the opened chart, with an inline row "Why (optional)".
    The fresh read must find that give, still standing, under a stat order of the opened chart, else `GIVE_GONE`
    ("That given dose is no longer on this chart, or is already struck; read it again"). The node reads the given's
    `completes` and signs `voids` equal to it, in its own transaction.
22. **The strike's words come from the state AFTER it** (ruling R7). "Entered in error: the stat dose is waiting to
    be given again." is said only when a fresh read after the commit finds the order waiting and its line current;
    otherwise "Entered in error.". A second standing give, or a real stop, keeps the line stopped, and a false
    "waiting again" invites a second dose. A failed read after the strike also says "Entered in error.".
23. **An act that landed always answers Ok.** When the pane cannot be read again, the outcome reads "Signed — but the
    medication list could not be read again (…). Use “Read this chart again”.". An error would read as "not
    recorded" and be offered for retry. The retry would only be refused as stale (`STAT_NOT_OPEN`), and the
    clinician would be left believing a dose they gave went unrecorded.

### Reading it back (maintainer, Q3)

24. **No new section.** A given stat order keeps its own row, as a stopped line does. Its status reads `given`
    instead of `ceased` when the line is not current and a give stands, and the window styles it as a ceased row.
25. **The stat line** sits under the drug line: `stat · ordered {o} {orderer} · not yet given`, or
    `stat · ordered {o} {orderer} · given {g} {giver}`, standing gives chained in time order, joined by ` · `. A give
    earlier than the minute the order was charted (its `t_recorded` truncated to the minute) reads
    `given {g} {giver}, before it was charted {o} {orderer}` in the order's place, said once, on the last such give.
    A struck give keeps its own line: `given {g} {giver} — entered in error by {who}, {when}`, then ` — {why}`.
    `{o}` is the order's `t_recorded`, `{g}` the given's `t_effective` and `{when}` the strike's `t_recorded`, each
    through the tab crate's `local_instant`.
26. **Names follow slice 2b's rule** (ADR-0103 decision 22): the vouched attester, else the envelope signer, named
    only when the registry enrolled that key for a human, else `signer not known`.
27. **The read shares the chart's snapshot** under its own savepoint (`chart_stat::read_stat`), so a line is never
    shown "not yet given" beside a list read after it was given. When it fails, the list still opens, a given line
    still reads stopped, and the pane says above the list "Stat doses could not be read: no line shows whether it
    was given".
28. **A row holding several stat orders** (reconciled duplicates, ADR-0047) shows one: the one with a standing give,
    else the first read. It offers Given only when no order under it has a standing give (ruling R8), because Rust
    judges "waiting" per thread, and a Given on the waiting twin would give the dose twice.
29. **Soft policy.** The `stat` rule, the buttons, the rows, the time refusals and the copy live in the window. The
    types, their two tiers, the same-transaction pairing, the void and the projections are the floor, the same for
    every client.

## What is not guaranteed

- **No administration record.** There are no dose slots, no regular-dose giving and no "omitted" (ADR-0081). The
  given type is ready for one; nothing else is built.
- **No "Waiting for you" group for an ungiven stat dose** ([#1108](https://github.com/cairn-ehr/cairn-ehr/issues/1108)).
  "not yet given" shows on the row only; a nurse has no worklist of stat doses to give.
- **Giving re-checks no allergy** ([#1109](https://github.com/cairn-ehr/cairn-ehr/issues/1109)). The check ran when
  the order was charted; an allergy recorded between the order and the give is not raised at Given.
- **A narrow owner heal can re-instate a voided stop** ([#1110](https://github.com/cairn-ehr/cairn-ehr/issues/1110)).
  An owner-run heal of `clinical.medication-cessation` takes no per-stop lock (decision 18), so, racing a door's
  strike, it can re-insert the stop the strike just voided. A full heal (`''` or `clinical.medication`) repairs it.
- **A peer's strike may void any stop of its thread** ([#1111](https://github.com/cairn-ehr/cairn-ehr/issues/1111)).
  The remote door checks structure only (ADR-0056), because a peer's strike may arrive before its given, so a
  faulty peer's `voids` naming a real stop of the same line is obeyed. Confining the void to `"stat dose given"`
  stops was rejected: it would tie currency to a free-text reason.
- **An older node keeps a struck stat dose stopped.** A node built before this slice holds the strike
  uninterpreted, so on it the line reads "Stopped: stat dose given" after the strike too, for as long as it runs the
  older schema.
- **One stat line per row** ([#1107](https://github.com/cairn-ehr/cairn-ehr/issues/1107)): decision 28.
- **`stat`, `ordered {o}` and the orderer follow the charting row's overlay winner.** `medication_charting` keeps one
  row per thread, so a later re-assertion of the thread carrying any charting key but no `stat` clears the flag, and
  it renames the orderer and moves `{o}` to its own signer and time.
- **Accessibility gaps** ([#1112](https://github.com/cairn-ehr/cairn-ehr/issues/1112)): a refusal beside the line is
  not a live region, focus is lost when the pane redraws, and the tab's semantics do not list Given or Entered in
  error.
- **Drafts do not cross nodes** ([#963](https://github.com/cairn-ehr/cairn-ehr/issues/963)). An open Given or strike
  row is not even stored: it lives in this window's memory, and closing the window loses it. The signed give and
  strike do cross.
- **The figures are owed by a human** (RUNBOOK §36): every stopwatch figure, the real window's WebKit and Tauri IPC,
  and VoiceOver. The webview was walked headlessly in Chromium and WebKit with a stubbed `invoke`. Neither act has a
  `ui_gesture_timing` kind, so the stopwatch figure is the only one. The DB-gated window tests run locally only
  ([#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)).

## Consequences

- Order: paper *N* = 1 (write and sign) → architecture *M* = 1 signed act → UI *K* = the line + Sign (slice 1a's
  gesture). Give: *N* = 1 (initial and time) → *M* = 1 signed act (the stop and the given under one key release) →
  *K* = 1 press + Sign, the time pre-filled (+ an edit when it was earlier). Entered in error: *N* = 1 (strike,
  initial, note) → *M* = 1 → *K* = 1 press + the optional why + Sign. Reading: *K* = 0, the words are on the row.
  Budget: ≤ 3 s from the press to a given recorded at now.
- ADR-0099's limit (f) ends here; ADR-0099 itself is not edited, since ADRs are immutable.
- `SCHEMA_GENERATION` moves 81 to 82 (`db/082` is a new file). The `db/031`, `db/037`, `db/039`, `db/048`, `db/060`,
  `db/077` and `db/079` edits, and a comment in `db/005`, are in place and reach an existing database by the content
  hash ([ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md)).
- The given is Cairn's first administration event. A future record reuses the type with `completes` null; a stat
  order's give stays paired to its stop by `completes`.
- `completes` bound to a stop of the same transaction is the floor's first use of `xmin` to tie two events signed as
  one act. A later pairing that must not reach an earlier act's event can reuse it.
