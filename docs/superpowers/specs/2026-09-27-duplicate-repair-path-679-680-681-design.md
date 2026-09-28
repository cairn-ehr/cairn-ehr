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
   record**; rows names (with use), earlier recorded names, DOB + provenance, sex at birth +
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

### R3 — the front door collapses by person

- Search results group by `cairn_person_charts`; a person row lists each member's name + DOB and
  ranks by its best member's keys (the seven keys of ADR-0075 decision 5, unchanged per member).
- Opening a person row opens the combined set; `AppState::shown` records every member.
- The step-3 prompt signs every member id of every shown row, in row order (D6). A row still takes
  one of the five `PROMPT_CAP` places.

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
- **R4**: anchored == sweep (property test); the trigger never fails a write (fault injected into the
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
