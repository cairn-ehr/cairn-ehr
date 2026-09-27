# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ R1 — THE COMBINED READ — IS BUILT ON PR [#688](https://github.com/cairn-ehr/cairn-ehr/pull/688)
> (2026-09-27), WHOLE-BRANCH REVIEWED (ready after fixes → fixed, re-reviewed), AWAITING THE MAINTAINER'S
> MERGE.** It is the first of
> five slices of the duplicate repair path (#679 · #680 · #681), one design —
> `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` — and
> [ADR-0076](spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)
> (spec **v0.78**). A linked chart now reads as **one combined record**: every member's medications in one
> list, each row naming its source chart; demographics are NOT combined (each member's own identity line,
> no winner chosen); writes stay per chart; every chart command names the displayed chart SET and refuses
> when it changed. `db/054_person_charts.sql` (`cairn_person_charts`, `cairn_medication_duplicate_groups`),
> `SCHEMA_GENERATION` **54**. **#334 is fixed by it** (the PR body closes it — never a commit message).
> Final tree (1ec920c3): full sweep + both trees' fmt/clippy/doc/deny/tests green, 2606 passed / 0 failed; the
> webview walked with a stubbed bridge over a linked payload. The CLI's text `medication-list` names a combined
> list's charts too (`medication/list_text.rs`).
>
> **⇒ NEXT, in order:**
> 0. **PR #688** — maintainer review → merge (the whole-branch review is done). Then check `gh pr list` before
>    trusting this list (house rule 8).
> 1. **R2 — link and unlink from an open chart (#681) + the precedence floor.** Plan it from the design
>    page's *R2* section: header **"Same person as…"** → the front door's search → a side-by-side panel
>    (names incl. aliases, DOB with provenance, identifiers, active medications, `cairn_match_veto`
>    findings as plain facts — the panel's safety is what it SHOWS, never an "are you sure?") → an attested
>    `identity.link.asserted`; each member line gets **"Not the same person"** → attested `unlink`;
>    `link_charts`/`unlink_charts` move a resolved proposal row in the SAME transaction; and the
>    safety-critical half, **`patient_link.attested`** ranked BEFORE `cairn_hlc_overlay_wins` in db/018's
>    applier (one definition of "attested" — the one the #190 veto check already computes; `ALTER … ADD
>    COLUMN IF NOT EXISTS` beside the `CREATE`, per #207). Then **R3** (the front door collapses by person;
>    the step-3 prompt signs every member id), **R4** (the per-node matcher worker, #679: queue + `NOTIFY`,
>    proposes, **never links**), **R5** (the §5.2 banner + the worklist, #680).
> 2. **Filed today, open:** **#689** (the db/034 attestation door admits an attestation naming a chart other
>    than its thread's own — a floor gap; needs a local-vs-apply-door decision) · **#690** (db/033's local
>    cross-patient guard refuses reconciling the duplicate threads of two LINKED charts — the write-side
>    mirror of R1; a decision: admit within one link component? what an unlink then does — a flagged
>    cross-patient group, never auto-separated) · **#691** (every row of a linked list names its chart by
>    full 36-char uuid, on screen and to the screen reader — want a short per-member tag keyed to the
>    header lines). **#333** gained the between-reads chart-set refusal (no DB test; same concurrency seam).
> 3. **Human acts still owed** (an agent cannot do them): the runbook stopwatch figures — now also a
>    **linked chart's open** — and the **live Tauri-IPC pass on a linked pair** (the mock walk cannot see an
>    IPC-only defect). See *Four things still owed are HUMAN acts* below.
> 4. **#620**, the only open item that can still change the wire (the COSE unprotected header is hashed
>    into the content address but lies outside the signature); brainstorm first. Then **#626** (the
>    clinical twin of #621, kept out because db/020 is the 100k-event hot path), **#652 + #655** together,
>    and the small advisory-tier **#640**, **#641**. Unchanged from 09-26: **#682–#686** (#685 = re-run the
>    ranking on the spread draw; clearing `cairn_test`'s fixtures needs the maintainer's permission).
>
> **⇒ THE COMBINED READ'S DURABLE RULES (R1, ADR-0076) — do not undo any of these:**
> - **A combined list's duplicate flag is db/054's `cairn_medication_duplicate_groups` over the SET, never
>   the per-patient `patient_medication_reconciliation_flag`.** The view groups by `patient_id`, so the same
>   drug on two LINKED charts is two groups on two patients and never flagged — "simplifying" back to it
>   re-hides two unflagged lines for one drug, a double-dose reading hazard. Its dup_key must stay
>   byte-identical to db/033's; `medication_dup_key_drift.rs` pins it.
> - **The medication read selects groups by MEMBERSHIP over the chart set.** Filtering the views by their
>   `patient_id` (the display winner) is the #334 defect returning. `cross_patient` = the group reaches a
>   chart OUTSIDE the set (flagged, withheld from sign-off).
> - **`MedicationRow.patient_id` is the chart the group DISPLAYS under** — not "the chart this line is on",
>   never an attestation target. Use `source_charts` / `MemberVouch::patient_id`.
> - **EVERY CHART COMMAND NAMES THE DISPLAYED CHART SET, not just the chart** (ADR-0076 decision 3; widens
>   PR #674's Critical below). `displayed_patient` is still checked FIRST, then the set
>   (`cairn-gui-tauri/src/chart_set.rs`). `sign_off_medication_list(…, displayed: Option<&ChartSet>)`
>   refuses when the displayed set ≠ the first read, or the first read ≠ the second; `None` is the CLI
>   (skips only the displayed compare).
> - **Sign-off and cease act on each thread's OWN chart** (`MemberVouch::patient_id`), never the opened
>   chart — the floor does not yet enforce it (**#689**).
> - **A failed member-identity read keeps the list and shows a warning** (availability over consistency):
>   `ChartPane { list, members, members_error }`. Never let the header's completeness hide the drugs.
> - **A never-linked chart reads exactly as before** — pinned by the golden
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

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems: parts A and B
(authority floor + operator surface) are BUILT, enforcing nothing beyond display/emission; C+D are DESIGNED and C1 is
the next §5.9 BUILD.** Read **ADR-0062/0063/0064/0065** before touching any of it. The authority floor is ONE predicate
`cairn_claim_authority` (db/005) at exactly ONE site (db/048's `NOT EXISTS`) — it gives **#245** its first SQL
counterpart, not its mirror. Operator-surface §1.2 budget met (residual **#436**). **Parts C+D (ADR-0065)** are a
custody ladder — admission → named nodes → named actors — under one invariant: **narrowing changes the cost and noise of
reading, never whether content can be REACHED** (audited break-glass at every rung; rung-1 glass is a NETWORK act,
**#498**). The node's own DEK is the keyring and the floor is the glass (LOCAL); C and D are not separable; custody
composes by INTERSECTION, which can EMPTY (**#499**); it narrows on `event`/`patient`, never `thread`. **C1** is rung 1
(`custody.nodes`, both doors, serve-door withholding) + audited break-glass + the in-chart location signal; rung 2 is
**#496** (blocked on a reader identity, §5.11); chart-wide `patient` is out of C1 (#499). Related: **#494** (ADR-0052's
`event_dek` sentence vs the built table), **#377**, **#235** (shred authorization hooks), **#236** (FTS/RAG must build on
`event_clear`).

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

**Session date:** 2026-09-27 (**R1 — the combined read — built**,
[ADR-0076](spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md),
spec **v0.78**, `db/054`, PR **[#688](https://github.com/cairn-ehr/cairn-ehr/pull/688)**, in which #334 is repaired; filed **#689**,
**#690**, **#691**) · 2026-09-26 (**#671**, ADR-0075, spec v0.77, PR #678; filed #679–#684) · 2026-09-23 (**funnel
slice 2c**, PR #674; its prerequisites, PR #661) · 2026-09-22 (funnel 2a + 2b, PRs #646, #653) · 2026-09-21 (#636
slice 1 + #639) · 09-20 #621 (ADR-0074) · 09-19 #619 (ADR-0073) · 09-17 #614 + #615 (ADR-0072, db/053) · earlier:
ROADMAP. · **Spec:** **v0.78** (newest ADR-0076; [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md)
supersedes ADR-0026 decision 2's implementation wording only) · **`SCHEMA_GENERATION`:** **54** (`db/054`) · **Phase:**
architecture complete (every original §11 question closed); **first production clinical surface RUNNING** —
`cairn-node` plus a Tauri 2 window: the funnel front door onto a medication chart that reads linked charts as one.

**Built so far** — orientation only; ROADMAP + the ADR log + git carry the detail. **Demographics slices 1–5** · **the
§5.2 advisory Python matcher** · **the §5.7 identity core C1–C5** (C5+ `reattribute` waits on a clinical-note surface)
· **the §5.4 John-Doe subsystem** (§5.12 push-alert open) · **the §5.3/§5.8 search-before-create funnel** (ADR-0061;
precedence rule #345 at db/005 step 8b; ranking ADR-0075) · **`clinical.medication` slices 1–6b** under **born-sealed
bodies** (ADR-0052) and **per-write human authorship** (ADR-0053 — grading half-live until #245) · **the §5.9 stream
through its read surface** · **the med-list node tier** (first clinical READ path + whole-list sign-off, now over a
linked chart set — ADR-0076 R1), **generic reprojection** (ADR-0057; a late key projects at the door, ADR-0070), the
**ADR-0056 admit-uninterpreted floor** · **the L3 reference UI** — `cairn-gui/`, a standalone workspace, one-way GUI →
crates, **`cairn-gui-tauri`** (the iced shell FAILED the accessibility bar, spike 0004): the funnel front door onto one
patient's (or one linked person's) medication chart (plain JS, no npm); pane/routing/freshness state machine tested but
**not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number**. This section keeps only what a *next* session
needs — the lessons that generalise past the slice that found them.

### 2026-09-27 — the duplicate repair path designed; R1, the combined read, built (ADR-0076, PR #688)

Brainstorm with the maintainer → one design for #679/#680/#681, five slices R1–R5 → ADR-0076 → plan
`docs/superpowers/plans/2026-09-27-repair-path-r1-combined-read.md` → subagent-driven, per-task reviews all clean.
- **⇒ SURVEY WHAT EXISTS BEFORE DESIGNING — THE BRIEF'S PREMISE WAS HALF-BUILT.** ADR-0075 said "accept duplicates and
  make repair by `link` easy". Reading the code first found a `link` repairs nothing a clinician can SEE (the med read
  filtered one `patient_id`; nothing read across `person_member`), "different people" had no home in the algebra, the
  matcher could not run on its own, and a machine link could silently override a human `unlink`. The design is built on
  those findings, not on the brief. **A brief is a claim about the tree; read the tree.**
- **⇒ THE DOUBLE-LINE HAZARD WAS MISSED BY THE DESIGN AND CAUGHT AT PLANNING.** The design saw #334's doubled group,
  but not that `patient_medication_reconciliation_flag` groups by `patient_id` — so the same drug on two LINKED charts
  would have been two UNFLAGGED lines on the combined list. Planning the SQL against the view's text found it; db/054's
  set-wide function exists because of it. **When a read widens from one key to a set, re-read every per-key aggregate
  it leans on** (the Slice 57 `array_agg` lesson, again).
- **⇒ THE PLAN'S OWN DRIFT GUARD CONTRADICTED ITS OWN SQL.** The guard required db/054's dup_key byte-identical to
  db/033's; the plan's SQL qualified the columns (`mc.coding_system`) where db/033's are bare, so the guard could never
  pass. Caught by the controller's pre-flight scan of producer/consumer pairs, not by an implementer. **Run a plan's
  verbatim code against its own guards before handing it out** (the #503 lesson, one level up).
- **⇒ CAPTURE THE GOLDEN BEFORE THE REWRITE.** `a_never_linked_chart_reads_exactly_as_before` was written and run
  against the UNMODIFIED read, its literal pasted, and only then the read rewritten. A golden captured after the change
  pins the change, not the invariant.
- **⇒ AVAILABILITY OVER CONSISTENCY APPLIES TO A HEADER READ.** The first build failed the whole `med_list` when a
  member's identity line could not be read — hiding every drug because the header was incomplete. Now the list renders
  with a visible warning. **A founding invariant binds small reads too; never let a decorative read gate the clinical
  one.**
- **Mechanics:** a pre-existing CSS defect (`#unlock-form {display:flex}` beating `[hidden]`, since 2026-08-03) kept the
  passphrase form visible after unlock — fixed (`[hidden] { display: none !important; }`); the webview was walked with
  Playwright over a stubbed `invoke` returning a linked payload (not a substitute for the live IPC pass).

### 2026-09-26 — #671: the step-3 prompt is a nudge (ADR-0075, PR #678)

- **⇒ THE MAINTAINER'S CLINICAL FRAME SETTLED WHAT THE DESIGN COULD NOT.** "Did the clerk look?" became "how fast is the
  duplicate FOUND?" — **ask what the person at the desk will actually do before designing what they must attest.**
- **⇒ THE MEASUREMENT FALSIFIED THE ADR DRAFT, BEFORE MERGE** ("a name typo never enters the candidate set" was false).
  **Measure the claim in the ADR, and give every arm a control the feature cannot help.**
- **⇒ A RANKING KEY IS ONLY AS GOOD AS ITS MATCH RULE'S AGREEMENT WITH THE SEARCH'S** (exact tokens vs db/046's
  prefix; an MRN match shown 48/500). **Every ranking change re-runs every arm.**
- **⇒ INVERTING A TEST CAN DELETE THE OTHER HALF OF A PAIR** — add the positive case back.
- **⇒ A LATENCY INSTRUMENT MUST SEE THE CODE PATH** (the SQL rig cannot time Rust-side reads).

### 2026-09-22 → 09-23 — funnel UI slices 2a → 2c (PRs #646, #653, #661, #674), condensed

- **⇒ A DESIGN'S QUANTITATIVE ASSUMPTION IS A CLAIM — MEASURE IT BEFORE BUILDING ON IT** ("few candidates by
  construction" was false; db/046 is a disjunction). **Read the query the UI sits on before wiring the UI.**
- **⇒ A MEASUREMENT RIG NEEDS A POPULATION WITH THE RIGHT SHAPE** (unique synthetic names cannot measure truncation).
- **⇒ A MOCK-MODE WINDOW CAN BE WALKED HEADLESS** over `src-ui/` with a stand-in `invoke`; a Tauri-IPC-only defect
  (argument casing) stays the human pass's.
- **⇒ WHEN A SLICE MAKES A CONSTANT VARIABLE, AUDIT EVERY READER OF IT** (2c's Critical: "open chart" = "chart on
  screen" stopped being true for three pre-existing commands). R1 widened the same invariant from a chart to a set.
  **Measure the case the feature exists for, not the easy one.**
- **⇒ A SWEEP WITHOUT ALL THREE DB STRINGS IS NOT A SWEEP** — use `scripts/run-db-gated-tests.sh`.
- **⇒ A second review round asks whether what the code SAYS is TRUE** (found a duplicate-chart path, two hollow guards).
  **Every claim a test makes about a mutation is verified by applying it. An agent's, and an issue's, claim about the
  tree is a claim — grep before planning** (#654 said one call site; there were fifteen).
- **⇒ A design page's sentence is a prediction until code meets it** (dated revision notes, never edited away). A test
  can pass for a reason not in its name; an atomicity probe that never reaches the server is decoration (#657); a signed
  flag nothing reads back is a claim nothing checks; a copied truncate list is a second-run failure, a derived one only
  as good as its predicate (#658); a new DB-gated suite in a non-root tree runs nowhere until wired (#656).

### 2026-08-20 → 09-21 — the restore, node-plane, door and search slices (condensed to one-liners)

Per-slice narrative: ROADMAP; the durable rules are traps 7–18; each plan carries its review ledger.
- **An issue's failure scenario, scope and blast radius are CLAIMS** — read the code for what it MISSED.
- **Validate a SQL value with the parser that will parse it** (`pg_input_is_valid`); two parsers for one value are
  two protocols (#624).
- **A guard that has only ever been green has proved nothing** — give it a positive control (#586), and every harness a
  control for the run not happening. **"Untestable" is a claim — try a `SET ROLE` seam first.**
- **A mutation is the RED phase of a pin over shipped behaviour**; negative assertions name what they negate (`Some(n)`).
- **Copying a guard spreads its fail-open** (#608's `<>`): extract, never paste a third copy (cf. #652).
- **Review the review's fixes** (three of four rounds found a defect the last fix created).
- **A door returning `Ok` is not the record coming back** — assert the projection; the headline test DECRYPTS a body.
- **Content-addressing over unsigned bytes is not content-addressing** (#620). An unpushed branch is invisible (house
  rule 8). A round-trip test proves self-consistency, not correctness. A scanner reads names, not values (rule 6b). A
  deferral is honest only while its precondition holds.
- **Mechanics:** a fixture can manufacture a SQLSTATE production never sees; `restore_node_event` refuses an enrolled
  node; a red gate can be a predecessor's (#583 — truncate `local_node`); fault injection via a test-scoped trigger or
  role; a new definer writes `SET search_path = public, pg_temp`; `nohup … &` and `cmd; echo exit=$?` both lie — read
  the log's last line; subagent briefs say FOREGROUND ONLY; a new ADR needs its `mkdocs.yml` nav line; zsh needs `${=T}`
  to word-split. Still open from this stretch: **#569**, **#598**.

**⇒ THE OPEN ISSUES OLDER SESSIONS OPENED, INDEXED RATHER THAN NARRATED.** The rule is never to drop an **open** issue
number while condensing; an index satisfies it where a paragraph does not.

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
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels, `/techdebt-next` runs one fresh headless session
  per issue; STOPPED by maintainer decision.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **⇒ The repair path, R2 → R5** — see ⇒ NEXT.
- **⇒ DR — closed and rehearsable end to end.** Two things a reader is led to expect and will not find: **2d does NOT
  drive `cairn-sync`'s puller through `MediumTransport`**, and **the per-peer quarantine quota does not apply to a
  restore-originated pen** (`restore_pen_is_uncapped.rs`). Open issues the chain filed: **#549**, **#551**, **#552**,
  **#553**, **#525**, **#541** (no CI job compiles `cairn_pgx`'s `pg_test` module), **#531**/**#329** (decompose
  `cairn-sync/src/main.rs` — a maintainer decision on which to keep), **#532**, **#534**, **#535**, **#536**, **#537**, **#538**, **#556**–**#563**,
  **#569**, **#575**, **#589**–**#592**, **#596**–**#599**, **#602**–**#611**, **#613**, **#616**, **#617**, **#620**,
  **#622**, **#624**–**#626**, **#628**, **#629**. (Ranges silently absorb issues that leave them: re-check each
  against GitHub before trusting it.)
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
