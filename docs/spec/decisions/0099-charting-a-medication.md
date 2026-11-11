# ADR-0099 — Charting a medication: the one line and the check at charting

- **Status:** Accepted (records the GUI milestone's fourth decision, *"prescribe" means charted*; builds slice 1a of the
  charting design's §5; reuses [ADR-0098](0098-allergy-coding-and-the-medication-check.md)'s check at the moment of
  charting, the first half of [#1054](https://github.com/cairn-ehr/cairn-ehr/issues/1054))
- **Date:** 2026-11-11
- **Spec version at acceptance:** 0.101
- **Issues:** PR [#1066](https://github.com/cairn-ehr/cairn-ehr/pull/1066) (the design, the plan and slice 1a). Design:
  `docs/superpowers/specs/2026-11-10-charting-a-medication-design.md` (approved 2026-11-11); plan:
  `docs/superpowers/plans/2026-11-11-charting-slice-1a-the-line-and-the-check.md`. Touches
  [#1054](https://github.com/cairn-ehr/cairn-ehr/issues/1054) (the check at charting: built here without the de-label
  offer), [#1053](https://github.com/cairn-ehr/cairn-ehr/issues/1053) (slice 1b),
  [#1055](https://github.com/cairn-ehr/cairn-ehr/issues/1055) and [#1065](https://github.com/cairn-ehr/cairn-ehr/issues/1065).
  Filed by this slice: [#1067](https://github.com/cairn-ehr/cairn-ehr/issues/1067) (the coding overlay cannot code a
  combination), [#1068](https://github.com/cairn-ehr/cairn-ehr/issues/1068) (no list shows the charting fields),
  [#1069](https://github.com/cairn-ehr/cairn-ehr/issues/1069) (a stat line stays current),
  [#1070](https://github.com/cairn-ehr/cairn-ehr/issues/1070) (the pick's accessibility),
  [#1071](https://github.com/cairn-ehr/cairn-ehr/issues/1071) (an override names one record per mark),
  [#1072](https://github.com/cairn-ehr/cairn-ehr/issues/1072) (*Already taking*, one Sign per line: an `M > N`
  candidate), [#1073](https://github.com/cairn-ehr/cairn-ehr/issues/1073) (the typed brand is not shown beside the
  generic).
- **Builds on:** [ADR-0098](0098-allergy-coding-and-the-medication-check.md) (the check and the drug read door) ·
  [ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) (the `{system, code, display}` coding; drugref is
  optional) · [ADR-0063](0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) (the safety claim) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) / [ADR-0053](0053-per-write-human-authorship.md) (born-sealed,
  human-signed) · [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (a newer peer never wedges sync) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (hard policy in the database, soft policy in the UI)

## Context

On paper a clinician writes "Amoxycillin 500 mg tds PO" on the drug chart and signs it. If the patient has a listed
penicillin allergy and the drug is wanted anyway, they write "penicillin allergy — benefit outweighs risk" beside the
order. Before this slice the window could show, hold, sign off and stop a medication, but it could not add one.

The maintainer set three things in the brainstorm. The allergy check matters most **at the moment of prescribing**.
A line must say whether the drug was **started here** or was **already being taken**, which paper does not record.
And **"prescribe" in this GUI milestone means charted**: no printed or electronic prescription. That fourth milestone
decision (HANDOVER, 2026-10-31) was in no ADR until this one.

Slice 1a builds the line, the two presses, the charting fields on the wire and the check at charting with its
override reason. The same-drug de-label offer on *Already taking* and the group comment are slice 1b (#1053, design
§3.5). Where the build changed the plan, the decision below states what was built.

## Decision

### The milestone's fourth decision

1. **"Prescribe" means charted.** Charting a medication is one signed `clinical.medication.asserted` on the patient's
   chart. Nothing is printed, sent or dispensed, and no prescription shape is reserved by this slice.

### The wire (`crates/cairn-event/src/medication/charting.rs`, frozen)

2. **Five optional keys, with no new event type and no new `schema_version`.** An assertion without them is the old
   assertion: `Charting::default()` writes the payload and the twin byte for byte as before
   (`no_charting_leaves_the_payload_byte_identical`).
   - `charted_as`, top level: `started-here` | `pre-existing` | `unknown` (`ChartedAs`). `unknown` is the honest word
     for an old list nobody can vouch for. The window never sends it.
   - `route`, top level: an open lower-case token (`po`, `iv`, `im`, `sc`, …).
   - `substance.strength`: free text, e.g. `"875/125 mg"`.
   - `substance.components`: a combination, `[{term, coding}]`, each `coding` a `{system, code, display}` triple or
     `null`. It takes the place of `substance.coding`, never sits beside it.
   - `warning_override`, top level: `{reason, against: [{source, ref}, …]}`, the reason typed and the warnings the
     line was charted against (`WarningOverride`, `OverrideRef`). `source` is an open vocabulary of lower-case words
     (`^[a-z][a-z0-9-]*$`, checked at the authoring door only); `allergy` is the only one today, and its `ref` is the
     allergy record's event id. A later source defines its own `ref`. The override is general, not allergy-only,
     because warnings that need a reason will also come from drug–drug interactions, drug–disease interactions and
     renal/hepatic dosing (mostly from drugref), and the wire is frozen once merged. The batch-sign slice (#1072)
     therefore keys on "a finding at or above the warning threshold", never on allergy by name.
3. **The twin** (`render_medication_twin`). Its order is
   `{term}{ amount unit}{ formulation}{ strength}{ route} — {sig} ({info_source}), started {value}{ [bracket]}{ · started here | · already taking}{ · charted despite a warning ({sources}): {reason}}`,
   each part omitted when absent. `{sources}` is the override's distinct non-blank sources in first-seen order,
   joined ", " (today always `allergy`); when no source is named, the parenthetical is omitted
   (`the_tail_names_each_source_once_in_first_seen_order`,
   `a_blank_or_missing_source_is_never_rendered_as_empty_parentheses`). A combination's bracket is always shown: each
   part's coding display, or its term when unpicked, joined " + ". Otherwise the single coding's `[display]` follows
   the rule it had before (omitted when it equals the term, case-folded). `unknown` adds nothing. Pinned by
   `twin_words_are_exact`.

### The floor (`db/077` §1, called from `db/031`)

4. **`cairn_check_medication_charting(p jsonb)`**, called by `cairn_check_medication_assertion` right after
   `cairn_check_medication_coding(p)`. It has db/041's two tiers, and an absent key reads the same as a JSON null.
   - **At both doors (types):** `charted_as`, `route` and `substance.strength` are strings; `substance.components` is
     an array of objects with a string `term`, each `coding` judged by db/041's `cairn_check_coding_object`;
     `warning_override` is an object, its `reason` a string and its `against` an array **when present**, and every
     `against` entry an object whose `source` and `ref` are strings when present. Nothing is cast: the projection
     keeps `against` as jsonb, so a `ref`'s shape is never a type rule.
   - **At the authoring door only:** `charted_as` is one of the three words; `charted_as`, `route`, `strength` and a
     component's `term` are non-blank; a component is never coded `drugref-class` (one drug, never a group);
     `components` has at least two entries and never sits beside a non-null `substance.coding`; the override has a
     non-blank `reason`, a non-empty `against`, and in each entry a non-blank `ref` and a `source` that is a
     lower-case word (`^[a-z][a-z0-9-]*$`, so "Allergy" cannot step around the next rule); an `allergy` entry's `ref`
     is a uuid (`pg_input_is_valid`). A `source` word this node does not know is admitted at both doors (an open
     vocabulary).
   - **A peer's override missing its `reason`, its `against`, or an entry's `source` or `ref` is admitted**, and so is
     a peer's `source` of any spelling and a peer's `allergy` entry whose `ref` is not a uuid. The remote door checks
     the type only of what is present, so a later window that leaves the reason optional never has its events
     refused by an older node.

   It is revoked from PUBLIC, as the whole `cairn_check_*` family is (#382). Pinned by
   `medication_charting_floor.rs` (`local_door_refuses_each_bad_shape`, `remote_door_admits_a_newer_word`,
   `an_event_without_charting_still_passes`).
5. **The reason is a window rule, never a floor rule** (design §3.5; ADR-0021 soft policy). The floor never requires
   an override: an assertion without one is valid at both doors, whatever its `charted_as`.

### The projection (`db/077` §2; `db/037` and `db/044` in place)

6. **`medication_charting`, its own table.** It holds one row per thread: `charted_as`, `route`, `strength`,
   `components` (jsonb, as signed), `override_reason`, `override_against` (jsonb, the `against` array as signed; NULL
   when absent), the overlay-winner tuple and `content_address`. `medication_charting_apply` is a second apply fn
   registered on `clinical.medication.asserted` (`run_order` 20, `heal_safe`), so no view of db/031 widens across
   files.
   - An assertion carrying none of the keys writes no row, and never clears an earlier row.
   - A later charted assertion of the same thread replaces the whole row under `cairn_hlc_overlay_wins`, so set-union
     sync converges.
   - It never raises on a shape the remote door admits: `charted_as` is stored as text, and `components` and
     `against` as given.
   - `patient_id` is the thread's standing chart when one is known, else the event's.

   **Readers join it by `medication_id`, never widen db/031's views.** `SELECT` is granted to `cairn_agent`. The shred
   (db/037) deletes its rows by `content_address`, beside `medication_coding`'s, because an override reason is clinical
   text. The generic reprojection rebuilds it (ADR-0057). Both `cairn_projection_apply` row counts moved by one
   (`projection_registry.rs` 38, `db/tests/039` 40). The twin registry did not move: there is no new event type.
7. **The timing kind `medication-chart`** is added to both of db/044's `gesture_kind` lists. `SCHEMA_GENERATION` is 77.

### The medication pick (`db/076` §5, the drug read door)

8. **`cairn_drug_medication_search(p_text text, p_limit int) RETURNS TABLE(code text, display text, uses bigint)`**
   lives in db/076, the one file that may read drugref, not in db/077 (a new drugref read goes into db/076). Its name starts `cairn_drug_` so
   `drug_door_callers.rs` guards it, and a replay drops the first name, `cairn_medication_drug_search`.
   - It is `SECURITY DEFINER` with `SET search_path = pg_catalog, pg_temp`, answers nothing unless
     `cairn_drug_available()`, is revoked from PUBLIC and granted to `cairn_window` only.
   - **Drugs only**, never an allergy group: rows of `drugref.substance_moiety`. One cannot take "penicillins".
   - The allergy pick's text rules: fewer than 3 characters after trimming answers nothing, LIKE's metacharacters are
     escaped, and a name matches when it or a word in it (split on spaces) starts with the text.
   - **Ranked by this site's use.** `uses` counts every `medication_coding` row coded `drugref-moiety` with that code,
     plus every `drugref-moiety` component with that code in `medication_charting.components`. It reads every
     patient's rows and every line, ceased ones included, and answers only a per-drug count. Most used first, then
     shorter names, then byte order (`COLLATE "C"`). At most `least(p_limit, 20)` rows; the window asks for 12.

### The node (`cairn_node::medication::{charting, line}`, `cairn_node::allergy::check`)

9. **`chart_medication`** signs one assertion carrying its `Charting` through the one strict door
   (`seal_sign_submit`, born-sealed, `author` for a human signature). `assert_medication` is `chart_medication` with
   `Charting::default()`, so every earlier caller is unchanged (`assert_medication_is_unchanged`).
   `build_assert_body` keeps its signature and delegates to `build_charted_body`, so its many callers are untouched.
10. **A combination emits no safety claim** (ADR-0063). The §5.9 class is looked up only for a single coding with no
    components. One class cannot speak for several drugs, and a claim for one part would read as the whole line's.
    Absent is the withholding direction, as for an uncoded line.
11. **The directions are read, never guessed** (`line::read_directions`, pure and table-driven, after the ACSQHC
    abbreviations list). It fills `dose_amount`/`dose_unit`, `route`, `frequency`, `form`, `strength`, `prn` and
    `stat` only when certain. Every word it cannot place goes to `unread`, and nothing is dropped.
    - A decimal is kept exactly as typed (`"1.5"`, `".5"`). A comma decimal (`1,5mg`) is never read.
    - **Two of a kind is ambiguous**: two doses, frequencies, routes or forms leave that field `None` and every
      competing token in `unread`.
    - A number with a strength unit straight after a form word is the strength, never the dose: `tab 500mg`
      reads strength `500 mg` and dose not known, never an assumed 1. A second strength goes to `unread`.
    - A number before a duration word ("5 days") is not a dose. Any other unread number ("2 x 500mg") makes the dose
      uncertain, so the dose too goes to `unread`.
12. **The term signed** (`line::term_to_sign`). The pick's display is signed only when the typed text is its
    case-folded prefix ("amox" picked as amoxicillin signs "amoxicillin"). Anything else typed, a brand or a
    spelling, is signed as typed. **A combination's term is its parts' signed terms joined " + "**, so the record shows the generic names: "amox
    + clav" with both picked signs "amoxicillin + clavulanic acid"; an unpicked part keeps its words.
13. **The check counts every part of a line.** `check_medications` merges lines that share a `group_id` instead of
    overwriting (`merge`). A part's mark is kept unless its words are already shown or its record already has a mark;
    notes are unioned; "not checked" stands only while there is no mark (ADR-0098's invariant). `LineCheck::against`
    holds, per mark, the event id of the live head whose hit produced it, and is what an override signs.

### The window (`cairn-gui-tauri`: `med_chart.rs`, `med_chart_write.rs`, `allergy_check.rs`; `med_chart.js`)

14. **Two presses instead of one Add.** Over a chart on screen the medication pane shows *Start medication* and
    *Already taking*; fixture mode shows neither, and Sign refuses a chart that cannot be written (decision 17). The press is the distinction, at no extra step:
    - *Start*: `charted_as` `started-here`, `info_source` `clinician-observed`, started today at day precision;
    - *Already taking*: `pre-existing`, `patient-reported`, start date not recorded.
15. **The one line.** The field splits at a lone `+` into drug parts. The directions start at the first word led by a
    digit or ".", at a word the reader reads on its own (a form, route, frequency or flag), or after a picked part.
    - From the 4th character of an edited part, after 150 ms, the pick lists drugs with their full display names. The
      top option is highlighted, so Enter takes it; ↓/↑ move, a click chooses, Escape closes the list.
    - **A coding vouches for the words it was picked with.** Editing a part drops its coding. While any part is coded, a
      visible "Coded as: …" line says what each part will be signed as, "(not coded)" for an unpicked one.
    - The read-back row draws the reading: "Drug · Form · Strength · Dose · Route · How often", "—" for each field not
      read, then "not read: …".
    - **Cancel, or Escape with no list open, discards the unsigned line**, as a line crossed out before it is signed. Leaving the chart discards it
      too.
16. **The check at charting** (`med_line_check`). The line is checked as it is typed, on each drug-part change and
    pick, and the line reads "not checked" at once until the new answer comes. It shows ADR-0098's loud **Allergy**
    tag and words, the muted notes, or the small **not checked** tag.
    - Each part is one `MedLine` under one group. Every part carries the line's signed term, so a by-name allergy on
      the line's name meets each part. A combination's parts are checked under its signed term **and** again under the
      drug as typed, so a by-name allergy on "Augmentin", or on "amoxicillin + clavulanic acid", marks it.
      Over-warning is the safe error.
    - The on-screen check and the check at Sign build their lines by one function, `the_line`, so the two cannot
      disagree.
17. **Sign re-runs the check; the webview's answer is never trusted** (`med_chart`). Every refusal comes before the
    key:
    1. the chart on screen and writable;
    2. the press, which must be `started-here` or `pre-existing`;
    3. each pick, which must be a `drugref-moiety` drug with a code that reads as a uuid (signed canonical), and no
       part may be blank ("Name the medication.");
    4. **the check, on a fresh allergy read over the chart set on screen.** A set that changed since (a link or unlink
       landed) is refused as a re-read, never signed unchecked against allergies not yet seen. An allergy list that
       cannot be read lets the line proceed "not checked";
    5. **a Start line with a mark needs a reason**: a blank trimmed reason refuses with "Charting despite a listed
       allergy needs a reason". The trimmed reason is signed with `against` = the fresh check's records, each as a
       `{source: "allergy", ref}` entry. *Already taking* never needs one.

    The webview also keeps Sign disabled while a needed reason is blank. One part is signed as the line's coding, two
    or more as `components`. `sig` is the directions exactly as typed; `formulation`, the dose, `route` and `strength`
    come from the reading. Then the medication list is read again: "Charted.", or, when the sign landed and the re-read
    failed, "Signed — but the medication list could not be read again …". An unknown commit outcome is worded "Whether
    this medication was charted is NOT known — …".
18. **The list reads combinations part by part, and shows the signed directions.**
    - `med_list_check` reads `medication_charting` for the list's threads (`read_charting`). A combination member is one
      line per component. The row's own line is kept beside them when a member is not a combination, or the row
      carries a coding, so a coding added later (db/042) is checked too. A failed read leaves each row its one line, as
      before.
    - **The dose cell is `dose — sig`**, the directions alone with no structured dose, or "dose not recorded" with
      neither, so a stat or tds line reads as signed. The row's accessible label says the same cell.
    - The drug search runs on its own lock, never inside a snapshot (its swallowed failure would abort a shared transaction). A search that fails reads as no drug list.

## What is not guaranteed

- **(a) A cephalosporin against a penicillin allergy reads clean at charting too** (#1055, ADR-0098 (a)). drugref has
  no such rule, so no reason is asked.
- **(b) Local and brand names find no drug** (#1065, drugref#231). "amoxycillin", "Augmentin" or "frusemide" lists
  nothing, so the line signs uncoded and reads "not checked". A combination is coded only by typing its parts
  (`amox + clav`), two picks where one would do (design §3.4).
- **(c) A combination carries no safety claim** (decision 10). A reader without custody of the body gets no signal
  for it.
- **(d) The coding overlay is still single-coded** (#1067). A combination charted uncoded can be coded later only as
  one drug.
- **(e) No list shows the charting fields** (#1068). The window shows the route only inside the directions, a
  combination only through its term, and `charted_as` and the override nowhere. The CLI list shows none of them.
- **(f) A stat line stays current** (#1069). `stat` is read on the screen but rides the wire only inside `sig`, until
  slice 2's "given" act.
- **(g) The reason gate is the window's.** A raw-SQL client, another UI or a peer can sign a Start line with a mark
  and no reason (decision 5). The window cannot judge a reason either: "." passes (design §7 risk 4).
- **(h) A failed check never blocks charting.** When the allergy list cannot be read at Sign, a Start line signs "not
  checked" with no reason asked. With no drug list, only a hit by name or by the same moiety code asks for one.
- **(i) An override names one record per mark** (#1071). Another record of the same allergen is not in `against`.
- **(j) The pick is not fully accessible** (#1070): the tag and marks are not live regions, and the combobox has no
  `aria-activedescendant`.
- **(k) The typed brand is not shown beside the generic** (#1073). `line::shown_name` exists, but no screen calls
  it. A single pick keeps the brand in the term and the twin only.
- **(l) *Already taking* costs one Sign per line** (#1072): twelve home medications are twelve Signs against one
  signature on paper. This is an `M > N` candidate for the maintainer.
- **(m) No de-label offer and no group comment** (slice 1b, #1053; #1054 stays open).
- **(n) The pick is a reflex risk** (design §7 risk 1). The top option is highlighted and Enter takes it. The order
  is by use, then length. The "Coded as" line and the read-back name the drug before Sign, but nothing prevents the
  wrong pick. The counts are recomputed per keystroke (a `shortcut:` in db/076).
- **(o) No PBS forms or strengths.** The pick offers the drug only. Form and strength come from the typed line.
- **(p) The window's commands against a database are pinned by DB-gated tests CI does not run** (#905), and the
  webview by source guards and stubbed headless walks. The real window, the real Tauri IPC, VoiceOver and the budgets
  are owed to RUNBOOK §30.

## Consequences

- **Easier.** A clinician charts a medication from one line, sees the drug, form, strength, dose, route, how often
  and any allergy hit on that line before Sign with no click, and cannot sign a new drug against a listed allergy
  without saying why. The record says whether the drug was started here or already taken, its route and strength,
  and a combination's parts. The steps (paper *N* → built *M* → target *K*):
  - **Chart a line:** 1 (write and sign) → 1 (one signed event) → 1 (the line and Sign).
  - **The override reason:** 1 (a written note) → 0 extra (it is in the event) → 1 typed field.
  - **Start vs Already taking:** 0 on paper → 0 extra → the press replaces the one Add.
  - **A combination before drugref has aliases:** two picks where one would do, still one event.

  The budgets, owed by a human in RUNBOOK §30, are the plan's §1.2: a common drug charted from the line in ≤ 10 s
  (paper ≈ 8–15 s), and the line legible before Sign with no click. *Already taking* across a long list is the open
  question of #1072.
- **Harder.**
  - Every key above is wire-forever (principle 11). A reader of the medication stream must read `medication_charting`
    by `medication_id`, never widen db/031's views.
  - A later slice that codes a combination after the fact (#1067), or gives it a safety claim, must settle a shape for
    several parts.
- **The bet.** The bet is that reading the line back, never guessing, and checking at the moment of charting stops
  the wrong drug and the wrong dose before Sign. We would know it is wrong if:
  - clinicians learn to type a throwaway reason (".", "ok") to pass the gate. The fix would then be review, never a
    dialog;
  - the "not checked" tag sits on most new lines because local names find no drug (#1065). The fix is drugref's
    aliases.
- **Pinned by:**
  - the wire: `crates/cairn-event/src/medication/charting.rs`'s tests;
  - the floor, projection and door: `crates/cairn-node/tests/{medication_charting_floor,medication_charting_projection,medication_drug_search_door,medication_chart}.rs`,
    `db/tests/039_projection_registry_test.sql`, `crates/cairn-node/tests/projection_registry.rs`;
  - the parser and the check: `crates/cairn-node/src/medication/line_tests.rs`,
    `crates/cairn-node/src/allergy/check_combination_tests.rs`;
  - the window: `cairn-gui/cairn-gui-tauri/src/{med_chart,med_chart_db,med_chart_rows,med_chart_webview,med_chart_line_webview}_tests.rs`,
    `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/{lib.rs,row_view_tests.rs}`.

  RUNBOOK §30 carries the human measurements.
