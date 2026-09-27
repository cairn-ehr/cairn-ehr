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
