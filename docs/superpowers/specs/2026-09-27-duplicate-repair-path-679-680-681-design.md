# Design — the duplicate repair path (#679, #680, #681)

- **Date:** 2026-09-27
- **Issues:** [#679](https://github.com/cairn-ehr/cairn-ehr/issues/679) (commit-time check) ·
  [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680) (worklist) ·
  [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681) (link gesture) · folds in
  [#334](https://github.com/cairn-ehr/cairn-ehr/issues/334) (cross-patient reconciled group)
- **ADR:** [ADR-0076](../../spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md) (spec v0.78) — D1–D6 below are its decisions 1–6; its decision 7 restates R4
- **Spec sections:** §5.2, §5.7, §5.8, §5.12, §5.13 ([identity.md](../../spec/identity.md))
- **Brief:** [ADR-0075](../../spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md)
  decision 2 — *the safety measure is how fast a duplicate is FOUND*.
- **Builds on:** [ADR-0049](../../spec/decisions/0049-commitment-based-sign-off-currency.md) (per-thread
  sign-off), [ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) (per-write human
  authorship), [ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)
  (the signed `displayed` list), [ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md)
  (the advisory matcher), the §5.7 C1/C2/C2b identity core (`db/018`, `db/019`).

## Context

ADR-0075 accepted that duplicate registrations happen (typos in hard names; the person at the desk
cannot be made to browse) and moved safety to the **window** — the time between a duplicate's
creation and its repair, in which an allergy can sit on chart A while a drug is charted on chart B.
The brainstorm (2026-09-27) surveyed what exists before designing, and found the brief's premise —
"repair by `link` is easy and safe" — rests on three things that are not there:

1. **A `link` repairs nothing a clinician can see.** The medication read filters
   `WHERE patient_id = $1` (`crates/cairn-node/src/medication/read.rs`); nothing reads across
   `person_member`. `person_chart` (db/018) is "thin by design … the REAL unified-chart read surface …
   deliberately out of scope". After a link, chart A still shows none of B's drugs.
2. **"These are different people" has no home.** The closed identity algebra has no such member, and
   `match_proposal` (db/017) is node-local and does not replicate, so a local `rejected` status is
   re-proposed by the hub sweep and by every peer.
3. **The matcher cannot run on its own.** It is a Python library: no CLI, no daemon, no
   LISTEN/NOTIFY, and no "score this one chart" mode — only a whole-population `sweep()` or a
   given pair (`runner.propose`). It also ignores `patient_link` entirely, so it re-proposes linked
   pairs and human-rejected pairs alike.

And two found while designing:

4. **#334 becomes the normal case.** `patient_medication_current` joins groups on `group_id` alone,
   so a reconciliation group spanning A and B shows **twice** on A and **not at all** on B. After a
   link, the same drug on both duplicates is exactly that shape; a combined read built on the view
   would show a doubled drug line — a double-dose reading hazard.
5. **A human judgement can be overridden by a machine.** `patient_link` is latest-HLC-wins, so an
   un-attested link from a peer's matcher, carrying a later HLC, silently displaces our reviewer's
   `unlink`.

Also: **no allergy stream exists** (no event type, table or projection). Everything below that says
"active medications" will say "allergies and active medications" when allergies exist; nothing here
waits for them.

## The maintainer's decisions (brainstorm, 2026-09-27)

| Question | Decision |
|---|---|
| What does a linked chart show? | **One combined record** — every member's medications in one list, each row labelled with its source chart (two paper folders clipped together). |
| How do linked charts appear at the front door? | **One row per person**; the step-3 prompt signs every member chart id of the rows it showed. |
| How is "different people" recorded? | A **human-attested `unlink`** on a pair that was never linked; a machine never overrides it. |
| Who resolves a possible duplicate, where? | **Any enrolled human with an unlocked key**, from the §5.2 banner on either chart **or** from a worklist tray. Role restriction is later policy (principle 9). |
| How does the commit-time check run? | A **matcher worker on each node**, talking only to Postgres (a queue + `NOTIFY`). |
| Does a commit-time hit in the auto band link automatically? | **No.** Every hit is banner + worklist; the auto band stays an explicit owner ceremony and may become policy once the worklist yields real precision figures (§5.13). |

## Decisions (to be recorded in ADR-0076)

- **D1 — A linked chart opens as one combined record.** Reads take the chart SET
  `cairn_person_charts(patient)`; every row carries its source chart(s). Demographics are NOT
  combined: the header lists each member's own identity line.
- **D2 — Writes stay per chart.** A sign-off attests each thread under its own chart; a cease writes
  to the thread's chart. A future new-content write from a combined view goes to the chart that was
  opened.
- **D3 — Chart commands name the displayed SET and refuse on change.** The 2c Critical rule ("every
  chart command names the chart on screen") widened: a link or unlink landing mid-review changes
  the set, and a sign-off of a list the human did not see is refused.
- **D4 — "Different people" is a human-attested `unlink`.** The algebra stays closed. The funnel
  design's rule stands: a candidate *displayed and not chosen* at registration is never an `unlink`
  — this is a deliberate side-by-side judgement, a different act.
- **D5 — An attested assertion outranks an un-attested one in `patient_link`.** Winner order becomes
  *attested first, then `(hlc_wall, hlc_counter, origin)`, then `content_address`* — still a total
  order, so still convergent. It protects both directions: a matcher's link (ours or a peer's) never
  displaces a human's `unlink`, and an un-attested `unlink` (the ADR-0030 agent writer can author
  one today — unlinks are not veto-gated) never splits a human's `link`. The losing event stays in
  the log and is listed on the worklist.
- **D6 — The step-3 prompt signs every member of a person row.** `displayed` keeps its shape
  (`db/045` unchanged); its contents are chart ids of every member of every row shown, so
  `displayed_count` and the twin's "N near-match(es) displayed" count **charts, not people**. Stated
  because the reading of a signed field changes (the same care ADR-0075 decision 3 took).
- **(Not a new decision, restated)** — the commit-time check proposes and never acts; a hard veto
  forces a human decision, never an automatic refusal (§5.13).

## Slices

Each slice has its own plan, PR and §1.2 section, and each is useful on its own. Order: **R1 → R2 →
R3 → R4 → R5.** Linking is pointless before the combined read (R1); finding a duplicate is pointless
before it can be repaired (R2); R4/R5 consume both.

### R1 — the combined read (and #334)

- **`cairn_person_charts(p uuid) → SETOF uuid`** (new migration): every chart in `p`'s
  `person_member` component, or `{p}` when it was never linked. The first function to return a
  cluster; every reader (medications now, allergies later, the banner) must agree on one set, so it
  lives in the database.
- **`list_patient_medications`** takes a set (`patient_id = ANY($1)`); a single chart is a set of
  one. Each row carries `source_charts`.
- **#334 fixed in the view**: the group join becomes group **and** patient; the combined read yields
  **one row per group**. A group wholly inside the set is the ordinary post-link case and shows once.
  A group reaching a chart **outside** the set is still flagged cross-patient and still refuses
  sign-off (#334's real hazard). The single-chart read inherits the fix.
- **Header**: the opened chart, then one line per linked member with that chart's own name, DOB and
  id — *"Linked: 2 charts — Mary SMITH b. 1950-07-01 · Mary SMYTHE b. 1950-01-07"*. No winner is
  chosen; the disagreement is often the typo that made the duplicate.
- **Commands**: `AppState::displayed_patient` becomes the displayed set (sorted ids);
  `med_list`/`sign_off`/`cease` carry it and refuse when `cairn_person_charts` now answers
  differently — *"The linked charts changed while this list was on screen — reload before signing."*
  The webview's `renderedPatient` becomes the rendered set.
- **Sign-off** loops `attest_thread_in_tx(patient_of_thread, thread)`.

> [!NOTE]
> **As built (2026-09-27, PR #688), where the build departed from the bullets above** (design sentences
> are predictions; the code and ADR-0076 win):
> - **#334 was fixed in Rust, not in the view.** `medication/read.rs` selects groups by *membership*
>   over the set and deduplicates per group; the views are unchanged.
> - **`AppState::displayed_patient` is kept.** The displayed set travels as a separate `charts`
>   argument. `med_list` never refuses; `sign_off` and `cease` do.
> - **Added after the PR review:**
>   - **A set holding a *doubted* link withholds every multi-chart line from sign-off.** A doubted
>     link is an un-attested one that db/018 flagged, or that trips the hard veto now: db/054
>     `cairn_chart_set_has_doubted_link`, which covers #220's late-clash path.
>   - **A cease on such a line stops only the opened chart's threads.**
>   - Whether a doubted set should also withhold the other member's one-chart lines is open (#697).

#### R1b — a doubted set withholds every line not on the opened chart (#697 (b), #701; designed 2026-10-03)

The maintainer decided #697 option (b) on 2026-09-27. While the chart set holds a doubted link, every
line not recorded on the opened chart is withheld from sign-off, with its own cause and remedy. A
signature is a claim about a person, and a hard veto is positive evidence against the link. Visibility
does not change: hiding the line would be the hazard if the two charts are one person.

- **The rule.** It is pure, in `cairn-node`'s `medication/read.rs`, and returns *reasons*, not a single bool:
  - `outside_set`: the group reaches a chart outside the set. Unchanged.
  - `doubted_link`: the set holds a doubted link, and the group's charts are not exactly `{opened}`.
    An empty chart list is not `{opened}`, so the rule over-warns there.

  Both can hold at once. `doubted_link` absorbs R1's "multi-chart line in a doubted set". The read is
  handed the opened chart: `list_patient_medications` passes its `patient`.
- **The row's shape is additive and fail-safe.**
  - `MedicationRow::cross_patient` keeps its meaning, "withheld as a wrong-chart hazard". Every
    existing reader, such as sign-off targeting and `cease_plan`, therefore stays correct unchanged.
  - A new `wrong_chart: WrongChartReasons { outside_set, doubted_link }` sits beside it. Both are built
    by one constructor, so `cross_patient == wrong_chart.any()` holds by construction; a DB test pins it.
  - Rejected: narrowing `cross_patient` back to "outside only". A reader that checked only that field
    would then under-warn.
- **Wording.** One shared constant per reason, as with `SEPARATION_INSTRUCTION`. They are used on the
  row, in the window's withheld report, in the CLI's list and in its sign-off output.
  - The outside-set wording is unchanged.
  - The doubted-link wording says the line was not recorded on the opened chart and may be another
    person's.
  - Its remedy (`DOUBTED_LINK_INSTRUCTION`) is a human judgement of the link, never thread separation:
    - **not one person:** "Not the same person…" beside the link (CLI `unlink-charts`);
    - **one person:** confirm with `link-charts`. The window cannot confirm a standing link yet (**#716**).

  A line carrying both reasons gets both sentences.
- **Sign-off after the fact.** `withheld_rows` returns each withheld group with its reasons, and
  `SignOffOutcome::withheld` carries them. The "were NOT signed" report can then word each reason
  from what the orchestrator did, not from a re-read.
- **Cease.** No change. `cease_plan` already stops only the opened chart's threads on any
  `cross_patient` line. For a doubted-set line that sits only on another member it therefore writes
  nothing and names every thread it held back.
- **#701.** `cairn_chart_set_has_doubted_link` reads the stored `patient_link.attested` (R2a's one
  definition). It no longer re-derives attestation through an `event_log` join, which a legacy row
  with a NULL `content_address` fell out of. This changes a function body only; no generation bump.
- **§1.2.** The paper counterpart is two folders clipped together while one page is in doubt: you sign
  for your own patient's page and not for the doubtful one until someone settles the clip. Reading
  and the sign-off gesture are unchanged (1 → 1 → 1). The withheld line's cost falls only on a doubted
  set, and it is lifted by one judgement: unlink (window) or confirm (CLI until #716).

> [!NOTE]
> **As built (2026-10-03, PR #717), where the build departed from the bullets above:**
> - **The rule lives in `cairn-node`'s `medication/hazard.rs`** (`wrong_chart_reasons`, pure), not in
>   `read.rs`. The two row fields come from **one reasons map** in `read.rs`, not from a constructor.
>   Every production reader goes through `MedicationRow::is_wrong_chart_hazard()` (either signal is
>   enough) or `hazard_reasons()` (a hazard with no recorded reason is worded as the outside case;
>   blind to status). What sign-off withholds is `targeting::withheld_reasons()` / `withheld_rows()`.
> - **"Doubted" gained a third case (maintainer decision during the final review):** an ATTESTED
>   unlink between two charts that are still in one set. It catches the A–C–X bridge: A–X is doubted,
>   and a sparse chart C, linked to both, trips no veto. A clinician unlinks A–X and the record stays
>   one through C. db/054 used to look only at `state = 'link'` rows, so it then found no doubt, and
>   every X line became signable from A just after a human attested that A and X are different
>   people. Pinned by `doubted_link_withholds.rs`'s bridge test.
> - **"Either judgement lifts this hold" was dropped.** It was false in reachable cases: after an
>   unlink a shared line reaches outside the set and stays withheld with the separation remedy;
>   confirming one of two doubted links leaves the hold; and the bridge case above. The remedy now
>   says the hold lifts once no link is in doubt, and to judge the links before separating threads.
>   It names `--attester-key`, `--from` (an unlink where neither chart is held) and that a link
>   needs both charts held. A row says "the node cannot yet vouch that it is this patient's" and
>   "until the record's links are no longer in doubt".
> - **The CLI prints the long remedy once, below the list** (`list_text::doubted_link_note`, decided
>   by the status-aware `withheld_rows`), not under every row. The outside-set row lines stay
>   byte-identical to before (pinned by a golden).
> - **The §1.2 count above is per doubted link.** "Lifted by one judgement" holds for one doubted
>   link. Two doubted links take two acts, and in the A–C–X bridge CONFIRMING A–C or C–X never lifts
>   it: the act is a human A–X relink, or an unlink of A–C or C–X that takes X out of A's record.
> - **The PR review round** (after the final review):
>   - **Pointers.** `withheld_because()` became `hazard_reasons()`, because the CLI had pointed every
>     hazard row at a note printed only for withheld ones. Only a line `withheld_reasons()` reports
>     now says "cannot be signed until …" or points below the list, in the CLI and the window alike.
>   - **Member charts.** The rule also judges each member thread's own chart, which is the chart
>     sign-off writes to.
>   - **Construction.** `MedicationRow` and `WrongChartReasons` lost `Deserialize`.
>   - **Tests.** The bridge test gained a line on the bridge chart, so its lift can fail. New tests:
>     a human relink lifts the bridge; an un-attested unlink is no doubt (a deliberate under-warn,
>     now stated in db/054 and `hazard.rs`); another record's doubt leaves this one alone.
> - **#718:** db/054's SECURITY DEFINER no longer has a reason since #701; it is kept, and the
>   decision is filed. **#719:** the review residuals (items 3–4 fixed for the doubted line in the
>   review round). **#720:** make `wrong_chart` the only Rust source of truth. #716 and #335 gained
>   comments (the window cannot show which link is in doubt; the doubt state can change between
>   display and sign-off, with or without a human act).

### R2 — link and unlink from an open chart (#681) + the precedence floor

- **Gesture**: header control **"Same person as…"** → the front door's search (only a chart some
  list on screen showed can be picked — `AppState::shown`) → a **side-by-side panel** (names incl.
  aliases, DOB with provenance, identifiers, **active medications**; any `cairn_match_veto` findings
  first, as plain facts) → **"Link — same person"** signs an attested `identity.link.asserted` under
  the unlocked human key and opens the combined chart. The panel's safety is what it SHOWS, not an
  "are you sure?" (principle 3). A vetoed pair can still be linked (§5.13).
- **Unlink**: each member line in the combined header carries **"Not the same person"** → the same
  panel → an attested `identity.unlink.asserted`. Both directions are events; both reversible.
- **Orchestration**: `cairn-node` gains `link_charts` / `unlink_charts(human)`; when the act resolves
  a proposal, the proposal row moves (`applied` / `rejected`, `applied_event_id`) in the SAME
  transaction. `apply_accepted_proposal` becomes a thin wrapper, so the C2 door gains its first
  production caller unchanged in behaviour.
- **Precedence floor (D5, safety-critical, in-DB)**: `patient_link` gains `attested BOOLEAN`
  (idempotent `ALTER … ADD COLUMN IF NOT EXISTS` beside the `CREATE`, per #207, with a backfill);
  the value is the one db/018's applier already computes for the #190 veto check —
  `e.attester_key IS NOT NULL AND cairn_attestation_vouched(e.event_id)` — so "attested" has ONE
  definition. The applier's `ON CONFLICT … WHERE` compares `attested` before
  `cairn_hlc_overlay_wins`. Both doors (db/005, db/020) reach the one applier. The #190
  flag-lifecycle block that follows the upsert derives from the standing winner, and must be
  re-read against the new order.

> [!NOTE]
> **As built (R2a, 2026-09-27, PR #698)**, where the build departed from or refined the bullets above:
> - **R2 split in two.** R2a is the floor + node + CLI: `patient_link.attested`, the D5 precedence
>   rule, and `cairn-node link-charts` / `unlink-charts` (a human's judgement given from the command
>   line, not yet the window). R2b — the "Same person as…"/"Not the same person" gesture and its
>   side-by-side panel — is still to come.
> - **The re-fold is db/055's generation-55 heal, not a column backfill.** A backfill can make
>   `attested` truthful for the standing row; it cannot re-decide which assertion *is* the standing
>   row. db/055 exists to bump `SCHEMA_GENERATION` so every node's next connect replays every link/
>   unlink through the new applier (`cairn_reproject`), which is what actually re-folds a winner an
>   un-attested machine link had already displaced. **And db/055 also re-folds the affected pairs
>   itself** (every vouched assertion of a pair whose standing winner is un-attested): cairn-sync
>   shares the generation number and runs the same heal but loads no identity migration, so a
>   cairn-sync that reached a shared database first would otherwise stamp 55 after an old-order
>   heal and cairn-node would skip its own (PR #698 review).
> - **Who may act, precisely:** `link_charts` refuses unless BOTH charts are held here (attaching a
>   stranger's future chart on a typo is the risk a link — but not an unlink — creates).
>   `unlink_charts` needs the open chart held here and the other either held too or already read as
>   part of that chart's record here (a displayed member this node hasn't synced the registration
>   for, R1's case) — same record, not merely named. Neither chart held is refused outright (#699).
> - **A third-chart join is recorded, not silently resolved.** Unlinking A from B when they are still
>   joined through C (A–C–B) commits the attested `unlink` — it is real and replicates — but
>   `LinkOutcome::effect` is `StillJoined` — the record didn't split. The machine never guesses which edge is
>   wrong (principle 2), so it is reported. R2b's per-member "Not the same person" line must name the
>   edge(s) actually joining that member, not just offer the verb.
> - **Filing follows what's held.** When only one chart is held here, the event files under that one
>   (db/005 step 8b refuses a local event about a chart with no history here) — never under the
>   unheld chart, even though `unlink_charts(unheld, held)` reads `unheld`'s chart set back (see
>   `LinkOutcome::filed_under`; the returned chart set is always that held chart's).
> - **Recorded is not took effect.** db/018 admits an assertion that loses the overlay, so each
>   judgement reads back, inside its own transaction, what the pair's standing assertion says.
>   `LinkEffect` is `TookEffect` (it says what this judgement says — this event, or a later one that
>   agrees), `Outranked` (a later judgement about the same pair that says the OPPOSITE — e.g. a
>   peer's from a clock ahead of this node's — stands instead) or `StillJoined`; the CLI prints
>   each differently and never "linked"/"unlinked" for the latter two. Because the read-back is in
>   the transaction, nothing can fail after the commit except the commit itself, whose error says
>   the outcome is unknown and names the event.
> - **Closed proposals are left alone; open ones move with the judgement.** A `match_proposal` already
>   `applied`/`auto_applied`/`rejected`/`retracted` is untouched — what stands is `patient_link`'s
>   business, not the proposal row's. An open (`pending`/`accepted`/`review`) proposal for the same
>   pair resolves
>   (`applied`/`rejected`, `applied_event_id`) in the SAME transaction as the link/unlink event.
> - **One lock order everywhere.** Every path that can touch a proposal row and take db/018's global
>   `CARNLK` advisory lock (`pg_advisory_xact_lock(x'4341524E4C4B')`) — `apply_auto_candidate` (which
>   set the order), `judge` (the CLI path; R2b's window will use it) and `apply_accepted_proposal`
>   — locks the `match_proposal` row `FOR UPDATE` FIRST and only submits
>   (which takes CARNLK) SECOND, so two concurrent judgements of the same pair cannot deadlock.
> - **Auto-apply now yields to a standing human judgement.** `apply_auto_candidate` re-checks for an
>   attested `patient_link` row before minting an un-attested matcher link, and skips the pair
>   (`AutoOutcome::AlreadyJudged`, counted apart in the batch summary) when one exists — so a matcher
>   link is never minted over a pair a human has directly judged (a join through a THIRD chart is
>   still possible; see `StillJoined`). That check runs BEFORE the veto re-check, so a pair a human
>   already linked is not sent back to review. And after submitting, auto-apply confirms its link
>   is the standing winner; if a judgement committed after the check (a peer's, racing it — #700's
>   race) or a later assertion outranks it, the transaction rolls back and nothing is written.
>   (The proposal of a human-judged pair still stays `pending` — the rest of #700.)
> - **`apply_accepted_proposal` now shares `judge`'s core** (`assert_link_in_tx`) instead of
>   duplicating it — still with no production caller; R5's worklist will be its first.
> - **Mixed fleet:** the winner order is a property of each node's own database (its schema
>   generation, [ADR-0012](../../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)),
>   not of the event log, which is identical everywhere. A peer still on a pre-R2a binary ranks the
>   old way — latest-HLC-wins, so a later machine link can still displace a human's unlink there —
>   until it upgrades and its own heal re-folds; the fleet converges once every node has upgraded.

#### R2b — the window's gesture (designed 2026-09-28)

The maintainer's decisions (brainstorm, 2026-09-28): **#699 → (a)** — a neither-held unlink is
filed under the opened chart when both subjects read as part of its record; **R2b ships as two PRs**
— R2b-1 "Same person as…" (link) then R2b-2 "Not the same person" (unlink + #699); the panel adds
**sex-at-birth and current addresses** to the bullets above (sex-at-birth is a veto field, so a
finding must not cite a fact the panel hides; address is the clerk's first disambiguator).
Contact details and next of kin are not projected streams yet.

**The comparison is SET against SET, never chart against chart.** With A open and already linked to
C, linking B to A also joins B to C — a pairwise A–B veto check would hide a B–C date-of-birth
clash. So the left side is every member of the displayed record, the right side every member of
B's record (B may itself be linked to D), and `cairn_match_veto` runs over every cross pair. For
two never-linked charts this is exactly the picture above.

**Node read** — `cairn-node/src/patient/compare.rs` (new; no SQL object, `SCHEMA_GENERATION` stays 55):
- `chart_facts(client, &ChartSet) → Vec<ChartFacts>`: per member `held`, `trust` (`person::trust_of`),
  **every** retained non-repudiated name with its `use` and provenance (not `patient_name_current`'s
  one winner — a maiden or preferred name is often the clue), repudiated names apart as `aliases`,
  DOB and sex-at-birth (`patient_demographic`, value + provenance), identifiers (system, value,
  provenance), current addresses (`patient_address_current`). One `ANY($1::uuid[])` query per
  section; each failure a `LocalDbFault` naming its step (#467 legibility guard).
- `cross_vetoes(client, left, right) → Vec<VetoFinding>`: every left×right pair, each finding
  tagged with its pair, `hard_veto` before `degrade_hold`.
- The right side's medications are NOT a new read: `read_chart_of(B)`, the same custody-applied
  combined read opening B would give (§5.9 sealing applies unchanged).

**Window commands** — `cairn-gui-tauri/src/link.rs` (new), one-line forwarders onto `*_impl`:
- **Search** reuses `browse` from a second form inside the chart: it already adds to `shown` and
  never touches the open chart (whereas `open()` clears `shown`, so the front door cannot be reused).
- `compare_impl(patient_id, charts, other)`: `displayed_patient` → `check_displayed_set` → `other`
  in `shown` → `other` not already in the record → reads; returns the panel view **including B's
  set as displayed**.
- `link_impl(patient_id, charts, other, other_charts)`: **names both displayed sets** and refuses if
  B's set changed ("the other record changed while you were comparing — nothing was done; compare
  again") — decision 3 widened to the right-hand side: a peer's link landing mid-review must not clip
  a chart into this record sight unseen. Fixture mode refuses; `live_key` (activity); calls
  `link_charts(opened, B)`. No server-side timing row — db/044's `gesture_kind` CHECK would refuse
  a `link` kind (as it does registration); the runbook's stopwatch measures the gesture.
- `link_report(&LinkOutcome)` (pure): `TookEffect` → "Linked — this record now combines N charts" and
  the pane reloads; `Outranked` → "Recorded, but NOT in effect: a later judgement on this pair says
  these are different people", no reload, no retry. Errors are classified the way
  `cairn-gui-live`'s `data_error_from` already does — a `P0001` from the floor OR a
  `db_diagnosis::DeliberateRefusal` marker on the chain is a refusal, anything else an outage — and
  worded with the funnel's `Retry` vocabulary, not a third one (#702). **This needs a node-side fix
  first:** `chart_link`'s own pre-check refusals (same chart, a chart not held, a key that is not an
  enrolled human) are bare `anyhow::bail!`s today, carrying neither marker nor SQLSTATE, so the
  window would call them outages ("try again") when they are verdicts. They are minted through
  `deliberate_refusal` in R2b-1. "Commit outcome unknown" passes through verbatim.

**Panel** — `src-ui/link.js` (new), a `<section>` between the header and the medication table, never
a dialog (Esc / "Close comparison" hides it). DOM order is clinical:
1. Veto findings first, `role="alert"`, plain facts, hard vetoes first; hidden when there are none
   — never "no conflicts", because an absent finding is not a clearance.
2. One `<table>`: a column per chart under two `<colgroup>` headers, **This record** / **Other
   record**; rows names (with use), names struck as false, DOB + provenance, sex at birth +
   provenance, identifiers, addresses, identity state; every absence worded ("not recorded", or
   "unknown — registration not yet received here" when unheld).
3. The other record's active medications, read-only, captioned *"On the other record — not part of
   this one until linked"*.
4. **"Link — same person"**, which says what it needs when the key is locked.

Focus moves to the panel heading on open and back to "Same person as…" on close. `MockData` gains
fixture `chart_facts` (name, DOB) so the panel can be walked headless and timed under `--mock`.
**A comparison that could not be read in full offers no Link button** — reading stays available,
but the judgement needs the whole picture.

**R2b-2 — "Not the same person" and #699 (a):**
- `record_edges(client, &ChartSet) → Vec<Edge>` (the standing `link` rows inside the set: pair,
  attested, provenance, when). Each member line lists the links that actually join it — *"linked to
  chart … — by a clinician's judgement, <date>"* or *"— by the matcher (not reviewed)"* — each with
  its own **"Not the same person"**. Per EDGE, not per member: in A–C–B the wrong clip may be A–C or
  C–B; the human picks, the machine never guesses (principle 2), and a per-member act would yield
  `StillJoined` by construction.
- The same panel on that edge's two charts; `unlink_impl(patient_id, charts, low, high)` refuses an
  edge no longer in `record_edges` of the displayed set.
- **#699 (a):** `unlink_charts` gains `opened`; `admit_judgement` gains the arm *unlink, neither
  held, `opened` held and its record contains both → file under `opened`*; `assert_link_in_tx`'s
  `about ∈ {low, high}` becomes a `FiledUnder::{Subject, RecordOf}` enum so the relaxation cannot
  reach `link`; CLI `unlink-charts --from <chart>`. A plan task audits every reader of identity
  events by `patient_id` (db/018/019/023–025, reprojection, the twin, sync scope) for a
  "filed-under ∈ subjects" assumption, with a test pinning the finding.
- Outcomes: `TookEffect` "Unlinked — chart … is no longer part of this record"; `StillJoined`
  "Recorded — but chart … still reads as part of this record through another link; that link is
  listed on its line"; `Outranked` as for link. Each reloads except `Outranked`.

**§1.2:** link — paper 3 (fetch the other folder, lay the front sheets side by side, clip) → forced 3
(find → Compare → Link; the Link click IS the signature, ADR-0053) → target 3; unlink — paper 2
(unclip, annotate) → forced 2 → target 2. `M ≤ N`. Review-and-link ≤ 20 s, measured by a new
runbook section 9 (a human act).

**As built (R2b-1, PR #707, 2026-09-29) — deviations from this design:**
- **Fixture facts carry the name only.** `MockData` is unchanged (it gained no `chart_facts`); the
  window's own `link::view::fixture_facts` builds a `--mock` chart's facts from the name the list
  showed — a `--mock` `Candidate` has an age, not a DOB (the front door's own fixture shape, #673)
  — so the headless walk exercises the panel's layout and wording but not a fixture DOB cell; the
  DOB row is only real over a live node. Fixture mode refuses the link itself, so a `--mock` run
  ends at the fixture-refusal line.
- **The right-set refusal reuses `chart_set::check_displayed_set`.** `link_impl` names B's
  displayed set the same way every other chart command names its own. That function has two
  failure arms: `CHANGED` (the set read now differs from the one sent back) and `UNREADABLE` (the
  list sent back is empty or does not parse). A changed set reads `OTHER_CHANGED` ("the other
  record changed while you were comparing — nothing was done; compare again"); an unreadable one
  keeps its own "could not tell which charts are on screen" wording — a window fault is not a
  change to the record (the left-hand rule, applied to the right in the PR #707 review round).
- **Link gesture timing is not recorded server-side.** `db/044_ui_gesture_timing.sql`'s
  `gesture_kind` CHECK admits only `'signoff'`/`'cease'` and would refuse a `link` row (as it
  already refuses registration); a widened CHECK is a migration this plan's Global Constraints
  forbade (`SCHEMA_GENERATION` stays 55). The runbook's stopwatch (section 9) is the only figure
  for this gesture, same as the front door's find/register gestures.
- **The DOB cell names a non-day precision.** `FieldFact` gained a `precision: Option<String>`
  facet (`facets->>'precision'`, principle 4): `"{value} ({provenance})"` when precision is absent
  or `"day"`, else `"{value} ({precision} precision, {provenance})"` — a year-precision DOB stored
  as `1950-01-01` must not read as a precise day it never claimed to be, which would look like a
  clash against a same-day fact that IS precise.
- **On a chart not held here, every ABSENT fact reads "unknown — registration not yet received
  here"**, including names struck as false, not only the fields this section named — the absence
  word is per-chart (`ChartFacts::held`), not per-field. Two exceptions: the Identity row shows the
  trust state itself (`unknown`, or a `chart_trust` state when a row exists), and struck names that
  HAVE arrived (a peer's repudiation can precede the registration on the sync door) are listed,
  not hidden behind "unknown" (PR #707 review round).
- **Address cells always carry provenance**, not only "the clerk's first disambiguator" framing
  above — matching every other fact cell's shape (never a bare value with no source).
- **The other record's medication warnings are prefixed "On the other record: "**, and the
  "No current medications recorded on the other record." line appears only when no warning note
  is already present — an empty list and a clean list read differently on the safety surface
  (never "no conflicts" collapsed into "nothing here").
- **`chart_link`'s pre-check refusals are now marked verdicts** (the node-side fix this section
  called out as needed): same-chart is `RefusalScope::Input` (no retry by anyone ever changes the
  answer); a chart not held here, and a key that is not an enrolled human actor, are both
  `RefusalScope::NodeState` (the identical call succeeds once the chart or the enrolment arrives).
  `link_error_view`'s error classification (`cairn_gui_live::error::data_error_from`, then the pure
  `link_error_from` wording) now sees a verdict instead of an unmarked error it classified as an
  outage (#702). `RefusalScope::NodeState`'s contract was widened to match: its state may change
  by an operator act OR by sync, and only the enrolment refusals name a command. A
  `NotProvisioned` refusal reads "This node cannot record the link yet: …" — not "until an operator
  acts", because a chart not held here resolves by sync.
- **The panel closes on ANY chart change** (`enterChart` as well as `closeChart`), not only on an
  explicit close, and closing on a chart change does not move focus — a panel that outlived its
  chart would let Compare/Link keep naming a chart no longer on screen. The Link button is
  disabled while a link is in flight (no double-submit under set-union semantics).
- **Link signs over the sets that were COMPARED, both of them** (final whole-branch review). The
  design named the opened chart's displayed set; as first built, Link sent the window's CURRENT set
  at click time, so a re-read between Compare and Link (a sign-off's refresh after a peer's link)
  could grow this record and the judgement would pass over a set nobody compared.
  `ComparisonView` now carries `left_charts` beside `other_charts`; the webview captures the chart
  synchronously at Compare and sends both sets back; a changed left set is refused as
  `THIS_CHANGED` ("this record changed while you were comparing — nothing was done; compare
  again"), worded like `OTHER_CHANGED` rather than the list's "reload the chart".
- **The outcome names charts the comparison never showed.** `link_report` receives the compared
  union; if the record now combines a chart outside it (a third chart linked to one side by the
  time the judgement landed), the sentence adds "The record now also includes chart(s) … that were
  not in the comparison — review them."
- **The alias row is "Names struck as false"**, not "earlier recorded names": `patient_alias_pool`
  holds names REPUDIATED as known-false (db/025), which a clerk must not read as former names.
- **Where an answer lands.** Every panel message goes through `setMessage` (an empty status line
  is `hidden`, and `[hidden]` wins in style.css — the first build's bare `.textContent` writes left
  every refusal and the Outranked sentence invisible). A link answer that lands after the clinician
  left the chart is still reported, prefixed "For chart <id>:", and the chart now open is re-read
  only when the link changed it (it is one of the compared charts); opening or closing the panel
  drops in-flight Compare and search answers; a refusal that cannot change on retry (`never` /
  `after_operator`) hides the Link button and forgets the comparison, leaving it on screen with the
  sentence. A LOCKED KEY is not such a refusal (`Retry::Now`): the button stays for after the
  unlock. Outranked read "pressing Link again records another judgement but changes nothing" as first built — corrected in R2b-2: a retry would normally record a newer judgement that overrules the other.
- **Names and shapes.** The window module is `src/link/` (`mod.rs`, `view.rs` + `view_tests.rs`,
  `search.rs`), not one `link.rs`; `link_report` takes `(effect, charts, compared)`, not
  `&LinkOutcome`, so it can name uncompared charts; `chart_facts`' two `person.rs` reads
  (`read_held`, `read_trusts`) name their step with `.context(…)`, and only its five own queries go
  through `LocalDbFault`; the SQL binds ids as `$1::text[]::uuid[]`; and a `#link-problems` line
  (what could not be read) sits ABOVE the findings — a partial comparison says so before anything
  else.
- **The PR #707 review round** (after the final whole-branch review):
  - Identifier findings are never labelled "verified": db/016's identifier severity says whether
    both values passed a format profile, not how either was sourced, so they read "Identifiers
    differ (both in a checked format)" / "(format not checked)"; only a dob / sex-at-birth hard
    veto (both winners provenance-rank ≥ 60) reads "Verified facts differ". Findings are ordered
    with `subject` in the key, so several identifier findings on one pair read in a fixed order.
  - The panel's search is its own `link_search` command (`link/search.rs`): `browse` minus this
    record's charts, with a summary counting the rows shown — filtering in the webview had left
    "1 existing chart(s) found." over an empty list. Debounced like the front door's.
  - Only a WHOLE comparison arms Link in the webview (not only the hidden button); with this
    record unread no "This record" head is drawn (a `colspan="0"` drew as 1, over the other
    record's chart); a failed medication read says so in its own section.
  - The other record's current drugs are selected by a typed `MedListRowView::current`, never by
    the display label `"current"`.
  - A failed re-read after a sign-off, cease or link is reported AFTER that act's outcome
    (`refresh(lead)`), never instead of it.

**As built (R2b-2, PR #711, 2026-09-30) — deviations from this design:**
- **One list of links, not links under each member line** (maintainer, 2026-09-30). Under the member lines
  the pane shows **"How these charts are linked"**: one entry per standing `patient_link` row, each with
  its own **"Not the same person…"**. Per LINK, never per member — putting each link under both of its
  charts would show every control twice. `cairn_node::patient::edges::record_edges` supplies the rows (pair,
  attested, the day it was recorded); an unlinked pair is not a link (the query keeps `state = 'link'`, and a
  test that queries a set holding BOTH charts pins it). A linked record whose read returned no links says so in
  words, and an unreadable list says so and offers no unlink — never a blank.
- **The un-attested wording is "without a clinician's confirmation on record here"**, not "by the matcher (not
  reviewed)": a peer's human link whose attester is not enrolled here also stores `attested = false`, so
  "matcher" can be untrue (principle 4). Each line reads "recorded {day} (UTC)" — the day is derived from
  the stored millisecond wall clock, so the zone is stated, not implied.
- **"Still joined?" is asked of the two SUBJECTS** (`high ∈ person_charts(low)`, read inside the judgement's
  transaction), not of the filed-under chart. The audit found the design's `other ∈ person_charts(about)`
  would, once a third chart could be the filing chart, turn on which subject was asked about: a successful
  A–B–C split judged from A would answer `StillJoined` whenever that subject was the near chart B.
- **#699 (a) is ADR-0077** (spec v0.79). `FiledUnder::{Subject, RecordOf}` lives in the new pure module
  `chart_link/admit.rs`; `RecordOf` is unlink-only (a link filed under a third chart is refused before
  anything is signed). CLI `unlink-charts --from <chart>`.
- **`--from` / the opened chart is checked whenever it names a chart that is not a subject** (build ruling):
  it must be held and its record must hold both subjects, or the unlink is refused — even when a held
  subject alone would have admitted the unlink. The design's `record_of = opened.unwrap_or(about)` trusted
  unchecked input and let the CLI print a record for a chart that does not exist. The refusal names what it
  is about (PR #711 review): a chart not held here is `NodeState` (sync may deliver it); a record that does
  not hold both is `Input` — "reload the chart and judge again", as the in-transaction re-check words it.
- **A `RecordOf` filing's record is re-read by `assert_link_in_tx` under db/018's CARNLK** (PR #711 review).
  As first built the re-read ran merely inside `judge`'s READ COMMITTED transaction, before any lock, so a
  peer's unlink arriving through the sync door could commit between the read and the submit and leave the
  event filed (and graded, db/048) under a record that no longer held the pair. The signing core now takes
  CARNLK (row lock first, as every path does) and then re-reads; a deterministic DB test parks the judgement
  on CARNLK, commits a peer's unlink meanwhile, and asserts the refusal.
- **Module split:** the judgement entry points (`judge`, `link_charts`, `unlink_charts`, `LinkOutcome`, …) moved
  to `chart_link/judge.rs` in a pure-move commit (house rule 4: `chart_link.rs` was 726 lines);
  `chart_link/admit.rs` holds the pure admission rule.
- **Outranked no longer says a retry "changes nothing"** — that was false. The sync door merges the peer's
  HLC — the clinical door, db/020, the only one identity events arrive through — so pressing Unlink again would normally record a NEWER judgement that overrules the colleague's; it
  does not settle the disagreement. The unlink panel says so, and the same correction was made to R2b-1's link
  sentence (`link/view.rs`) and to `LinkEffect::Outranked`'s doc. (This also corrects the R2b-1 note above.)
- **A locked key names its own button**: `key_locked_for(button)` (R2b-1's `key_locked` is
  `key_locked_for("Link — same person")`), so an Unlink click never says "press Link again". The unlink commands' final
  stage is the pure `standing_edge(edges, low, high, act)`: a present link (either order) passes; an absent one
  is `LINK_GONE`; an unreadable edge list is `Retry::Now`, never `LINK_GONE` (a refusal is not an outage).
- **The unlink panel is a separate `<section id="unlink-panel">`**, not a mode of the link panel (one panel with
  two verbs can show the wrong verb's button over the other's comparison), and the two are **mutually
  exclusive**: opening either closes the other (two opposite judgements on screen at once is cognitive load
  paper-parity forbids). Focus: on open to the panel heading; on Close (Esc or "Close comparison") back to the
  link button that opened it (by low/high, else the first); after a successful unlink — whose re-read
  destroys the focused button — to the patient's heading. `renderLinks` lives in `main.js` (the
  webview-fields guard scans only that file); `unlink.js` borrows `updateLinkLock`/`keyUnlocked`.
- **No SQL object, no wire change:** `SCHEMA_GENERATION` stays 55. Gesture timing is not recorded server-side
  (db/044's CHECK, as for link); the stopwatch is runbook §10's.

### R3 — the front door collapses by person

- Search results group by `cairn_person_charts`; a person row lists each member's name + DOB and
  ranks by its best member's keys (the seven keys of ADR-0075 decision 5, unchanged per member).
- Opening a person row opens the combined set; `AppState::shown` records every member.
- The step-3 prompt signs every member id of every shown row, in row order (D6). A row still takes
  one of the five `PROMPT_CAP` places.

#### R3 — designed 2026-10-03

The maintainer decided two questions in the brainstorm:
- **Each member line is its own open target.** The opened chart matters: in a doubted set only its own
  lines can be signed (R1b), and a future new-content write goes to it (ADR-0076 decision 2). So the
  clerk picks the folder, usually the line matching what they typed, and the window never picks one
  for them. The combined set reads either way.
- **The browse list collapses too, not only the prompt.** There is one result shape everywhere (CLI,
  browse, prompt), so a person looks the same at step 1 and at step 3.

**The result shape** (`cairn-patient-search`):
- `CandidateList.candidates: Vec<Candidate>` becomes `people: Vec<PersonRow>`. A `PersonRow` holds its
  member `Candidate`s and is never empty: it is built only by a constructor, and its fields are private.
  Within a row, the members the search matched come first, in rank order. Linked members the search
  did not match follow, oldest chart first (UUIDv7 order).
- `CandidateList::displayed_charts()` is the ONE flattening: every member of every row, in row order.
  `SearchAttestation::from_displayed` uses it, so the signed `displayed` list is the screen by
  construction (D6). db/045 and the wire are unchanged.
- `TrustState` gains `Unknown` (serialized `"unknown"`), only for a chart whose registration this node
  does not hold.
- A row shows each member's **age**, as today (`Candidate` has no DOB field). The chart header R1 built
  shows each member's DOB once a chart is open.

**The node** (`cairn-node`, `patient/search.rs` + a new `patient/search_person.rs`; `search.rs` is
already 488 lines):
- The matched charts are ranked exactly as today (the seven ADR-0075 keys, unchanged).
- Each matched chart's link component is read in ONE query (`unnest($1)` × `cairn_person_charts`), not
  in one call per chart.
- A **pure** `group_by_person(ranked, components)` builds the rows. A row takes the position of its first
  (best-ranked) member, and that is all "ranked by its best member" needs. A linked chart the search
  did not match is a member of the row: it is part of who this person is, and it is literally on screen.
- The display reads run over every member id, matched or not.
- **A chart not held here** (no `patient_chart` row — `person::read_held`'s test) reads trust `unknown`
  through R1's `person::trust_of`. If it has no name it reads "(registration not yet received here)".
  It does **not** set the signed `incomplete` flag, because the SEARCH was not partial.
  - This also fixes a latent defect for matched charts: the search reported `confirmed` for any chart
    with no `chart_trust` row, held or not. R1 fixed that for the header; the search never got the fix.
  - A HELD chart with no readable name keeps today's rule: "(name unavailable)" plus `incomplete`.
  - "Not held" means exactly a chart whose registration has not synced here. Since #345 every
    registration creates the `patient_chart` row, and db/005 step 8b makes a locally written chart
    begin with its registration. `search.rs`'s comment "no `patient_chart` row is normal" predates
    #345 and is corrected in this slice.
- A failed component read fails the search loudly, as every read failure does. It never falls back to
  per-chart rows, which would put one person in two prompt places with nothing said.

**Bounding, signing and wording** (`cairn-gui-funnel`, `cairn-gui-tauri`'s `funnel/view.rs`):
- `bound_for_prompt` takes the first `PROMPT_CAP` (five) ROWS whole, so a person is never split across
  the cap. `withheld` counts people not shown, and is still never signed (ADR-0075).
- `displayed` therefore names every member of the rows shown. `displayed_count`, and the legibility
  twin's "N near-match(es) displayed", count charts (ADR-0076 decision 6).
- The summaries count people, and name charts when the two differ: browse says "3 existing patients
  found (4 charts)"; the prompt says "the 5 closest of M", with M in people. A summary over rows that
  are all single charts stays **byte-identical to today**, pinned by a golden.

**The window and the CLI:**
- `BrowseView`/`PromptView` carry `people: Vec<PersonRowView>`. Each row is one `<li>` holding a nested
  list of member lines, and every member line has its own open button, naming the member as today's
  buttons do ("Open chart: Mary SMYTHE — 76 y — identity confirmed").
- A linked row is labelled "One person — 2 linked charts", so a reader, and a screen reader, hears why
  a name that was not typed appears.
- `AppState::shown` records every member of every row on screen. Only a chart on screen can be opened,
  as now, and opening any member opens the combined set (R1's read).
- The CLI (`patient-search`, `patient-register`) prints a linked member indented under its row
  ("↳ linked: …"). `patient-register` attests the flattened list it printed, as today.
- The webview-fields guards are updated for the new fields.

**The mock** has no link concept, so every mock row is a person of one, and `--mock` cannot show a
linked row. A mock linked pair is filed as an issue rather than built as a second, fake link model; the
live walk on a linked pair is already an owed human act.

**Tests (TDD):**
- Pure: `group_by_person` (a row is placed by its best member; unmatched members are appended oldest
  first; the output is deterministic); the cap never splits a row; the flattening is the members in
  row order; the golden summaries.
- DB: a linked pair is one row; a linked chart the search did not match is shown and signed; a member
  not held here reads `unknown` and does not set `incomplete`; a never-linked search returns exactly
  what it returned before (golden).
- Window: a register walk over a linked row signs both ids; a member the search did not match can be
  opened, and a chart on no row cannot.

**§1.2.** The paper counterpart is the card index, where clipped folders sit in one slot. At the desk:
paper 1 (see the card) → forced 1 → target 1. Grouping removes reading effort (one slot per person); it
adds no act. Measurement is the runbook's front-door section (section 8), a human act already owed.

**Out of scope:** a demographic winner across members; R4/R5; links in the mock.

> [!NOTE]
> **As built (2026-10-03, PR #721), where the build departed from the bullets above:**
> - **`group_by_person` lives in the SHARED crate** (`cairn-patient-search/src/person.rs`, beside `PersonRow`,
>   `EmptyRow` and `MissingComponent`), not in `search_person.rs`. It is pure, and any picker must agree with the
>   node on what a row is, exactly as it must agree on what was displayed. The node's `patient/search_person.rs`
>   holds what feeds and renders it: `read_components` (the one `unnest` × `cairn_person_charts` statement),
>   `display_name_for` and `trust_state_for`. `search.rs` is the only production caller of `group_by_person`, and
>   the mock builds rows of one with `PersonRow::each_alone`.
> - **A component missing its own chart is refused** (`assemble_components`, called by `read_components`).
>   `group_by_person` trusts that every component contains the chart it is keyed by; one that did not would drop
>   that chart from every row, silently. db/054 always unions the chart in, so this cannot fire today; it is
>   refused, as `person::person_charts` refuses, rather than guessed around. Pinned by
>   `search_person.rs::a_component_that_omits_its_own_chart_is_refused`; a chart with NO component read is
>   `MissingComponent` (`person.rs::a_chart_with_no_component_read_is_an_error_never_a_silent_row_of_one`).
> - **The people/charts phrase lives in `cairn-gui-tauri`'s `funnel/rows.rs`**, not in `view.rs`: `people_phrase`
>   (the step-3 prompt's "N existing patient(s) (C charts)") and `charts_suffix`, which the link panel's search line
>   shares. `browse_summary` keeps its own arms, each a golden sentence. `PromptCounts` gained `shown_charts`.
> - **The "Same person as…" panel's search collapses by person too** (`link/search.rs`). With linked rows it says
>   "N other patient(s) found (C charts)". This record's own row is left out WHOLE, unmatched members included, and
>   a linked own record is named as a record: "This record (N charts) also matched and is not listed." The old
>   "N chart(s) of this record also matched" would be false for a member the search did not match (principle 4).
>   A single-chart own record keeps the old sentence verbatim. Pinned by
>   `this_records_whole_row_is_left_out_even_its_unmatched_member` and `only_this_records_own_chart_matched`.
> - **A HELD member with no readable name sets `incomplete` whether or not the search matched it**
>   (`display_name_for`'s doc; `search_person.rs::a_held_chart_with_no_name_ever_is_still_unreadable`). It is on
>   screen and signed as displayed, and the node could not read it, so the build errs toward warning. The cost is
>   a rare over-set flag on a signed registration.
> - **A MATCHED chart not held here no longer sets `incomplete`.** Before R3 it read "(name unavailable)" and set
>   the SIGNED flag; now it reads "(registration not yet received here)", trust `Unknown`, and the search is
>   complete. This is a deliberate change to a signed flag, pinned by the DB test
>   `search_by_person.rs::a_matched_chart_not_held_here_with_no_name_is_not_a_partial_search`.
> - **The linked-row label reaches a screen reader through `aria-describedby`.** Tab lands on a member's open
>   button, never on the label, so `funnel.js`'s `personItem` points every member button at the label's id. No
>   automated test pins it (a JS harness is #332); the live VoiceOver pass on a linked pair is owed.
> - **`link.js`'s search result is guarded.** Its binding is named `found`, so
>   `link/search.rs::link_js_search_reads_no_field_the_backend_does_not_send` can scan exactly that payload. The
>   comparison view's fields stay unguarded; that is #715.
> - **The CLI's text moved into `cairn-node`'s `patient/candidate_text.rs`** (`candidate_lines`, `ellipsize`,
>   `NAME_COLUMN_WIDTH`), out of `main.rs`, so it is pinned by goldens. A linked member prints under its row as
>   "↳ linked: …" in the same columns. `the_printed_chart_order_is_the_attested_order` pins print order ==
>   `displayed_charts()`.
> - **An end-to-end registration test** (`search_by_person.rs::a_registration_signs_every_member_of_a_linked_row`)
>   registers through `register_patient` after a search that matched only one chart of a linked pair, reads the
>   stored `search.displayed` back from `event_log`, and finds both charts, matched member first. The "Tests" list
>   above asks for a register over a linked row that signs both ids; the plan had no task for it, and the final
>   review restored it at the node (the pure half is `attestation.rs::a_linked_row_signs_every_member_in_row_order`).
>   The window's half is `commands.rs`'s `a_member_the_search_did_not_match_can_be_opened` and
>   `an_id_on_no_row_is_still_refused` (`AppState::shown` is filled from `CandidateList::charts()`, every member).
> - **Filed:** #722 (the `--mock` window has no linked pair, so a person row cannot be walked in `--mock`) and #723
>   (the front door calls a doubted set "One person"; its wording belongs with R5's doubt work, and R1b already
>   surfaces the doubt on open).

### R4 — the commit-time worker (#679)

- **Targeted blocking**: `candidate_pairs_for(conn, patient)` runs the six `_GROUPS_SQL` passes (and
  the two dob-range passes) **anchored on one patient**. Linear in the block, so the oversized-block
  guard sits higher; it still reports what it skips, never silently truncates.
- **Drift canary**: over a generated population, for every patient, anchored pairs == the full
  sweep's pairs involving that patient.
- **Skip rule**: never propose a pair already in one component, or with any `patient_link` row.
- **Queue**: `match_pending(patient_id PK, reason, queued_at)`; a trigger on the name, demographic
  and identifier projections upserts the patient and `pg_notify('cairn_match_pending', …)`. It
  **can never fail the write** — built like db/029's collision recorder (`ON CONFLICT DO NOTHING`,
  no raising path).
- **Worker** `cairn-matcher watch`: `LISTEN`; drain (reason `registration`/`assertion` before
  `config`); score; upsert `match_proposal` and delete the queue row in ONE transaction (a crash
  re-scores, never loses a patient); drain the backlog on start; store the last `matcher_version`
  run and, when it differs at start, queue every patient with reason `config`. **It never applies a
  link.**
- **Honest lag**: when the oldest queue row is older than a threshold, the front door and the
  worklist say *"Duplicate check is behind — last ran HH:MM"* — a down worker is a visible fact,
  never a false "no duplicates" (principle 4).

#### R4 — designed 2026-10-04

The bullets above were written before anyone read the matcher's write path. Three things in them do
not survive contact with the code, and the maintainer decided three questions in the brainstorm.

**The maintainer's decisions:**
- **R4 ships the node's status reads and a CLI line; the window's lines ship with R5.** Before R5,
  nothing in the window shows a proposal, so there is no "no duplicates" claim on screen for a down
  worker to falsify yet. The claim arrives with the banner, and its honesty line comes with it.
- **The worker is a standalone, operator-run process** (`cairn-matcher watch`), with a runbook
  section and example launchd/systemd units. It is not spawned by `cairn-sync`, which would tie the
  safety-critical daemon to the advisory tier, and not by the window, which would mean no checks
  while it is closed and one worker per window.
- **The queue is an append-only log of change notices**, not one keyed row per patient.

**What the bullets above get wrong, and the fix:**
- **"Delete the queue row" loses an update.** A name corrected while its chart is being scored would
  be deleted together with the request it replaced. The worker therefore deletes only the notices
  it read: `WHERE patient_id = X AND id <= <highest id read for X>`. A notice that arrives
  mid-check has a higher id and survives.
- **A keyed upsert lets the clinical write wait on the worker.** `ON CONFLICT` on a row the worker's
  transaction holds waits for that transaction, and under REPEATABLE READ it would raise. A plain
  insert of a fresh `bigserial` key cannot conflict and cannot wait. That is why the queue is
  append-only.
- **`runner.propose()` commits each pair itself**, so "proposal and queue delete in ONE transaction"
  needs a non-committing split (below).
- **"Oldest queue row older than a threshold" raises a false alarm for hours.** A restore, a
  `reproject --rebuild` or a new node's first pull queues every chart, and the oldest notice ages
  while fresh registrations are being checked within seconds. "Behind" is therefore measured by the
  NEWEST waiting notice. With a newest-first drain, a stale newest notice can only mean the worker is
  not running or is stuck. The precise claim a banner needs is per chart (below).
- The `registration`/`assertion` reasons are dropped: nothing reads the difference, and telling them
  apart would need a lookup inside the hook. `reason` is `change` or `config`, and labels the status;
  it does not order the drain.

**The database (`db/056`, `SCHEMA_GENERATION` 55 → 56):**
- `match_pending(id bigserial PRIMARY KEY, patient_id uuid NOT NULL, reason text NOT NULL CHECK
  (reason IN ('change','config')), queued_at timestamptz NOT NULL DEFAULT clock_timestamp())`,
  indexed on `(patient_id, id)`.
- **The hook** is one `SECURITY DEFINER` trigger function (`SET search_path = public, pg_temp`),
  attached `AFTER INSERT OR UPDATE … FOR EACH ROW` to the matcher's inputs: `patient_name`,
  `patient_demographic`, `patient_identifier`, `chart_identity_state` and `name_repudiation`. It is
  also attached `AFTER INSERT` only to `patient_chart`, for a new chart: that table carries its own
  name/dob/sex copy and db/002 updates it on EVERY clinical event (`last_activity`, `note_count`),
  so an UPDATE hook would queue a check on every medication write. The id column's
  name is a trigger argument (`subject` on the two identity tables). The body is `INSERT … SELECT …
  WHERE <id> IS NOT NULL` and then `pg_notify('cairn_match_pending', '')`.
- **"Never fails the write" is structural** (db/029's precedent): no `RAISE`, a null guard, and an
  insert that cannot conflict. It is deliberately not wrapped in `EXCEPTION WHEN OTHERS`, because a
  swallowed failure is a silently skipped check. What can still raise (a full disk, a dropped table)
  would fail the clinical write anyway. This replaces the bullet's "fault injected into the queue
  insert" test with a source guard and a lock test.
- **Which writes queue a check.** The projection upserts are conditional (`DO UPDATE … WHERE (new) >
  (old)`), so a row trigger fires only when an input actually changes. Normal use, app launch and
  reconnect queue nothing. An upgrade heal queues only the charts whose winner changed. A
  `reproject --rebuild`, a restore, a new node's first full pull and a matcher version change queue
  every chart. `NOTIFY` collapses identical payloads within one transaction, so a rebuild wakes the
  worker once.
- `match_worker_state` is a single row holding `matcher_version` and `last_drained_at`.
- Grants: `cairn_agent` gets `SELECT, INSERT, DELETE` on `match_pending` (INSERT for the config
  re-queue) and `SELECT, INSERT, UPDATE` on `match_worker_state`. The worker needs no actor and no
  key: a proposal is an advisory row, not an event.

**The worker (`matcher/`):**
- **One-chart blocking**: a new module `pipeline/targeted.py` (`db.py` is already 541 lines) with
  `candidate_pairs_for(conn, patient, max_block_size)`. It composes the SAME CTE constants as the
  sweep and keeps only the groups containing the patient. For the range passes, the patient may be
  the anchor or a member. It pairs patient × member only. The cap is higher than the sweep's 100,
  because pairs grow linearly here; its value is set from a measurement. Oversized blocks are
  reported, never silently dropped.
- **Skip rule**: `judged_partners(conn, patient)` returns every chart in the patient's component
  (`cairn_person_charts`) plus every chart sharing any `patient_link` row with it, and a pure filter
  drops those pairs. `sweep()` gains the same filter as an OPT-IN parameter, so its default behaviour
  and existing tests are unchanged.
- **`propose()` is split** into `assess()` (score, veto, band, no writes) and `persist()` (upsert or
  retract, no commit). `propose()` becomes `assess + persist + commit`, unchanged for the sweep.
- **One chart's check** (`worker.check_chart`). The read phase runs outside any long transaction:
  the targeted pairs, then the skip filter, then `assess` on each pair. It also re-assesses every
  PENDING proposal involving the chart that blocking no longer generates (the #210 reconciliation,
  per chart). The write phase is one short transaction: persist every outcome, delete the chart's
  notices up to the highest id read, stamp `last_drained_at`, commit. A crash re-checks the chart.
- **The loop** (`cairn-matcher watch`; `--once` drains and exits, for tests and cron):
  - `LISTEN cairn_match_pending`, and drain whatever is queued at start.
  - If the stored `matcher_version` differs from the running one, queue every chart with reason
    `config` and store the new version, in one transaction.
  - Each round picks a mode from the backlog. **Per chart, newest change first**: a fresh
    registration is checked within seconds whatever the backlog. **Above a threshold, one opted-in
    sweep**, then a delete of every notice up to the watermark read before the sweep started.
    Per-chart blocking scans the whole names table each time, so a full backlog checked chart by
    chart is ~N² work, where one sweep is a single pass.
  - A 60 s poll backs up `NOTIFY` (a notification can be missed across a reconnect), and a lost
    connection reconnects with backoff.
  - A chart whose check raises is rolled back, logged, and retried after a backoff held in memory.
    Its notices stay, so its per-chart status keeps saying "not yet checked".
  - Load: one connection, so at most one backend's worth of work at a time, plus an optional
    `--pace-ms` between units. No stronger claim is made.
  - **It never applies a link.** It only calls `persist()`, which writes `match_proposal`.
- Entry point: `[project.scripts] cairn-matcher = "cairn_matcher.cli:main"`. It takes the standard
  libpq environment or `--dsn`, and connects as the `cairn_agent` role.

**The node (`crates/cairn-node/src/duplicate_check.rs`):**
- `duplicate_check_status(client)` reads the number of charts waiting, the newest and oldest waiting
  notice, whether a `config` re-check is in progress, and `last_drained_at`. A pure
  `classify(status, now, threshold)` returns one of four states:
  - **NeverRun** (no worker-state row): *"Duplicate check has never run on this node."*
  - **Stalled** (the newest waiting notice is older than the threshold): *"Duplicate check is behind
    — last ran HH:MM; N charts waiting."*
  - **CatchingUp** (notices are waiting, the newest is fresh): *"Duplicate check running — N charts
    waiting"*, plus *"(re-checking all charts after a matcher update)"* during a `config` re-check.
  - **Current** (nothing waiting): *"Duplicate check up to date — last ran HH:MM."*
  
  The threshold is one named constant (5 min), soft policy.
- `chart_check_pending(client, patient) -> bool` is the per-chart truth R5's banner will use. On a
  chart still waiting, it lets the banner say *"Duplicate check not yet run for this chart"*, never
  a silent absence of a banner.
- `cairn-node duplicate-check [--patient <id>]` prints the line. All the wording lives in one pure
  function with a golden test. It exits 0: there is no monitoring contract yet.

**Tests** (replacing the R4 line under *Testing* below):
- **DB:**
  - every input table queues a notice on insert and update (`patient_chart` on insert only);
  - a re-assertion that does not win queues nothing;
  - the null guard holds;
  - a clinical write completes while an open worker transaction holds a `DELETE` on that patient's
    notices (`pg_stat_activity`, never a sleep);
  - a source guard: the hook contains no `RAISE`;
  - the grants, the generation and the pinned-count guards.
- **Python:**
  - the drift canary: over a generated population, for every chart, the targeted pairs equal the
    uncapped sweep's pairs that include it;
  - the skip rule (linked, unlinked, any `patient_link` row);
  - `propose()` behaves the same after the split (the existing suites stay green);
  - registering a near-duplicate produces a proposal;
  - a notice injected between the read and write phases survives;
  - a crash between the phases re-checks the chart;
  - a version change queues `config` notices;
  - the newest-first order and the mode choice (pure tests);
  - bulk mode's watermark;
  - an auto-band pair after a worker run leaves `patient_link` empty.
- **Rust:** `classify` in each state and at the threshold boundary; the wording golden; DB tests for
  both reads.

**§1.2:**
- Paper counterpart: the records clerk's possible-duplicate tray, filled overnight.
- Steps: at the desk, paper 0 → forced 0 → target 0 (the check is invisible).
- Time and cognitive load: zero added at the desk. The operator's status line is not a clinical
  gesture. R4 owes the registration-to-proposal latency on a generated population; the same
  measurement on the Pi is a filed follow-on.

**Out of R4:** the window's status lines and the banner's per-chart "not yet checked" (R5); a
monitoring exit code; the Pi measurement (filed).

### R5 — the banner and the worklist (#680)

- **Banner** (§5.2's): when any chart in the displayed set has an unresolved proposal, a banner sits
  **above** the medication list — *"Possible duplicate — not yet reviewed: Mary SMYTHE b. 1950-01-07
  (chart …)"* — with that chart's active medications read-only, labelled *"on the other chart — not
  part of this record"*, and **Review** (R2's panel: *Same person* → link, *Different people* →
  unlink). Ambient, never a modal, never re-popped (§5.12). The other chart's medications come
  through the SAME read, so §5.9 custody/sealing applies unchanged.
- **Worklist** "Possible duplicates (N)" from the front door, newest proposal first: both identity
  lines, band, veto findings, **Review**. Also lists D5's "a matcher link lost to a human judgement".
- **Convergence without syncing `match_proposal`**: the worklist is a VIEW — pending proposals minus
  pairs already resolved by identity events (same component, or any `patient_link` row). A
  colleague's link/unlink arriving by sync clears our entry.

## Error handling

- Worker down or behind → the honest-lag line; writes are never affected (the trigger cannot raise).
- A proposal whose pair has since been linked/unlinked → filtered from the banner and worklist by
  the view, not by a status write.
- The set changed under a command → refused with a reload sentence (D3), never a silent retarget.
- No unlocked human key → the link/unlink controls say what is needed (the existing unlock), never
  fall back to the node key: an identity judgement is a human's (ADR-0053).
- A §5.9-sealed body on the other chart → shown exactly as the read shows it on its own chart.

## Testing (TDD, per slice)

- **R1**: a linked pair reads as one list with source labels; a group inside the set shows once; a
  group reaching outside the set is flagged and sign-off-refused, from both sides (#334 regression);
  set changed between read and sign-off → refused; unlink splits the read; **a never-linked chart
  reads byte-identically to today** (golden comparison).
- **R2**: link/unlink author attested events and refuse without an unlocked human key; a vetoed pair
  can still be linked and shows its veto; **D5**: an un-attested link arriving by sync with a LATER
  HLC than a human `unlink` loses, at both doors, and converges on two nodes; the mirror — an
  un-attested `unlink` with a later HLC does not split a human `link`; the #190 veto flag still
  tracks the standing winner under the new order; the `attested` backfill
  replays idempotently (migration-replay guard).
- **R3**: a person row signs all member ids; ranking uses the best member; opening any member opens
  the set.
- **R4** (superseded by *R4 — designed 2026-10-04*'s test list): anchored == sweep (property test); the trigger never fails a write (fault injected into the
  queue insert); register a near-duplicate → proposal; linked and unlinked pairs never proposed;
  changed `matcher_version` queues a re-check; crash mid-drain re-scores.
- **R5**: the banner shows on both charts with the other chart's medications; the worklist hides a
  pair resolved by a synced identity event; the honest-lag line appears when the queue is stale.

## Paper-parity benchmark (§1.2)

Per-slice plans carry their own section; this is the whole path's claim.

- **Paper counterpart**: the records clerk's "possible duplicate" tray, and clipping two folders of
  one patient together after laying them side by side.
- **Steps**: at the desk, paper 0 → forced 0 → target 0 (the check is invisible; `M ≤ N`). To
  repair from an open chart, paper 3 (fetch the other folder, lay them side by side, clip) → forced
  3 (find, look, sign — the signature click IS the clip; per-write authorship, ADR-0053, adds no
  act while the key is unlocked) → target 3. From the banner or the worklist the "find" is already
  done: paper 3 → forced 2 → target 2. `M ≤ N` throughout. An unlock, when the key is locked, is
  the existing session act, not one this path adds.
- **Time + cognitive load**: budget owed by R2 (the first runnable gesture): review-and-link from
  the banner ≤ 20 s, of which the side-by-side read is the load; the banner adds zero acts to a chart
  open. Measured by the runbook, human act.

## Out of scope

- An allergy stream (every "active medications" above extends to allergies when one exists).
- Combining demographics across linked charts (a winner across members).
- The hub-tier sweep and cross-node matching (ADR-0014; this is the local node).
- Auto-linking in the auto band at commit time (a future policy decision, on evidence).
- `dispute` authoring, and role gating of who may link (policy; the mechanism does not foreclose it).
- New-content writes from a combined view (D2 states the rule for when they arrive).
