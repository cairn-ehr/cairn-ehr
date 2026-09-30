# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ R2b-2 — "NOT THE SAME PERSON" (UNLINK) + #699 (a) — IS BUILT ON PR
> [#711](https://github.com/cairn-ehr/cairn-ehr/pull/711) (2026-09-30), DRAFT until the final whole-branch review +
> gates.** R2b-1 (PR #707), R2a (PR #698) and R1 (PR #688) are merged. Repair path #679 · #680 · #681; design
> `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (R2b section + both as-built
> notes), [ADR-0076](spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)
> and [ADR-0077](spec/decisions/0077-an-unlink-may-be-filed-under-the-record-it-was-judged-from.md) (#699 (a)), spec
> **v0.79**; plan `docs/superpowers/plans/2026-09-30-repair-path-r2b2-not-the-same-person.md`. Built: `chart_link/{admit,judge}.rs`
> (`FiledUnder`, the pure admission rule; `unlink-charts --from`); `patient::edges::record_edges`; the pane's
> "How these charts are linked" list (one entry per link); `link/{record_links,unlink,unlink_view}.rs`
> (`compare_linked`/`unlink_records`); `src-ui/unlink.js`; runbook §10. No SQL object (generation still 55).
>
> **⇒ NEXT, in order:**
> 0. Check `gh pr list` before trusting this list (house rule 8).
> 1. **#697 (b)** (decided: while a set holds a doubted link, every line not on the opened chart is withheld from
>    sign-off, with its own wording; do **#701** alongside), then **R3** (the front door collapses by person), **R4**
>    (per-node matcher worker, #679 — proposes, never links), **R5** (banner + worklist, #680 — the worklist must
>    filter pairs with an attested `patient_link` row, #700). Merge R2b-2 first (PR #711 — the maintainer closes
>    #699 if he agrees ADR-0077 resolves it; no closing keyword was used).
> 2. **Filed 2026-09-29 (R2b-1):** **#708** (`link_charts` should re-check both compared sets inside its
>    transaction; plus a DB-gated window test of `compare_impl`/`link_impl`) · **#709** (a link outcome can go
>    unseen when it lands after the chart changed) · **#710** (review-round residuals: Link offered for an unheld
>    chart, pre-load Compare wording, fixture facts, typed veto severity, one read snapshot). **Filed 2026-09-27 (R2a):** **#699** (an unlink where neither chart is held here was refused, though both
>    show on a held chart's record — DECIDED (a) and built in R2b-2, ADR-0077; awaiting the maintainer's close) · **#700** (auto-apply's human-judged skip has a race; a skipped
>    proposal stays `pending`) · **#701** (db/054's doubted-link check should read `pl.attested`) · **#702** (a
>    floor refusal through `chart_link` surfaces as a bare `db error` — addressed on PR #698, now pinned).
>    **Filed 2026-09-28 (PR #698 review):** **#703** (the generation heal can be used up by `cairn-sync init` on
>    a shared database, silently skipping a node-only migration's heal — the class behind db/055's own re-fold) ·
>    **#704** (make db/019's `applied_event_id ⇔ applied` a CHECK) · **#705** (`link-charts` against a database
>    at an older generation judges under the old order, silently) · **#706** (R2a test follow-ups: precedence
>    over the sync wire, deferred-promoted vouch, upgrade re-fold of vetoed pairs). #700's race half is handled
>    on PR #698 (auto-apply confirms its link stands); its pending-forever half remains. **From R1:** #689 (db/034 admits an
>    attestation naming another chart — floor gap) · #690 (reconciling across LINKED charts is refused — a
>    decision) · #691 (full-uuid source labels) · #692 · #693 · #694 · #695 · #696; #333 and #220 gained comments.
> 3. **Human acts still owed** (an agent cannot do them): the runbook stopwatch figures — now also a **linked
>    chart's open** — and the **live Tauri-IPC pass on a linked pair**. See *Four things still owed are HUMAN
>    acts* below.
> 4. **#620**, the only open item that can still change the wire (the COSE unprotected header is hashed into the
>    content address but lies outside the signature); brainstorm first. Then **#626**, **#652 + #655** together,
>    the advisory-tier **#640**, **#641**, and **#682–#686** (#685 needs the maintainer's permission to clear
>    `cairn_test`'s fixtures).
>
> **⇒ THE LINK PRECEDENCE FLOOR'S DURABLE RULES (R2a, ADR-0076 decision 5) — do not undo any of these:**
> - **`patient_link`'s winner order is ATTESTED FIRST, then HLC** (`cairn_link_overlay_wins`, db/018). A total
>   order, so every node converges. "Attested" is ONE definition (`attester_key IS NOT NULL AND
>   cairn_attestation_vouched(event_id)`), evaluated ONCE per applied event and STORED with the winner — safe
>   because a vouch never changes after first projection (db/043 gate 1 runs before gate 4). Never add a second
>   spelling; a new reader reads `pl.attested`.
> - **`db/055`'s EXISTENCE is load-bearing** — it moved the generation to 55, and the loader's generation-change
>   heal re-decides winners the old order chose. A column backfill cannot. **Never fold db/055 into db/018.**
>   **And never delete its re-fold block**: cairn-sync shares the generation and heals with the OLD applier, so
>   a cairn-sync-first load would otherwise use up the heal (#703). A peer still on an older binary ranks the
>   old way until it upgrades (stated in identity.md §5.2).
> - **Recorded is not took effect.** db/018 admits an assertion that loses the overlay. Every judgement path
>   reads back, in its own transaction, what now stands (`chart_link::standing_link`): a human judgement
>   reports `LinkEffect` (by whether the standing assertion AGREES — a later agreeing one is not a
>   disagreement), auto-apply rolls back a matcher link that is not itself the winner rather than mark it applied.
> - **One lock order everywhere: the `match_proposal` row, then db/018's CARNLK advisory lock**
>   (`chart_link::assert_link_in_tx` pre-locks the row; `auto_apply` and `apply_accepted_proposal` read it `FOR
>   UPDATE` first). Reversing it deadlocks a same-pair race (40P01). Pinned by a `pg_stat_activity` Lock-wait
>   test in `tests/chart_link.rs`.
> - **A judgement is a human's**: `link_charts`/`unlink_charts` take a `Reviewer` (the human key signs AND
>   attests); there is no node-key fallback. `link` needs BOTH charts held here; `unlink` also admits a displayed
>   member not held here (same record), and is then filed under the HELD chart (db/005 step 8b). Only OPEN
>   proposals (`pending`/`accepted`/`review`) move; closed rows are never touched (`applied`/`auto_applied` by
>   db/019's invariant; `rejected`/`retracted` by design — `patient_link` holds what stands).
>
> **⇒ R2b-1'S DURABLE RULES ("Same person as…", PR #707) — do not undo any of these:**
> - **The comparison is SET against SET, never chart against chart.** With A open and already linked to C,
>   linking B to A also joins B to C — `cross_vetoes` runs `cairn_match_veto` over every left×right pair across
>   BOTH sets, not one A–B pair, or a B–C clash would go unseen. Pinned by
>   `chart_compare.rs::a_clash_with_a_third_chart_already_in_the_record_is_found`.
> - **Link names BOTH COMPARED sets**, widening decision 3 to the right-hand side: `ComparisonView` carries
>   `left_charts` and `other_charts`, the webview sends exactly those back (never its current `renderedCharts`,
>   which a re-read can grow between Compare and Link), and `link_impl` refuses a changed left set as
>   `THIS_CHANGED` and a changed right set as `OTHER_CHANGED` — a judgement is never signed over a set nobody
>   compared. Pinned by `link/mod.rs`'s `compare_is_bound_to_the_chart_on_screen`,
>   `compare_refuses_a_changed_set`, `link_refuses_when_this_record_changed`,
>   `link_refuses_when_the_other_record_changed`, `link_is_bound_to_the_chart_on_screen`.
> - **Every panel message goes through `setMessage`, never a bare `.textContent`.** An empty status line is
>   `hidden`, and style.css's `[hidden]` wins — a bare write puts the words in the DOM but never on screen nor
>   in the screen reader (the first build's refusals and Outranked sentence were all invisible). No JS harness
>   pins it (#332): a headless walk must assert `getComputedStyle(el).display !== "none"`, not text.
> - **A partially-read comparison offers no Link.** Reading stays available on a failed member read (availability
>   over consistency), but the judgement needs the whole picture. Pinned by
>   `view_tests.rs::a_partial_comparison_names_what_is_missing_and_cannot_link`.
> - **Absence and precision wording live in `link/view.rs`, not in the node read or the JS panel.** On a chart not
>   held here every ABSENT fact reads "unknown — registration not yet received here" (not just DOB) — but struck
>   names that HAVE arrived are listed; a coarser-than-day DOB names its precision (`FieldFact::precision`,
>   principle 4). Pinned by `view_tests.rs::an_absent_fact_says_not_recorded_or_unknown`,
>   `::struck_names_on_a_chart_not_held_here_are_still_shown`, `::a_coarse_dob_names_its_precision`.
> - **An identifier finding is never labelled "verified".** db/016's identifier severity says whether both values
>   passed a format profile, not how either was sourced; only a dob/sex-at-birth hard veto (both provenance-rank
>   ≥ 60) is "Verified facts differ". Pinned by `view_tests.rs::an_identifier_finding_is_never_called_verified`.
> - **A locked key is not a verdict** (`view::key_locked`, `Retry::Now`): the Link button survives it — as
>   `Never` it forced a second Compare. Pinned by `view_tests.rs::a_locked_key_is_not_a_verdict`.
> - **Code selects by a typed field, never a display label** (`MedListRowView::current`, not `status_label ==
>   "current"`: a reworded label would have emptied the list into a false "no current medications"); and **the
>   panel's search counts what it shows** (`link_search` filters this record's charts in Rust). Pinned by
>   `view_tests.rs::only_current_medications_are_listed_and_warnings_carry_over` and `link/search.rs`'s tests.
> - **`chart_link`'s pre-check refusals are marked verdicts — never revert to bare `anyhow::bail!`.** Same chart
>   is `RefusalScope::Input`; a chart not held here or a non-human attester key is `RefusalScope::NodeState`
>   (#702). Pinned by `chart_link.rs`'s `refusal_scope` assertions in `a_chart_cannot_be_linked_to_itself`,
>   `a_chart_this_node_has_never_seen_is_refused_before_signing`, `a_non_human_key_is_refused_and_nothing_moves`.
>
> **⇒ R2b-2'S DURABLE RULES ("Not the same person…", ADR-0077, PR #711) — do not undo any of these:**
> - **`FiledUnder::RecordOf` is UNLINK-ONLY, and re-checked in the transaction.** A link filed under a third chart
>   is refused before anything is signed; a `RecordOf` unlink re-reads `person_charts(opened)` inside the
>   judgement's own transaction and refuses unless it holds both subjects. Pinned by `admit.rs`'s
>   `a_link_is_never_filed_under_a_third_chart` and `an_unlink_neither_held_is_filed_under_the_opened_record_that_holds_both`,
>   `record_holds_both`'s `a_record_holds_both_only_when_it_contains_each`, and `tests/unlink_from_record.rs`
>   (`the_open_chart_must_hold_both_in_its_record`, `without_an_open_chart_a_neither_held_unlink_is_still_refused`).
>   The in-transaction re-read has no race test (only the pure helper is unit-tested).
> - **"Still joined?" asks the SUBJECTS** (`high ∈ person_charts(low)`), never the filed-under chart — else every
>   successful A–B–C split reads `StillJoined`. Pinned by `unlink_from_record.rs::a_chain_split_from_the_opened_chart_took_effect`
>   and `::a_link_on_a_cycle_is_recorded_and_says_still_joined`, and `tests/chart_link.rs::an_unlink_through_a_third_chart_is_recorded_and_says_it_did_not_split`.
> - **An opened chart / `--from` unrelated to the pair is REFUSED**, even when a held subject alone would admit the
>   unlink (a stray flag is never silently ignored). Pinned by `admit.rs::an_unrelated_open_chart_is_refused_even_when_a_subject_is_held`
>   and `unlink_from_record.rs::an_open_chart_unrelated_to_the_pair_is_refused_even_when_a_subject_is_held`.
> - **The list is per LINK, never per member** (in A–C–B only a human can say which clip is wrong; per member the
>   outcome is `StillJoined` by construction). `record_edges` keeps `state = 'link'` — an unlinked pair is not a
>   link. Pinned by `tests/record_edges.rs::an_unlinked_pair_is_not_a_link` (queries a set holding BOTH charts —
>   the first shape could not see its own predicate) and `record_links.rs::a_link_line_names_both_charts_how_it_was_made_and_when`.
> - **The unlink panel is its OWN `<section>`, and the two panels are mutually exclusive** (opening either closes
>   the other); after a successful unlink focus goes to the patient heading, not `<body>`. Walked headless, no
>   committed JS harness (#332).
> - **`Outranked` never says a retry "changes nothing"** — a retry records a NEWER judgement that overrules the
>   other (HLC merge at both sync doors). Pinned by `unlink_view_tests.rs::outranked_does_not_call_a_retry_a_no_op`
>   (and `view_tests.rs`'s link twin). A locked key names its own button (`key_locked_for`;
>   `view_tests.rs::a_locked_key_names_the_button_that_was_pressed`); `standing_edge`'s unread list is
>   `Retry::Now`, never `LINK_GONE` (`unlink.rs::an_unread_edge_list_is_retryable_and_not_a_verdict`).
> - **The webview-fields guard scans only `main.js`**, so `renderLinks` (and every payload field it reads) must
>   live there — `commands.rs::the_webview_reads_no_field_the_backend_does_not_send`. Never say "by the matcher" for
>   an un-attested link: a peer's human link with an un-enrolled attester is also un-attested ("without a
>   clinician's confirmation on record here").
>
> **⇒ THE COMBINED READ'S DURABLE RULES (R1, ADR-0076) — do not undo any of these:**
> - **A combined list's duplicate flag is db/054's `cairn_medication_duplicate_groups` over the SET, never
>   the per-patient `patient_medication_reconciliation_flag`.** The view groups by `patient_id`, so the same
>   drug on two LINKED charts is two groups on two patients and never flagged — "simplifying" back to it
>   re-hides two unflagged lines for one drug, a double-dose reading hazard. Its dup_key must stay identical
>   (whitespace-normalised) to db/033's; `medication_dup_key_drift.rs` pins it.
> - **The medication read selects groups by MEMBERSHIP over the chart set.** Filtering the views by their
>   `patient_id` (the display winner) is the #334 defect returning. `cross_patient` (`read.rs`
>   `is_wrong_chart_hazard`) = the group reaches a chart OUTSIDE the set, OR the set holds a **doubted link**
>   (db/054 `cairn_chart_set_has_doubted_link`: an un-attested link db/018 flagged, or that trips the hard
>   veto NOW — #220's late-clash path) and the group spans more than one chart. Dropping the second half makes
>   a line across a vetoed machine link signable with the other person's dose. `list_chart_set_medications`
>   is PRIVATE: re-reading a stale displayed set would silently drop the withholding.
> - **A cease on a `cross_patient` line stops only the opened chart's threads** and names the rest
>   (`chart_set::cease_plan`) — never writes a cessation onto a chart that may be someone else's.
> - **`MedicationRow::display_chart`** (serialized `patient_id`) is the chart the group DISPLAYS under — not
>   "the chart this line is on", never a write target. Use `source_charts` / `MemberVouch::patient_id`.
> - **A linked chart whose registration is not held here** (`ChartIdentity::held`, a `patient_chart` row)
>   reads trust `unknown` (`person::trust_of`), never the no-row default `confirmed`.
> - **A missing group is no longer the cross-patient case**: its reports use `MISSING_GROUP_INSTRUCTION`
>   (reload / projection repair), never the separation remedy.
> - **EVERY CHART COMMAND NAMES THE DISPLAYED CHART SET, not just the chart** (ADR-0076 decision 3; widens
>   PR #674's Critical below). `displayed_patient` is still checked FIRST, then the set
>   (`cairn-gui-tauri/src/chart_set.rs`). `sign_off_medication_list(…, displayed: Option<&ChartSet>)`
>   refuses when the displayed set ≠ the first read, or the first read ≠ the second; `None` is the CLI
>   (skips only the displayed compare).
> - **Sign-off and cease act on each thread's OWN chart** (`MemberVouch::patient_id`), never the opened
>   chart — the floor does not yet enforce it (**#689**).
> - **A failed member-identity read keeps the list and shows a warning** (availability over consistency):
>   `ChartPane { list, members, members_error }`. Never let the header's completeness hide the drugs.
> - **A never-linked chart gains no header, label or line** — pinned by the golden
>   `combined_read.rs::a_never_linked_chart_reads_exactly_as_before`, captured BEFORE the read was
>   rewritten. `groups_missing_from_chart` is now empty by construction; sign-off's handling of a non-empty
>   one has no DB coverage (same class as #333).
>
> **⇒ THE FUNNEL'S DURABLE RULES — do not undo any of these** (full text: the funnel design page's dated
> notes and ROADMAP's funnel entries):
> - **`search_patients` RANKS BY SEVEN KEYS** (`cairn_patient_search::rank_candidates`, inputs in
>   `patient/search_rank.rs`): passes → identifier matched → a §5.4 callsign typed WHOLE → name tokens
>   matched (exact or a ≥3-byte prefix, as `db/046` matches; callsigns never split) → DOB near-miss →
>   tokens matched EXACTLY → chart age. It only REORDERS. Each key has a victim if dropped (an MRN-only
>   match buried, a whole-callsign John Doe sunk below every "Ed …", "Alex" tying every namesake); plain
>   `ids.sort()` shows the OLDEST charts. `patient_search_ranking.rs` fails on it. Tokens are counted over the
>   RETAINED names (`patient_name`, repudiated included — #349), never `patient_name_current`.
> - **`incomplete` is the SEARCH's partiality only; truncation is `withheld`** (ADR-0075). Folding them
>   back re-creates #671. `attestation_through_the_port.rs` pins both halves. **Never raise `PROMPT_CAP`
>   to make a number look better.**
> - **The raw typed name travels WITH its token** (`FunnelSession`); a search for an older form revision
>   is DROPPED; the webview forgets its held token synchronously on every edit.
> - **`require_provisioned` runs in the window's register command, BEFORE `take`**, not inside
>   `PatientRegistration::register` (a pre-check in the port would leave the port suites' db/005 proofs
>   green and empty).
> - **Only a candidate some list on screen showed can be opened** (`AppState::shown`).
> - **EVERY CHART COMMAND NAMES THE CHART ON SCREEN** (`AppState::displayed_patient`; 2c's Critical — a
>   sign-off once signed patient B on a review of patient A's list). A chart command acts on the chart
>   DRAWN (`renderedPatient`), each has a `*_impl` pinned by a not-on-screen test, and `open_patient` is
>   private to `funnel`. **Never let a new chart command resolve `open_patient()` alone.** A registration
>   never writes, or switches charts, behind an open chart (`register_impl` + `open_after_registering`).
> - **A click within 800 ms of a step-3 prompt landing is "show me", never "register"**
>   (`PROMPT_READ_GUARD_MS`) — SOFT POLICY, stays in `funnel.js` (decided 2026-09-23, #677).
> - **The launch probe matches all four `ActorStanding` arms**, never a boolean (a `Retired` key meets
>   db/004's resurrection refusal, #152); pair the standing with the key it was probed for (#670).
> - **Every sentence and its retry advice lives in `funnel/view.rs`** (`Retry::{Now, AfterOperator,
>   Never}`). A refusal (`P0001`, `DeliberateRefusal`) and an outage are different clinical facts; the
>   non-`P0001` remainder is #655.
> - **`TokenStore::settle` is the sanctioned end of a `take`**; a success INVALIDATES; `discard` does NOT
>   clear `in_flight`. **A dropped `register` future still latches the store (#669), and `register` is
>   cancellation-unsafe (#649): never race it against a timeout or `select!`.**
> - **The step-3 trigger is advisory, never a gate** (a mononymous patient or an unknown DOB registers).
> - **Nothing provisions an actor on a write path.** `init` enrols; `enroll-device-actor` is the remedy;
>   `enrolment_is_never_a_write_side_effect.rs` scans every shipped `.rs` — **when it goes red, do not add
>   your call site to `ALLOWED`.** `resolve_matcher_actor` still enrols (#663); what a SUPERSEDED key
>   classifies as is undecided (#666 first, then #664). **`init` must not `?` its enrolment.**
> - **`cairn-gui-live` is where a DB-backed port implementation goes.** The P0001 rule has three homes
>   (#652). The `gui` job declares `CAIRN_ALLOW_DB_SKIP=1` on the STEP; deleting it is invisible (#656).
>   Derived truncate lists miss identity-stream tables (#658).
> - **`--mock` holds ONE `MockData` for the window's life** (half of #668); its matching rule is NOT
>   db/046's — never generalise a timing from it.
>
> **Open from the funnel run:** #355 · #645 · #647 · #649 · #650 · #652 · #655 · #656 · #657 (multi-event
> rollback untested in both trees) · #658 · #662 · #663 · #664 · #665 · #666 · #667 · #668 · #669 · #670
> · #672 (identifier entry) · #673 (the header shows age, not DOB) · #676 (the clerk reads `operator_chain`
> text) · #682 (an NFD trailing accent is lost) · #683 · #684 (`1980-3-7` misses `1980-03-07` in db/046's
> DOB pass — a SET gap) · #685 · #686.
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
> - **The pen-release rule (#578):** a pen row carrying a wrapped DEK is released only when custody for its
>   event is SETTLED (`cairn_release_pen_row`, db/052; `pen_rows_leave_through_one_door.rs`). #585:
>   nothing reads Postgres notices.
> - **`verify-backup` (#567)** fails `backup SHORT` only on evidence; operators run it AFTER `backup`.
>   Residuals #551 · #553 · #589 · #590 · #591 · #592.
> - **#527/#562's triage note is false**; the real fix is **#575** (the minted recovery code reaches
>   stderr). A retry after a crashed restore must move the installed `<key>.unwrap` aside first (#596).
> - **Open decisions (none a patch):** #575 · #602 (any client can set `cairn.remote_apply`) · #611 · #613 ·
>   #620. **Restore residuals:** #616 · #617 · #596–#599. **Races (reasoned, not reproduced):** #603 ·
>   #604. **PR #601's wave:** #605 · #606 · #607 · #608 · #609 · #610. **Node plane:** #268 · #301 (both
>   `loop:needs-human`) · #569. **From #619/#621:** #622 · #624 · #625 · #626 · #628 · #629 · #631 · #632 ·
>   #633 · #634. **Search:** #636 · #637 (the materialised token table — the right fix for the ~860 ms Pi
>   floor, its own slice) · #639 · #640 · #641 · #643.
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
> **Eighteen traps. Each is a step a next session takes in good faith.** The full argument for each is in
> its ADR, its test's header or ROADMAP; this list keeps the rule, the pin and the tempting wrong fix.
>
> 1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY** (`keystore::adopt_derived_unwrap_secret`);
>    calling it anywhere else re-creates the #495 coupling. `unwrap_secret_is_not_derived.rs` sweeps every
>    shipping tree and asserts every allow-list entry is still live: **when it fails, delete the entry;
>    never add one.** The sweep and `is_a_test_gate_attribute` (pgrx's `#[cfg(any(test, feature =
>    "pg_test"))]`) move **together**.
> 2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6). A
>    database recreated under an existing key file needs `cairn-node establish-unwrap-key` before its first
>    sealed write. Never make a red fixture green by weakening `ensure_unwrap_key`.
> 3. **`cairn-sync`'s ONE derived fallback** (no `<key>.unwrap` AND the derived key equals the registered
>    one → start, warn every startup). An absent file may fall back; a present-but-unusable one never may.
>    **Never simplify those two into one arm.** Retiring the fallback is **#514**.
> 4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It registers a
>    secret derived from the NEW seed and the singleton registrar then refuses the real key permanently.
>    Recover the export first; the way out is another restore into a fresh database.
> 5. **⇒ `Secret32` DOES NOT SEPARATE ONE SECRET ROLE FROM ANOTHER (#511).** `Secret32::from_bytes(sk.to_bytes())`
>    compiles. The count is pinned per file in `secret32_conversions_are_named.rs` — exactly **two** sites turn
>    the signing seed into an unwrap secret (the ADR-0066 migration and trap 3's fallback); a third is the
>    defect returning. `unwrap_secret_is_the_signing_seed` and `secret_opens_the_carried_custody` are not
>    made redundant by the types.
> 6. **`init` refuses a database that already has custody registered** (it reads `node_unwrap_key` first);
>    the remedy it names is `establish-unwrap-key` — see trap 4 first on a restored node.
> 7. **⇒ A BODY SHREDDED AFTER A CAPTURE KEEPS ITS DEK ON THAT MEDIUM. THAT IS NOT A LEAK — DO NOT "FIX" IT.**
>    A backup restores the state at capture time; invalidating old backups is policy (rotation, the
>    clinic's call — principle 9; ADR-0067 decision 2). **Never filter old segments.** Pinned by
>    `medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`. Operator half owed: **#589**.
> 8. **⇒ A PEN ROW WHOSE DEK "BELONGS TO ANOTHER NODE" IS STILL RETAINED (#578).** *"Did not open with the key
>    we have right now"* is not *"not ours"* — the right `<key>.unwrap` may be on a USB stick. The escape is
>    db/021's human `acked`. Pinned by `requeue_releases_custody.rs::a_penned_dek_from_another_node_is_kept_until_a_human_decides_otherwise`
>    and `requeue_retains_unlanded_custody.rs`. **The look-alike narrow fix:** keying the guard on the opened
>    `dek` rather than the row's own `dek_wrapped` passes the headline test and skips the check whenever the
>    key did not open. Do not drop the Rust check because the floor also catches it — it says WHY.
> 9. **RETIRED — HISTORY (#584, ADR-0070).** Live residue: **do not remove or move the
>    `cairn_project_late_custody` calls.** In both doors they sit AFTER the substitution guard, and in db/020
>    BEFORE the `cairn.remote_apply` clear; wrapping db/005's call in `cairn.remote_apply = 'on'` turns a
>    strict refusal into a flag. Pinned in `late_custody_reaches_the_chart.rs`, each placement pin with its
>    own positive control. (#597's notice is still misleading.)
> 10. **⇒ WHEN `late_custody_guards.rs` FIRES, THE GUARD IS RIGHT.** Every function that `INSERT`s into
>     `event_clear` calls `cairn_project_late_custody` (writer set pinned: `apply_remote_event`,
>     `submit_event`), and every registered applier that reads custody is `heal_safe = TRUE`. **Wrong
>     fixes:** registering a custody reader `heal_safe = false` (the chart silently owes a rebuild — **#610**),
>     flipping a non-idempotent one to `TRUE`, exempting a writer. A third writer is a DECISION. A custody
>     read hidden in a helper, a `MERGE`, or dynamic `EXECUTE` is invisible to the guard — review by hand.
> 11. **⇒ `restore` EXITS 3 FOR AN INCOMPLETE RECOVERY; A TEST EXPECTING 1 IS THE BUG (#594, ADR-0071).**
>     Exit 1 = the ceremony was BLOCKED, checked FIRST. **Wrong fixes:** asserting `success()` for a torn
>     medium; making a wrong recovery code exit 1 (its end state is the no-registry state, 3); "simplifying"
>     `past_chain_break` to the medium's clinical total (M8). The precedence has exactly ONE test,
>     `restore_cli_surface.rs::without_the_flag_a_piped_restore_still_inherits_no_custody` — never weaken
>     it to `!success()`. `restore_exit_vocabulary.rs` pins the value and
>     `exit_incomplete_matches_cairn_nodes_restore` the agreement. ⚠️ **`!status.success()` IS NO LONGER AN
>     ASSERTION — write `Some(n)`.** ⚠️ An ACKED pen row is counted in `penned` and `requeue` will not clear
>     it; do not subtract acked rows. Residuals **#611**, **#616**.
> 12. **⇒ THE SUBSTITUTION REFUSAL HAS ONE HOME (`cairn_refuse_substitution`, db/053), AND A DOOR CALLS IT.**
>     It compares `IS DISTINCT FROM` ("cannot tell" is a refusal); a fourth inline copy is the #608 `<>`
>     fail-open returning (`substitution_guard_is_single_source.rs`). db/005/db/020 read under `GET
>     DIAGNOSTICS` (hot path); **db/009 reads unconditionally — do NOT "tidy" it into a `ROW_COUNT` check**,
>     and its guard sits AFTER the `IF/ELSE` (M7). A db/009 refusal aborts the WHOLE restore, correctly. A
>     DEFERRED clinical record is REPORTED, never an exit-3 cause. It guards all five event-log doors; the
>     inventory is `substitution_guard_covers_every_writer.rs` over `pg_proc`. Residuals #608, #605, #569,
>     #622.
> 13. **⇒ THE NODE PLANE'S SUBSTITUTION REFUSAL LIVES IN EACH DOOR'S TAIL; THE PULLER ASKS THE TABLE, NOT THE
>     ERROR (#619, ADR-0073).** An arm that `RETURN`s early bypasses the guard (the genesis arm, safe only
>     for want of an `ON CONFLICT`), and the catalogue guard will NOT notice a new one. **Wrong fixes:** a
>     dedicated SQLSTATE (P0001 is a contract, #228); matching the sentence; penning only when the GUARD
>     raised (`a_rival_refused_by_an_earlier_check_is_still_penned`); turning the failed-lookup freeze into a
>     skip (`node_substitution_lookup_freezes.rs`, M10); computing `offered` over the whole frame (#268's
>     alarm fatigue, M11). A sixth writer: give it the call. Residuals #620, #622, #605.
> 14. **⇒ A DOOR THAT LETS POSTGRES RAISE ON CALLER-SUPPLIED BYTES BREAKS THE P0001 CONTRACT (#621,
>     ADR-0074).** Every signed-bytes field goes through a P0001 helper (`cairn_uuid_or_raise` on
>     `pg_input_is_valid`, `cairn_hlc_nonneg_or_raise`, `cairn_node_role_or_raise`);
>     `node_door_input_guards.rs` fails on a bare `::uuid`. **Wrong moves:** a REGEX narrower than the cast
>     (M9); re-inlining the role list into the CHECK (M10); `USING ERRCODE`; deleting the CHECKs (they are the
>     raw-SQL floor). ⚠️ **The puller's default for an UNKNOWN SQLSTATE is pen; "tightening" it to freeze
>     reinstates #621.** The local classes (`08 40 42 53 55 57 58`, no SQLSTATE) must equal `cairn-sync`'s
>     (`sqlstate_classes_agree.rs`; merging the copies is #626); **`XX001`/`XX002` are LOCAL** on both planes.
>     A pen row of this kind leaves only by applying or an ack. **The role CHECK stays `NOT VALID`** (a
>     validating pair would re-scan on every connect and could stop a node STARTING). Known exception:
>     `cairn_body`'s `22P05` on a NUL (**#628**). Residuals #626, #629, #605, #268, #625, #631, #632, #633,
>     #634.
> 15. **⇒ THREE THINGS IN `db/046`'s PASS 3 LOOK LIKE NOISE AND ARE EACH WORTH HUNDREDS OF MS ON A PI (#639).**
>     (a) **`OFFSET 0` is an optimisation fence, not a limit** — removing it changes no result, so nothing
>     fails. (b) **The lateral is `UNION ALL` on purpose** (the branch `DISTINCT` + outer `UNION` dedupe).
>     (c) **The parts-branch skip must test `lower(normalize(pn.value, NFC))`** — U+0130 `İ` lowercases to
>     `i` + U+0307; dropping the `lower` LOSES A TOKEN (ICU only). `the_subset_argument_holds_for_every_unicode_code_point`
>     checks all 1,114,111 code points; `the_subset_probe_still_describes_the_query_db046_runs` pins the
>     composed expression. **A probe list is a sample, not an argument; a neutrality claim needs a test that
>     can see a GAIN; re-measure with `scripts/measure_patient_search.py`, do not reason.** Residuals #641,
>     #640, #643, #637.
> 16. **⇒ A GUARD AND THE THING IT GUARDS MUST BE ASKED ABOUT THE SAME STRING.** Pin the COMPOSED expression as
>     a literal (`include_str!` + `contains`), not its pieces.
> 17. **⇒ LOCAL POSTGRES IS ICU, CI's IS libc, AND `lower()` DIFFERS** (full vs simple case mapping). It burned
>     a CI run in each direction. Derive such rows from the SERVER (`patient_search_equivalence.rs`'s
>     `full_case_mapping`). To reproduce CI: `CREATE DATABASE cairn_test_libc TEMPLATE template0
>     LOCALE_PROVIDER libc LOCALE 'en_US.UTF-8' ENCODING UTF8;` + `CREATE EXTENSION cairn_pgx;`. **Any test
>     touching case, collation or character classes runs against both before pushing.**
> 18. **⇒ A TARGETED `cargo test --test X` REPORTING `ok` IS NOT PROOF — ONLY THE SWEEP IS (#661).** A targeted
>     run reported `ok` twice against source it contradicts; the sweep failed it. Most likely a stale test
>     binary (shared `target/`, a running rust-analyzer) — silent and green, worse than the loud exit-101
>     kill. **Gate on the sweep**; use `CARGO_TARGET_DIR=/tmp/…` when an IDE is open; re-run every CI gate in
>     CI's order AFTER the final edit.

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)): parts A+B (authority floor + operator
surface) BUILT, enforcing nothing beyond display/emission; C+D DESIGNED, C1 the next §5.9 BUILD.** Read
**ADR-0062/0063/0064/0065** first. The authority floor is ONE predicate `cairn_claim_authority` (db/005) at ONE site
(db/048's `NOT EXISTS`) — **#245**'s first SQL counterpart, not its mirror; operator-surface §1.2 budget met (residual
**#436**). **C+D (ADR-0065)** are a custody ladder (admission → named nodes → named actors) under one invariant:
**narrowing changes the cost and noise of reading, never whether content can be REACHED** (audited break-glass at every
rung; rung-1 glass is a NETWORK act, **#498**). The node's DEK is the keyring, the floor the glass (LOCAL); C and D are
inseparable; custody composes by INTERSECTION, which can EMPTY (**#499**); it narrows on `event`/`patient`, never
`thread`. **C1** = rung 1 (`custody.nodes`, both doors, serve-door withholding) + audited break-glass + in-chart
location signal; rung 2 is **#496** (needs a reader identity, §5.11); chart-wide `patient` is out of C1 (#499).
Related: **#494** (ADR-0052's `event_dek` sentence vs the built table), **#377**, **#235** (shred authorization hooks),
**#236** (FTS/RAG must build on `event_clear`).

**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant stands, so
`cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new column must be granted in db/049
§8 (`safety_read_grants.rs`), and that grant is cost-raising, not a floor (**#425**, **#427**; **#432** asks whether a
node should attempt one at all). Slice-65 follow-ons: **#374**, **#378** (withdrawal rationale is clear text forever —
the UI must warn today), **#379**, **#436** (#374/#379 each need a DECISION). The `arrayref` residue is **#454**.

> [!IMPORTANT]
> **Two code traps that outlive their slices, because both look like tidy-ups.**
>
> 1. **`content_address IS NOT NULL` is the "did anything win" test — never `subject_kind <> 'none'`**
>    (`none` is a legal open-vocabulary value; ADR-0062 E6).
> 2. **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0 withholds reject
>    power (safe); here it would withhold protection. ADR-0065 adds a third member that agrees for a
>    DIFFERENT reason (it withholds quiet access, reachable by break-glass) — do not carry that
>    justification into a site where reachability is not guaranteed.

**Four things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 stopwatch figures** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy: sections 1–7 for the med-list gestures (only the *write* half is measured, median 222 ms,
**PARTIAL**; write-cost half **#360** unwired; now also a **linked chart's open**, budget ≤ the single-chart open) and
**section 8 for the front door** (find ≤ 5 s, register ≤ 20 s, live and `--mock`; db/044's `gesture_kind` CHECK still
refuses a registration timing row until widened); (2) **the accessibility pass** — a live VoiceOver run through the
runbook's checks, front door included, keyboard-only, and on a **linked pair** (the source-chart label, #691); DOM
assertions automated by **#332**; (3)+(4) **make CI jobs REQUIRED status checks** (**#444**, admin-only — "clippy +
cargo test (cairn-gui)", "cargo doc (API surface)", and `CodeQL (rust)`). **If a measurement falls outside its budget,
that is the finding — file an issue, never adjust the budget.**

**Other build candidates** (nothing blocks a choice): the **drugref term→anchor lookup** (the §9 advisory tier; closes
the coded↔uncoded case ADR-0059 decision 5 leaves open; needs a connection-model decision; `safety_class_map` its empty
seam) · **the node/actor plane's two divergences** — db/007 fail-closes on an unmappable type (**#301**), and the node
puller skips-and-advances a verifiable refusal where the clinical one pens it (**#268**); neither symmetric, both
`loop:needs-human`.

**Standing gate:** whole-project review cycles repeat periodically; no release for clinical use before repeated cycles
pass cleanly. Last full pass 2026-07-15 (#187–#217), fully closed; the runnable clinical surface has never been
through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human session
> holds the main repo — they contend on one cargo lock and one `test_serial_guard` advisory lock. **A live IDE
> contends the same way** (rust-analyzer holds `target/`); use a scratch `CARGO_TARGET_DIR=/tmp/…`, never kill
> the IDE. Loop gaps **#326**, **#312**, **#322**.

---

**Session date:** 2026-09-30 (**R2b-2 — "Not the same person" + #699 (a)**, ADR-0077, PR
**[#711](https://github.com/cairn-ehr/cairn-ehr/pull/711)**, draft) · 09-29 (**R2b-1 — "Same person as…"**, PR
**[#707](https://github.com/cairn-ehr/cairn-ehr/pull/707)**, merged; filed #708, #709) · 2026-09-27 (**R2a — the link
precedence floor + `link_charts`/`unlink_charts`**, PR **[#698](https://github.com/cairn-ehr/cairn-ehr/pull/698)**,
`db/055`, generation 55; #697 decided (b); filed #699–#702; earlier that day **R1 — the combined read**, ADR-0076, spec
**v0.78**, `db/054`, PR #688, #334 repaired) · 09-26 **#671** (ADR-0075, PR #678) · 09-23 funnel 2c (PR #674) · 09-22
funnel 2a + 2b · 09-21 #636 slice 1 + #639 · 09-20 #621 (ADR-0074) · earlier: ROADMAP. · **Spec:** **v0.79** (newest
ADR-0077; [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's
implementation wording only) · **`SCHEMA_GENERATION`:** **55** (`db/055`) · **Phase:** architecture complete; **first
production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 window: the funnel front door onto a medication
chart that reads linked charts as one.

**Built so far** (orientation only; ROADMAP + ADR log + git carry the detail): demographics 1–5 · §5.2 advisory Python
matcher · §5.7 identity core C1–C5 (C5+ `reattribute` waits on a clinical-note surface) · §5.4 John-Doe (§5.12
push-alert open) · §5.3/§5.8 funnel (ADR-0061; precedence #345 at db/005 step 8b; ranking ADR-0075) ·
`clinical.medication` 1–6b under born-sealed bodies (ADR-0052) + per-write human authorship (ADR-0053 — grading
half-live until #245) · §5.9 stream through its read surface · med-list node tier (read + whole-list sign-off over a
linked set, R1) · human link/unlink judgements (`chart_link`, attested-first `patient_link`, R2a) · the compare-and-link
panel (R2b-1, PR #707) + "Not the same person" / unlink from a record (R2b-2, PR #711) · generic reprojection (ADR-0057; ADR-0070) · ADR-0056 admit-uninterpreted floor · **the L3
reference UI** `cairn-gui/` (standalone workspace, one-way GUI → crates; `cairn-gui-tauri`, the iced shell FAILED
a11y, spike 0004; plain JS, no npm); pane/routing/freshness state machine tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and every open issue number; this keeps only lessons that generalise.

### 2026-09-30 — R2b-2: "Not the same person…" and #699 (a) (PR #711)

Plan `docs/superpowers/plans/2026-09-30-repair-path-r2b2-not-the-same-person.md`; subagent-driven (seven tasks, per-task
review), controller ran the sweeps and the final review.
- **⇒ A PLAN CAN MANDATE A FALSE CLAIM, AND A REVIEW THAT CHECKS THE CODE IT DESCRIBES CATCHES IT.** The plan (and
  #707's own sentence) said pressing Unlink again after `Outranked` "changes nothing"; the HLC merge at both sync
  doors (db/020, db/007) makes a retry record a newer judgement that OVERRULES the colleague's. Fixed at three sites.
- **⇒ A TEST THAT CANNOT SEE ITS OWN PREDICATE PROVES NOTHING — MUTATE TO CHECK.** The planned
  `an_unlinked_pair_is_not_a_link` still passed with `state = 'link'` deleted; only a query over a set holding BOTH
  charts bites. Same class as the `[hidden]` walk in R2b-1: name the assertion that would fail.
- **⇒ A DB-SUITE "ok" CAN BE A SELF-SKIP.** The controller re-ran the DB suites with `--nocapture` and looked for
  `skipped:` before accepting the implementer's green (trap 18's cousin).
- **⇒ A PLAN'S "CHECKED BY THE CALLER" IS AN UNENFORCED PROMISE.** `filing_for`'s doc left record containment to the
  caller and `assert_link_in_tx` is `pub`; the fix was an in-transaction re-read plus an unrelated-`--from` refusal
  (the plan's `opened.unwrap_or(about)` trusted unchecked input).
- **⇒ CHECK `git ls-files` BEFORE DECLARING WALK DEBRIS UNTRACKED.** R2b-1 had committed two `.playwright-mcp/`
  files by accident; this slice's walk nearly did too. Now removed and ignored.
- **Mechanics:** `chart_link.rs` was split (`admit.rs` pure, `judge.rs` entry points) — new guarded files go into
  `db_errors_stay_legible.rs`; `chart_link.rs` is now 446 lines; `cairn-gui-tauri/src/chart_set.rs` (598) remains over 500
  (deferred); runbook §10's stopwatch (≤ 15 s) is still a HUMAN act.

### 2026-09-28 → 09-29 — R2b-1: the "Same person as…" panel (PR #707)

Plan `docs/superpowers/plans/2026-09-29-repair-path-r2b1-same-person-as.md`; subagent-driven (seven tasks, per-task
review), controller ran the final whole-branch review and gates.
- **⇒ A LOCAL-ONLY COMMIT IS INVISIBLE — PUSH EARLY.** Connectivity dropped mid-session and work piled up locally;
  house rule 8 means push as soon as anything is worth seeing, not only at the end.
- **⇒ A REVIEW CAN FIND A PRINCIPLE-4 DEFECT IN THE PLAN'S OWN CODE.** The plan mandated "none" for an unheld chart's
  aliases; that list is not KNOWN empty, only unread — the word is the shared absence wording ("unknown — registration
  not yet received here"). Principle 4 outranks a plan's literal text.
- **⇒ A HEADLESS WALK THAT CHECKS `textContent` PROVES NOTHING ABOUT WHAT IS ON SCREEN** (final review C1: every panel
  message sat in a `hidden` element; the walk passed). The rule and its assertion are in R2b-1's durable rules above.
- **⇒ A GUARD FILE'S COUNT PINS TRAVEL WITH THE CODE THAT MOVES.** `db_errors_stay_legible.rs` (#467) counts
  `LocalDbFault::new(` per file; `patient/compare.rs` needed both a sweep entry and `COMPARE_LOCAL_DB_FAULT_SITES` — a
  file missing from both passes with zero coverage (cf. the twin-registry and helper-registry pins).
- **Mechanics:** `OTHER_CHANGED` words only a CHANGED right-hand set — an unreadable one keeps "could not tell which
  charts are on screen", like the left side (review round); `chart_link`'s bare `anyhow::bail!`s became verdicts so the
  window tells a refusal from an outage (#702, for this surface), and `RefusalScope::NodeState`'s doc now admits a
  state that sync (not only an operator) changes.
- **⇒ `/review-pr` AFTER A CLEAN FINAL REVIEW STILL FOUND 7 IMPORTANT DEFECTS**, all in wording/state the earlier
  reviews read as correct (a label claiming "verified", a colspan of 0, a filter on a display string). Five narrow
  agents + controller verification of each claim (two were partly wrong) is worth its cost on a clinical surface.

### 2026-09-27 → 09-28 — R2a: the link precedence floor, the node's judgement, and its PR review (PR #698)

Subagent-driven (six tasks, opus on safety-critical tasks + whole-branch review), then a five-agent PR review + fix round.
- **⇒ A MIGRATION THAT CHANGES A PROJECTION'S ORDER MUST RE-DECIDE WHAT THE OLD ORDER ALREADY DECIDED** — a backfill
  leaves the old winner standing; db/055 reuses the generation-change heal. Ask: what did the old order choose, and what
  re-chooses it?
- **⇒ A STORED DERIVED VALUE IS SAFE ONLY IF ITS INPUT CANNOT CHANGE AFTER IT IS STORED** — checked, not assumed.
- **⇒ A SECOND CALLER OF A SHARED CORE CAN INVERT A LOCK ORDER** (same-pair deadlock). A concurrency test waits on the
  real signal (`pg_stat_activity.wait_event_type = 'Lock'`), never a sleep.
- **⇒ CHECK EACH RULE AGAINST EVERY VERB THAT REACHES IT** — the whole-branch review found link's "both held here"
  applied to unlink, and a transitive unlink reporting "unlinked" while still joined (`LinkEffect::StillJoined`).
- **⇒ RECORDED IS NOT TOOK EFFECT, AND A TRIGGER ANOTHER PROCESS CAN CONSUME IS NOT ONE YOU OWN** — read the winner
  back in the same transaction; db/055 re-folds its pairs itself because `cairn-sync init` can use up the heal (#703).
  `event_log.body` IS the payload (no `payload` wrapper) — a wrong-shape probe matched nothing, caught by the red test.
- **Mechanics:** survey the tree before designing (a brief is a claim); when a read widens from key to set, re-read every
  per-key aggregate; run a plan's verbatim code against its own guards; capture a golden BEFORE the rewrite;
  availability over consistency binds a header read too.

### 2026-09-22 → 09-26 — funnel UI 2a → 2c and #671 (PRs #646, #653, #661, #674, #678)

- **Ask what the desk will actually do before designing what it must attest** (#671); **measure the ADR's claim with a
  control the feature cannot help**; a design's quantitative assumption is a claim (read the query the UI sits on first).
- **When a slice makes a constant variable, audit every reader** (2c's Critical; R1 widened it to a set); a mock-mode
  window walks headless, an IPC-only defect stays the human pass's.
- **A sweep without all three DB strings is not a sweep**; **a claim about the tree is a claim — grep** (#654); an
  atomicity probe that never reaches the server is decoration (#657); a derived truncate list is only as good as its
  predicate (#658); a new DB suite in a non-root tree runs nowhere until wired (#656).

### 2026-08-20 → 09-21 — the restore, node-plane, door and search slices

Narrative: ROADMAP; durable rules: traps 7–18; each plan carries its review ledger.
- **An issue's scenario, scope and blast radius are CLAIMS** — read the code for what it MISSED. **Validate a SQL value
  with the parser that will parse it** (#624). **A guard only ever green proved nothing** — positive control (#586);
  **"untestable" is a claim — try a `SET ROLE` seam first.**
- **A mutation is the RED phase of a pin over shipped behaviour** (`Some(n)`). **Copying a guard spreads its fail-open**
  (#608; cf. #652). **Review the review's fixes.** **A door returning `Ok` is not the record coming back** — assert the
  projection. **Content-addressing over unsigned bytes is not content-addressing** (#620). A scanner reads names (6b).
- **Mechanics:** a fixture can manufacture a SQLSTATE production never sees; `restore_node_event` refuses an enrolled
  node; a red gate can be a predecessor's (#583 — truncate `local_node`); a new definer writes `SET search_path =
  public, pg_temp`; `nohup … &` and `cmd; echo exit=$?` lie — read the log's last line; a new ADR needs its `mkdocs.yml`
  nav line; zsh needs `${=T}`. Still open: **#569**, **#598**.

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
- **⇒ The repair path, R2 → R5** — see ⇒ NEXT.
- **⇒ DR — closed and rehearsable end to end.** Not there, though a reader expects it: **2d does NOT drive
  `cairn-sync`'s puller through `MediumTransport`**; **the per-peer quarantine quota does not apply to a
  restore-originated pen** (`restore_pen_is_uncapped.rs`). Filed by the chain: #549 · #551 · #552 · #553 · #525 · #541
  (no CI job compiles `cairn_pgx`'s `pg_test`) · #531/#329 (decompose `cairn-sync/src/main.rs` — maintainer picks one) ·
  #532 · #534 · #535 · #536 · #537 · #538 · #556–#563 · #569 · #575 · #589–#592 · #596–#599 · #602–#611 · #613 · #616 · #617 · #620 · #622 ·
  #624–#626 · #628 · #629. (Ranges absorb issues that leave them: re-check against GitHub.)
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

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)): PASS twice.
  Remaining: fold the un-caveated B4 number into ADR-0015 to drop "provisional"; **#272** (reproject bench on the Pi).
- **easyGP session** — port [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s
  deferred items with live schema access (`rx!`/`tx!` parser + state machine; formulation/drug source + forced-manual
  rule table; prefetch warming daemon). Pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection per slice.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** legal entity & jurisdiction — deferred until momentum/funding
  geography is clearer; trademark registration — principle recorded, instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session.** Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured.
