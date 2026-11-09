# ADR-0098 — Allergy coding and the medication check

- **Status:** Accepted (opens [ADR-0096](0096-the-allergy-list.md)'s reserved `substance.coding` slot for allergy
  entries, as ADR-0096 decision 5 and [ADR-0097](0097-allergy-overlays-and-differing-accounts.md) (b) foresaw; amends
  ADR-0097 decision 16's "same allergen" with coded equality (D10); builds slice 3 of the allergy design's §5, with the
  maintainer's 2026-11-10 replan replacing the design's per-allergen "not fully checked" line (M6))
- **Date:** 2026-11-10
- **Spec version at acceptance:** 0.100
- **Issues:** PR [#1052](https://github.com/cairn-ehr/cairn-ehr/pull/1052) (allergy slice 3). Design:
  `docs/superpowers/specs/2026-11-08-allergy-stream-design.md` §3.2, §3.7, §4, §7 risks 2–3; plan:
  `docs/superpowers/plans/2026-11-09-allergy-slice-3-drugref-and-the-medication-check.md`. drugref's side: drugref PR
  #225 (`db/067_allergy_groups.sql`, `db/068_allergy_check.sql`, `db/069_allergy_check_comment.sql` and its design
  `docs/superpowers/specs/2026-11-08-drugref-allergy-groups-design.md`, in the drugref repository). Touches
  [#871](https://github.com/cairn-ehr/cairn-ehr/issues/871) (decision 17), [#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)
  ((h)) and [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) / [#401](https://github.com/cairn-ehr/cairn-ehr/issues/401)
  (decision 4). Filed by this slice: [#1053](https://github.com/cairn-ehr/cairn-ehr/issues/1053) (code the tolerated
  drug, D2), [#1054](https://github.com/cairn-ehr/cairn-ehr/issues/1054) (the check at the moment of charting, (f)),
  [#1055](https://github.com/cairn-ehr/cairn-ehr/issues/1055) (drugref's cross-allergy rule, (a)),
  [#1056](https://github.com/cairn-ehr/cairn-ehr/issues/1056) (an unknown reaction word ranks last, (e)),
  [#1057](https://github.com/cairn-ehr/cairn-ehr/issues/1057) (the group was crowded out of the pick and its real
  name doubled up; fixed before merge, decisions 9 and 23, (g)), [#1058](https://github.com/cairn-ehr/cairn-ehr/issues/1058)
  (the reviews' deferred minors), [#1059](https://github.com/cairn-ehr/cairn-ehr/issues/1059) (an unreadable allergy
  record left the medication list untagged; fixed before merge, decision 21, (k)).
- **Builds on:** [ADR-0096](0096-the-allergy-list.md) (the allergy list) ·
  [ADR-0097](0097-allergy-overlays-and-differing-accounts.md) (overlays, the fold, differing accounts) ·
  [ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) (drugref is optional; the `{system, code, display}`
  coding; the moiety anchor) · [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
  (strict submit, lenient apply) · [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (a newer peer never
  wedges sync) · [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) (the window's own login) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (the floor in the database)

## Context

On paper the prescriber or pharmacist reads the allergy box against every line of the drug chart before a dose is
given. They know that amoxicillin is a penicillin, and that flucloxacillin is in the same group as amoxicillin. Slices
1 and 2 (ADR-0096, ADR-0097) put the allergy box on screen, but nothing read it against the medication list. The
words were free text, so "penicillin" and "Augmentin" were two unrelated strings.

This slice does two things:

- **It codes a drug allergen.** In the entry form the clinician may pick from drugref one drug or one drugref
  *allergy group*, such as *Penicillins*. Picking is optional, and a name left as typed is recorded exactly as before.
- **It checks every current medication line against the live drug allergies.** drugref, the optional sister drug
  database (ADR-0059), answers the group question. The answer is shown on the line itself.

The maintainer took three decisions before the plan (M1–M3) and four more after drugref shipped its allergy groups
and `allergy_check` (drugref PR #225, 2026-11-10: M4–M7). M4 supersedes M3. M4–M6 supersede the plan's first-draft
calls D4–D6, which had walked MED-RT classes in Cairn and printed "not fully checked" lines. The plan's designer made
the calls D1–D3 and D7–D11. The build's rulings are recorded in the slice's ledger and cited by letter below where
they changed the plan (T2-a, T2R-b, T3-a, T4-a to T4-f, T5-b to T5-f, T6-a, F2, F3, R1–R3). The first signed coded entry
freezes the coding's wire shape (principle 11), so that shape is written down exactly.

## Decision

### The wire (`crates/cairn-event/src/allergy.rs`, frozen)

1. **The coded entry needs no new type and no new `schema_version`.** An entry stays `clinical.allergy.asserted` with
   `clinical.allergy/1`. `substance.coding` is `null` for a name as typed. Otherwise `AllergyEntry::coding` writes
   exactly `{"system", "code", "display"}` (`allergy_payload`), the ADR-0059 shape.
   - `system` is `drugref-moiety` (one drug) or `drugref-class` (an allergy group, **M2**).
   - `code` is drugref's immortal uuid: a `moiety_uuid`, or a group's `class_uuid`.
   - `display` is the name drugref gave at the moment of the pick. It is signed as it was then, even if drugref later
     renames the drug (ADR-0059).
   - The token `drugref-class` is wire-forever once the first coded group is signed.
2. **A coded entry's twin ends `" (coded as {display} [{system}])"`** (`render_allergy_twin`). This comes after the
   correction prefix of ADR-0097 decision 1 when there is one. An uncoded twin is unchanged, byte for byte.
3. **A de-label's `tolerated.coding` stays null (D2).** The tolerated drug feeds no check. A de-labelled line has
   already left every check, because the check reads live heads only (decision 18). db/075 still refuses a
   non-null `tolerated.coding` at the authoring door. Opening it is additive later (#1053).
4. **Still no clear safety signal** (the no-safety-signal ruling, ADR-0096 decision 21, design §4). A coded entry
   writes `safety: None`, as every allergy event does. A signal emitted now would warn forever after "entered in
   error", because db/049's signal has no supersession yet (#406/#401). The check is computed on every read, from the
   decrypted bodies with overlays applied (decisions 15–21). Nothing about it rides the wire. A reader without
   custody of the allergy bodies therefore gets no check. The strip tells them a record cannot be read (ADR-0096
   decision 10), and every current medication line without a mark reads "not checked" (decision 21, (k), #1059).

### The floor (`db/074` and `db/041`/`db/042` changed in place)

5. **The coding slot opens for drug entries only (D1).**
   - **At both doors** (`cairn_check_coding_object`, db/041), the check is structural. A coding is null, absent, or
     an object whose `system`, `code` and `display` are non-empty strings. An allergy entry must carry the key
     (db/074 refuses an absent `coding` at both doors: null when uncoded).
   - **At the authoring door only** (`cairn.remote_apply` is not `'on'`), three more rules apply:
     - `system` must be registered in `medication_coding_system`, and a uuid-format system's `code` must be the
       canonical lowercase hyphenated uuid (db/041);
     - an entry whose `allergen_kind` is not `drug` may not carry a coding ("substance.coding is for a drug entry
       only");
     - `system` must be `drugref-moiety` or `drugref-class` (db/074 section 3).

   The remote door keeps the structural rule only, so a newer peer that codes an allergen in a way this node does not
   know still syncs (ADR-0056).
6. **`drugref-class` is registered in `medication_coding_system`, and the medication doors refuse it (D3).** There is
   one coding registry, so the allergy floor's registry tier vouches for the new system like any other. A medication
   is one drug, never a group. `cairn_check_medication_coding` (db/041 2b) refuses `drugref-class` at the authoring
   door, and so do both of db/042's coding overlays, the coding and the coding correction. `SCHEMA_GENERATION` is 76.

### The drug read door: the drugref contract Cairn consumes (`db/076_drug_read_door.sql`)

7. **The door is SQL in Cairn's own database (M1).** A deployment that wants drugref installs it into the Cairn
   database as the schema `drugref`. Most nodes will not have it. db/076 is **the one file that reads drugref**.
   - `crates/cairn-node/tests/no_drugref_dependency.rs` exempts exactly that file and asserts that the file exists.
   - `crates/cairn-node/tests/drug_door_callers.rs` forbids any other `db/` or `extensions/` file to call the doors, so
     no floor, projection or sync path can come to depend on an optional database.
   - **When drugref changes, only db/076 changes.**

   The window's login (`cairn_window`, db/062) gets `EXECUTE` on the four doors and no grant on drugref. PUBLIC's
   `EXECUTE` is revoked. Each door is `SECURITY DEFINER` with `SET search_path = pg_catalog, pg_temp`, and every
   `drugref.` and Cairn name inside it is schema-qualified. **No door's name contains "drugref"**, so the Rust that
   calls them never has to name it, and `no_drugref_dependency.rs`, which scans every `.sql` and `.rs` file under
   `db/`, `crates/` and `extensions/` (files under a `tests/` directory excluded), stays strict for all of it. The
   window's own callers under `cairn-gui/` are outside that scan.
8. **`cairn_drug_available() RETURNS boolean`.** It is true only when the door's owner holds all of these:
   - USAGE on the schema `drugref`;
   - SELECT on `drugref.substance_moiety`, `drugref.substance_class` and `drugref.allergy_group` (each a table,
     partitioned table, view, materialized view or foreign table);
   - EXECUTE on a `drugref.allergy_check(uuid, uuid)` whose result type, as `pg_get_function_result` spells it, is
     exactly drugref db/068's: `TABLE(outcome text, match_kind text, reason text, group_class_uuid uuid, group_name
     text, group_mesh_ui text, review_status text, allergen_component uuid, prescribed_component uuid)` (ruling
     T2R-b).

   **Ruling T2-a.** The door reads all of this from the system catalogue: `pg_namespace`, `pg_class` and `pg_proc`
   rows, then `has_schema_privilege`, `has_table_privilege` and `has_function_privilege` on the oids it found. It never
   uses `to_regclass`, because `to_regclass('drugref.x')` *raises* "permission denied for schema" when the schema
   exists but the owner lacks USAGE on it, which is exactly the "installed but unreadable" case that must answer
   false. So `cairn_drug_available()` never raises.

   A drugref from before allergy groups (no `allergy_group`, no `allergy_check`) is "not there".
9. **`cairn_drug_search(p_text text, p_limit int) RETURNS TABLE(system text, code text, display text)`**, the pick.
   - **Matching.** Text shorter than 3 characters after trimming answers nothing. LIKE's `%`, `_` and `\` are
     escaped, so the text matches literally. A drug matches when its name, or a word in it (split on spaces), starts
     with the text. A group matches anywhere in its name.
   - **Groups (M5).** The only groups offered are drugref **allergy groups**: `drugref.allergy_group` joined to
     `drugref.substance_class`, answered as `system` `drugref-class`, `code` the `class_uuid` and `display` the
     `class_name`. A MED-RT class such as "Penicillin-class Antibacterial [EPC]" is never offered, because
     `allergy_check` answers `allergen-not-a-group` for one and an allergen coded with it could never be checked.
   - **Order.** Groups come first, then drugs (ruling F2, #1057). There are few groups, and a class allergy is the
     commonest intent. With the real drugref, "penicillin" matches more drugs than the window shows, so a group listed
     after them was cut off. Within each tier, shorter names come first, then byte order (`COLLATE "C"`, so every
     node orders alike).
   - **Limit.** At most `least(p_limit, 20)` rows. The window asks for 12.
10. **`cairn_drug_facts(p_allergens uuid[], p_meds uuid[]) RETURNS TABLE(allergen uuid, med uuid, outcome text,
    match_kind text, group_name text)`**, the check's answers.
    - **What it returns.** It takes every distinct non-null (allergen, medication) pair, calls
      `drugref.allergy_check(med, allergen)` (drugref's argument order: the prescribed drug first), and returns each
      row's `outcome`, `match_kind` and `group_name` unchanged, one output row per drugref row. It drops drugref's
      `reason`, the group's uuid, MeSH root and `review_status`, and the path's two component uuids, none of which the
      window shows.
    - **drugref owns the fold (M4).** Whether a salt or hydrate is "the same drug", and which drugs an allergy group
      holds, is drugref's question. drugref folds **every** composition component, not only the pharmacologically
      active ones: its owner ruled that activity is not what triggers an allergy. Cairn keeps no fold, membership or
      class walk of its own, so a wrong answer is fixed in drugref, in one place, for every client.
    - **Replay.** The door's shape changed during the build. A guarded `DROP FUNCTION` drops any shape other than the
      current one, and a converged node's replay leaves the current one untouched, so every connect converges.
11. **`cairn_drug_group_status(p_groups uuid[]) RETURNS TABLE(group_code uuid, review_status text)`** returns each
    named allergy group's `drugref.allergy_group.review_status`. drugref's two words today are `structural-seed` and
    `reviewed`, and every group is `structural-seed`. A code that is not an allergy group has no row. The search, the
    facts and the status doors each answer zero rows when `cairn_drug_available()` is false.

    **The contract, as drugref states it** (db/068, db/069). `allergy_check(p_prescribed, p_allergen)` answers in
    one of two ways:
    - **`match`**: one row per path, with `match_kind` `same-drug` (the two folds share a component) or `same-group`
      (an allergy group holds a component of each side, or the allergen *is* the group and holds a prescribed
      component). A `same-group` row carries the group's `class_uuid`, name, MeSH root and `review_status`.
    - **Otherwise exactly one row**: `no-match` (both sides placed in MeSH, nothing shared) or `unplaced`, with a
      `reason`:
      - `unknown-prescribed`;
      - `unknown-allergen`;
      - `allergen-not-a-group`;
      - `allergen-in-no-group`;
      - `prescribed-not-in-mesh`.

    drugref says that `unplaced` means "not checked" and must never be shown as safe.

    **drugref's known gap.** There is no penicillin–cephalosporin cross-allergy rule, so cefalexin against a
    penicillin allergen answers `no-match`. drugref states that this "is not a statement that cephalosporins, or any
    other beta-lactam (e.g. carbapenems), are safe" ((a), #1055).
12. **What availability cannot see, and why callers map an error to "unavailable" (ruling T2R-b).** `allergy_check` is
    invoker-rights `LANGUAGE sql`. Inside the definer door it runs as the door's owner, and it reads drugref's
    *internal* tables: `moiety_allergy_fold`, `allergy_group_member`, `allergy_mesh_placement` and the composition
    tables behind the fold. Those tables are drugref's implementation, not its interface, so `cairn_drug_available()`
    does not list them. An owner who can read the three interface tables but not one of these makes
    `cairn_drug_facts` **raise**. A drugref whose interface table has a renamed column makes the search or the status
    door raise in the same way.

    So every caller maps an error from the doors to "no drug list":
    - `read_drug_facts`'s `Err` becomes `DrugFacts::Unavailable` (`allergy_check::check_list`);
    - a failed search becomes `available: false`;
    - a failed status read runs under a savepoint and becomes an empty map.

    A fault in drugref never stops the medication list or the allergy list drawing.
13. **The trust boundary.** Each time the window reads a medication list, `drugref.allergy_check` runs as the door's
    owner, who in practice is the Cairn schema's owner, normally a superuser. **Whoever can replace
    `drugref.allergy_check`**, or the drugref objects it reads, can therefore run SQL with the door owner's rights.
    drugref must be owned by the door's owner or by an equally trusted role. RUNBOOK §29 installs it with
    `pg_dump --no-owner`, restored as the owner, so the owner owns it.
14. **The exact-shape check is strict on purpose.** `cairn_drug_facts` reads `allergy_check`'s columns by name, so
    another shape would raise there. The cost is that even an *additive* change to `allergy_check`'s `RETURNS TABLE`
    (a new column) reads as "no drug list", and every line then reads "not checked", until db/076's string is
    updated. That direction is the honest degrade, never a silent clean. drugref is asked to freeze or version
    `allergy_check`'s shape (#1055).

### Reading the answers (`cairn_node::allergy::{read, drug_facts, thread}`)

15. **The read.**
    - `AllergyRow::coding` reads `clinical_allergy.coding`. A JSON object that lacks a key reads as `None`, never as a
      fault.
    - `MedicationRow` gains `coding_system` and `coding_code`, both `#[serde(skip_serializing)]` (ruling T3-a). The
      CLI's `--json` golden holds a coded row and stays byte-identical; only the Rust check reads the two fields.
    - `read_drug_facts` asks `cairn_drug_available()` first, and then `cairn_drug_facts` on the same client. It folds
      each pair's rows (`fold_rows`):
      - any `match` row makes the pair `Answer::Match`, keeping every match row's path;
      - otherwise the pair is `NoMatch` when every row is `no-match`;
      - otherwise it is `Unplaced`, which covers an outcome word this build does not know.
    - A pair with no row is absent from the map.
16. **Two accounts are the same allergen (D10)** when their terms match by ADR-0097 decision 16's folding **or** they
    carry the same `(system, code)` (`thread::same_allergen_rows`). The "differing accounts" test between heads of
    *different* threads uses this rule now. Between sibling heads of *one* thread (a fork of concurrent corrections)
    ADR-0097 decision 16's rule stands unchanged, with the coding counted among the recorded words: they compete when
    their terms differ by that folding even if both carry one code, and when they differ only in their coding — a
    shared code makes two threads one allergen, never two versions of one record the same words
    (`a_coded_fork_competes_on_its_words_and_its_coding`).

### The check (`cairn_node::allergy::check`, pure)

17. **What a line can say (M6).** Only positive checks are named.
    - **A confirmed allergy hit** gets the loud `Allergy` tag and its words.
    - **A previous adverse effect** that meets the line gets a muted note.
    - **A line that could not be fully checked** gets the small `not checked` tag and no words of its own.

    The strip lists nothing unverified, and the design's per-allergen "medications not fully checked" line is dropped.
    The tags and words are visible text on the line, never a hover-only `title` (#871).

    `LineCheck`'s invariant is that `unverified` is false whenever there is a mark, because the loud tag already sends
    the clinician to the allergy list. A line with nothing to say has no entry in the answer.
18. **Which heads take part (D9).**
    - **Every live head.** That includes a doubted chart's heads and a suspect key's heads, because a suspect act may
      only add a warning (ADR-0097).
    - **Allergen kinds.** Every `allergen_kind` except `other` takes part, including a peer's newer kind (ruling
      T4-e). An `other` allergy, such as latex, never touches the medication list.
    - **Entry kinds.** An `adverse-effect` head gives notes. Every other entry kind counts as an allergy, including a
      peer's word this build does not know (ruling T4-d). Over-warning is the safe error.
    - **What is left out.** De-labelled and withdrawn entries check nothing, because they are not live heads. A
      ceased line is never checked. A withheld line is current, so it is checked.
19. **A hit (D7).** A pair (head, current line) is a hit when any of these holds:
    - the head's term matches the line's term or the line's coding display (`same_allergen`);
    - the head is coded `drugref-moiety` with the line's own moiety code, which works with no drugref at all;
    - drugref answered `match`.

    Ruling T4-a: a `match` whose paths are not all `same-group` is worded as the same drug. That covers `same-drug`,
    an empty kind and a newer word. Otherwise the first named group is used.
20. **One mark per allergen, worded with its most severe live account (D7, ruling T4-f).** An allergen is a connected
    component of heads under `same_allergen_rows`, closed transitively (`allergen_groups`). The relation itself is not
    transitive: uncoded "Augmentin" and coded "Augmentin" share a term, and the coded one shares a code with
    "amoxicillin". The heads are sorted by the expanded list's order: reaction rank, then newest, then event id. In
    that rank `not-known` comes directly after anaphylaxis and before angioedema, so it outranks every known reaction
    but one, and a word this build does not know comes last ((e), #1056).

    Components are built separately over allergy-kind and adverse-effect heads, so "most severe account" means the
    most severe *allergy-kind* account; an adverse effect of the same allergen is its own note and never words the
    mark. The reaction words come from the component's most severe account. The sentence comes from the first account that
    hit. The words are exact (`reaction_phrase` gives `{reaction}`; `{line name}` is `MedicationRow::display_name()`):
    - **The same drug** (by name, by code, a path that is not purely `same-group`, or a moiety allergen's
      `same-group` path that names no group): "Allergy recorded: {allergen term} → {reaction}".
    - **A group allergen with a `same-group` path**: "{line name} is a {allergen term}; allergy recorded: {reaction}".
    - **A moiety allergen with a `same-group` path**: "{line name} is in the same group as {allergen term} ({group
      name}); allergy recorded: {reaction}".
    - **An adverse effect**: "Previous adverse effect: {allergen term} → {reaction}". This is a note, never a mark,
      even when drugref says `match`.
21. **Not checked (D8; rulings R1–R3).** A current line is unverified when it has no mark and either of two things
    holds. The first is that the patient holds an allergy record this terminal cannot read
    (`PatientAllergies::unreadable > 0`, passed to `check_medications` as `unread_records`; #1059). Such a record has
    no head, but it may be the allergen, so every unmarked current line is unverified, whether the drug list was asked
    or not. The second is that at least one live allergy-kind head was neither a hit nor answered `no-match` for the
    line. That happens when:
    - either side is uncoded;
    - the allergen is coded under a system other than the two;
    - there is no drug list;
    - drugref answered `unplaced`, for any reason, `allergen-in-no-group` included (R1, after drugref's own rule that
      `unplaced` means "not checked" and must never be shown as safe);
    - there is no answer row.

    **Only `no-match` is a verified negative (R2).** Its known gap, the missing cross-allergy rule, is stated here and
    not on the screen. Adverse-effect heads never make a line unverified (R3, design risk 3).

    **Never trim the tag to hits only.** Removing it would make silence read as checked (design risk 2).
22. **A group's review status shows in the expanded allergy list only (M7, D11).** The suffix is
    `" (drug group not clinically reviewed)"`, added after an entry line coded to a group whose status is not
    `reviewed` (`unreviewed_groups`, `note_unreviewed_groups`). That covers `structural-seed`, a status word this build
    does not know, a group the door does not return, and a failed read. Ruling T4-c replaced the plan's wording
    "(group from drugref, not clinically reviewed)" rather than build the word "drugref" past the dependency guard. The
    strip and the medication line stay terse.

### The window (`cairn-gui-tauri`: `allergy_check.rs`, `allergies.rs`, `allergy_pane.rs`; `drug_pick.js`, `allergy_tags.js`)

23. **The pick** (`drug_search`, `drug_pick.js`).
    - **When it asks.** The pick shows only while the allergen kind pressed is *Drug*. It asks `drug_search` 150 ms
      after the last keystroke, once the trimmed text has at least 3 characters.
    - **Choosing.** The arrow keys move the highlight. Enter chooses the highlighted option, a click chooses the
      clicked one, and Escape closes the list. Groups are listed first (decision 9). A group's option reads
      "{display} (group)", unless its name already contains the word "group" (a whole word, any case). drugref's
      "Penicillins (allergy group)" therefore reads as it is.
    - **What a choice does.** The term becomes the option's display and the coding is held. **Any later edit of the
      term drops the coding, and so does leaving *Drug*.**
    - **What it is coded as (ruling F3).** While a coding is held, a visible line under the Substance field says what
      the entry will be signed as. A drug reads "Coded as: {display} (drug)", and a group reads "Coded as: " followed
      by its option's words ("Coded as: Penicillins (allergy group)", "Coded as: Cephalosporins (group)"). Within the
      drugs, shorter names come first, so "amox" lists amoxapine before amoxicillin, and one arrow press too many
      codes the wrong drug. The line shows that before *Sign*, as text and with no dialog. `setHeldCoding` is the only
      writer of the held coding and draws or hides the line with it. The line therefore goes whenever the coding goes:
      on an edit, on leaving *Drug*, and when the form is emptied (a sign included). A correction's held coding shows
      it too.
    - **No drug list.** `available: false` covers fixture mode, no database, a lost connection, no usable drugref, and
      a search that raised. It shows "No drug list on this terminal: the name is recorded as typed" (a raising search
      looks the same, #1058).
24. **Recording a coded entry, before the key** (`allergy_record`'s `coding`).
    - **Not a drug.** A pick sent on an entry that is not a drug is refused before the key with "Only a drug can be
      picked from the drug list — choose Drug, or type the name as it is." (ruling T5-d). The floor would refuse the
      entry, so no passphrase is asked first.
    - **The system.** A pick under any system but `drugref-moiety` or `drugref-class` is refused before the key with
      "That pick is not a drug or drug group from the drug list; type the name instead". That covers a webview defect
      and the reserved `drugref-product`.
    - **The code.** It is re-spelled as the canonical uuid. One that does not parse is refused with "That drug's code
      could not be read; type the name instead".
    - **Correct.** *Correct* goes through `allergy_record` too. It keeps the line's coding (`LineWords::coding`, ruling
      T6-a) until the term is edited, and the pick follows the pressed kind, never a cached flag.
    - **After a sign.** After any allergy act answered `Ok`, the window reads the medication list again, so the tags
      follow the allergy list.
25. **The check on the medication pane** (`med_list` → `allergy_check::med_list_check`).
    - **How it reads.** The pane reads the allergy list in its own snapshot, then asks the door on the window's
      connection. It hands each line's `LineCheck` to `chart_pane` as `MedListRowView.allergy_marks`,
      `.allergy_notes` and `.allergy_unverified`.
    - **When the allergies cannot be read.** A failed allergy read gives `ChartPane.allergy_check_failed`: "Allergies
      could not be read: medications not checked against them", above a list that still draws, and every current
      line on it carries the "not checked" tag, as one unreadable record leaves it (#1059) — a banner alone scrolls
      away on a long list (`a_failed_allergy_read_keeps_the_list_and_says_so`). So does an allergy
      read whose chart set differs from the medication list's, when a link or unlink landed between the two reads
      (ruling T5-c).
    - **Which moiety a line names.** A line names its moiety only when it is coded `drugref-moiety` and its threads
      agree on the coding. A coding-conflicted group shows one of its codings, and checking that one alone could read
      clean.
    - **Fixture mode** checks nothing and says nothing.
26. **The tags are drawn on the patient's own medication list only** (ruling T5-f). `allergy_tags.js`'s `nameCell`
    and `allergyRows` are called only by `main.js`'s `renderRow`. The duplicates and link comparison views build
    their rows with `allergy_unverified` false and have their own renderers, so a row that was never checked can never
    look checked and clean there.

## What is not guaranteed

- **(a) `no-match` is not "safe" for cross-reactivity.** A penicillin-allergic patient's cefalexin line carries no tag
  at all (ruling R2), because drugref has no cephalosporin or carbapenem cross-allergy rule yet. The rule is drugref's
  to build, and #1055 tracks Cairn's side: the shape string, the wording of a new `match_kind`, and the mark-or-caution
  decision. Until it lands, this is a known under-warning, written here because a tag on every beta-lactam would need a
  class rule that Cairn must not keep (M4).
- **(b) The check trusts drugref's fold and groups.** A drug that drugref places in no group, or does not place in
  MeSH, reads "not checked", which is honest. A group that drugref under-fills answers `no-match` for a placed drug
  it should hold, and the check takes that as a verified negative. Every group today is `structural-seed`, and that is
  said in the expanded allergy list only (decision 22), never on the medication line.
- **(c) Availability is the owner's view of the interface only.** An internal table the owner cannot read makes the
  facts door raise, and the caller then degrades to "not checked" (decision 12). On screen a raising door looks the
  same as no drugref at all; only stderr tells them apart.
- **(d) The full `med_list` command path against a real drugref is not run by any test** (ruling T5-b). The DB-gated
  window tests drive `check_list`, the function `med_list_check` hands its connection to, on a rolled-back
  transaction with the stub installed. The whole command opens its own connection and cannot see an uncommitted stub,
  and a committed stub on the shared test database would break other tests. RUNBOOK §29 owes it.
- **(e) An unknown reaction word ranks last** (`view::category_rank`, from slice 1). A peer's severe new word beside an
  older "rash" can make a mark read "→ rash" (#1056).
- **(f) There is no check at the moment of charting.** The window cannot chart a medication yet. That slice reuses
  this matcher (#1054), and its ADR records the GUI milestone's "prescribe means charted".
- **(g) A wrong pick is made visible, not prevented.** #1057 was fixed before merge. Groups are listed first
  (decision 9), so the real "Penicillins (allergy group)" heads the list for "penicillin". Its option reads
  "Penicillins (allergy group)", with no second "(group)", and a mark reads "amoxicillin is a Penicillins (allergy
  group); …". The stub now carries that real name, so the door and window DB tests use it. The label rule itself is
  pinned by a source guard and was seen in the headless walk. Within the drugs, the order is
  length, not likelihood: "amox" lists amoxapine before amoxicillin. Nothing stops a pick of the wrong drug. The
  "Coded as" line (decision 23) shows it before *Sign*.
- **(h) The window's commands against a database are pinned only by DB-gated tests** (`allergy_check_db_tests.rs`),
  which CI does not run (#905). The webview is pinned by source guards (`drug_pick_webview_tests.rs`). It was walked
  headlessly in Chromium with a stubbed `invoke`, and the pick's labels and "Coded as" line were walked again in
  Chromium and WebKit. Still owed to RUNBOOK §29: the real Tauri argument names, the real
  window's WebKit, VoiceOver on both tags, and the budgets.
- **(i) The allergy list and the medication list are read in two snapshots.** A changed chart set is caught
  (decision 25), but an allergy signed elsewhere between the two reads shows on the next read. Each pane is consistent
  in itself.
- **(j) A drug search's word match splits on spaces only** ("clav" does not find "amoxicillin/clavulanate"; #1058).
- **(k) An allergy record this terminal cannot read makes lines "not checked", never more** (#1059, fixed before
  merge). An event held without custody, or shredded, has no projected row, so it is no head.
  `PatientAllergies::unreadable` counts it, and the strip leads with "… cannot be read on this terminal". The check
  cannot say which line the unread record would mark. So every current line without a mark reads "not checked"
  (decision 21), and a line that a readable allergy marks shows only its mark. Before the fix, `check_list` never read
  that count. With no readable heads every line had no tag at all, and silence read as checked for exactly the reader
  whom decision 4 leaves without a safety signal.

## Consequences

- **Easier.** An allergy coded to the Penicillins group warns on a coded amoxicillin line, in words, with no click,
  through drugref's group. Without drugref, a line still warns by name or by identical moiety code. A line nobody could check says so with one small tag, so silence never reads as checked. The
  steps (paper *N* → built *M* → target *K*):
  - **Recording a coded entry:** 1 (write the line) → 1 (one signed event carrying the coding) → 1 (type, choose or
    keep typing, Sign).
  - **Checking the list:** 1 (read the box against the chart) → 0 (computed on every read) → 0.

  There is no `M > N`. The budgets, owed by a human in RUNBOOK §29, are the plan's §1.2: a coded entry in ≤ 15 s, a
  free-text entry in ≤ 10 s, the pick within ≤ 300 ms of the third letter, a hit legible with no click or hover, and
  the medication pane's open time growing by no more than 100 ms with drugref present.
- **Harder.**
  - Every drugref release that touches `allergy_check`'s shape needs a db/076 change in step (decision 14).
  - A deployment that installs drugref takes on the trust boundary of decision 13.
  - A slice that emits a clear safety signal for allergies must carry the coding onto it pre-seal, as ADR-0059 says
    for medications.
- **The bet.** The bet is that delegating the fold to drugref, and naming only positive checks, gives a warning that is
  both trusted and read. We would know it is wrong in either of two cases:
  - clinicians learn to ignore the `not checked` tag, because it sits on most lines of a typical chart. The fix would
    then be coding, never removing the tag;
  - a drugref answer is wrong and nobody can tell, because Cairn shows no path. drugref's `allergen_component` and
    `prescribed_component` are dropped by the door today.
- **Pinned by:**
  - the floor and the doors: `db/tests/076_drug_read_door_test.sql`, `db/tests/074_clinical_allergy_test.sql`, and
    `crates/cairn-node/tests/{drug_read_door,drug_read_door_shape,drug_answers_read,drug_door_callers,no_drugref_dependency}.rs`;
  - the wire and the read: `crates/cairn-event/src/allergy.rs`'s tests, `crates/cairn-node/src/allergy/drug_facts.rs`'s
    tests and `thread_tests.rs::same_allergen_rows_matches_on_code_or_term`;
  - the check: `crates/cairn-node/src/allergy/{check_tests,check_unverified_tests}.rs`;
  - the window: `cairn-gui/cairn-gui-tauri/src/{allergy_check,allergy_check_db,allergies,drug_pick_webview}_tests.rs`.

  RUNBOOK §29 carries the human measurements.
