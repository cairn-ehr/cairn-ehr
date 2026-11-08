# ADR-0096 — The allergy list

- **Status:** Accepted (supersedes no decision; builds slice 1 of the allergy design's §5)
- **Date:** 2026-11-09
- **Spec version at acceptance:** 0.98
- **Issues:** draft PR [#1026](https://github.com/cairn-ehr/cairn-ehr/pull/1026) (allergy slice 1). Design:
  `docs/superpowers/specs/2026-11-08-allergy-stream-design.md`; plan:
  `docs/superpowers/plans/2026-11-08-allergy-slice-1-stream-and-strip.md`. Touches
  [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) and [#401](https://github.com/cairn-ehr/cairn-ehr/issues/401)
  (decision 21), [#814](https://github.com/cairn-ehr/cairn-ehr/issues/814) (decision 18) and
  [#905](https://github.com/cairn-ehr/cairn-ehr/issues/905) (not guaranteed (h)). Filed by its review:
  [#1027](https://github.com/cairn-ehr/cairn-ehr/issues/1027) (no unconfirmed / repudiated-key marker on a byline),
  [#1028](https://github.com/cairn-ehr/cairn-ehr/issues/1028) (a refusal before the key clears the passphrase),
  [#1029](https://github.com/cairn-ehr/cairn-ehr/issues/1029) (no length limit, not guaranteed (e)),
  [#1030](https://github.com/cairn-ehr/cairn-ehr/issues/1030) (the unrecognised-word suffix and count, (f)),
  [#1031](https://github.com/cairn-ehr/cairn-ehr/issues/1031) (a test grants a power to a shared role).
- **Builds on:** [ADR-0052](0052-born-sealed-clinical-bodies.md) (born sealed) ·
  [ADR-0053](0053-per-write-human-authorship.md) (the human signs, the node seals) ·
  [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) (strict submit, lenient apply) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (a newer peer never wedges sync) ·
  [ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) (the `substance` shape) ·
  [ADR-0063](0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) (the clear safety signal) ·
  [ADR-0076](0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md) (a linked
  chart reads as one) · [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) (the window's own login)

## Context

On paper an allergy is written in the box on the drug chart and on a red sticker on the folder: "Penicillin —
anaphylaxis 2014", or "NKDA", initialled. Until this slice Cairn had no allergy stream. The only record of a reaction
was a medication's free-text cessation reason (db/060), shown only on hover (#871).

The maintainer's framing (design §1): the allergy failure with the most clinical consequence is a **false or
inflated** one. A minor adverse effect, such as loose bowel motions on metformin, is recorded as "allergy", and it
then stops an inexperienced junior from prescribing a drug the patient needs. So the list must make two things
legible at 3 a.m.: what is dangerous, and what is not.

This slice builds the first of the design's three slices: two event types, their floor, projections and shred hook
(db/074); signing and reading in `cairn_node::allergy`; a pure view; and the strip and its form in the window's
patient header. The first signed `clinical.allergy.asserted` freezes its shape forever (principle 11), so what went
on the wire, and what was left out, is written down here. The build took seventeen rulings (R1–R17), which the window's
code cites by number where it relies on one; each that bears on the architecture is stated below under its number.

## Decision

### The wire (`crates/cairn-event/src/allergy.rs`, frozen)

1. **Two event types, both `additive` and targeting no other author** (db/074 step 1), born sealed like every
   `clinical.*` type (ADR-0052):
   - `clinical.allergy.asserted`, `schema_version` `clinical.allergy/1`, payload exactly
     `{allergen_kind, substance: {term, coding}, kind, reaction: {category, text}, onset: {precision, value}, source,
     refs}`. `allergen_kind` is `drug` | `other`; `kind` is `allergy` | `adverse-effect` (an intolerance is an adverse
     effect: two kinds, not three); `reaction.category` is `anaphylaxis` · `angioedema` · `urticaria` · `rash` ·
     `gi-upset` · `other` · `not-known`, with `reaction.text` a string or null; `onset.precision` is `day`
     (`YYYY-MM-DD`) · `month` (`YYYY-MM`) · `year` (`YYYY`) · `not-known` (value null); `source` is `patient-stated` ·
     `seen-documented` · `another-record`.
   - `clinical.allergy-none.asserted`, `schema_version` `clinical.allergy-none/1`, payload exactly
     `{scope}`, `drug` | `all`. "None known" is a **statement**, never an absence (design §3.4).
2. **Times.** The envelope's `t_effective` is unset on both: a reaction's time is `onset`, inside the body. When a
   "none known" statement was asked is the event's own HLC wall; there is no separate field.
3. **Authorship.** Both go through `seal_sign_submit` (the medication stream's sealed submit path) under the human's
   key, with the contributor set `{human, authored}` + `{node, recorded}` (ADR-0053). `cairn_node::allergy::sign`
   commits nothing itself.
4. **The twin is the strip's wording, its onset in ISO form** (R17). `entry_words_dated` is the one wording of an
   entry: the signed twin (`render_allergy_twin`) is it with the onset's ISO value — "Allergy: penicillin →
   anaphylaxis · 2014", "Previous adverse effect (2019-03): metformin → GI upset", nothing when the onset is not
   known — and the window's line is it with the onset as a clinician reads it ("Mar 2019"), so they cannot drift (R1:
   it takes wire words, not enums, so a peer's newer word reads as itself). The "none known" twins are "No known allergies" and "No known drug allergies".
5. **Two reserved slots are written empty:** `substance.coding` is `null` (drugref coding is slice 3) and `refs` is
   `[]` (corrections are slice 2), so filling them later is additive.

### The floor (`db/074_clinical_allergy.sql`)

6. **Two tiers, the db/041 idiom** (strict submit, lenient apply). Each rule raises its own message.
   - **Structural, at both doors:** the payload is an object; every key is present with its JSON type;
     `substance.term` is a string that is not blank by `cairn_twin_is_present` (the twin's Unicode-exact test, so
     differently-collated nodes agree); `substance.coding` is present, and is null or a coding object that
     `cairn_check_coding_object` (db/041) accepts; `reaction.text` is present, a string or null; `onset.value` is null
     exactly when `onset.precision` is `not-known`, else a string; `refs` is an array; `scope` is a string. No node
     can read a body that fails these.
   - **Closed vocabularies, at the authoring door only** (`cairn.remote_apply` is not `'on'`): the `schema_version`;
     every enumerated word above; the shape of `onset.value` for its precision (a `day` must be a real date, and no
     precision admits the year `0000`); an onset after the recording day; `substance.coding` must be null and `refs` empty. At the remote door a newer peer's
     word is admitted and projected as itself: refusing it would wedge sync on every node older than its author
     (ADR-0056). That is why the projection's vocabulary columns carry no `CHECK`.
7. **"After the recording day" is judged against UTC+14** (`cairn_allergy_onset_problem`, R5). The database does not
   know the terminal's time zone, and UTC+14 is the furthest-ahead zone, so a date after that day is in the future
   everywhere; a clinician who types today's date before the database's own day has begun is never refused. The
   window makes the stricter check before the key, against the terminal's own calendar day at the precision typed
   (`allergies::parse_onset`: "11/2026" in November 2026 is not the future), and refuses the year `0000` at every
   precision, naming what was typed.
8. **`schema_version` is checked at the authoring door only, and the applier projects every version.** This departs
   from db/064, where a note's remote door skips its checks for a newer profile and `clinical_note_apply` projects
   `clinical.note/1` alone. A note's narrative is rendered as markdown, so a relabelled `/2` body could put
   unchecked markup on screen; allergy text is plaintext drawn with `textContent` only (decision 20), and withholding
   a newer version's allergy would hide a possible allergy, the unsafe direction. The trade-off: the structural tier
   runs on every version, so a future `/2` that changed the type of a `/1` key (an `onset.value` that is a number,
   say) would be refused at the remote door and stall sync from its author. Additive-only evolution (principle 11)
   already forbids that change.
9. **Two projections, one row per event** (`clinical_allergy`, `clinical_allergy_none`), keyed by `event_id`, filled
   from the clear payload by `ON CONFLICT DO NOTHING` appliers registered `heal_safe` at run order 20; a node with no
   custody of a body projects nothing for it, and the late-custody path re-runs the applier when a key lands. The
   runtime role `cairn_agent` (which `cairn_window` inherits) reads both. The shred hook in db/037 deletes both
   tables' rows by content address, and db/044 gains the timing kinds `allergy-record` and `allergy-none` in place.
   Both types stay off db/048's no-thread list, so they keep the conservative sensitivity bound.

### Reading (`cairn_node::allergy::{read, view}`)

10. **The union of every linked chart's rows, read from the projections in the window's one snapshot**, with the
    chart set (`person_charts`) the pane is sent back with on every write, and `unreadable`: the count of
    allergy-stream events on those charts that `event_log` holds with no row in their projection. An applier projects
    every body it can read, so such an event is one this node cannot read: no custody of its sealed body, or shredded
    (the shred deletes the row, and the honest statement then is still "cannot be read").
11. **A chart whose link is in doubt: shown, marked** (the designer's call, put to the plan review and built as
    planned). Notes and
    medications withhold a doubted link's other charts; an allergy that might be a stranger's is still shown, because
    over-warning is the safe error here. While the set holds a doubted link, every member chart but the opened one is
    named in `doubted_charts`, and each of its lines, on the strip and in the expanded list, ends "(on a chart whose
    link to this one is in doubt)".
12. **The lists.** The drug list holds rows whose `allergen_kind` is `drug`; the other list holds every other row,
    including a newer peer's allergen kind (a `food` allergy files under Other and its line ends " (food)"). An
    adverse effect goes to its own list. A row of a `kind` this node does not know is listed after the allergies of
    its list, never prominent. Only an entry of kind `allergy` is prominent (the red sticker). Within a list:
    anaphylaxis, not known, angioedema, urticaria, rash, other, GI upset, then an unknown category; then newest by HLC;
    then event id.
13. **The override rule is per list** (design §3.4: an allergy overrides "none known" "in its list"; R16 reversed the
    plan's "an `all` statement is one statement" rule, which made "No known allergies" plus a latex allergy read
    "Drug allergies not yet asked"). A recorded allergy overrides a "none known" statement in its own list, with no
    flag (maintainer): a `drug` statement is overridden by a drug allergy; an `all` statement stands for the drug list
    while no drug allergy exists, and for the other list while no other allergy exists. An adverse effect never
    overrides; a row of an unknown kind never overrides; an allergy of an unknown allergen kind is an other allergy, so
    it overrides an `all` statement's other half. A statement of a scope this node does not know covers no list. For
    each list the newest statement standing for it sets its status, and the expanded list opens on that statement
    with who signed it (paper's initialled NKDA): "No known allergies — asked 8 Nov 2026 by Dr Bea" when one statement
    stands for both lists, else "No known drug allergies — …" and "No known other allergies — …" for the list each
    stands for; " by …" is left out when this node cannot name the key. A statement standing for neither list
    (overridden in every list it covers, or superseded by a newer one) stays readable as "Previously recorded: no
    known allergies (asked 8 Nov 2026 by Dr Bea)".
14. **The strip is never blank.** Its states:

    | Recorded | The strip reads |
    |---|---|
    | nothing | **Allergies not yet asked** |
    | `all` | **No known allergies (asked 8 Nov 2026)** |
    | `drug`, nothing on others | **NKDA (asked 8 Nov 2026) · other allergies: not yet asked** |
    | `drug` + a latex allergy | **NKDA (asked 8 Nov 2026) · Other: latex → urticaria** |
    | `all` + a latex allergy | **No known drug allergies (asked 8 Nov 2026) · Other: latex → urticaria** |
    | `all` + a penicillin allergy | **Allergy: penicillin → rash · no known other allergies (asked 8 Nov 2026)** |
    | `drug`, and one record this terminal cannot read | **1 allergy record cannot be read on this terminal · NKDA (asked 8 Nov 2026)** |

    A record this terminal cannot read (decision 10's `unreadable`) LEADS the strip — "1 allergy record cannot be read
    on this terminal", "3 allergy records …" — and then no "not yet asked" phrase is shown, since that record may be
    the answer; a readable "none known" statement still shows, after the warning. The window shows such a strip
    prominent and hides the strip's one-press "none known" buttons.

    Entries within a list join with "; " and sections (the drug part, "Other:", the counts) with " · " (R7), because
    an allergy's onset already reads " · 2014" and two dated allergies would otherwise read ambiguously. In the Other
    section a line's leading "Allergy: " is dropped, since "Other:" carries it (R9); every other prefix stays. On the
    strip adverse effects are counted, never listed: "+1 adverse effect". Rows of an unknown kind are counted too, "+2 unrecognised
    entries" (R8): never prominent, and never overriding a statement. A drug list with nothing asked beside an other
    list with entries reads "Drug allergies not yet asked · Other: …". Until a chart's first read lands the strip
    says "Reading the allergy list…", and a failed read says "Could not read the allergy list: " and why, never an
    older list.
15. **Dates are the terminal's calendar day** (R3): `build_view` takes the UTC offset the window passes
    (`chrono::Local::now()`'s), never the database's. A day reads "8 Nov 2026" and an onset "14 Mar 2019", "Mar 2019"
    or "2019", so day and month cannot be confused.

### Writing (`cairn-gui-tauri`'s `allergies.rs` and `allergies.js`)

16. **One form, one Sign; one press for "none known".** The form starts with nothing pressed for "What kind" and
    "What happened" (design §3.2's anti-goal: a default would sign an intolerance as an allergy by a press that chose
    nothing); the allergen defaults to Drug, the source to Patient stated, and an empty "When" is not known. The two
    "none known" buttons are on the strip only while both lists are "not yet asked" and no record is unreadable, and
    always in the expanded list.
    A write is refused, in this order: not the chart on screen; not the chart set the list was read over; fixture
    mode; then, before the key is asked for, "Name the substance.", "Choose Allergy or Adverse effect.", "Choose what
    happened, or Not known.", an onset the window cannot read or that is after today; and a wire word the window should
    never send is refused naming it. Each of those four form refusals is shown under its own field. Then one signed event, timed into
    `ui_gesture_timing` as `allergy-record` or `allergy-none`, and the list read again.
17. **A sign that landed is an `Ok`, never an error** (R11). Both writes answer `AllergyWrite { outcome, pane }`:
    "Recorded." with the list read again, or, when that read failed, `pane` null and "Signed — but the allergy list
    could not be read again (…). Use “Read this chart again”." The window then says that sentence where the strip was,
    takes the lists down and allows no write until a list draws; after an entry, the form is emptied either way. An error would offer a retry, and a
    retry would record the allergy twice.
18. **An outcome that is not known is said so** (#814, worded as the note sign's): "Whether this allergy was recorded
    is NOT known — no ordinary answer came back while saving it (event …: …). Once the window can reach the node again,
    the chart shows whether it is there.", with a re-read and never a live retry; the form is emptied so the same
    entry cannot be signed twice. A fault in the window's own script mid-press is said too ("This window failed while
    recording …"), with the form emptied and no write allowed until the chart is read again.
19. **A late answer for a chart the clinician has left touches nothing of the chart on screen** (R13). A write's answer, refusal or fault
    that arrives after the chart changed shows nothing, and in particular never outdates the read in flight for the
    chart now on screen; that left chart's next read says the truth.
20. **No markup.** Allergy text is plaintext from end to end: the twin is plaintext and the window draws every line
    with `textContent`; nothing is escaped or parsed.

### What this slice does not do

21. **No clear safety signal.** `allergy::sign` writes `safety: None` on both types. db/049's signal has no supersession: a ceased
    drug warns forever (#406) and a correction emits no signal (#401). An allergy signal emitted today would keep
    warning after slice 2's "entered in error" and "challenged: tolerated", making the metformin problem permanent on
    the wire. ADR-0063 decision 5 also puts class and severity in `safety_class_map`, which ships empty, and decision 7
    emits nothing for an uncoded medication, and every allergy is uncoded in this slice. Not emitting is additive-safe:
    a later event type version may emit. Every read in this slice is a custody read of the projections.
22. **No co-sign** (maintainer). A student's or supervisee's entry reads exactly like anyone's, with no "— not
    co-signed" marker; ADR-0094's note rule does not extend to allergies.
23. **Deferred by the design:** corrections, de-labelling ("challenged: tolerated"), entered in error and the
    differing-accounts flag are slice 2; drugref coding, drug groups and the medication check are slice 3.

## What is not guaranteed

- **(a) A reader without custody is told a record cannot be read, not what it says** (design §7 risk 1). With no
  clear signal (decision 21), a node or role that cannot decrypt an allergy body projects no row for it; its strip
  leads with "1 allergy record cannot be read on this terminal" (decisions 10 and 14), never a false "not yet asked",
  but it cannot say whether that record is an anaphylaxis, an adverse effect or a "none known" statement. An event
  not yet synced to this node is not counted, and a shredded one stays counted for good. Custody is normally held;
  the gap stays open until #406/#401 give the signal supersession.
- **(b) Nothing recorded can be withdrawn in this slice.** An entry signed on the wrong chart, or an adverse effect
  signed as an allergy, stays on the strip until slice 2's overlays; a "none known" statement can be overridden by an
  entry but not withdrawn.
- **(c) The byline is a name only.** An expanded line ends " · recorded {YYYY-MM-DD} by {name}"
  (`cairn_note_signer_name`: display name, else staff id); the design's unconfirmed-key marker (§3.8) is not shown,
  and the line's ISO day differs in form from the strip's "8 Nov 2026".
- **(d) The terminal's offset is today's.** `build_view` gets the terminal's current UTC offset, so a record made
  near midnight under the other side of a daylight-saving change can read one day off.
- **(e) Typed text has no length limit at the window.**
- **(f) The "+N unrecognised" count spans both lists and ignores doubt**, and the " ({allergen kind})" suffix is also
  added to a newer allergen kind's adverse-effect and unrecognised rows.
- **(g) A refused form clears the typed passphrase**, as every window writer does, so a slip on a shared terminal
  costs a retype.
- **(h) The window's commands against a database are pinned only by DB-gated tests** (`allergies_tests.rs`), which CI
  does not run (#905). The webview was checked by source guards (`allergies_webview_tests.rs`) and walked in Chromium
  and Playwright's WebKit with a stubbed `invoke`: the real Tauri argument casing, the real window's WebKit,
  VoiceOver and the time budgets are owed to RUNBOOK §27.
- **(i) Decision 21 is pinned for what this node signs, not for what its floor admits.**
  `crates/cairn-node/tests/allergy_sign.rs` asserts `event_log.safety IS NULL` on a signed entry and a signed
  statement (`safety_is_null`); db/074 adds no rule of its own about a body's `safety`.

## Consequences

- **Easier.** A clinician reads on the patient header, with no click, whether a dangerous allergy is recorded, and
  whether anyone has asked; an adverse effect is visibly not an allergy. Steps (paper *N* → built *M* → target *K*):
  read the allergies 0 → 0 → 0; record an entry 1 (write the line) → 1 (one signed event) → 1 (the form and Sign);
  "none known" 1 → 1 → 1 press. No `M > N`. Budgets (design §6), owed by a human in RUNBOOK §27: a free-text entry
  ≤ 10 s and "none known" ≤ 3 s, with the key held (the plan's gesture); a shared terminal's typed credential is
  timed beside them.
- **Harder.** Every reader of these rows must keep an "unknown" arm: vocabulary columns stay strings, so a newer
  peer's word reads as itself. A new allergen kind is an other allergy until this node learns it. A slice that emits a
  safety signal must first settle supersession (#406/#401).
- **The bet.** That one terse line, with adverse effects visibly quiet and "not yet asked" never mistaken for "none",
  stops both the missed allergy and the inflated one. We would know it is wrong if adverse effects are still signed as
  allergies (the kind choice is being pressed without reading), or if clinicians stop reading the strip because it is
  long.
- **Pinned by** `db/tests/074_clinical_allergy_test.sql`; `crates/cairn-event/src/allergy.rs`'s tests;
  `crates/cairn-node/src/allergy/view_tests.rs`; `crates/cairn-node/tests/{allergy_floor,allergy_sign,allergy_read,window_role_grants}.rs`;
  `cairn-gui/cairn-gui-tauri/src/{allergies,allergies_webview}_tests.rs`. RUNBOOK §27 carries the human measurements.
