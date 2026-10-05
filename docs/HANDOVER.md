# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ TWO PRs AWAIT THE MAINTAINER'S MERGE, IN ORDER (2026-10-05):**
> 1. **PR [#724](https://github.com/cairn-ehr/cairn-ehr/pull/724) — R4, the commit-time duplicate check** (#679). Its
>    `ruff + pytest` job was RED at session start (six R4 test modules could not import psycopg in CI's no-extra job);
>    fixed in `38d6c393`, **all checks green**.
> 2. **PR [#733](https://github.com/cairn-ehr/cairn-ehr/pull/733) — #725, STACKED on #724** (base `feat/r4-commit-time-worker`; GitHub retargets it to `main` when #724
>    merges and its branch is deleted). Per-chart p95 **9.65 s → 1.4 s** at 10 000 charts (budget ≤ 2 s);
>    `bulk_threshold` 30 → 250. Plan `docs/superpowers/plans/2026-10-05-725-range-blocking-cost.md`; the design page's
>    "#725 — as fixed" note. #725 is left OPEN for the maintainer (the Pi figure is #728).
>
> R3 (PR #721), R1b (#717), R2b-2 (#711), R2b-1 (#707), R2a (#698) and R1 (#688) are merged. Repair path #679 · #680 ·
> #681. Design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`; runbook
> `docs/developers/running-the-duplicate-check.md`. `db/056`, **`SCHEMA_GENERATION` 56**; spec **v0.79**.
>
> **⇒ NEXT, in order:**
> 0. Check `gh pr list` before trusting this list (house rule 8).
> 1. **R5** (banner + worklist, #680 — R4's comment there lists what the banner must read; the worklist filters pairs
>    with a `patient_link` row, #700). **#716** and **#723** belong beside it. #699 stays open until the maintainer closes it.
>    **#728** (the Pi re-measure: `python -m cairn_matcher.eval.measure_check --sizes 2000 10000` on a vacuumed scratch
>    DB) can run beside it; if the Pi misses 2 s, the levers are in the #725 plan's "Out of scope".
> 2. **Open repair-path issues:** **#726** (every-connect DDL takes ACCESS EXCLUSIVE) · **#727** (bulk mode has no poison
>    isolation) · **#729** (mode chosen once per round) · **#730** (the 101–1000 skipped-block band — a decision; since
>    #725 the cap bounds real work) · **#731** (no "charts failing" signal) · **#732** (the matcher conftest leaves
>    `patient_chart`). Earlier: **#708** · **#709** · **#710** · **#712** · **#713** · **#714** · **#715** · **#716** ·
>    **#718** · **#719** · **#720** · **#722** · **#723** · **#699** · **#700** · **#702** (pinned) · **#703** · **#704** ·
>    **#705** · **#706** · from R1: #689 · #690 (a decision) · #691 · #692 · #693 · #694 · #695 · #696; #333, #220 and
>    #335 gained comments.
> 3. **Human acts still owed** — see *Four things still owed* below; the repair path adds a **linked chart's open**,
>    **runbook §10's unlink** (≤ 15 s), the **live Tauri-IPC + VoiceOver pass on a linked pair** (the #699 (a) third-chart
>    unlink, a doubted pair's withheld lines, R3's person row), and R4's **running a supervised worker** on a real node
>    per the runbook (launchd/systemd; one worker per node; its role's `statement_timeout` above the sweep's ~3 s range
>    query).
> 4. **#620**, the only open item that can still change the wire (brainstorm first). Then **#626**, **#652 + #655**,
>    the advisory-tier **#640**, **#641**, and **#682–#686** (#685 needs the maintainer's permission to clear
>    `cairn_test`'s fixtures).
>
> **⇒ #725'S DURABLE RULES — do not undo** (pins: `matcher/tests/test_blocking_sql_shape.py`,
> `test_targeted_blocking.py`, `test_pure_modules_import_without_psycopg.py`):
> - **No range arm joins a sex scan.** A chart's set of blocking sexes rides on its birth-window row (`sexed_window`,
>   one join by `patient_id`); the `+sex` arm is a FILTER (`anchor_sexes && member_sexes`). A CTE scan has no
>   statistics, so an equi-join of two sex scans on a ~2-valued column is a planner cross-product (14.7 M rows) —
>   even two scans of per-patient ARRAYS were cross-joined first.
> - **The anchored per-chart range statement is the sweep's plus `relevant_anchor` and `_ANCHOR_CLAUSE`, nothing
>   else**; the clause restricts the ANCHOR, never the member (a kept window is computed in full — its size is the
>   cap's verdict). Add a range arm to `_RANGE_ARMS` and both statements get it.
> - **`pipeline/db.py` is the ONLY module that imports psycopg at import time** (plus `eval/measure_check`, a DB tool).
>   Every local run uses `--extra pipeline`, so only the import guard catches a CI-only collection failure; blocking
>   SQL text lives in the pure `pipeline/blocking_sql.py` (`db.py` re-exports it). A psycopg-needing TEST imports it
>   inside the test or `importorskip`s it. Reproduce CI's pure job with `CAIRN_ALLOW_DB_SKIP=1 uv run --isolated pytest`.
>
> **⇒ R4'S DURABLE RULES (PR #724) — do not undo any of these** (pins in `tests/match_pending.rs`,
> `tests/duplicate_check.rs`, `matcher/tests/test_{check_chart,worker_drain,watch_backoff,targeted_blocking,judged}.py`):
> - **The hook never raises and never waits** (`the_hook_has_no_raising_path`, the `lock_timeout` test): a plain INSERT
>   of a fresh bigserial, a null guard, no RAISE/EXCEPTION/ON CONFLICT. **Never "fix" it with `EXCEPTION WHEN OTHERS`** —
>   a swallowed failure is a silently skipped check. `patient_chart`'s hook is **INSERT-only**; the catalogue test pins
>   the whole `tgtype` and that each argument names a `uuid` column.
> - **Delete exactly the notice ids READ, captured before the projection reads — never `id <= max`** (a bigserial is
>   assigned at INSERT, not commit; `test_a_slow_transaction_with_a_lower_id_is_not_deleted_by_the_check`). Bulk mode
>   captures the id set before the sweep.
> - **No read transaction across slow work**: `check_chart` rolls back after its read-prep and after every assess (the
>   NOWAIT probe test); writes commit in one short transaction. A no-op DDL replay WAITS behind any open reader (#726).
> - **"Behind" = charts waiting AND no completed work AND the OLDEST notice older than 5 min** (`quiet_age_s`;
>   `a_stopped_worker_on_a_busy_node_reads_stalled`, `a_backlog_queued_at_once_reads_running_while_the_worker_progresses`).
>   **Progress is stamped ONLY by completed work** — a checked chart, a successfully scored sweep pair (≤ 1/30 s), a sweep
>   with something scored or nothing to score, an empty round. Never before work, never for a failed pair: either masks
>   a crash-loop as "running" (`test_a_sweep_whose_blocking_raises_stamps_no_progress`, `…every_pair_fails…`).
> - **`cairn_chart_check_pending` is TRUE until a worker has run, while a notice waits, and for a chart not held here**
>   (definer; never a false "checked"). **The worker never links** (it writes only via `persist`; the auto-band test
>   pins `patient_link` and `event_log` unchanged) and **never proposes a judged pair, in either mode or in
>   reconciliation**. **`me` is canonicalised once** (`str(uuid.UUID(...))`) — `.lower()` silently dropped pairs.
> - **`cairn-node duplicate-check` uses `db::connect`, never the schema replay** (it is cron-able; the replay is #726).
>   Status wording lives only in `duplicate_check.rs` (goldens); the runbook quotes it verbatim.
>
> **⇒ R3'S DURABLE RULES (ADR-0076 decision 6, PR #721) — do not undo any of these:**
> - **Rows are built ONLY by `group_by_person`** (`cairn-patient-search/src/person.rs`: pure, shared so any picker
>   agrees with the node on what a row is; `search.rs` its only production caller). A row sits at its best-ranked
>   member; matched members first in rank order, then unmatched members oldest first; every chart exactly once; the
>   seven ADR-0075 keys still rank CHARTS. Pinned by `person.rs`'s tests and `tests/search_by_person.rs`.
> - **The signed list is ONLY `CandidateList::displayed_charts()`** — every member of every row shown, in row order;
>   whatever signs or prints candidates follows it. Pinned by `attestation.rs::a_linked_row_signs_every_member_in_row_order`,
>   the end-to-end `search_by_person.rs::a_registration_signs_every_member_of_a_linked_row` and
>   `candidate_text.rs::the_printed_chart_order_is_the_attested_order`.
> - **The cap counts ROWS and never splits one** (`withheld` counts people, never signed; `shown_charts` is what is
>   signed). Pinned by `prompt.rs::the_cap_counts_people_and_never_splits_a_linked_row`.
> - **A chart not held here reads `Unknown` and never sets `incomplete`** (a deliberate change to a SIGNED flag);
>   **a HELD chart with no readable name does, matched or not** (`search_person.rs::display_name_for`). Pinned by
>   `search_person.rs`'s `a_chart_not_held_here_reads_unknown_never_confirmed` and
>   `a_held_chart_with_no_name_ever_is_still_unreadable`, and `search_by_person.rs`'s
>   `a_matched_chart_not_held_here_with_no_name_is_not_a_partial_search`.
> - **A missing component fails the search — never a row of one** (one person in two prompt places is the
>   duplicate-creating failure): `MissingComponent`, and `read_components` refuses a component lacking its own chart.
>   Pinned by `person.rs::a_chart_with_no_component_read_is_an_error_never_a_silent_row_of_one` and
>   `search_person.rs::a_component_that_omits_its_own_chart_is_refused`.
> - **Every member of every row shown can be opened, and nothing else** (`AppState::shown` via `remember_shown`, from
>   `CandidateList::charts()`); the window never picks the chart for the clerk. Pinned by `commands.rs`'s
>   `a_member_the_search_did_not_match_can_be_opened` and `an_id_on_no_row_is_still_refused`.
> - **Over single-chart rows every sentence is byte-identical to before R3** — goldens
>   `view.rs::single_chart_summaries_are_byte_identical_to_before_r3`,
>   `candidate_text.rs::golden_two_never_linked_rows_print_exactly_as_before_r3`,
>   `link/search.rs::only_this_records_own_chart_matched`. People-vs-charts wording lives in `funnel/rows.rs`.
> - **The linked-row label reaches a screen reader through `aria-describedby`** on every member button (`funnel.js`
>   `personItem`; Tab never lands on the label). **No test pins it** (#332); the live VoiceOver pass checks it.
>
> **⇒ R1b'S DURABLE RULES (#697 (b), #701, PR #717) — do not undo any of these:**
> - **A doubted set withholds every line not recorded ONLY on the opened chart** (`medication::hazard::wrong_chart_reasons`,
>   pure, given the opened chart; an EMPTY chart list is not "only on the opened chart"). The opened chart's own lines
>   stay signable; the line stays on screen. Pinned by `hazard.rs`'s tests and `tests/doubted_link_withholds.rs`
>   (incl. the mirror test opened from the OTHER chart — the only test that proves `opened` is used).
> - **`cross_patient` is KEPT and set from the same reasons map as `wrong_chart`**; every production reader goes
>   through `MedicationRow::is_wrong_chart_hazard()` (either signal withholds) or `hazard_reasons()` (a reasonless
>   hazard is worded as the outside case). Never read either field alone in new code (#720).
> - **`hazard_reasons()` is STATUS-BLIND; `withheld_reasons()` / `withheld_rows()` say what THIS gesture withholds.**
>   Anything that says a line "cannot be signed until …" or points at the withheld report asks `withheld_reasons()`.
> - **The rule judges every chart a group touches, INCLUDING each member thread's own chart** (`read.rs`
>   `touched_charts`) — the chart sign-off writes to. A row missing from the reasons map is a hazard (`row_reasons`).
> - **"Doubted" has THREE cases in db/054 `cairn_chart_set_has_doubted_link`:** a `link_veto_flag` row; an UN-attested
>   standing link that trips the hard veto now (#220's path); an ATTESTED unlink between two charts still in the set
>   (the A–C–X bridge). It reads the STORED `pl.attested` (#701) — never re-derive attestation through `event_log` —
>   and must be handed one record's charts (`person_charts`).
> - **The doubted remedy is a judgement of the LINKS, never thread separation, and never promises "either judgement
>   lifts this hold"** (`DOUBTED_LINK_INSTRUCTION`). Each sentence must stay true in every case shown.
> - **The CLI prints the long remedy ONCE below the list** (`list_text::doubted_link_note`, decided by `withheld_rows`);
>   the outside-set row lines stay byte-identical (golden in `list_text.rs`).
> - **An UN-attested unlink inside the set is deliberately NOT a doubt** (db/054, `hazard.rs`, a DB test): counting an
>   agent's unlink would let any unreviewed writer freeze sign-off.
>
> **⇒ THE LINK PRECEDENCE FLOOR'S DURABLE RULES (R2a, ADR-0076 decision 5) — do not undo any of these:**
> - **`patient_link`'s winner order is ATTESTED FIRST, then HLC** (`cairn_link_overlay_wins`, db/018), a total order.
>   "Attested" is ONE definition (`attester_key IS NOT NULL AND cairn_attestation_vouched(event_id)`), evaluated ONCE
>   per applied event and STORED with the winner; a new reader reads `pl.attested`, never a second spelling.
> - **`db/055`'s EXISTENCE is load-bearing** (generation 55; the loader's heal re-decides winners the old order chose).
>   **Never fold it into db/018, and never delete its re-fold block** — cairn-sync heals with the OLD applier, so a
>   cairn-sync-first load would use up the heal (#703). A peer on an older binary ranks the old way (identity.md §5.2).
> - **Recorded is not took effect**: every judgement path reads back, in its own transaction, what now stands
>   (`chart_link::standing_link`, `LinkEffect`); auto-apply rolls back a matcher link that is not itself the winner.
> - **One lock order: the `match_proposal` row, then db/018's CARNLK** (reversed, a same-pair race deadlocks, 40P01;
>   a `pg_stat_activity` Lock-wait test in `tests/chart_link.rs`).
> - **A judgement is a human's** (`Reviewer`: the human key signs AND attests; no node-key fallback). `link` needs BOTH
>   charts held; `unlink` also admits a displayed member not held here, filed under the HELD chart (db/005 step 8b).
>   Only OPEN proposals move.
>
> **⇒ R2b-1'S DURABLE RULES ("Same person as…", PR #707) — do not undo any of these** (pins: `chart_compare.rs`,
> `link/mod.rs`, `link/view_tests.rs`, `chart_link.rs`, each named after its rule):
> - **The comparison is SET against SET** — `cross_vetoes` over every left×right pair of BOTH sets, or a B–C clash goes
>   unseen. **Link names BOTH COMPARED sets** (`left_charts`/`other_charts` sent back verbatim; `link_impl` refuses a
>   changed set as `THIS_CHANGED` / `OTHER_CHANGED`).
> - **Every panel message goes through `setMessage`, never a bare `.textContent`** (an empty status line is `hidden`).
>   No JS harness pins it (#332): a headless walk must assert `getComputedStyle(el).display !== "none"`.
> - **A partially-read comparison offers no Link.** **Absence and precision wording lives in `link/view.rs`** (on a
>   chart not held here every ABSENT fact reads "unknown — registration not yet received here"; arrived struck names
>   are listed; a coarse DOB names its precision). **An identifier finding is never "verified"**; only a dob/sex-at-
>   birth hard veto (both provenance-rank ≥ 60) is "Verified facts differ". **A locked key is not a verdict**
>   (`Retry::Now`).
> - **Code selects by a typed field, never a display label** (`MedListRowView::current`); **the panel's search counts
>   what it shows**. **`chart_link`'s pre-check refusals are marked verdicts — never a bare `anyhow::bail!`** (same
>   chart → `RefusalScope::Input`; not held or a non-human key → `NodeState`, #702).
>
> **⇒ R2b-2'S DURABLE RULES ("Not the same person…", ADR-0077, PR #711) — do not undo any of these** (pins:
> `admit_tests.rs`, `tests/unlink_from_record.rs`, `tests/chart_link.rs`, `tests/record_edges.rs`, `unlink_view_tests.rs`):
> - **`FiledUnder::RecordOf` is UNLINK-ONLY, re-checked UNDER CARNLK in `assert_link_in_tx`** (after the proposal-row
>   lock), then `person_charts(opened)` is re-read — a re-read merely inside READ COMMITTED raced a sync-door unlink.
>   It lives in the signing core because `assert_link_in_tx` is `pub`
>   (`a_peer_unlink_landing_mid_judgement_is_seen_before_anything_is_signed` parks on the advisory lock).
> - **"Still joined?" asks the SUBJECTS** (`high ∈ person_charts(low)`), never the filed-under or open chart
>   (`a_split_reads_took_effect_whichever_way_round_the_pair_is_named`); the record REPORTED is the open chart's.
> - **An opened chart / `--from` unrelated to the pair is REFUSED**, even when a held subject alone would admit it.
>   **The list is per LINK, never per member**; `record_edges` keeps `state = 'link'`
>   (`an_unlinked_pair_is_not_a_link` queries a set holding BOTH charts).
> - **The unlink panel is its OWN `<section>`; the two panels are mutually exclusive**; after an unlink focus goes to
>   the patient heading (walked headless, #332). **`Outranked` never says a retry "changes nothing"** (the sync merge
>   is bounded at 24 h of drift). A locked key names its own button; `standing_edge`'s unread list is `Retry::Now`.
> - **An open chart's refusal scope follows the FACT** (`AdmitRefusal`): not held → `NodeState`; held but its record
>   lacks the pair → `Input`, "reload the chart and judge again".
> - **The webview-fields guards scan `main.js`, `funnel.js` and `unlink.js`**; `link.js` has only its search read
>   guarded (R3) — the rest is #715. `renderLinks` stays in `main.js`
>   (`commands.rs::the_webview_reads_no_field_the_backend_does_not_send`). Never say "by the matcher" for an
>   un-attested link ("without a clinician's confirmation on record here").
>
> **⇒ THE COMBINED READ'S DURABLE RULES (R1, ADR-0076) — do not undo any of these:**
> - **A combined list's duplicate flag is db/054's `cairn_medication_duplicate_groups` over the SET**, never the
>   per-patient `patient_medication_reconciliation_flag` (it groups by `patient_id`, so one drug on two LINKED charts
>   is never flagged — a double-dose reading hazard). Its dup_key stays db/033's (`medication_dup_key_drift.rs`).
> - **The medication read selects groups by MEMBERSHIP over the chart set** (filtering by `patient_id` is the #334
>   defect returning); `list_chart_set_medications` is PRIVATE. **A cease on a wrong-chart-hazard line stops only the
>   opened chart's threads** and names the rest (`chart_set::cease_plan`).
> - **`MedicationRow::display_chart`** (serialized `patient_id`) is where the group DISPLAYS, never a write target —
>   use `source_charts` / `MemberVouch::patient_id`. **Sign-off and cease act on each thread's OWN chart**; the floor
>   does not yet enforce it (**#689**).
> - **A linked chart not held here reads trust `unknown`** (`person::trust_of`) — the search shares it since R3. **A
>   missing group** uses `MISSING_GROUP_INSTRUCTION`, never the separation remedy.
> - **EVERY CHART COMMAND NAMES THE DISPLAYED CHART SET** (ADR-0076 decision 3): `displayed_patient` FIRST, then the set
>   (`cairn-gui-tauri/src/chart_set.rs`); `sign_off_medication_list(…, displayed: Option<&ChartSet>)` refuses a changed
>   set; `None` is the CLI. **A failed member-identity read keeps the list and warns** (`ChartPane::members_error`).
> - **A never-linked chart gains no header, label or line** (golden
>   `combined_read.rs::a_never_linked_chart_reads_exactly_as_before`). Sign-off over a non-empty
>   `groups_missing_from_chart` has no DB coverage (same class as #333).
>
> **⇒ THE FUNNEL'S DURABLE RULES — do not undo any of these** (full text: the funnel design page's dated notes):
> - **`search_patients` RANKS BY SEVEN KEYS** (`rank_candidates`, inputs in `patient/search_rank.rs`): passes →
>   identifier → a §5.4 callsign typed WHOLE → name tokens (exact or a ≥3-byte prefix, as `db/046`; callsigns never
>   split) → DOB near-miss → EXACT tokens → chart age. It only REORDERS; each key has a victim if dropped
>   (`patient_search_ranking.rs`). Tokens count over the RETAINED names (`patient_name`, #349).
> - **`incomplete` is the SEARCH's partiality only; truncation is `withheld`** (ADR-0075; folding them re-creates #671;
>   `attestation_through_the_port.rs`). **Never raise `PROMPT_CAP` to make a number look better.**
> - **The raw typed name travels WITH its token** (`FunnelSession`); an older revision's search is DROPPED; the
>   webview forgets its token on every edit. **`require_provisioned` runs in the register command, BEFORE `take`.**
> - **Only a chart some list on screen showed can be opened** (`AppState::shown`; since R3 every member of a row).
>   **EVERY CHART COMMAND NAMES THE CHART ON SCREEN** (`AppState::displayed_patient`; 2c's Critical — a sign-off once
>   signed patient B on a review of A's list): each has a `*_impl` pinned by a not-on-screen test; `open_patient` is
>   private to `funnel` — **never let a new chart command resolve `open_patient()` alone.** A registration never
>   writes, or switches charts, behind an open chart (`register_impl` + `open_after_registering`).
> - **A click within 800 ms of a prompt landing is "show me", never "register"** (`PROMPT_READ_GUARD_MS`, SOFT POLICY
>   in `funnel.js`, #677). **The launch probe matches all four `ActorStanding` arms** (a `Retired` key meets db/004's
>   resurrection refusal, #152), paired with the key it probed (#670).
> - **Every sentence and its retry advice lives in `funnel/view.rs`** (`Retry::{Now, AfterOperator, Never}`; person-row
>   wording in `funnel/rows.rs`). A refusal (`P0001`, `DeliberateRefusal`) is not an outage; the remainder is #655.
> - **`TokenStore::settle` ends a `take`**; a success INVALIDATES; `discard` does NOT clear `in_flight`. **A dropped
>   `register` future still latches the store (#669) and `register` is cancellation-unsafe (#649): never race it
>   against a timeout or `select!`.** The step-3 trigger is advisory, never a gate.
> - **Nothing provisions an actor on a write path** (`enrolment_is_never_a_write_side_effect.rs` — **when it goes red,
>   do not add your call site to `ALLOWED`**). `resolve_matcher_actor` still enrols (#663); a SUPERSEDED key's class is
>   undecided (#666, then #664). **`init` must not `?` its enrolment.**
> - **`cairn-gui-live` holds the DB-backed ports.** The P0001 rule has three homes (#652); the `gui` job's step-level
>   `CAIRN_ALLOW_DB_SKIP=1` is load-bearing and its deletion invisible (#656); derived truncate lists miss identity
>   tables (#658). **`--mock` holds ONE `MockData`** (half of #668); its matching is NOT db/046's and it has no link
>   concept — every mock row is a person of one (#722).
>
> **Open from the funnel run:** #355 · #645 · #647 · #649 · #650 · #652 · #655 · #656 · #657 (multi-event rollback
> untested in both trees) · #658 · #662 · #663 · #664 · #665 · #666 · #667 · #668 · #669 · #670 · #672 (identifier
> entry) · #673 (the header shows age, not DOB) · #676 (the clerk reads `operator_chain` text) · #682 (an NFD trailing
> accent is lost) · #683 · #684 (`1980-3-7` misses `1980-03-07` in db/046's DOB pass — a SET gap) · #685 · #686.
>
> **⇒ THE NODE PLANE AND DR ARE CLOSED OUT; NO DECIDED-AND-UNBUILT ITEM REMAINS THERE.** Newest first:
> #621 ([ADR-0074](spec/decisions/0074-a-deterministic-door-failure-is-a-refusal-not-a-fault.md)), #619
> (ADR-0073), #614 + #615 (ADR-0072), #594 (ADR-0071: `restore` exits **3 INCOMPLETE**, **1 = BLOCKED**),
> #584 (ADR-0070), ADR-0069, the §1.2 restore measurement (100 003 events in 116.7 s against 600 s;
> **#512** stays open, `M > N`), DR slices 2c/2d (ADR-0067/0068). A solo clinic can lose its disk,
> restore, **open a chart** and rehearse it unattended. The durable rules are traps 1–18 below.
> - **⚠️ Citation discipline.** Rows coming back is NOT a body opening: cite
>   `restore_reads_the_clinical_plane.rs` and `restore_cli_surface.rs::a_scripted_restore_brings_the_clinical_record_back`,
>   never `dr_clinical_guarantee_gap.rs`'s counts. **Never cite ADR-0026 decision 1's promise 2** as met: no
>   node-default key tier exists (ADR-0067).
> - **The pen-release rule (#578):** a pen row carrying a wrapped DEK is released only when custody for its event is
>   SETTLED (`cairn_release_pen_row`, db/052; `pen_rows_leave_through_one_door.rs`). #585: nothing reads Postgres
>   notices. **`verify-backup` (#567)** fails `backup SHORT` only on evidence; operators run it AFTER `backup`.
>   Residuals #551 · #553 · #589 · #590 · #591 · #592.
> - **#527/#562's triage note is false**; the real fix is **#575** (the minted recovery code reaches stderr). A retry
>   after a crashed restore must move the installed `<key>.unwrap` aside first (#596).
> - **Open decisions (none a patch):** #575 · #602 (any client can set `cairn.remote_apply`) · #611 · #613 · #620.
>   **Restore residuals:** #616 · #617 · #596–#599. **Races (reasoned, not reproduced):** #603 · #604. **PR #601's
>   wave:** #605 · #606 · #607 · #608 · #609 · #610. **Node plane:** #268 · #301 (both `loop:needs-human`) · #569.
>   **From #619/#621:** #622 · #624 · #625 · #626 · #628 · #629 · #631 · #632 · #633 · #634. **Search:** #636 · #637
>   (the materialised token table — the right fix for the ~860 ms Pi floor, its own slice) · #639 · #640 · #641 · #643.
> - **Still broken, named rather than assumed away:** #549 · #552 · #536 · #502 item 4 · #101 items 2–3 ·
>   #583 · #586 · #587 · #556–#563.

> [!WARNING]
> **⇒ CODEQL: ZERO OPEN ALERTS (measured 2026-09-12), KEPT THAT WAY BY A MODEL PACK — AND ONE HUMAN
> ACT IS DUE: make `CodeQL (rust)` a required check (#444).** PR #576 replaced default setup with a
> committed workflow + model pack (`.github/codeql/packs/cairn/codeql-models`, one `barrierModel` row per
> NAME-heuristic source, each with its reason). The flip needed the **organization-level** configuration as
> well as the repository's. **Read the alert list with `scripts/codeql-alerts.sh`, never assume it.**

> [!IMPORTANT]
> **⇒ GITHUB CLOSES ON ADJACENCY, NOT ON SENTENCES (2026-09-04).** Seven issues (#101, #115, #434,
> #441, #468, #500, #534) were closed by prose *disclaiming* the close; all reopened. Guarded by
> `scripts/check_closing_keywords.py` + `.github/workflows/closing-keywords.yml` (**#444** would make it
> required). **`fix(#500):` is SAFE** — the parenthesis breaks the adjacency. Run the script BEFORE you
> commit: the slip lives in the commit message and fixing it needs a force-push. Residuals **#547**, **#548**.

> [!IMPORTANT]
> **Eighteen traps. Each is a step a next session takes in good faith.** The full argument for each is in its ADR,
> its test's header or ROADMAP; this keeps the rule, the pin and the tempting wrong fix.
>
> 1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY** (`keystore::adopt_derived_unwrap_secret`); anywhere
>    else re-creates the #495 coupling. `unwrap_secret_is_not_derived.rs` sweeps every shipping tree and asserts each
>    allow-list entry is live: **when it fails, delete the entry; never add one.** It and `is_a_test_gate_attribute`
>    (pgrx's `#[cfg(any(test, feature = "pg_test"))]`) move **together**.
> 2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6): a database
>    recreated under an existing key file needs `cairn-node establish-unwrap-key` before its first sealed write. Never
>    make a red fixture green by weakening `ensure_unwrap_key`.
> 3. **`cairn-sync`'s ONE derived fallback** (no `<key>.unwrap` AND the derived key equals the registered one → start,
>    warn every startup). An absent file may fall back; a present-but-unusable one never may — **never merge those two
>    arms.** Retiring the fallback is **#514**.
> 4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It registers a secret
>    from the NEW seed and the singleton registrar then refuses the real key for good; recover the export, or restore
>    again into a fresh database.
> 5. **⇒ `Secret32` DOES NOT SEPARATE ONE SECRET ROLE FROM ANOTHER (#511)** — `Secret32::from_bytes(sk.to_bytes())`
>    compiles. `secret32_conversions_are_named.rs` pins exactly **two** seed→unwrap-secret sites (the ADR-0066
>    migration, trap 3's fallback); a third is the defect returning. `unwrap_secret_is_the_signing_seed` and
>    `secret_opens_the_carried_custody` are not made redundant by the types.
> 6. **`init` refuses a database that already has custody registered** (it reads `node_unwrap_key` first) and names
>    `establish-unwrap-key` as the remedy — on a restored node read trap 4 first.
> 7. **⇒ A BODY SHREDDED AFTER A CAPTURE KEEPS ITS DEK ON THAT MEDIUM. NOT A LEAK — DO NOT "FIX" IT.** A backup
>    restores the state at capture time; invalidating old backups is rotation policy (principle 9; ADR-0067 decision
>    2). **Never filter old segments** (`medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`).
>    Operator half owed: **#589**.
> 8. **⇒ A PEN ROW WHOSE DEK "BELONGS TO ANOTHER NODE" IS STILL RETAINED (#578).** *"Did not open with the key we
>    have now"* is not *"not ours"* (the right `<key>.unwrap` may be on a USB stick); the escape is db/021's human
>    `acked`. Pinned by `requeue_releases_custody.rs::a_penned_dek_from_another_node_is_kept_until_a_human_decides_otherwise`
>    and `requeue_retains_unlanded_custody.rs`. **Look-alike fix:** keying the guard on the opened `dek`, not the
>    row's `dek_wrapped`, passes the headline test and skips the check whenever the key did not open. Keep the Rust
>    check although the floor also catches it — it says WHY.
> 9. **RETIRED (#584, ADR-0070); live residue: never remove or move the `cairn_project_late_custody` calls** — in both
>    doors AFTER the substitution guard, in db/020 BEFORE the `cairn.remote_apply` clear (wrapping db/005's call in
>    `cairn.remote_apply = 'on'` turns a strict refusal into a flag). Pinned in `late_custody_reaches_the_chart.rs`,
>    each placement with its own positive control. (#597's notice is still misleading.)
> 10. **⇒ WHEN `late_custody_guards.rs` FIRES, THE GUARD IS RIGHT.** Every `event_clear` writer (`apply_remote_event`,
>     `submit_event`) calls `cairn_project_late_custody`; every applier reading custody is `heal_safe = TRUE`. **Wrong
>     fixes:** a custody reader `heal_safe = false` (a silent owed rebuild — **#610**), flipping a non-idempotent one
>     to `TRUE`, exempting a writer. A third writer is a DECISION; a custody read in a helper, `MERGE` or dynamic
>     `EXECUTE` is invisible to the guard — review by hand.
> 11. **⇒ `restore` EXITS 3 FOR AN INCOMPLETE RECOVERY; A TEST EXPECTING 1 IS THE BUG (#594, ADR-0071).** Exit 1 =
>     BLOCKED, checked FIRST. **Wrong fixes:** `success()` for a torn medium; a wrong recovery code exiting 1 (its end
>     state is the no-registry state, 3); "simplifying" `past_chain_break` to the medium's clinical total (M8). The
>     precedence's ONE test is `restore_cli_surface.rs::without_the_flag_a_piped_restore_still_inherits_no_custody`
>     — never weaken it to `!success()`; `restore_exit_vocabulary.rs` pins the value,
>     `exit_incomplete_matches_cairn_nodes_restore` the agreement. ⚠️ **Write `Some(n)`, never `!status.success()`.**
>     ⚠️ An ACKED pen row counts in `penned` and `requeue` will not clear it; do not subtract it. Residuals **#611**,
>     **#616**.
> 12. **⇒ THE SUBSTITUTION REFUSAL HAS ONE HOME (`cairn_refuse_substitution`, db/053), AND A DOOR CALLS IT.** `IS
>     DISTINCT FROM` ("cannot tell" is a refusal); a fourth inline copy is #608's `<>` fail-open returning
>     (`substitution_guard_is_single_source.rs`). db/005/db/020 read under `GET DIAGNOSTICS`; **db/009 reads
>     unconditionally — do NOT "tidy" it into a `ROW_COUNT` check**, its guard AFTER the `IF/ELSE` (M7); a db/009
>     refusal aborts the WHOLE restore, correctly. A DEFERRED clinical record is REPORTED, never an exit-3 cause. All
>     five event-log doors: `substitution_guard_covers_every_writer.rs` over `pg_proc`. Residuals #608, #605, #569, #622.
> 13. **⇒ THE NODE PLANE'S SUBSTITUTION REFUSAL LIVES IN EACH DOOR'S TAIL; THE PULLER ASKS THE TABLE, NOT THE ERROR
>     (#619, ADR-0073).** An early `RETURN` bypasses the guard (the genesis arm, safe only for want of an `ON
>     CONFLICT`) and the catalogue guard will NOT notice a new one. **Wrong fixes:** a dedicated SQLSTATE (P0001 is a
>     contract, #228); matching the sentence; penning only when the GUARD raised
>     (`a_rival_refused_by_an_earlier_check_is_still_penned`); a skip for the failed-lookup freeze
>     (`node_substitution_lookup_freezes.rs`, M10); `offered` over the whole frame (#268's alarm fatigue, M11). A
>     sixth writer gets the call. Residuals #620, #622, #605.
> 14. **⇒ A DOOR THAT LETS POSTGRES RAISE ON CALLER-SUPPLIED BYTES BREAKS THE P0001 CONTRACT (#621, ADR-0074).** Every
>     signed-bytes field goes through a P0001 helper (`cairn_uuid_or_raise` on `pg_input_is_valid`,
>     `cairn_hlc_nonneg_or_raise`, `cairn_node_role_or_raise`); `node_door_input_guards.rs` fails on a bare `::uuid`.
>     **Wrong moves:** a REGEX narrower than the cast (M9); re-inlining the role list into the CHECK (M10); `USING
>     ERRCODE`; deleting the CHECKs (the raw-SQL floor). ⚠️ **The puller's default for an UNKNOWN SQLSTATE is pen;
>     "tightening" it to freeze reinstates #621.** The local classes (`08 40 42 53 55 57 58`, no SQLSTATE) equal
>     `cairn-sync`'s (`sqlstate_classes_agree.rs`; merging the copies is #626); **`XX001`/`XX002` are LOCAL**. Such a
>     pen row leaves only by applying or an ack. **The role CHECK stays `NOT VALID`** (a validating pair re-scans on
>     every connect). Known exception: `cairn_body`'s `22P05` on a NUL (**#628**). Residuals #626, #629, #605, #268,
>     #625, #631, #632, #633, #634.
> 15. **⇒ THREE THINGS IN `db/046`'s PASS 3 LOOK LIKE NOISE AND ARE EACH WORTH HUNDREDS OF MS ON A PI (#639).** (a)
>     **`OFFSET 0` is an optimisation fence** — removing it changes no result, so nothing fails. (b) **The lateral is
>     `UNION ALL` on purpose.** (c) **The parts-branch skip must test `lower(normalize(pn.value, NFC))`** — U+0130 `İ`
>     lowercases to `i` + U+0307; dropping `lower` LOSES A TOKEN (ICU only).
>     `the_subset_argument_holds_for_every_unicode_code_point` checks all 1,114,111 code points;
>     `the_subset_probe_still_describes_the_query_db046_runs` pins the composed expression. **A probe list is a sample,
>     not an argument; re-measure with `scripts/measure_patient_search.py`, do not reason.** Residuals #641, #640,
>     #643, #637.
> 16. **⇒ A GUARD AND THE THING IT GUARDS MUST BE ASKED ABOUT THE SAME STRING** — pin the COMPOSED expression as a
>     literal (`include_str!` + `contains`), not its pieces.
> 17. **⇒ LOCAL POSTGRES IS ICU, CI's IS libc, AND `lower()` DIFFERS** (full vs simple case mapping). Derive such rows
>     from the SERVER (`patient_search_equivalence.rs`'s `full_case_mapping`). To reproduce CI: `CREATE DATABASE
>     cairn_test_libc TEMPLATE template0 LOCALE_PROVIDER libc LOCALE 'en_US.UTF-8' ENCODING UTF8;` + `CREATE EXTENSION
>     cairn_pgx;`. **Any test touching case, collation or character classes runs against both before pushing.**
> 18. **⇒ A TARGETED `cargo test --test X` REPORTING `ok` IS NOT PROOF — ONLY THE SWEEP IS (#661).** A stale test
>     binary (shared `target/`, a running rust-analyzer) reported `ok` twice against source it contradicts — silent
>     and green. **Gate on the sweep**; `CARGO_TARGET_DIR=/tmp/…` when an IDE is open; re-run every CI gate in CI's
>     order AFTER the final edit.

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)): parts A+B (authority floor + operator
surface) BUILT, enforcing nothing beyond display/emission; C+D DESIGNED, C1 the next §5.9 BUILD.** Read
**ADR-0062/0063/0064/0065** first. The authority floor is ONE predicate `cairn_claim_authority` (db/005) at ONE site
(db/048's `NOT EXISTS`) — **#245**'s first SQL counterpart, not its mirror; operator-surface §1.2 budget met (residual
**#436**). **C+D (ADR-0065)** are a custody ladder (admission → named nodes → named actors) under one invariant:
**narrowing changes the cost and noise of reading, never whether content can be REACHED** (audited break-glass at every
rung; rung-1 glass is a NETWORK act, **#498**). The node's DEK is the keyring, the floor the glass (LOCAL); custody
composes by INTERSECTION, which can EMPTY (**#499**); it narrows on `event`/`patient`, never `thread`. **C1** = rung 1
(`custody.nodes`, both doors, serve-door withholding) + audited break-glass + in-chart location signal; rung 2 is
**#496** (needs a reader identity, §5.11). Related: **#494** (ADR-0052's `event_dek` sentence vs the built table),
**#377**, **#235** (shred authorization hooks), **#236** (FTS/RAG must build on `event_clear`).
**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant stands, so
`cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new column must be granted in db/049
§8 (`safety_read_grants.rs`), and that grant is cost-raising, not a floor (**#425**, **#427**; **#432** asks whether a
node should attempt one at all). Slice-65 follow-ons: **#374**, **#378** (withdrawal rationale is clear text forever —
the UI must warn today), **#379**, **#436** (#374/#379 each need a DECISION). The `arrayref` residue is **#454**.

> [!IMPORTANT]
> **Two code traps that outlive their slices, because both look like tidy-ups.** (1) **`content_address IS NOT
> NULL` is the "did anything win" test — never `subject_kind <> 'none'`** (`none` is a legal open-vocabulary value;
> ADR-0062 E6). (2) **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0
> withholds reject power (safe); here it would withhold protection. ADR-0065 adds a third member that agrees for a
> DIFFERENT reason (it withholds quiet access, reachable by break-glass) — do not carry that justification into a
> site where reachability is not guaranteed.

**Four things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 stopwatch figures** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy: sections 1–7 for the med-list gestures (only the *write* half is measured, median 222 ms,
**PARTIAL**; write-cost half **#360** unwired; also a **linked chart's open**, budget ≤ the single-chart open) and
**section 8 for the front door** (find ≤ 5 s, register ≤ 20 s, live and `--mock`; db/044's `gesture_kind` CHECK still
refuses a registration timing row until widened); (2) **the accessibility pass** — a live VoiceOver run through the
runbook's checks, front door included, keyboard-only, and on a **linked pair** (the source-chart label, #691; R3's
person-row label); DOM assertions automated by **#332**; (3)+(4) **make CI jobs REQUIRED status checks** (**#444**,
admin-only — "clippy + cargo test (cairn-gui)", "cargo doc (API surface)", and `CodeQL (rust)`). **If a measurement
falls outside its budget, that is the finding — file an issue, never adjust the budget.**

**Other build candidates** (nothing blocks a choice): the **drugref term→anchor lookup** (the §9 advisory tier; closes
the coded↔uncoded case ADR-0059 decision 5 leaves open; needs a connection-model decision; `safety_class_map` its empty
seam) · **the node/actor plane's two divergences** — db/007 fail-closes on an unmappable type (**#301**), and the node
puller skips-and-advances a verifiable refusal where the clinical one pens it (**#268**); both `loop:needs-human`.

**Standing gate:** whole-project review cycles repeat periodically; no release for clinical use before repeated cycles
pass cleanly. Last full pass 2026-07-15 (#187–#217), fully closed; the runnable clinical surface has never been
through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human session
> holds the main repo — they contend on one cargo lock and one `test_serial_guard` advisory lock. **A live IDE
> contends the same way** (rust-analyzer holds `target/`); use a scratch `CARGO_TARGET_DIR=/tmp/…`, never kill
> the IDE. Loop gaps **#326**, **#312**, **#322**.

---

**Session date:** 2026-10-05 (**#725 — the per-chart check's range blocking**, PR #733 stacked on #724; PR #724's red CI
fixed) · 10-04/05 R4 (ADR-0076 decision 7, PR **[#724](https://github.com/cairn-ehr/cairn-ehr/pull/724)**, awaiting merge) · 10-03 R3 (PR #721) and R1b (PR #717) ·
09-30 R2b-2 (ADR-0077, PR #711) · 09-29 R2b-1 (PR #707) · 09-27 R2a (PR #698, `db/055`) and R1 (ADR-0076, `db/054`,
PR #688) · 09-26 #671 (ADR-0075, PR #678) · 09-23 funnel 2c (PR #674) · earlier: ROADMAP. · **Spec:** **v0.79** (newest
ADR-0077; [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's
implementation wording only) · **`SCHEMA_GENERATION`:** **56** (`db/056`) · **Phase:** architecture complete; **first
production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 window: the funnel front door (one row per
person) onto a medication chart that reads linked charts as one; a per-node matcher worker proposes duplicates.

**Built so far** (orientation only; ROADMAP + ADR log + git carry the detail): demographics 1–5 · §5.2 advisory Python
matcher · §5.7 identity core C1–C5 (C5+ `reattribute` waits on a clinical-note surface) · §5.4 John-Doe (§5.12
push-alert open) · §5.3/§5.8 funnel (ADR-0061; precedence #345 at db/005 step 8b; ranking ADR-0075; one row per
PERSON since R3, PR #721) · `clinical.medication` 1–6b under born-sealed bodies (ADR-0052) + per-write human authorship
(ADR-0053 — grading half-live until #245) · §5.9 stream through its read surface · med-list node tier (read + whole-list
sign-off over a linked set, R1) · human link/unlink judgements (`chart_link`, attested-first `patient_link`, R2a) · the
compare-and-link panel (R2b-1, PR #707) + "Not the same person" / unlink from a record (R2b-2, PR #711) + a doubted set
withholds every line not on the opened chart (R1b, PR #717) · the commit-time duplicate check (`db/056`, `cairn-matcher
watch`, `cairn-node duplicate-check`; R4, PR #724; per-chart range blocking anchored, #725) · generic reprojection (ADR-0057; ADR-0070) · ADR-0056
admit-uninterpreted floor · **the L3 reference UI** `cairn-gui/` (standalone workspace, one-way GUI → crates;
`cairn-gui-tauri`, the iced shell FAILED a11y, spike 0004; plain JS, no npm); pane/routing/freshness state machine
tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and every open issue number; this keeps only lessons that generalise.

### 2026-10-05 — #725, and PR #724's red CI

- **⇒ A CI job that installs a different dependency set from every local run cannot be gated locally — turn the
  difference into a test.** R4 passed every local gate (`--extra pipeline`) and failed CI's no-extra job at COLLECTION;
  `test_pure_modules_import_without_psycopg.py` imports the package in a child where `import psycopg` fails. **⇒ "All
  local gates green" was reported while a PR check was red** — read `gh pr checks` before calling a PR ready.
- **⇒ Diagnose before choosing between an issue's candidate fixes.** #725 named "the blocking SQL" and offered two
  remedies; the cost was ONE arm's planner cross-product, and my own first fix (sex arrays) made the same mistake — an
  `EXPLAIN ANALYZE` of the prototype showed it. Prototype against the old statement and assert identical rows per chart.
- **⇒ Editing an `include_str!`'d `db/*.sql`, even a comment, relinks every cairn-node test binary** (hours under macOS
  Gatekeeper); a stale-but-resolvable pointer there was left as is. **⇒ `git push` over HTTPS had no credentials in
  this shell** while `gh` was authenticated: `git -c credential.helper= -c 'credential.helper=!gh auth git-credential'
  push`, no config change.

### 2026-10-04/05 — R4: the commit-time duplicate check (PR #724)

Brainstorm (three maintainer decisions) → design addendum → plan (seven tasks) → subagent-driven; four task fix loops,
an opus final review, one fix wave. **Most of the review effort went into one sentence: when may the node say "behind"?**
- **⇒ A health signal must be walked through every way its input fills.** "Behind = the newest notice is old" failed for
  an all-at-once backlog (a restore: everything old at once); "newest" then hid a stopped worker on a busy node. The rule
  that survived: the OLDEST notice or the last COMPLETED work, whichever is later. **⇒ Progress is stamped only by
  completed work** — a stamp before work, or for a failed pair, makes a crash-loop read "running" forever (both shipped
  in fix rounds before a re-review caught them).
- **⇒ A bigserial is assigned at INSERT, not at commit:** `DELETE … WHERE id <= max_read` deletes a notice a slower
  transaction committed later. Capture the ids you read; delete exactly those.
- **⇒ A no-op DDL replay waits:** `ALTER TABLE … IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS` and `CREATE OR REPLACE
  TRIGGER` take their lock before checking (probed on PG 18). Any long reader — and every new routine worker is one —
  turns a node connect into a write stall (#726). A read-only CLI should use `db::connect`, not the replay.
- **⇒ Diagnose a slow measurement before ruling on it.** The plan's stop rule (> 5 s → file, don't tune) held; my
  hypothesis (the cap) was wrong, the 30-second diagnostic showed the blocking SQL (#725).
- **⇒ A callee's `rollback()` silently undoes a test's uncommitted `SET ROLE`** — a role test went false-green; assert
  `current_user` inside the code under test. **⇒ An interrupted subagent's uncommitted edits are evidence** (again):
  the resumed fixer found the inherited test right and the inherited SQL incomplete.

### 2026-08-20 → 10-03 — restore, node plane, doors, search, funnel UI 2a–2c, #671, repair path R1–R3

Narrative: ROADMAP; durable rules: traps 7–18 and the per-slice blocks above; each plan carries its review ledger.
- **⇒ A remedy or count sentence is a safety claim — walk it through every case it is shown in** ("either judgement
  lifts this hold" was false in three reachable cases). A plan can mandate a false claim and silently drop a design test
  — diff the two test lists; fact-check an ADR sentence by sentence against the SQL; ask what the desk will do before
  designing what it must attest (#671); a design's quantitative assumption is a claim.
- **⇒ "Inside the transaction" is not "serialized"** — take the lock every writer holds (CARNLK) first; test by parking
  the victim on it (`pg_stat_activity`), never a sleep. Recorded is not took effect. A migration that changes a
  projection's order must re-decide what the old order decided (db/055; #703). A predicate keyed on standing LINKS
  misses standing UNLINKS; keep an old flag, add reasons beside it, route every reader through one method.
- **⇒ A test that cannot see its own predicate proves nothing — mutate to check** (a mutation is the RED phase of a pin
  over shipped behaviour; SQL predicates need a REBUILD). A guard only ever green proved nothing (#586); copying a guard
  spreads its fail-open (#608); "untestable" is a claim — try a `SET ROLE` seam; a DB-suite "ok" can be a self-skip; a
  headless walk asserts visibility; an issue's scenario and scope are CLAIMS; a door returning `Ok` is not the record
  coming back; content-addressing over unsigned bytes is not content-addressing (#620).
- **Mechanics:** a sweep without all three DB strings is not a sweep; grep every claim about the tree (#654); an
  atomicity probe that never reaches the server is decoration (#657); a new DB suite in a non-root tree runs nowhere
  until wired (#656); a derived truncate list is only its predicate (#658); a red gate can be a predecessor's (#583); a
  new definer writes `SET search_path = public, pg_temp`; `nohup … &` and `cmd; echo exit=$?` lie; a new ADR needs its
  `mkdocs.yml` nav line. Filed: #712 · #716 · #718 · #719 · #720 · #722 · #723; #335 comments. Still open: **#569**,
  **#598**, #624.

**⇒ OPEN ISSUES OLDER SESSIONS OPENED, INDEXED** (never drop an open number while condensing):

[#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) · [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327) · [#394](https://github.com/cairn-ehr/cairn-ehr/issues/394) · [#402](https://github.com/cairn-ehr/cairn-ehr/issues/402) · [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) · [#407](https://github.com/cairn-ehr/cairn-ehr/issues/407) · [#408](https://github.com/cairn-ehr/cairn-ehr/issues/408) · [#409](https://github.com/cairn-ehr/cairn-ehr/issues/409) · [#413](https://github.com/cairn-ehr/cairn-ehr/issues/413) · [#420](https://github.com/cairn-ehr/cairn-ehr/issues/420) · [#422](https://github.com/cairn-ehr/cairn-ehr/issues/422) · [#428](https://github.com/cairn-ehr/cairn-ehr/issues/428) · [#430](https://github.com/cairn-ehr/cairn-ehr/issues/430) · [#431](https://github.com/cairn-ehr/cairn-ehr/issues/431) · [#447](https://github.com/cairn-ehr/cairn-ehr/issues/447) · [#458](https://github.com/cairn-ehr/cairn-ehr/issues/458) · [#463](https://github.com/cairn-ehr/cairn-ehr/issues/463) · [#464](https://github.com/cairn-ehr/cairn-ehr/issues/464) · [#470](https://github.com/cairn-ehr/cairn-ehr/issues/470) · [#483](https://github.com/cairn-ehr/cairn-ehr/issues/483) · [#484](https://github.com/cairn-ehr/cairn-ehr/issues/484) · [#485](https://github.com/cairn-ehr/cairn-ehr/issues/485) · [#487](https://github.com/cairn-ehr/cairn-ehr/issues/487) · [#488](https://github.com/cairn-ehr/cairn-ehr/issues/488) · [#490](https://github.com/cairn-ehr/cairn-ehr/issues/490) · [#491](https://github.com/cairn-ehr/cairn-ehr/issues/491) · [#492](https://github.com/cairn-ehr/cairn-ehr/issues/492) · [#494](https://github.com/cairn-ehr/cairn-ehr/issues/494) · [#504](https://github.com/cairn-ehr/cairn-ehr/issues/504) · [#505](https://github.com/cairn-ehr/cairn-ehr/issues/505) · [#506](https://github.com/cairn-ehr/cairn-ehr/issues/506) · [#507](https://github.com/cairn-ehr/cairn-ehr/issues/507) · [#508](https://github.com/cairn-ehr/cairn-ehr/issues/508) · [#509](https://github.com/cairn-ehr/cairn-ehr/issues/509) · [#513](https://github.com/cairn-ehr/cairn-ehr/issues/513) · [#518](https://github.com/cairn-ehr/cairn-ehr/issues/518) · [#521](https://github.com/cairn-ehr/cairn-ehr/issues/521) · [#522](https://github.com/cairn-ehr/cairn-ehr/issues/522) · [#529](https://github.com/cairn-ehr/cairn-ehr/issues/529) · [#530](https://github.com/cairn-ehr/cairn-ehr/issues/530) · [#543](https://github.com/cairn-ehr/cairn-ehr/issues/543) · [#545](https://github.com/cairn-ehr/cairn-ehr/issues/545) · [#557](https://github.com/cairn-ehr/cairn-ehr/issues/557) · [#558](https://github.com/cairn-ehr/cairn-ehr/issues/558) · [#559](https://github.com/cairn-ehr/cairn-ehr/issues/559) · [#560](https://github.com/cairn-ehr/cairn-ehr/issues/560) · [#561](https://github.com/cairn-ehr/cairn-ehr/issues/561)

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not. **`docs/spikes/`** — 0001 (walking
skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor, C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on
Android, G0–G3 ✓); 0004 (iced UI — FAIL on a11y → Tauri 2). **`docs/case-studies/0001`**: 16 GP-software failure modes,
all absorbed, **0 new architecture**. **`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance.
Code workspace: `/crates` (`cairn-event`, `cairn-keystore`, `cairn-medium`, `cairn-wire`, `cairn-sync`, `cairn-node`,
`cairn-medication-view`, `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace);
`poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS pinned to the
  trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate, genesis-stable `node_id`.
  Custody travels, the clinical event log reaches the medium, and a restore reads it back (ADR-0067); optional escrow
  rungs (Shamir/QR/TPM) remain. **Dual-identifier discipline** (ADR-0031): the canonical plane (UUIDv7 + multihash) is
  the only identifier on the wire; the projection plane may intern node-local `bigint` surrogates (`db/008`).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx`, self-serializing via a Postgres advisory lock
  (`db::test_serial_guard`). **Advisory locks are scoped PER DATABASE** (#467; ~124 comments still say otherwise,
  **#476**), so every caller takes the guard against `CAIRN_TEST_PG`.
- **Tech-debt loop** — `/techdebt-loop` triages, `/techdebt-next` works one issue per headless session; STOPPED (above).

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **⇒ The repair path, R4 → R5** — see ⇒ NEXT.
- **⇒ DR — closed and rehearsable end to end.** Not there, though a reader expects it: **2d does NOT drive
  `cairn-sync`'s puller through `MediumTransport`**; **the per-peer quarantine quota does not apply to a
  restore-originated pen** (`restore_pen_is_uncapped.rs`). Filed by the chain, beyond ⇒ NEXT's node-plane list: #525 ·
  #541 (no CI job compiles `cairn_pgx`'s `pg_test`) · #531/#329 (decompose `cairn-sync/src/main.rs` — maintainer picks
  one) · #532 · #534 · #535 · #537 · #538 · #624–#626. (Ranges absorb issues that leave them: re-check against GitHub.)
- **§5.9 parts C/D** (#232) — above.
- **`clinical.medication` — slices 1–6b DONE, read over a linked set since R1.** Next: #690 (reconciling across linked
  charts), the **drugref term→anchor lookup**, fuzzy/automatic reconciliation + a Tier-A dictionary, structured
  sig/frequency, correcting a dose event's effective date. Cross-cutting debt **#185**. Spine: `db/031`–`db/035`,
  `db/041`, `db/042`, `db/054`. **No allergy stream exists yet** — the repair path's "active medications" becomes
  "allergies and active medications" when one does.
- **Demographics / matcher / identity — next slices** (`db/010`–`db/030` + `cairn-event::demographics`). B3-driven: gold
  set, locale packs, hub-tier duplicate sweep (R4 is its per-node sibling), proposal retraction. Identity: C5+
  `reattribute` (waits on a clinical-note surface); §5.12 push-alert. Deferred **#168**, **#287**; rest in ROADMAP.
- **⇒ Test env — `scripts/run-db-gated-tests.sh` is the ONE command for the local gate**, the only one catching all
  three demonstrated hiding modes (fail-fast · a piped exit status · a cross-crate suite `-p <crate>` never builds).
  **The CLUSTER is discovered** (`scripts/pg-target.sh`, PG ≥ 18; set `PGPORT` to name one). **Cost depends on what
  changed — measure, never budget from a figure here.** Without the three env strings the DB suites self-skip, so a
  bare run FAILS unless `CAIRN_ALLOW_DB_SKIP=1` is declared (#450). Mirrors are DESTRUCTIVE, refusing any DB lacking the
  `cairn_scratch_database` marker (#169). Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest` (uv,
  never pip; gap **#314**). `clinical_pull` flakes under a full run: `--test-threads=2`.
- **Clinical case-mining** — historically the highest-signal generative mode. Bring a real ED/hospital failure mode;
  record in [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation**, a projection surfaced by salience not a modal; **③ impossible-vs-uncertain**
  for the in-DB floor.
- **Landing-page polish** — a non-developer page for the generated site (`web/`).

**Blocked on hardware / external access:** **Bet B — Pi compute-cost run**
([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)): PASS twice; fold the un-caveated B4 number into
ADR-0015 to drop "provisional"; **#272** (reproject bench on the Pi). **easyGP session** — port
[ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s deferred items with
live schema access (`rx!`/`tx!` parser + state machine; formulation/drug source + forced-manual rule table; prefetch
warming daemon); pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`. **Byte-tier throughput lever** — connection
reuse / persistent streaming instead of one TCP connection per slice.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** legal entity & jurisdiction — deferred until momentum/funding
  geography is clearer; trademark registration — principle recorded, instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session.** Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured.
