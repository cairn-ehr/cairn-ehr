# Durable rules — what a tidy-up must not undo

Moved here from `docs/HANDOVER.md` on 2026-10-15 (R7) so the handover stays a short, current
scaffold. These are long-lived: each rule is a step a later session could undo in good faith, with
its pin (the test that fails if it is undone) and, where one exists, the tempting wrong fix. The
full argument for each is in its ADR, its test's header, the design page
(`docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`) or ROADMAP.
**Read the section for any code you are about to touch.** When a slice adds rules, add its block
here, newest first, and leave a one-line pointer in HANDOVER.

## Per-slice rules (the repair path, the funnel)

**⇒ #807 — THE WINDOW'S CONNECTION IS REOPENED WHEN LOST — do not undo** (pins:
`cairn-gui/cairn-gui-live/tests/reconnect.rs`, `connection.rs`'s backoff and wording unit tests and `Send` pin):
- **Reopen only when the client `is_closed()`, and only when the lock is taken** (`NodeConnection::lock`,
  `cairn-gui-live/src/connection.rs`). Never on an error: a refusal is an answer on a live connection. Never
  mid-command, and never a retried write: a commit with no answer stays unknown, and the next command just gets a
  working connection for the re-read once the node answers. A failed or unanswered attempt (5 s cap) starts a 2 s
  backoff, kept in the same lock as the client.
- **A silently dead socket is noticed by TCP keepalives, set for every connection either binary opens** (#875,
  #878; the `cairn-db-keepalive` crate: idle 30 s, 3 probes 10 s apart, `tcp_user_timeout` 30 s on Linux only).
  `cairn_node::db::connect` applies them for the window, the CLI and `cairn-node sync`; `cairn-sync` builds every
  production connection from `db_config(conn)`. A setting the connection string names wins, unless it is zero
  (tokio-postgres drops a zero, so ours applies). Pins: `db_connect_applies_the_dead_peer_settings` (db.rs, by
  source), cairn-sync's `keepalive_tests` (the config read back; every production `NoTls` connect in `src/*.rs`
  matched by a `db_config(` call).
  **Tempting wrong fixes:** setting them in the window only — the CLI and `cairn-node sync` hang on the same dead
  socket; a new `postgres::Client::connect(conn, NoTls)` in `cairn-sync` — it compiles and silently drops them.
- **A failed reopen is an ERROR that says why** (#876): `lock()` returns `Err(ConnectionLost)` naming the cause;
  a press inside the backoff tries nothing and says when the last attempt failed. It is an outage (`Retry::Now`:
  `From` into `DataError::Unavailable` and into the window's `ErrorView`, "Nothing was done: …") unless the server
  refused the reopen (#880, below). **Tempting wrong
  fixes:** `?` inside a per-thread write loop (cease, release) — an earlier thread may be written, so the lock
  failure becomes THAT thread's `Err` and is reported with the others; and gesture timing taking `lock()` — it uses
  `lock_if_open`, so it cannot spend the reopen attempt the act's re-read needs. Pins: `reconnect.rs`'s
  `lose_then_press_five_times` assertions and `lock_if_open_never_spends_a_reopen_attempt` (the primitive);
  `a_lost_connection_fails_one_thread_and_timing_never_reopens` (write_failure_tests.rs, a source guard: both
  wrong fixes compile).
- **A reopen the server REFUSED is an operator's, not a retry** (#880): class `28`, `3D000`, `42501`,
  `55000` (`server_refused`) make `ConnectionLost` classify as `DataError::NotProvisioned` (`Retry::AfterOperator`); busy or
  starting up (`53300`, `57P03`) stays an outage. The code survives `db::connect` as `cairn_node::db::ConnectFailed`
  (text unchanged, `sqlstate()` beside it, NO `source()`); `refused` is stored WITH the cause, so a press inside the
  backoff says "operator" too; `data_error_from` downcasts `ConnectionLost` first (cease and release wrap it in
  `anyhow`); `read_chart_of` returns `anyhow::Error`, so `chart_read_failed` sees it too — that read is where the four
  write gestures (sign-off, cease, withhold, release) REOPEN, so it is the route that matters most. **Tempting wrong fixes:** `.context()` over the `tokio_postgres::Error` in `db::connect` — `main`'s
  `{e:?}` then loses the `[SQLSTATE]` (`db_diagnosis.rs`'s 3D000 test fails); classifying only the press that tried.
  Pins: `a_reopen_the_server_refuses_asks_for_an_operator_on_every_press` (reconnect.rs, a real `3D000`),
  `a_refused_reopen_names_the_operator_by_every_route`, `only_a_refusing_server_needs_an_operator`,
  `a_chart_read_lost_to_a_refused_reopen_needs_an_operator` (also a source guard: no `to_string` in `read_chart_of`).
  **A refusal tokio-postgres raises itself** ("password missing", an unsupported auth method, a SCRAM failure) has
  no code: `ConnectFailed::refused_by_client()` marks it from the error's rendered category (`Kind` is private), and
  the reopen classifies it as `Refusal::Client` beside `server_refused`'s `Refusal::Server` (#884). Its text must
  never say the node refused (`Refusal::clause`): an operator would look for a rejected login the node never logged,
  and a SCRAM failure is the CLIENT rejecting the server's proof. **Tempting wrong fix:** adding the words of `Connect`,
  `Io`, `Timeout` or `Closed` to `client_refusal` — those are outages the next press may get past
  (`a_node_that_hangs_up_is_an_outage_not_a_refusal`, no database needed). Pins: `a_refusal_the_client_raises_itself_is_marked_as_one`
  (db_diagnosis.rs, a fake node driving the real library, so an upgrade that rewords a kind fails it),
  `a_reopen_the_client_cannot_authenticate_asks_for_an_operator` (reconnect.rs), `a_client_refusal_is_not_said_to_be_the_nodes`.
  `cairn_gui_live::failed_reopen_cause` reads any reopen text back for a line that is not a press (#888). **Every
  `NotProvisioned` renderer must be true of a refused reconnect too**: the front door's search arm keeps "The search
  FAILED — this is NOT a "no match"" (`a_search_that_needs_an_operator_still_never_reads_as_nothing_found`), and
  the register arm claims nothing about the node's enrolment. Before #880 the search arm was bare text — safe only
  while no live path reached it.
- **A refused reopen says its remedy ONCE** (#885): its text ends in `OPERATOR_REMEDY` (`connection.rs`); a renderer
  wrapping a `NotProvisioned` asks `says_operator_remedy` and does not restate it (register and search arms, the
  judgement arm, which then drops its "yet"). Other causes keep their wrapper's remedy: an enrolment refusal does not
  say what pressing again does, and the judgement arm's not-held cause resolves by sync. The stale-list banners
  (`chart_stale.js`) name "Read this chart again" but never order a press: they cannot tell a refused reopen from an
  outage. Pins: `only_a_refused_reopen_says_the_operator_remedy`, `a_refused_reopen_s_remedy_is_said_once_at_the_front_door`,
  `every_data_error_arm_is_worded_for_a_link`.
- **A search clears the front door's provisioning line only by asking the node again** (#882): `funnel_status` probes the
  standing on EVERY call (`FunnelBackend::provisioning`; there is no launch-time copy), and `funnel.js`'s
  `recheckProvisioning` asks after a search that succeeded while the line shows a warning. **Tempting wrong fix:**
  clearing the line on any successful search — an unenrolled node searches fine, so that wipes a true warning. Pin:
  `a_search_clears_the_provisioning_line_only_by_asking_the_node_again`. A registration that succeeded clears it too: that is the
  node's answer as well. The probe is bounded (`probe_within`, `PROBE_WITHIN`, twice a reopen's wait) because `boot()` shows the front door only
  after it answers: pin `funnel_status_probes_the_live_node_afresh_and_bounded`.
- **Known limit, filed:** a query sent on a dead socket before the keepalives notice still hangs holding the lock,
  bounded only on Linux (#875 stays open for a window-side bound, which overlaps #649).
- **The reopen is `cairn_node::db::connect`, not `connect_and_load_schema`.** The schema was loaded at launch, and a
  replay takes DDL locks a read never needs (#726). It also skips the deferred-event re-adjudication; a restart
  still runs it. Unpinned — the tempting "keep it the same as launch" fix.
- **`LiveData::new` is `NodeConnection::fixed`**: a caller-made client may carry session state (`lost_commit.rs`'s
  kill switch is a `SET`) that a fresh one would silently lack. Only the window's launch uses `reopening`.
- Not checked on reopen: that the database is still the SAME node. Writes there are expected to be refused if this
  window's key is not enrolled; reads are not checked at all. A launch against the wrong database has the same
  exposure.

**⇒ #863 + #864 — A SHRED HANDS ITS ROWS ON; A LIST IS ONE SNAPSHOT — do not undo** (pins:
`crates/cairn-node/tests/shred_refolds_overlay_winner.rs`, `tests/medication_list_one_snapshot.rs`):
- **A shred ends by re-folding the threads its event named (#863)** — `cairn_execute_shred` (db/037) deletes only
  the rows its event produced; for a one-winner projection whose winner that was, the key was left EMPTY (shred the
  newer of two stops: the drug read current). `cairn_refold_after_shred` then re-runs the heal-safe appliers of the
  other eligible events that can win the same rows: same threads (`cairn_medication_threads_named`: `medication_id`,
  `subject_a`, `subject_b`, CANONICALISED — the floor admits any uuid spelling) and a type sharing a non-`*_flag`
  projection table with the shredded type (the registry's `projection_tables` — exhaustive by CONVENTION only, which
  db/039's narrow rebuild also relies on). A NEW medication event type naming a thread under another key must be added
  to the helper. The TARGET is excluded by id: a sealed target names no thread once its clear view is deleted, but the
  sync door also shreds a peer's PLAINTEXT event, whose body still names its thread (PR #865 review). An EARLIER-shredded
  plaintext sibling is still replayed, as `cairn_reproject`'s heal does — **#866** (filtering it needs the shred log,
  whose predicate `shred_predicate_has_one_home.rs` keeps in one place). A sibling whose replay raises aborts the shred;
  a sibling's refusal is re-raised BARE (still P0001 — `USING ERRCODE` is forbidden in db/ by
  `floor_refusals_carry_no_errcode.rs`) naming the sibling; any other code propagates untouched (whether it should block the erasure at all: **#867**). It runs lenient
  (`cairn.remote_apply = on`) and puts the caller's posture back — left `on`, a later write in the strict door's
  transaction skips its raise-not-clamp checks. It WAITS on each sibling's custody lock; shreds are serialized first
  (`cairn_lock_shred_refold`, before the custody lock) so two never wait on each other; a door meeting a shred in a
  cycle gets a transient 40P01 (db/005's deadlock note). Cost: a scan of medication events' clear payloads per shred
  (`shortcut:`). **Tempting wrong fixes:** "never over-erase" read as "nothing to restore"; TRY the sibling's lock and
  skip it (a holder that rolls back or only re-receives the event writes nothing — PR #865 review); compare thread
  ids as raw text; run the whole-log `cairn_reproject` from a shred. Pins:
  `shredding_the_newest_stop_leaves_the_earlier_stop_in_force`, `shredding_the_winning_coding_restores_the_earlier_coding`,
  `shredding_a_separation_restores_the_reconciliation_it_overrode`, `the_refold_puts_the_callers_apply_posture_back`,
  `a_thread_is_matched_however_its_uuid_is_spelt`, `a_shred_waits_for_a_sibling_whose_holder_rolls_back_then_folds_it`,
  `a_peers_shred_of_a_plaintext_stop_stays_scrubbed`, `a_deferred_sibling_is_not_promoted_by_a_shred`,
  `a_peers_flagged_stop_is_replayed_lenient_so_the_shred_goes_through`,
  `a_sibling_that_fails_to_replay_is_named_and_keeps_its_class`.
- **Every production medication-list read runs in ONE snapshot (#864)** — `medication::snapshot::in_snapshot`
  (`REPEATABLE READ READ ONLY`): the CLI's `medication-list` and both cease notes, sign-off's two reads, the
  window's `read_chart_of`. The readers keep `GenericClient` so tests can still pass a bare client or an aborted
  transaction (the notes' failure seam). Sign-off's two reads are two snapshots ON PURPOSE (comparing them is the
  point, #335), and its writes stay READ COMMITTED: a door write at REPEATABLE READ reopens the custody/shred race
  (db/005's `cairn_lock_custody` note). **Tempting wrong fix:** one snapshot spanning sign-off's reads AND writes.
  Pins: `a_stop_committed_after_the_read_began_is_not_seen`, `production_list_reads_run_in_one_snapshot` (counts six call
  sites — a new production reader is added there), and `snapshot.rs` in `db_errors_stay_legible.rs`'s
  `UNMARKED_COMMITS` (a read-only commit).

**⇒ #832 — THE WINDOW'S WITHHOLD (ADR-0081 slice B) — do not undo** (pins: `cairn-gui-tauri/src/hold/plan_tests.rs`,
`hold/command_tests.rs`, `src/holds_webview_tests.rs`, `cairn-gui-tab-medications` `hold_view.rs`,
`crates/cairn-node/tests/ui_timing.rs`):
- **A Release sends only the hold ids the row DREW.** A fresh read at press time would end a hold a colleague
  wrote after the clinician last looked, unseen. **Tempting wrong fix:** have `release` re-read the holds "so it
  is never stale". Pins: `a_release_ends_only_the_holds_it_was_shown` (+ `a_hold_that_ended_meanwhile_is_named_gone`),
  and the webview guard `a_release_sends_only_the_holds_it_drew` (the only `invoke("release"` sends the ids it was handed).
  **And the report SAYS the unseen hold is still on the line** ("Released 1" alone reads as "givable"): `ReleasePlan::unseen`
  counts holds on the line not in `shown` (pending too, so the words are "still on this line", never "in force",
  #839), and the stale refusal for an all-gone Release says it too (`all_shown_gone`). Pins:
  `a_hold_not_shown_is_counted_unseen`, `a_release_says_a_hold_it_was_not_shown_is_still_on_the_line`,
  `a_release_with_nothing_left_to_end_names_an_unseen_hold`.
- **A cease or release that wrote nothing is a refusal (`Err`), not a report (#840).** When every write failed and
  none is unknown, `retry_when_nothing_written` gives ONE class — **stale wins** (maintainer's ruling), else the
  first failure's — and the text is the same outcome sentence. As a report it wiped the typed reason and lost the
  stale class. A gesture whose threads were all HELD BACK (no write tried) still returns its report. **Tempting
  wrong fix:** return the report "so the outcome line names every thread" — the `Err` text already does. Pins:
  `a_gesture_that_wrote_nothing_takes_one_class_and_stale_wins`, `a_gesture_that_wrote_nothing_is_a_refusal_with_its_class`.
- **Withhold and Stop are drawn only where their command can write (#841)** — `chart_pane` asks `withhold_target`
  and `cease_plan` with the opened chart, so a hazard line with nothing on this chart offers no press that can only
  refuse or stop nothing — and SAYS where to go instead (`NOT_ON_OPENED_CHART`): the refused press used to. Hazard
  lines only: an ordinary line `withhold_target` refuses is an anomaly (a reconcile raced the list's reads), and the
  press names it where a hidden button says nothing. A cease that can write nothing answers before the key, marked
  behind the record (as release); one that stopped nothing is no timing sample. **Tempting wrong fixes:** compute
  either from `status` in `build_row` (it has no opened chart); narrow every row on `.is_ok()`. Pins:
  `withhold_and_stop_are_offered_only_where_they_can_write`, `the_refusals_come_before_the_key`.
  **Release too (#844):** `chart_pane` asks `release_plan` with EVERY hold the row drew (the ids the window
  sends) and hides Release when it would write nothing. Its own sentence (`HOLD_NOT_ON_OPENED_CHART`), because
  Withhold and Stop may still stand on that line, where "no record on the chart you opened" would be false. Said
  only when every hold is on a MEMBER thread: a non-member's hold sits on a chart outside the record, may be another
  person's own W, and "release it from its own chart" would send the clinician to end it. Separation speaks
  instead, and it speaks ON THE ROW (the window's outside-set sentence carries `SEPARATION_INSTRUCTION`, as the
  CLI's does): the report covers only withheld lines, so a signed or ceased line would otherwise show a W with no
  Release and no way forward (PR #845 review). Pin: `release_is_offered_only_where_it_can_write`.
- **A synced event naming another chart makes its line a hazard (#836)** — `WrongChartReasons::patient_conflict`,
  read from db/031's `medication_patient_conflict_flag` (`read.rs` `read_conflict_charts`). The sync door
  converges-and-flags, the event is projected anyway, so another person's W, release, dose or stop sits on the
  line; the flag reached no screen before. A linked chart is the same person (no hazard) unless the set holds a
  doubted link, and then ANY conflict is a hazard (db/031 flags only two different charts, so "unless the named
  chart is the opened one" could never apply). Remedy by STATUS (`conflict_remedy`): a current line is
  stopped and recorded again (`CONFLICT_INSTRUCTION`) — never separation (the line may be one thread); a ceased
  line gets `CONFLICT_CEASED_INSTRUCTION` (the stop itself may be the stray event, and it may be the line the
  clinician stopped to clear it). "Cannot be signed" only where `withheld_reasons` withholds the line (the #717
  lesson), and the window puts the remedy ON THE ROW, because its report covers withheld lines only. **Trap:** `medication_group_member` has NO row for a
  thread no reconciliation touched, so the read is `LEFT JOIN` + `COALESCE(group_id, medication_id)`; the inner
  join first written missed every ordinary line, and only the DB pin caught it. Pins:
  `tests/medication_wrong_chart_hold.rs`, `hazard.rs` `a_conflict_*`.
  **Arrival order (#846):** the flag is written only when the thread's chart is ALREADY known, and a dose, dose
  correction or release never makes it known, so one that arrived first is never flagged. db/059's
  `medication_thread_stray_chart` view finds it at read time and `read_conflict_charts` unions it in. **Tempting
  wrong fixes:** "the flag table is the conflict record" (it is order-dependent; reading it alone re-opens the gap);
  "the view replaces the flag" (the view sees only doses, corrections and releases; a stray CLAIM — assert, stop,
  coding, W — is only ever in the flag); dropping the `WITH t` filter (the view then calls
  `cairn_medication_thread_patient` on every dose on the node), or narrowing it to the group ids (a stray on a
  reconciled line's non-key member is missed). The stray can come first on the LOCAL door too: the dose door does not
  check that a thread exists. Pins: the `a_stray_*` tests, `the_stray_chart_view_names_only_a_known_chart_that_differs`,
  and `the_runtime_role_reads_a_list_whose_conflict_comes_from_the_view` (tests run as the owner; only this catches a
  missing `cairn_agent` grant).
- **The chart an event was recorded FOR hears about it too (#848)** — `PatientMedicationList::elsewhere`
  (`medication/elsewhere.rs`), every renderer through `elsewhere_report` (CLI list, the window's `chart-elsewhere`
  paragraph, the compare panel's notes). The list selects lines by the thread's CURRENT chart, so a thread a later-HLC
  assert moved to another chart, or this patient's dose/W/stop on another chart's thread, had no line here and the
  conflict read (#836) never looked: the drug vanished unsaid. Read-side only (maintainer, 2026-10-23); the overlay
  still moves the thread (#192's converge contract untouched — freezing it only moves the blindness to the other
  chart). **Tempting wrong fixes:** reading only the flag's `asserted_patient` (the moved-thread case is
  `standing_patient`; a thread claimed by a stop, W or coding — anything but an assert — has no point-0 dose row, so
  the view cannot see it either); reading the stray view's `standing_patient` (that is the thread's chart NOW — a set
  chart there means the thread is this set's, listed or claimed here and on no list, never elsewhere);
  dropping the listed-THREAD or listed-GROUP exclusion (the line is then said twice, once as a hazard on the row);
  showing it as a row (`outside_set` would add the separation remedy, wrong for one moved thread, and a member-less
  row reaches Withhold/Stop/Release logic that never saw one); dropping the `EXISTS` on `medication_thread_group`
  (a thread claimed only by a stop/W/coding, this patient's or another chart's, is on NO list, and "on another
  chart" would be false — this patient's event on it is said on NEITHER chart until its assert arrives); trusting
  the earlier member read alone for "already listed" (a separate READ COMMITTED statement — a line re-homed onto a
  set chart between the reads came back "on chart <own chart>"; the query re-checks in its own snapshot, PR #851
  review). The remedy is CONDITIONED: if the drug is already on this list, the
  misfiled dose/stop/release/W is recorded again on that line, never a second line for the drug (double-dose reading). An
  empty, all-stopped or all-signed list with a report makes no absence claim (window `empty_message`, CLI "! but"). The view is filtered by
  `asserted_patient`, so db/059 indexes `patient_id` on its three source tables. The sign-off outcome carries it
  too (`SignOffOutcome::elsewhere`, #849: CLI "! " line in every outcome, the window's outcome notes; the CLI's
  empty, all-stopped and all-signed sentences make no absence claim beside it; pins
  `a_stopped_or_signed_chart_with_a_drug_elsewhere_points_at_it`, `a_sign_off_outcome_carries_the_lines_elsewhere`,
  `the_sign_off_verb_prints_the_report`, `an_empty_chart_with_a_drug_elsewhere_makes_no_absence_claim`); the
  report is permanent (**#850**, a decision). Pins: `tests/medication_rehomed_thread.rs`
  (each SQL predicate mutation-checked, the three CLI calls source-guarded, a linked set from each side), the
  semantic tree's `the_chart_warnings_are_announced_not_merely_displayed`, the webview's
  `a_drug_elsewhere_alone_shows_the_warnings_section`.
- **A release of a hold that has ENDED is a stale refusal at the local door (#843, maintainer 2026-10-23)** — a
  colleague already released it, or it ran out, by the release's `t_effective`. ONE predicate,
  `cairn_medication_hold_ended` (db/059 section 7b), with three callers: the read-time fold (section 10 — so a hold
  the floor calls ended is never still on screen, where its release would be refused on every re-read), section 8's
  local-door `RAISE` (unbypassable, any client) and `release_medication_holds`'s pre-check, which refuses BEFORE
  signing with `stale_picture_refusal` — a floor `P0001` carries no scope, so without the pre-check the window could
  not re-read (#840's "stale wins" now fires for a release). The pre-check asks only about holds on the release's
  OWN thread: a hold id on another thread is a wrong input, left to the floor's thread sentence ("release it
  there"), never "stale — re-read and retry" (PR #853 review). A PENDING hold is standing: releasing it cancels it.
  **Tempting wrong fixes:** match the floor's message text in Rust for the scope (the pre-check reuses the predicate
  instead); re-inline the fold's tests (they drift); refuse at sync apply or replay (`cairn.remote_apply = 'on'`: a
  peer's dead release is admitted, the fold makes it a no-op — a local veto forks the event set, and a strict replay
  could not start the node). **#855:** section 8 locks each named hold (sorted) before it checks, and a floor
  refusal of a release the pre-check passed re-asks the predicate, so an overtaken release is STALE too, carrying the
  floor's own sentence (it names the FIRST bad hold, which may be another fault) — never
  drop the lock to "simplify" (two local releases are admitted together again) and never re-ask on an UNKNOWN commit
  outcome (the release may be our own). Pins: `tests/medication_release_stale.rs`, `tests/medication_release_race.rs`.
- **A Stop that lands on a line already stopped is WRITTEN, and the window says so (#843)** — no refusal: its reason
  (an adverse reaction, say) may be the only record of it while no allergy stream exists. `cease_impl` appends
  `already_stopped_note` (the earlier stop's reason, or "no reason was recorded") when its press-time read shows the
  line `Ceased` AND the press landed in full — every thread stopped for certain. Beside a commit with no answer
  (#814) or a failed thread, "your stop is recorded" would contradict the outcome's own "NOT known" / "NOT stopped"
  (PR #853 review). The note names the shown stop's reason AND every other stop's (#857): "no reason was recorded"
  only when no stop gave one, never beside a reasonless latest stop (PR #860 review). It closes with "the list keeps every
  stop's reason", true of a line that reads stopped (#857) and, since #861, of a stopped thread on a line still current
  (`stopped_member_reasons`) — so the sentence takes no `list` argument: the PR #860 caller-chosen claim and both its
  constants (`LIST_KEEPS_EVERY_REASON`, `LIST_SHOWS_REASONS_ONLY_WHEN_STOPPED`) are gone. The CLI's
  `medication-cease` says it too (#854), from the same sentence (`already_stopped_sentence`) and line rule
  (`MedicationRow::already_stopped_note`, `cairn-medication-view`), asked through `medication::already_stopped_note`
  BEFORE it ceases (after, its own stop would read as "already"). The window stops every thread of a line, the CLI
  ONE: so the CLI also asks the thread's own cessation, because a thread stopped beside a later active sibling leaves
  the line current and the line rule silent (PR #856 review). That the line still reads current is said AFTER the
  stop by `still_current_note` (#859, next bullet), never by this note. That read is display-only: its failure becomes a
  sentence (`not_known_whether_stopped`), never `?` — a stop is a safety write — and a thread on no line of the
  patient's list here says it cannot tell (silence would read as "not stopped"). The note goes to stderr, beside the
  CLI's other warnings. **Tempting wrong fix:** refuse it as stale like a release. Pins:
  `a_stop_of_a_line_already_stopped_says_so`, the `cease_sorts_an_interrupted_commit_from_a_failure` source shape,
  `tests/medication_cease_already_stopped.rs`.
- **Every stop's reason stays on the list (#857, maintainer 2026-10-23: "every stop's reason")** — `medication_cessation`
  (per thread, through db/033's `medication_group_status`) still decides the status, and db/033's
  `medication_group_cessation` view picks the SHOWN stop (`cessation_reason`, the one whose date the line shows); db/060's `medication_cessation_reason`, one
  row per stop that gave a reason, keyed by content address, supplies `other_cessation_reasons` — every OTHER
  distinct reason (surrounding whitespace ignored), newest recorded first, read in the SAME statement as the shown one
  (`read_cessation_reasons`, members resolved once and aggregated — a per-line subquery re-scans
  `medication_statement` for every stopped line). A line that reads CURRENT (a sibling's later dose wins) carries every stopped member's reason in
  `stopped_member_reasons` instead (#861, maintainer 2026-10-24: "show it, worded apart"), in a stopped line's order,
  said "a stopped record of this drug gave: …" (`stopped_record_lines`, `stopped_record_detail`) — never "Stopped",
  since the drug is not. **Tempting wrong fix:** fill `cessation_reason`/`other_cessation_reasons` on a current line
  (every renderer would then say "Stopped"/"ceased" of a current drug); keep the ceased fields ceased-only. A shown stop with no reason beside another stop's reason is said first ("no reason
  given"), so the other reason is not read as the shown stop's (`ceased_reason_lines`, `cessation_detail`). db/060
  bumped the generation so the loader's heal backfills; db/037's shred scrubs the row. **Tempting wrong fixes:**
  `COALESCE` the old reason into `medication_cessation` (still loses a reasoned earlier stop and a sibling's);
  reading the two parts in two statements. (#863 and #864 are fixed: the block above.) Pins: `tests/medication_stop_reasons.rs` (incl.
  `a_line_kept_current_by_a_sibling_still_shows_the_stopped_threads_reasons`), `other_reasons_drop_the_shown_one_and_repeats_only`, the
  renderers' unit tests, `registry_row_count_is_pinned` (30) and db/tests/039 (32).
- **A CLI stop of one thread says when the line still reads current (#859)** — `medication-cease` re-reads the line
  AFTER the stop (`still_current_note`) and names its threads with no stop: under latest-effective-wins (db/033)
  only an unstopped thread's dose can keep a line current. They are read from the whole group
  (`medication_thread_group`), not `row.members` (this patient's charts only): a thread on a chart not linked to this
  patient is counted, never named, and the note then drops "stop that one too". It says "no
  stop … is readable here", never "recorded": a stop sealed without custody, or a thread's only stop shredded, leaves no
  `medication_cessation` row though a stop was recorded (PR #860 review; #863 re-folds an earlier stop). Display-only:
  a failed re-read becomes `not_known_whether_current`, never `?`, since the stop is already written. Both notes go
  to stderr after the success line. Pins: `a_stop_of_one_thread_on_a_line_kept_current_names_the_sibling`,
  `a_sibling_already_stopped_is_not_named_as_unstopped`, `a_sibling_on_an_unlinked_chart_is_counted_never_named`,
  `the_cease_verb_reads_the_note_first_and_prints_it_last`.
- **A release recorded for a LATER instant is said on its hold (#835)** — the hold still stands until then (the
  fold is unchanged), and `MedicationHold::release_pending` carries the instant from db/059 section 7c's
  `cairn_medication_hold_release_pending` (the same thread-and-name match as 7b, and only a release BEFORE a timed
  hold's own `until`: one at or after it ends nothing, so "release at <t>" would be false — PR review). 7c does NOT
  look at `starts_at`: a release at or before a pending hold's start cancels it, and both surfaces then say
  "cancelled" (`MedicationHold::release_cancels_it`), since "from 08:00, release at 16:00" reads as a W from 08:00
  to 16:00. The CLI's `hold_line` and the window's `hold_view` say it BEFORE the free-text reason, and
  `next_hold_change` counts it, so the window re-reads when it takes effect. `read_group_holds` asks the fold and 7c
  at ONE instant (`n.now` in FROM); **tempting wrong fixes:** `clock_timestamp()` in the select list (re-evaluated
  per row), a `released_at >= starts_at` "tightening" of 7c, or a new fold column (a return-type change
  `CREATE OR REPLACE` refuses, and an older binary's replay of db/059 then fails). Pins:
  `tests/medication_release_pending.rs` (incl. its one-instant source guard), `a_pending_release_is_said_on_the_hold`
  and `a_release_before_the_start_*` (both surfaces), `a_pending_release_is_a_hold_change`.
- **A stale refusal keeps the gesture's button down (PR #842 review)** — `withhold`/`release` resolve "spent" for a
  report AND for a refusal whose retry is `after_reread` (`Retry::AfterReread` promises no live retry); any other
  refusal re-enables it. Pin: `both_gestures_re_read_like_cease`.
- **The W is a BUTTON whose accessible name carries its reason** (symbol + " — " + detail), and a ceased line
  whose stops gave a reason has its status word as such a button, revealing `cessation_detail`'s text ("Stopped:
  <reason>", then "Another stop's reason: <reason>" per other stop, #857; no reason on any stop: plain text) — and so does a CURRENT
  line with a stopped member's reason ("A stopped record of this drug gave: <reason>", #861). The semantic tree (`cairn-gui-tab-medications` `lib.rs`) declares the same labels — pin
  `every_hold_and_hold_control_is_a_labelled_button`. Hover is never the only way to the reason: WebKit does not
  focus a clicked button, so click pins it; Escape or an outside click unpins. **Tempting wrong fix:** a `title`
  tooltip or a hover-only popover. Pins: `the_w_is_a_button_whose_label_is_its_reason`,
  `escape_or_a_click_elsewhere_unpins_a_detail`.
- **The revealed detail is out of flow (`position: absolute`).** In flow it re-laid the table out, the button jumped
  from under a real pointer, `mouseleave` hid it, and the W flickered. Only a real mouse shows it (found walking
  #832). Pin: `a_revealed_detail_moves_nothing`.
- **"Until reviewed" is the default, and the select and the time field never disagree silently.** Typing a time
  picks "until a time"; a partial `datetime-local` is refused by Rust; and choosing "until reviewed" back EMPTIES the
  time field — `value = ""` plus a type reset, which WebKit needs to drop a PARTLY typed time (else the press
  still sent the typed time and the W lapsed silently at that hour). **Tempting wrong
  fix:** treat an empty or unparseable time as open-ended, or let a leftover time win over the select. Pins:
  `until_reviewed_is_the_default_and_never_an_empty_time`, `typing_a_time_chooses_until_a_time`,
  `choosing_until_reviewed_clears_the_time`, `a_re_read_keeps_the_until_choice_and_time`.
- **One press, one W.** The pressed button is disabled before it sends and stays so until the re-read; a returned
  report clears that line's fields (so the next press cannot repeat the W), a THROWN failure keeps them (the
  clinician's typing is not lost) and re-enables its own button — nothing was written, and the re-read may have
  failed and kept the old line. **Tempting wrong fix:** clear on every outcome, or re-enable in `finally` (that
  makes a recorded W pressable again). Pins: `a_gesture_button_disables_itself_before_it_sends`,
  `a_returned_report_clears_the_line_before_the_re_read`.
- **`status_label` "withheld" is wording only; `current` is the fact.** `current`, `can_cease` and sign-off are
  unchanged by a hold. **Tempting wrong fix:** key behaviour off the label. Pin: `a_withheld_line_stays_current_signable_and_ceasable`.
  The row's styling (`data-withheld`, `data-ceased`) is the one reader of the words, tied to Rust's by
  `the_row_draws_the_w_and_its_controls`.
- **The list is re-read when a hold starts or ends.** A hold changes with the clock, not with an event, so a screen
  left open kept "withheld · W until 20:00" into the next morning (where the dateless time read as today's) and
  "current" past a pending W's start. Rust sends `next_hold_change`; `drawList` arms one timer at it, a failed re-read
  retries each minute. **Tempting wrong fix:** always print the date instead — the status word would still be
  stale. Pins: `the_next_hold_change_is_the_earliest_instant_still_ahead`,
  `the_list_is_read_again_when_a_hold_starts_or_ends`.
- **A Release that can write nothing from this chart says so before the key** (a hazard line's other-chart holds),
  and only a release that released something is timed. Pin: `the_refusals_come_before_the_key`, which also pins
  that a withhold's reason and end are refused before the line lookup and the key.
- **Times on screen are LOCAL; the wire stays UTC.** `local_instant` converts for display only. **Tempting wrong
  fix:** send the local string. Pins: the `hold_view` display tests (`a_hold_ending_today_shows_the_local_clock_time`
  and its siblings) for the local half; for the UTC half, the `toISOString()` conversion pinned in
  `until_reviewed_is_the_default_and_never_an_empty_time` (`holds_webview_tests.rs`).
- **An unknown commit outcome is "NOT known", never "failed"** (a dropped connection mid-commit may have recorded the
  event). Pin: `both_gestures_sort_an_interrupted_commit_from_a_failure`. Release writes one event per thread
  (`a_release_writes_one_event_per_thread`, the plan half), each in its own call — the transaction scope is not
  pinned in the window.
- **The two gestures are timed** (`withhold`, `release` in `ui_gesture_timing`, db/044's check constraint).
  Pin: `the_withhold_gestures_are_timed`.

**⇒ #803 — THE MEDICATION WITHHOLD (ADR-0081) — do not undo** (pins: `tests/medication_withhold.rs`,
`medication_withhold_fold.rs`, `medication_withhold_signoff.rs`, `medication_withhold_clock.rs`, `medication_read.rs`):
- **The start of an un-`from`'d hold and a release's instant are the authoring node's PHYSICAL clock, carried as
  `t_effective`; the authoring node never stamps them from its HLC wall.** A peer whose clock runs fast pushes this node's HLC up to 24 h ahead (db/020),
  and a "W from now" stamped with that wall starts in the future: pending, so the drug reads as givable. db/059 has
  **no HLC-wall fallback**: the floor refuses, at both doors, a withhold with neither `from` nor `t_effective` and a
  release without `t_effective` (another client's, or a hand-built body). **Tempting wrong fix:** restore the
  `COALESCE(…, to_timestamp(hlc_wall/1000))` fallback "for robustness" — it makes a third-party client's "W from now"
  read as not started. The instant is read BEFORE the HLC tick, so the bitemporal ceiling never flags it. A test that
  hand-builds such a body stamps it with `medication_kit::effective_at_wall`.
  Pins: `medication_withhold_clock.rs` (both tests, each mutation-checked against its own line);
  `a_missing_t_effective_is_refused_at_the_sync_door_too`.
- **A hold sits BESIDE `MedicationStatus`; there is no `Withheld` variant.** **Tempting wrong fix:** add one.
  `signoff.rs` filters `status == Active` and the window's `view.rs`/`row_view.rs` (`current`, `can_cease`) would
  silently drop withheld drugs out of sign-off. Pin: `a_withheld_line_stays_current_and_signable`.
- **Expiry writes no event.** Whether a line is withheld is the read-time fold `cairn_medication_group_holds(groups,
  now)` (db/059); `now` is a parameter so tests pin instants (the fold tests; the staleness-clock pin
  `expiry_does_not_stale_a_sign_off`).
- **A release names the withholds it ends by EVENT ID, and only on its own `medication_id` THREAD** (`r.medication_id =
  w.medication_id` in the fold is the only guard on the sync path; the list shows holds per LINE, so `hold_line`
  prints each hold's event id and thread, and the local door's refusal names the thread a sibling's hold is on).
  Pins: `a_synced_release_on_another_line_ends_nothing`, `a_release_naming_another_lines_withhold_is_refused`.
  A release also cancels a not-yet-started hold (`a_release_cancels_a_pending_hold`).
- **Refusals split in two.** The structural floor runs at both doors (`an_empty_span_is_refused_by_the_floor_at_sync_apply_too`);
  state refusals (ceased line, `until` <= resolved start, release naming a non-withhold or another thread's) live at the
  LOCAL door only. **Tempting wrong fix:** move a state refusal to the sync door; a peer's valid event then wedges or
  forks sync. Pins: `a_sync_applied_withhold_on_a_ceased_line_is_admitted`,
  `a_peer_hold_already_over_is_admitted_at_sync_but_never_shown`; the structural tables
  `every_structural_{withhold,release}_refusal_holds_at_the_local_door`.
- **A ceased row carries no holds** (cessation wins, ADR-0081 decision 5). Pin: `a_ceased_line_shows_no_holds`.
- **`cairn_medication_thread_patient` has ONE definition, in db/031; its withhold arm is `to_regclass`-guarded
  there and orders by `(hlc_wall, hlc_counter, origin COLLATE "C", content_address)`**: the earliest claim stands. The
  other arms hold one row per thread; this one holds many. **Tempting wrong fix:** redefine the fn in db/059 — every
  replay then briefly runs db/031's body without the arm. Pin:
  `the_earliest_orphan_withhold_claim_stands_whatever_the_arrival_order`.
- **`cairn_event_thread` (db/048) probes one table per source file — db/031, db/032 AND db/059.** Pin: cairn-sync's
  `schema_subset_alone_satisfies_every_door`, whose two crash-mid-replay windows each leave exactly one probe deciding
  (window 1 probe 2, window 2 probe 3). Deleting probe 2 or probe 3 each turns it red (mutation-checked separately).
  Probe 1 has no window of its own: a replay that lacks db/031's table lacks db/032's and db/059's too (they load
  later), so probes 2 and 3 already return NULL there. A new
  medication source file needs a probe AND a window of its own.
- **The sign-off thread index was RENAMED `event_log_medication_thread_idx2`** (db/034) and the old one dropped in
  db/059: an `IF NOT EXISTS` index never picks up an edited predicate, so a type added to the commitment lists needs a
  NEW name again, never an edited predicate. Pin: `only_the_six_type_thread_index_survives_a_schema_load`.
- **Pinned counts this slice moved:** twin registry 26 (`twin_registry.rs`, `db/tests/034`), projection registry 29/31
  (`projection_registry.rs`, `db/tests/039`), `thread_resolution_cost.rs` 7 tables / 8 types.

**⇒ #822 + #823 — THE CLI AFTER AN UNKNOWN COMMIT OUTCOME — do not undo** (pins: `db_diagnosis.rs`'s
`only_a_stale_refusal_or_an_unknown_outcome_gets_the_cli_remedy`, `tests/event_lookup.rs`, `auto_apply.rs`'s
`run_verdict` tests, and `tests/identity_commit_lost.rs`'s `a_lost_auto_apply_commit_stops_the_run_with_its_summary`,
mutation-checked; `a_failed_hlc_tick_stops_the_run_with_its_summary` and `event_lookup.rs`'s unit tests since the
second review):
- **The marker's "check whether it was recorded" has a command: `cairn-node event <id>`**, named by `cli_remedy`'s
  line, `--conn` BEFORE the subcommand (it is not a global flag; the binary test runs that form). It reads clear
  envelope columns only (never the body, never `safety`) and says "IS / is NOT recorded on this node". The remedy
  claims only "if it does, do not run the command again", never "if not, run it again": a command could have committed
  earlier steps, which its own sentence reports. **"NOT recorded" says "as of now" and names the check before a
  re-run** (the interrupted session has ended, `pg_stat_activity`): when only the client's side of the connection was
  cut, the server can still be finishing that commit, and a bare "not recorded" invites the re-run that doubles it. A
  lookup that cannot read `event_log` is an ERROR, never "absent" (`a_lookup_that_cannot_read_is_an_error_not_absent`).
- **Auto-apply STOPS at the first pair whose outcome is unknown** and returns its summary (`outcome_unknown`,
  `not_attempted`; never counted `errored`). **Tempting wrong fix:** `errored += 1` and carry on, as for any pair. The
  session is dead, so the next pair's first call ends the run as a plain "connection closed" with no summary.
  "Running auto-apply again is safe" holds only because the link and the proposal's `auto_applied` status commit in
  ONE transaction: split them and that sentence becomes false. That pair's stderr line renders the cause through
  `commit_outcome_unknown_cause`, WITHOUT the marker's "it may be recorded twice", which would contradict it.
- **A connection that dies anywhere else stops the run too** (`connection_lost`: a `client.is_closed()` check at the
  top of each pair, and ANY `next_hlc` failure — not only one on an already-closed client, because a FATAL answering
  the tick arrives before the close; `node_hlc_tick` raises nothing of its own, so a failed tick is the session. Both
  print their cause, so the verdict's "the cause is on stderr above" holds even with `errored == 0`; pins
  `a_connection_lost_mid_pair_stops_the_run_with_its_summary`, `a_failed_hlc_tick_stops_the_run_with_its_summary`).

**⇒ #811 — A RE-READ AFTER AN INTERRUPTED COMMIT SAYS ONLY WHAT IT KNOWS — do not undo** (pins: `funnel/view_tests.rs`'s
wire test, `judgement_view_tests.rs`, `duplicates/verdict_tests.rs`, and the source guards in
`reread_webview_tests.rs` / `read_again_webview_tests.rs`):
- **`Retry::AfterReread(Recorded)`: `No` = a stale refusal, nothing recorded; `Unknown` = an interrupted commit
  (#713), it may have been.** It words only the out-of-date notice (`LIST_OUT_OF_DATE` vs `LIST_MAY_BE_OUT_OF_DATE`).
  **Tempting wrong fix:** collapsing it back to a unit variant because the window "does the same" for both. It does
  the same, but must not SAY the same.
- **`No` wins** (`Recorded` is ordered `Unknown < No`; `combined_retry`, `behind_the_record`, and `staleAfter` in
  `chart_stale.js`, which a later `"unknown"` never clears): one stale refusal PROVES the list is out of date.
  **Tempting wrong fix:** letting `Unknown` win "because it is worse". The banner then says "whether your last act
  was recorded is not known" right after a Stop whose own line said "nothing was done".
- **…but only until a list read AFTER the stale refusal draws** (`drewRead` clears `staleAfter` on its own, before
  `behindAfter` ends). That list shows the change the refusal was over; if only an interrupted commit is still behind
  it, a failed read says "may be". **Tempting wrong fix:** one `behindRecorded` value that stays `"no"` until the
  whole state ends — it then claims "the record changed" over a list that already shows that change.
- **Only `"no"` words "is out of date"** (`markListStale`'s `proven`): anything else, an unknown future value
  included, gets the weaker "may be", and the sign-off button says what the notice says.
- **"A change the re-read names may be your own" is Rust's, in the interrupted act's OWN sentence, for Link and
  Unlink only** (`judgement_error_view(act, Charts::MayMove, e)`). A confirmation (`confirm_error_view`) and a
  "different people" judgement are `Charts::Kept`: they never move membership, so a change named after one is
  certainly another source's. **Tempting wrong fix:** a JS sentence keyed on `"unknown"`. It cannot tell the acts
  apart, so it excuses a colleague's link as the clinician's own.
- **That sentence is window state too** (`unknownLeads`, said by `withOwed` from whichever read ends the state). The
  act's own re-read can be overtaken ("Read this chart again", an own act's draw floor), and the read that draws
  would otherwise name the clinician's own link as another source's change. **Tempting wrong fix:** relying on the
  re-read's `lead` argument alone.
- **#814: every `.commit()` and every autocommit event write in the tree is inventoried** by
  `db_errors_stay_legible.rs` (autocommits are FOUND, by a door call on a receiver other than `tx`, since the #817
  review; never declare a count by hand): in `MARKED_COMMIT_SITES`
  (each through `db_diagnosis::commit_failure`, counted per file: judge, each sign-off line, both `seal_sign_submit`
  arms, the attested reconciliation, the CLI's attestation, a registration since #815, and since #817 every identity
  write: identify, a John-Doe chart, evidence, a proposal's link, an auto-applied link, and the autocommit submits of a
  sensitivity grade/withdrawal, a shred and a text evidence) or in `UNMARKED_COMMITS` with its reason (a
  restore's `finalize_identity`, whose landed commit fences the door; the node-plane events and actor enrolments,
  which a re-run cannot double; a read-only snapshot; the sync harness's load generator). A marked file's
  commit that writes NO event (auto-apply's two send-to-review commits) is counted in `EVENTLESS_COMMITS` with its
  reason. An unattested submit's autocommit statement IS its commit, so it is marked too.
  Only an ordinary ERROR answer proves a rollback; no answer, or a FATAL/PANIC (which can follow a landed commit), is
  marked. `data_error_from` classifies a marked error as `DataError::OutcomeUnknown` (#815), so `node_failure`
  re-reads it. **Tempting wrong fix:** a bare `tx.commit().await?` "because
  the statements before it already succeeded". It compiles, reads as an outage, and offers the #713 retry. **Also:** a
  cease's unknown threads are neither stopped nor "NOT stopped", and `write_failure::cease_outcome` drops "Stopped 0"
  when one is unknown.
- **#815: an interrupted REGISTRATION spends its search, exactly as a success does** (`TokenStore::settle` →
  `Restored::Consumed`; pins: `token.rs`'s `an_unknown_outcome_consumes_the_form_as_a_success_does`,
  `commands_register_tests.rs`'s unknown-outcome walk, mutation-checked; end to end through a real lost COMMIT,
  `cairn-gui-live/tests/lost_commit.rs` (#820), and in the root tree `tests/identity_commit_lost.rs`, which pins that
  the marker names the REGISTRATION event and the error text the chart, John Doe's headline included).
  `data_error_from` classifies the marker as `DataError::OutcomeUnknown` FIRST; `settle` itself asks `RegistrationFailure::may_have_written`, so the short call
  stays the safe one (#659) and no caller special-cases it; `register_error_view` says "NOT known" with
  `Retry::AfterReread(Unknown)`, which funnel.js reads as "every search is spent": it forgets the token it holds even
  when a newer search replaced the one sent, so the next Register press searches first, and that search finds the
  chart if it landed. **Tempting wrong fix:** restoring the search "so the clerk can press
  again" — `register_patient` mints a fresh `patient_id` per call, so that press registers the same person twice.
  Merely dropping the attestation is not enough either: a background search minted mid-flight would still license a
  second chart, which is why it is `commit()`'s invalidation.
- **#818: once a sign-off line may have committed, `sign_off_medication_list` never returns `Err`.** A lost commit,
  or a transaction that will not open, ends the loop with `SignOffOutcome::interrupted` (the line it stopped at, a
  `StoppedLine` — `MayBeSigned { event }` or `NotSigned`, never an `Option` whose `None` reads as "not known" — and the
  untried rest); a failed ROLLBACK keeps the line's own reason. Its `cause` is the root cause, not the marker's text.
  The CLI's empty-`attested` sentence is the pure `nothing_attested_sentence`: never "nothing was signed off" beside a
  line that may be signed. The window's
  whole sentence is Rust's `write_failure::sign_off_outcome` (truth-table tested; "Signed 0" is dropped beside an
  unknown line, as `cease_outcome` drops "Stopped 0") and `signOff` says `report.text` and re-reads with
  `report.behind_the_record` (returned by `sign_off_outcome` with the text, so they cannot disagree). Pins:
  `tests/signoff_interrupted.rs` — a test-only trigger on one named line answers its COMMIT FATAL, kills the backend
  inside the line (its own reason survives the dead ROLLBACK; mutation-checked), or answers the COMMIT with an ERROR
  (that line alone fails; the rest are signed). **Tempting wrong fix:** a `?` on `client.transaction()` or
  `tx.rollback()` in the loop — it compiles, and discards every line already signed and every reason.
  **And (#820):** `SignOffOutcome`'s three production readers (the CLI's `sign-off` arm and its
  `nothing_attested_sentence`, `sign_off_impl`) destructure it naming every field, no `..`, so a new field breaks the
  build at each reader. **Tempting wrong fix:** a `..` "to shorten it" — Rust never warns on an unread `pub` field,
  which is how `failed` was once dropped. Pinned by source guards in both trees:
  `tests/signoff_readers_name_every_field.rs` and `commands_tests.rs`'s `sign_off_impl_names_every_outcome_field`.
- **No script spells the string `"after_reread"`**, not even in a comment: the retry is an object, and
  `failure.retry === "after_reread"` is always false (no re-read, a live button). Guarded over every `src-ui/*.js`.
- **`forgetBehind` is called only by `drewRead` and `holdCharts`** (counted). One more call in an own-act path would
  end the stale state silently.
- **Any re-read that names a change says `READ_AGAIN` ("review it before you act on it")**, including after the
  clinician's own successful act (M5), and once per line: "Read this chart again" keeps the line as its lead. A
  colleague's change is news to review either way.

**⇒ #797 — A CHANGED RECORD RE-READS ITSELF AND SAYS WHAT CHANGED (PR #805) — do not undo any of these** (pins: the Rust
halves by `crates/cairn-node` tests of `stale_picture_refusal`'s callers, `cairn-gui-live`'s error mapping tests,
`cairn-gui-tauri`'s `chart_set/change.rs`, `write_failure.rs` and `link/judgement_view.rs` tests; the JS halves by SOURCE
guards in `cairn-gui-tauri/src/reread_webview_tests.rs` and `read_again_webview_tests.rs`, each mutation-checked, and
a headless walk on the PR). The
maintainer's governing rule, which every item below serves: *"The chart I hold in my hand when I make changes is my
responsibility — my obligation to know its contents. I need only to be informed of what changed between me taking the
chart and finishing amends that I cannot know (that is, anything that wasn't in the chart when I took it and I didn't
change myself)."*
- **A "the record changed under you" refusal is routed by TYPE, never by text.** `RefusalScope::Stale` (cairn-node,
  raised by the record check in `chart_link.rs`, `admit.rs`'s `LacksPair`, `shown.rs`'s `stale_record`/`stale_pair`,
  `medication/signoff.rs`'s `ensure_same_charts` and `ensure_same_targets`) → `DataError::Stale` → `Retry::AfterReread` (the funnel maps it to
  `Never`: it has no chart to re-read); on the Rust side `ChartSetError { Unreadable, Changed }`. **Tempting wrong
  fix:** matching `e == CHANGED` (or any sentence) again — a reworded node sentence then silently stops re-reading. The
  neither-record arm of `shown.rs` stays `Input` (nothing is stale; the input is wrong).
- **Node sentences name no window control.** They say only what happened ("— nothing was done" / "nothing was
  signed"); the CLI shows them too. The window's `chart_stale.js` adds what to do; the CLI prints
  `db_diagnosis::cli_remedy`'s line after a `Stale` refusal (`main` wraps `run()`). Pinned by
  `crates/cairn-node/tests/node_sentences_name_no_window_act.rs` (#806). **Tempting wrong fix:** putting
  "press Read this chart again" into the node sentence — it is nonsense at a terminal.
- **`READ_AGAIN` ("The chart has been read again — review it before you act on it.") is said only after an ACCEPTED
  answer whose list actually DREW** (`renderedPatient === patient` after `render`). **Tempting wrong fix:** saying it
  when the re-read is sent — a failed or dropped read then claims a chart the screen does not show.
- **"The list is behind the record" is WINDOW state (`behindAfter`, `chart_stale.js`), not one read's flag.** A
  stale refusal records the newest read revision sent before it (`knowListBehind`, in `refresh` before its revision is
  taken); WHICHEVER read fails last then marks the list (`listKnownBehind`); only a list drawn from a read sent AFTER
  the refusal ends it (`drewRead`), and taking the list down ends it (`holdCharts(null)`). A success report can carry
  it too: a partly-recorded "Different people" whose failed pair was `AfterReread` sets
  `LinkReportView::behind_the_record`. An OLDER read landing after a newer one already failed and marked the list
  re-marks it and leaves the outcome line (`sayAfterDraw`; whether it was marked is read BEFORE `render`). Pins:
  `a_known_stale_list_outlives_the_read_that_learnt_it`, `an_older_read_landing_late_keeps_a_stale_mark` (walked
  headlessly: an overtaking "Read this chart again", an older read landing late before and after the failure). **Tempting wrong fixes:** a `staleIfFails`
  argument — the re-read it rides is overtaken by "Read this chart again" or an own act's draw floor, its failure is
  never said, and the overtaking read fails unmarked; or a plain boolean cleared by ANY draw — `mayDraw` lets an older
  read, sent before the refusal, draw after it.
- **`markListStale` keeps the rows and disables sign-off** (`#chart-stale` says `LIST_OUT_OF_DATE`, or since #811
  `LIST_MAY_BE_OUT_OF_DATE`: see that block). **Tempting wrong
  fix:** hiding the rows — the clinician then has no chart at all during a partition (availability over consistency),
  and the node's set check refuses the write regardless, so the rows were never the danger; the danger is acting on
  them unwarned.
- **`set_change` (the sentence "Now part of it: B.") is `None` for an equal seen set, an unreadable one, or one that
  does not hold the opened chart** (`chart_set/change.rs`: `seen_set` drops an unreadable or foreign set,
  `set_change_sentence` says nothing for an equal one; both pure). **Tempting wrong fix:** a
  sentence whenever the sets "differ" in any way — an unreadable or foreign baseline would announce every member as new.
- **The window HOLDS the chart set it drew (`heldCharts`), moved only by being shown or by the clinician's own act**
  (`applyOwnAct`, `holdOwnAct`; every `refresh` sends it as `seenCharts`). Only what the clinician did not do and could
  not see is announced. **Tempting wrong fix:** the "set formula" `(held − (sent − after)) ∪ (after − sent)` — in the
  table's fourth row (a Link answered on the OTHER chart, drawn before its commit) it announces the clinician's OWN link
  back to them; the anchored `applyOwnAct` passes every case in the table above it in `chart_stale.js` (pinned only
  by source guard — its helpers `without`/`ordered`/`sameCharts` by body — and the headless walk; there is no JS
  runner, #332) and is the only spelling to use. Do not pass the baseline as a one-call argument either: a second
  re-read starting within one round trip announces the own Link.
- **`heldCharts` is deliberately SEPARATE from `renderedCharts`.** `renderedCharts` keeps feeding every write's set
  check; `heldCharts` is only the baseline for naming changes. **Tempting wrong fix:** merging them, or sending
  `heldCharts` to writes.
- **The draw floor is raised (`drawnRevision = ++chartReadRevision`) only when the own act MOVED the held set.**
  **Tempting wrong fix:** raising it unconditionally on every own act — that drops a colleague's in-flight read that
  would have been the first news of a real change.
- **`charts_after` (the Link / Unlink / Confirm reports' record as the act left it) is read INSIDE the act's
  transaction** (`chart_link/judge.rs`, after the apply, before the commit). **Tempting wrong fix:** reading after the
  commit — a colleague's link landing in that gap is then folded into the clinician's "own" baseline and never
  announced. The guard `the_set_left_is_read_after_the_apply_and_before_the_commit` (`chart_link/judge_tests.rs`)
  pins that order. **And it is attached only when the act changed the record** (`reload` true;
  `LinkReportView::left`, pinned by the `*_reports_no_set` tests): an Outranked act's set would move the held set
  and raise the draw floor with no re-read following, dropping a colleague's read already in flight.
- **"Read this chart again" (`#reread`) is always visible in the chart view, and its label is pinned to every sentence
  that quotes it** (`LIST_OUT_OF_DATE`, `REVIEW_WAITS_FOR_LIST`, `NO_LIST_TO_COMPARE`, `chart_set`'s `UNREADABLE`, the
  banner's and the record-links' read-failure sentences; `every_sentence_that_names_the_control_quotes_its_label`).
  `LINK_GONE`'s `Never` arms deliberately name no button: a re-read redraws the same lines, so it cannot mend them.
  **Tempting wrong fix:** showing it only after a failure (the first failed read then has no way out), or relabelling it in one place.
- **"Read this chart again" re-reads with the outcome line as its lead (`rereadLead`), less a read failure it is
  retrying** (`sayReadFailed` remembers the failure's line and lead; a drawn re-read with nothing to add takes a
  standing failure back to its lead). Pins: `read_again_keeps_the_outcome_line_but_not_a_failure_it_retries`,
  `the_only_lead_less_re_read_is_a_chart_s_first`. **Tempting wrong fixes:** a lead-less `refresh()` — a change it
  names REPLACES a partial sign-off report (ADR-0060: partial completion is reported, never implied); or the raw
  outcome text as the lead — each failed press then piles another "Could not read the chart" onto the line, and a
  recovered read leaves the failure up. The one lead-less re-read is a chart's first (`enterChart`, after `clearChart`
  empties the held set, so it can name no change).
  Sign-off and cease return an `ErrorView` through `write_failure.rs`: a locked key is `Now`; no node key or no DB is
  `AfterOperator`; a failed chart read before the write is `Now` (`chart_read_failed` — an outage, never `refused`);
  never a bare string (it is worded "could not reach its backend").
- **Accepted limitations (honesty, not safety — do not "discover" them later):** a stale refusal's out-of-date mark is
  lost if an own act's re-read outdates it and then fails (the node's set check refuses writes regardless); a foreign
  change whose re-read hits a DRAWING fault is never named, because `showUnshownList` nulls `heldCharts`.
- **No window sentence says "reload the chart" or "refresh and try again"** (guard
  `no_window_sentence_says_reload_the_chart`, over every `.rs`, `.js` and `.html` source but tests): the window has a
  named control, so name it.

**⇒ THE CHART RE-READ (#799 · #800 · #801, PR #802) — do not undo any of these** (pins: the JS halves by SOURCE
guards in `cairn-gui-tauri/src/chart_view_webview_tests.rs`, each mutation-checked; walked headlessly on the PR):
- **Two questions, two predicates, each pinned as its exact expression.** An ANSWER draws while
  `mayDraw(patient, revision)` holds — its chart is on screen and nothing NEWER has been DRAWN (`drawnRevision`,
  recorded just before `render`). A FAILURE is said only while `isLatestRead(patient, revision)` holds — no newer
  read has STARTED (`chartReadRevision`, taken before the send). **Tempting wrong fix:** one "newest started" rule
  for both — the older of two overlapping re-reads usually lands first and was dropped, so if the newer failed the
  screen kept a list older than both (PR #802 review). Or back to `patient !== displayedPatient`, which cannot tell
  two reads of one chart apart.
- **`clearChart` sets a floor: `drawnRevision = ++chartReadRevision`.** It is LOAD-BEARING: `mayDraw` asks only
  about what is drawn, so without the floor a first-visit read could land on the second visit (A → front door → A).
- **`redrawBannerAlone` notes the revision and never bumps either counter** (a bump would make an in-flight chart
  read fail `isLatestRead`, so its failure would go unsaid).
- **A re-read keeps the typed stop reasons** (`stop_reasons.js`: `typedReasons` before the rows are emptied,
  `restoreReasons` after, focus and caret included). The banner-only re-read after a decided refusal (below) still
  stands, on its own reason.
- **`render` = the list (`drawList`, which sets `renderedPatient`/`renderedCharts` and draws EVERYTHING but the
  banner) under one guard, THEN the banner.** A drawing fault takes the whole list down (`clearDrawnList`, shared
  with `clearChart`), NULLS `renderedPatient`/`renderedCharts` — so the banner drawn next offers no Review and
  `compare` refuses, the rule a failed or in-flight read already keeps — disables sign-off with a worded reason, and
  says `LIST_NOT_SHOWN` in `#chart-unshown`, inside the warnings ABOVE the table, **never on the outcome line** and
  never as "Could not read the chart". A new drawer goes inside `drawList`; the banner must stay after it.

**⇒ THE BANNER'S AND TRAY'S FAILURE STATES (#745 · #752 · #740) — do not undo any of these** (pins:
`cairn-gui-tauri/src/duplicates/{tests.rs,view_tests.rs,verdict_tests.rs}`, `worklist/tests.rs`,
`commands_tests.rs::every_command_the_webview_calls_is_registered`; the JS halves by SOURCE guards —
`duplicates/webview_tests.rs` and `worklist/tests.rs`, each mutation-checked — since there is no JS runner):
- **The banner hides ONLY on Rust's `silent`** (`DuplicateSection::is_silent`, sent by a hand-written `Serialize`).
  Both destructure EXHAUSTIVELY: a new field must not compile until someone decides whether it un-silences the banner.
  **Tempting wrong fix:** `#[derive(Serialize)]` back, or a JS-side conjunction over the fields.
- **A failed `med_list` read redraws the banner ALONE** (`duplicate_banner` → `banner_alone_impl`; `main.js`'s `refresh`
  catch → `redrawBannerAlone`): the warning needs only the chart set. An unread set is a worded banner, never a hidden
  one; unread member names are said (`note_unread_names`), never dropped; a refusal of the read is an `ErrorView`
  (a bare string would be worded "could not reach its backend"). **`render` draws the banner through
  `drawDuplicates`** — a drawing fault is said ON the banner, keeping its own error and check lines, never thrown into
  `refresh`'s "Could not read the chart". **duplicates.js builds ONE section itself (`emptyBanner`)**, its keys pinned
  to what Rust sends — a second hand-built literal is a second copy of the shape to drift.
- **A decided (`"never"`) refusal of a comparison or a judgement re-reads the BANNER** (`redrawBannerAlone` from
  `link.js`'s `compare` and `sendJudgement`; #713's `after_reread` still re-reads the whole chart). **Tempting wrong
  fix:** `refresh()` — a heavier read that rebuilds every row over a refusal that changed nothing but the banner
  (a typed stop reason survives it since #799; the set-changed refusals, which DO stale the list, re-read the whole
  chart since #797 — see that block). `NOTHING_OPEN` / `NOT_SHOWN_OR_RESOLVED` never say "reload": no sentence says "reload the chart" or "refresh and try again" (`no_window_sentence_says_reload_the_chart`); the
  way out is the "Read this chart again" button (#797).
- **No comparison without a drawn list:** `compare` refuses while `renderedPatient` is null (`NO_LIST_TO_COMPARE`),
  and a banner drawn while no list is on screen (after a failed FIRST read, or a list that could not be DRAWN — #801)
  shows `REVIEW_WAITS_FOR_LIST` in place of Review. Both sentences must be true of all three cases (failed, in flight,
  not drawable — "could not be read or shown, or is still being read") and name the way out ("Read this chart again", the
  always-visible button #797 added — its label is pinned to the sentences that quote it).
- **The tray's count and list each carry a revision** (`trayCountRevision`, `trayListRevision`): an older answer is
  dropped. **The list is emptied when a re-read starts** — never the old entries, whose Review `close_chart` made
  unopenable. **Wherever the list is emptied, `trayListRevision` is bumped too** (`refreshTray`'s clear): an older
  list answer still in flight would otherwise land during the count's await and draw the entries straight back.

**⇒ R6'S RESIDUALS (PR #792: #787 · #786 · #783 · #782) — do not undo any of these** (pins:
`tests/standing_link_legacy_address.rs`, `cairn-patient-search/src/list_wire.rs`, `cairn-medication-view`'s
`the_doubted_link_remedy_names_the_bridge_lines_door_too`, `cairn-gui-tauri/src/link/{lock_hint_tests.rs,confirm_view.rs}`):
- **`standing_link` reads `content_address` as an `Option`** and refuses a NULL as a NODE-STATE refusal (a pre-#115
  row; db/018's `ADD COLUMN IF NOT EXISTS` leaves it nullable). **Tempting wrong fix:** back to `r.get(0)` "because
  the column is NOT NULL" — it is only on a fresh database. The remedy names the HEAL first (`cairn-node reproject
  --prefix identity.`, no deletes: replaying the winner beats a NULL address, db/002's `COALESCE(cur_addr, '\x')`)
  and a whole `--rebuild` only as the fallback (it truncates every projection and holds chart reads). Never name
  `--prefix identity. --rebuild` — `cairn_reproject` refuses it (it would truncate `patient_chart`). The test runs both.
- **A row's doubt mark crosses the wire only as `CandidateList`'s additive `doubted_rows`** (`list_wire.rs`). The
  row's own flag is the one in-memory truth; `doubted_rows` exists only on the wire, absent when empty. Never put
  the mark into `PersonRow`'s serde (its bare member array is what the attested list is built from), and never give
  `CandidateList` a second in-memory doubt field.
- **Both bridge remedies name the unlink's door**: `DOUBTED_LINK_INSTRUCTION` and the review panel's `Separated` note
  (`confirm_view.rs`) say to open "Review this link…" on a link still joining the pair, "marked in doubt or not" —
  never "unmarked" (a joining link can also clash).
- **The first lock poll starts on `DOMContentLoaded`**, never at `main.js`'s top level, and **no script tag may be
  `defer`, `async` or a module**: that ordering is what makes `updateLinkLock` exist when the answer lands.

**⇒ R7'S DURABLE RULES (PR #788) — do not undo any of these** (pins: `tests/{unverified_link_note,
duplicate_check_status}.rs`, `cairn-gui-tauri/src/duplicates/{read_order_tests,view_catchup_tests}.rs`,
`cairn-gui-tauri/src/link/record_links_unverified_tests.rs`, `unlink_tests.rs::an_unread_note_list_never_blocks_a_verdict`):
- **A soft veto stays OUT of `cairn_chart_set_doubts`.** `cairn_chart_set_unverified_links` is a separate function so
  the R1b hold, `EXISTS` over the doubts, can never see a `degrade_hold` (db/016: it must never demote a link). Folding
  it in as a third kind would freeze sign-off on formatting differences.
- **The banner and the tray read the check state BEFORE the open pairs** (`duplicate_section`, `tray_count_impl`). The
  worker never deletes a notice before the proposals it caused are committed (`queue_db.clear_notices`); pairs read
  first can miss one that
  lands between the reads, then "nothing waiting" — a silent "checked, none open". **Tempting wrong fix:** "tidy" the
  reads back into display order.
- **A failed unverified-notes read never costs the links list** — `RecordLinks.unverified` is a `Result`, the lines
  and their doors draw, and the section says the notes are unknown. A line without a note must never be read as
  "verified", so an unread note is said, never shown as none.
- **`read_snapshot` reads every status column with `try_get`** (`status_column`): `cairn-node duplicate-check` never
  replays the schema (#726), so a binary newer than its database meets an older function shape and must say so, not
  panic.
- **db/056's heal guard is the NEWEST OUT column's absence** (`charts_changing`), so one guard covers every older shape.
  A future column change moves the guard to the new newest column.

**⇒ R6'S DURABLE RULES (PR #777) — do not undo any of these** (pins: `tests/{link_doubts,confirm_link,unlink_as_shown,
search_doubted_row}.rs`, `cairn-gui-tauri/src/link/{confirm_view.rs,record_links.rs,unlink_tests.rs,
unlink_view_tests.rs,unlink_offer_tests.rs}`, the `unlink.js` field-name drift guards):
- **ONE SQL spelling of "doubted": db/054's `cairn_chart_set_doubts`** (`clash` = a `link_veto_flag` row or an
  un-attested standing link that trips the hard veto now; `separated` = an attested unlink between two charts the set
  still joins). R1b's hold, `cairn_chart_set_has_doubted_link`, is `EXISTS` over it, so the hold and the window's
  per-line marks cannot disagree. Never write a second predicate for either.
- **Never sign a verdict without the standing address the panel read.** `RecordsShown::pair` is re-checked under
  CARNLK for BOTH the confirm (`chart_link::confirm_link_as_shown`) and the panel's Unlink
  (`chart_link::unlink_charts_as_shown_from`): a colleague's judgement of the SAME pair landing mid-review, while a
  third chart keeps the record joined, leaves the set unchanged — only the pair's address shows it. A changed pair is
  refused (`stale_pair`); an empty address is refused (`require_standing`). The CLI's `unlink-charts` (no panel)
  keeps `unlink_charts`. **When you guard one verdict, ask the same question of every verdict on that panel.**
- **`standing_agrees`: the list's read and the panel's read must agree, or NEITHER verdict is offered** (a standing
  read of nothing included — `PAIR_CHANGED_MID_READ`). It checks that two reads agree; it is NOT a second doubt rule.
- **The doubt never enters the signed list or a row's JSON**: `displayed_charts()`, `SearchAttestation`, the cap and
  the ranking ignore it (`search_doubted_row.rs` compares the attestation's bytes with and without it).
- **Both verdicts are disabled in flight and hidden once one lands — Outranked included** (`unlink.js`, and R5a's
  `link.js`). `same_verdict` offers nothing for an attested link and needs both charts of the pair known held.
- **"Review this link…" / "Review this judgement…" is the ONE door per line**; the panel, not the list, decides which
  verdicts apply. The clash mark says "while", never "until" (two doubted links: judging one leaves the hold), names
  what CAN be signed ("only lines recorded on this chart alone" — hazard.rs withholds shared lines too), and
  `DOUBTED_LINK_INSTRUCTION` names both doors.
- **A door carries the attestation its line showed** (`RecordLinkView::attested` → `compare_linked`): a pair judged
  since the list was drawn is refused (`line_still_shows`, `LINE_STALE`) before any panel is drawn — otherwise an
  attested link opens a silent panel offering only Unlink. The list's two reads (`read_record_links`) share ONE
  read-only REPEATABLE READ snapshot, and a clash naming no standing link is said (naming "Read this chart again" since #797), never dropped.
- **Any recorded verdict disarms a newer comparison of the same pair**, in both panels (`disarmNewerReview` /
  `disarmNewerComparison` — the scripts share one global scope, so never one name for both). Never key it on
  `reload`: "Different people" reloads even when every pair was outranked.
- **One Rust spelling of the doubt boolean: `patient::edges::record_is_doubted`** — the hold (via the medication
  read) and the duplicate-review tray's doubted side (#723 on the tray) both ask it; an unread doubt is a worded
  side error, never "not doubted". An empty standing address is refused by the node itself (`judge()`), with the
  true reason.

**⇒ R5b'S DURABLE RULES (PR #749) — do not undo any of these** (pins: `matcher/tests/test_{judged,proposal_retraction}.py`,
`tests/{auto_apply,duplicate_worklist,duplicate_review_flags}.rs`, `duplicate_review/{worklist.rs,tests.rs}`,
`cairn-gui-tauri/src/worklist/`):
- **The matcher's skip rule IS db/057's** (save ADR-0078 decision 1's clamp-and-flag case; `judged.py`, the six-case
  drift test): one record, or an ATTESTED `patient_link` row. "Judged" is not "a human judged it" — only an UNLINK needs
  attestation to settle a pair (ADR-0078). `AWAITING_HUMAN` is exactly `pending` + `review`: the matcher never
  retracts a human's verdict.
- **`DISPUTED_SQL` is the ONE spelling of "disputed"** (a standing un-attested unlink): the banner, the worklist's flags
  and `auto_apply` step 2b format it in — never write `state = 'unlink' AND NOT attested` again. Auto-apply never links
  over a dispute: the proposal goes to `review`, with no event and no `patient_link` row.
- **The count and the list share one grouping** (`RECORDS_FROM`, `LOW_RECORD`/`HIGH_RECORD`), so they cannot
  disagree. **The count never evaluates the veto or the dispute; flags are read only for the shown pairs**
  (`read_pair_flags`, one statement); **`candidate_read`'s `read_display_facts` is the one display read** — ONE batched
  call per list read. An open tray re-reads on every return, over the connection the search shares.
- **The tray re-reads on every return because `close_chart` clears `shown`** (drop it and Review is refused).
  **"Different people" refuses an `accepted` pair IN THE BACKEND** (`DifferentPeople::AcceptedAsSame`, #736). **A
  failed record read is worded per side and withholds Review** when the newer side failed; it never fails the list.
- **The tray's Review buttons are `data-candidate`** (disabled while a registration saves); **the status line sits
  OUTSIDE the collapsed `<details>`** ("(0)" must never read as "checked, none"); never `role="alert"` or focus-taking.

**⇒ R5a'S DURABLE RULES (PR #735)** (pins: `tests/{match_proposal_open,duplicate_review}.rs`,
`cairn-gui-tauri/src/duplicates/{view_tests.rs,verdict_tests.rs,tests.rs}`):
- **`match_proposal_open` (db/057) is the ONE "still needs a human" predicate**: a pair leaves it only when it reads as
  one record or an **ATTESTED** unlink stands — **never count an un-attested unlink** (any unreviewed writer could
  clear a banner). Its statuses are pinned to `OPEN_PROPOSAL_STATUSES`.
- **An absent banner means only "checked, none open"**: every failed read is a worded line (`duplicates/view.rs`); a
  Current node never appends "up to date" after an unheld member's "not yet run" line.
- **`AppState::shown` is NOT widened**: Review's other chart is admitted by `duplicates::admit_other` re-reading the view
  (after `ALREADY_IN_RECORD`). **`duplicate_section` reads in two phases** — never hold `state.db` (a non-re-entrant
  tokio Mutex) across `read_chart_of` / `chart_set_of`.
- **"Different people" = an attested unlink on every pair STILL open between the two records, read fresh**; a pair's
  failure is carried, never `?`; a failed pair reads "For charts A and B, <its own sentence continued>" (#740 — never
  "not recorded", never a stutter); all-failed → one ErrorView naming every pair. **The veto note is computed fresh**
  (`cairn_match_veto`), never from stored `veto_findings`.
- **Webview:** `duplicates.js` loads after `main.js` and BEFORE `funnel.js` (whose last line boots) — never a `typeof`
  guard on `render`; "Different people" shows only on the banner's own comparison token.

**⇒ #725'S DURABLE RULES** (pins: `matcher/tests/test_{blocking_sql_shape,targeted_blocking,pure_modules_import_without_psycopg}.py`):
- **No range arm joins a sex scan.** A chart's blocking sexes ride on its birth-window row (`sexed_window`, one join by
  `patient_id`); the `+sex` arm is a FILTER (`anchor_sexes && member_sexes`). A CTE scan has no statistics, so an
  equi-join of two sex scans on a ~2-valued column is a planner cross-product (14.7 M rows).
- **The anchored per-chart range statement is the sweep's plus `relevant_anchor` and `_ANCHOR_CLAUSE`, nothing
  else**; the clause restricts the ANCHOR, never the member. Add a range arm to `_RANGE_ARMS` and both statements get it.
- **`pipeline/db.py` is the ONLY module that imports psycopg at import time** (plus `eval/measure_check`); blocking SQL
  lives in the pure `pipeline/blocking_sql.py`; a psycopg-needing test imports inside or `importorskip`s; a guard
  collects the suite without psycopg. **The canary runs capped too** (10, 6). Reproduce CI's pure job with
  `CAIRN_ALLOW_DB_SKIP=1 uv run --isolated pytest`.

**⇒ R4'S DURABLE RULES (PR #724)** (pins: `tests/{match_pending,duplicate_check}.rs`,
`matcher/tests/test_{check_chart,worker_drain,watch_backoff,targeted_blocking,judged}.py`):
- **The hook never raises and never waits** (`the_hook_has_no_raising_path`, the `lock_timeout` test): a plain INSERT,
  a null guard, no RAISE/EXCEPTION/ON CONFLICT. **Never "fix" it with `EXCEPTION WHEN OTHERS`** — a swallowed failure
  is a silently skipped check. `patient_chart`'s hook is INSERT-only; the catalogue test pins `tgtype` and the `uuid`
  arguments.
- **Delete exactly the notice ids READ, captured before the projection reads — never `id <= max`** (a bigserial is
  assigned at INSERT, not commit); bulk mode captures before the sweep. **No read transaction across slow work**
  (`check_chart` rolls back after its read-prep and after every assess); a no-op DDL replay waits behind any reader (#726).
- **"Behind" = charts waiting AND no completed work AND the OLDEST notice older than 5 min** (`quiet_age_s`).
  **Progress is stamped ONLY by completed work** — never before work, never for a failed pair: either masks a
  crash-loop as "running" (`test_a_sweep_whose_blocking_raises_stamps_no_progress`, `…every_pair_fails…`).
- **`cairn_chart_check_pending` is TRUE until a worker has run, while a notice waits, and for a chart not held here**
  (never a false "checked"). **The worker never links** (it writes only via `persist`) and **never proposes a judged
  pair**, in either mode or in reconciliation. **`me` is canonicalised once** (`str(uuid.UUID(...))`).
- **`cairn-node duplicate-check` uses `db::connect`, never the schema replay** (it is cron-able). Status wording lives
  only in `duplicate_check.rs` (goldens); the runbook quotes it verbatim.

**⇒ R3'S DURABLE RULES (ADR-0076 decision 6, PR #721)** (pins: `person.rs`, `tests/search_by_person.rs`,
`attestation.rs`, `candidate_text.rs`, `prompt.rs`, `search_person.rs`, `commands.rs`, the byte-identical goldens):
- **Rows are built ONLY by `group_by_person`** (pure): a row sits at its best-ranked member; matched members first in
  rank order, then unmatched oldest first; every chart exactly once; the seven ADR-0075 keys rank CHARTS.
- **The signed list is ONLY `CandidateList::displayed_charts()`**; **the cap counts ROWS and never splits one**
  (`withheld` counts people; `shown_charts` is signed).
- **A chart not held here reads `Unknown` and never sets `incomplete`; a HELD chart with no readable name does. A
  missing component fails the search — never a row of one.**
- **Every member of every shown row can be opened, and nothing else** (`AppState::shown`); the window never picks the
  chart for the clerk. Single-chart rows are byte-identical to before R3; people-vs-charts wording lives in
  `funnel/rows.rs`. The row label reaches a screen reader via `aria-describedby` — unpinned (#332).

**⇒ R1b'S DURABLE RULES (#697 (b), #701, PR #717):**
- **A doubted set withholds every line not recorded ONLY on the opened chart** (`medication::hazard::wrong_chart_reasons`,
  pure; an EMPTY chart list is not "only on the opened chart"); the opened chart's own lines stay signable and every
  line stays on screen. Pins: `hazard.rs`, `tests/doubted_link_withholds.rs` (its mirror test, opened from the OTHER
  chart, is the only one proving `opened` is used).
- **`cross_patient` is KEPT, set from the same reasons map as `wrong_chart`**; readers go through
  `is_wrong_chart_hazard()` or `hazard_reasons()`, never either field alone (#720). **`hazard_reasons()` is
  STATUS-BLIND; `withheld_reasons()` / `withheld_rows()` say what THIS gesture withholds** — anything saying a line
  "cannot be signed until …" asks them.
- **The rule judges every chart a group touches, INCLUDING each member thread's own chart** (`read.rs`
  `touched_charts`); a row missing from the reasons map is a hazard (`row_reasons`).
- **"Doubted" has three cases** — a `link_veto_flag` row; an UN-attested standing link tripping the hard veto now
  (#220's path); an ATTESTED unlink between two charts still in the set (the A–C–X bridge) — spelled once, since R6, in
  `cairn_chart_set_doubts`. It reads the STORED `pl.attested` (#701), never re-derives it, and takes one record's
  charts (`person_charts`). **An UN-attested unlink inside the set is deliberately NOT a doubt** (any unreviewed writer
  could freeze sign-off).
- **The remedy is a judgement of the LINKS, never thread separation, and never promises "either judgement lifts this
  hold"** (`DOUBTED_LINK_INSTRUCTION`); each sentence must stay true in every case shown. The CLI prints it ONCE below
  the list (`list_text::doubted_link_note`); the outside-set row lines stay byte-identical.

**⇒ THE LINK PRECEDENCE FLOOR'S DURABLE RULES (R2a, ADR-0076 decision 5):**
- **`patient_link`'s winner order is ATTESTED FIRST, then HLC** (`cairn_link_overlay_wins`, db/018). "Attested" is ONE
  definition (`attester_key IS NOT NULL AND cairn_attestation_vouched(event_id)`), evaluated once per applied event and
  STORED with the winner; a new reader reads `pl.attested`, never a second spelling.
- **`db/055`'s EXISTENCE is load-bearing** (generation 55; the loader's heal re-decides winners the old order chose).
  **Never fold it into db/018, and never delete its re-fold block** — cairn-sync heals with the OLD applier (#703). A
  peer on an older binary ranks the old way (identity.md §5.2).
- **Recorded is not took effect**: every judgement path reads back, in its own transaction, what now stands
  (`chart_link::standing_link`, `LinkEffect`); auto-apply rolls back a matcher link that is not itself the winner.
- **One lock order: the `match_proposal` row, then db/018's CARNLK** (reversed, a same-pair race deadlocks, 40P01;
  pinned in `tests/chart_link.rs`).
- **A judgement is a human's** (`Reviewer`: the human key signs AND attests; no node-key fallback). `link` needs BOTH
  charts held; `unlink` also admits a displayed member not held here, filed under the HELD chart (db/005 step 8b).
  Only OPEN proposals move.

**⇒ R2b-1'S DURABLE RULES ("Same person as…", PR #707)** (pins: `chart_compare.rs`, `link/same_person_tests.rs`,
`link/{view_tests,judgement_view_tests}.rs`, `chart_link.rs`):
- **The comparison is SET against SET** (`cross_vetoes` over every left×right pair, or a B–C clash goes unseen).
  **Link names BOTH COMPARED sets**, sent back verbatim; a changed set is `THIS_CHANGED` / `OTHER_CHANGED`.
- **Every panel message goes through `setMessage`, never a bare `.textContent`** (an empty status line is `hidden`).
  No JS harness pins it (#332): a headless walk asserts `getComputedStyle(el).display !== "none"`.
- **A partially-read comparison offers no Link. Absence and precision wording lives in `link/view.rs`** (an absent fact
  on a chart not held here reads "unknown — registration not yet received here"; struck names are listed; a coarse DOB
  names its precision). **An identifier finding is never "verified"**; only a dob/sex-at-birth hard veto (both
  provenance-rank ≥ 60) is "Verified facts differ". **A locked key is not a verdict** (`Retry::Now`).
- **Code selects by a typed field, never a display label** (`MedListRowView::current`); the panel's search counts what
  it shows. **`chart_link`'s pre-check refusals are marked verdicts — never a bare `anyhow::bail!`** (same chart →
  `RefusalScope::Input`; not held or a non-human key → `NodeState`, #702).

**⇒ R2b-2'S DURABLE RULES ("Not the same person…", ADR-0077, PR #711)** (pins: `admit_tests.rs`,
`tests/{unlink_from_record,chart_link,record_edges}.rs`, `unlink_view_tests.rs`):
- **`FiledUnder::RecordOf` is UNLINK-ONLY, re-checked UNDER CARNLK in `assert_link_in_tx`** (after the proposal-row
  lock), then `person_charts(opened)` is re-read; it lives in the signing core because `assert_link_in_tx` is `pub`
  (`a_peer_unlink_landing_mid_judgement_is_seen_before_anything_is_signed`).
- **"Still joined?" asks the SUBJECTS** (`high ∈ person_charts(low)`), never the filed-under or open chart; the record
  REPORTED is the open chart's. **An opened chart / `--from` unrelated to the pair is REFUSED.**
- **The list is per LINK, never per member**; `record_edges` keeps `state = 'link'`. **The unlink panel is its OWN
  `<section>`, mutually exclusive with the link panel**; after an unlink focus goes to the patient heading.
  **`Outranked` never says a retry "changes nothing"** (the merge is bounded at 24 h of drift). A locked key names its
  own button; `standing_edge`'s unread list is `Retry::Now`.
- **An open chart's refusal scope follows the FACT** (`AdmitRefusal`): not held → `NodeState`; held but its record
  lacks the pair → `Stale` since #797 (routed by type, see that block; it was `Input` and said "reload the chart").
- **The webview-field guards scan `main.js`, `funnel.js` and `unlink.js`**; `link.js` has only its search read guarded
  (the rest is #715). `renderLinks` stays in `main.js`. Never say "by the matcher" for an un-attested link ("without a
  clinician's confirmation on record here").

**⇒ THE COMBINED READ'S DURABLE RULES (R1, ADR-0076):**
- **A combined list's duplicate flag is db/054's `cairn_medication_duplicate_groups` over the SET**, never the
  per-patient `patient_medication_reconciliation_flag` (one drug on two LINKED charts would go unflagged — a
  double-dose reading hazard). Its dup_key stays db/033's (`medication_dup_key_drift.rs`).
- **The read selects groups by MEMBERSHIP over the set** (filtering by `patient_id` is #334 returning);
  `list_chart_set_medications` is PRIVATE. **A cease on a wrong-chart-hazard line stops only the opened chart's
  threads** and names the rest (`chart_set::cease_plan`).
- **`MedicationRow::display_chart`** (serialized `patient_id`) is where the group DISPLAYS, never a write target — use
  `source_charts` / `MemberVouch::patient_id`. **Sign-off and cease act on each thread's OWN chart** (the strict door
  refuses a misfiled attestation since #689's fix).
- **A linked chart not held here reads trust `unknown`** (`person::trust_of`). **A missing group** uses
  `MISSING_GROUP_INSTRUCTION`, never the separation remedy.
- **EVERY CHART COMMAND NAMES THE DISPLAYED CHART SET** (decision 3): `displayed_patient` FIRST, then the set
  (`chart_set.rs`); `sign_off_medication_list(…, displayed: Option<&ChartSet>)` refuses a changed set; `None` is the
  CLI. A failed member-identity read keeps the list and warns (`ChartPane::members_error`).
- **A never-linked chart gains no header, label or line** (golden
  `combined_read.rs::a_never_linked_chart_reads_exactly_as_before`). Sign-off over a non-empty
  `groups_missing_from_chart` has no DB coverage (same class as #333).

**⇒ THE FUNNEL'S DURABLE RULES — do not undo any of these** (full text: the funnel design page's dated notes):
- **`search_patients` RANKS BY SEVEN KEYS** (`rank_candidates`, inputs in `patient/search_rank.rs`): passes →
  identifier → a §5.4 callsign typed WHOLE → name tokens (exact or a ≥3-byte prefix, as `db/046`; callsigns never
  split) → DOB near-miss → EXACT tokens → chart age. It only REORDERS; each key has a victim if dropped
  (`patient_search_ranking.rs`). Tokens count over the RETAINED names (`patient_name`, #349).
- **`incomplete` is the SEARCH's partiality only; truncation is `withheld`** (ADR-0075; folding them re-creates #671;
  `attestation_through_the_port.rs`). **Never raise `PROMPT_CAP` to make a number look better.**
- **The raw typed name travels WITH its token** (`FunnelSession`); an older revision's search is DROPPED; the
  webview forgets its token on every edit. **`require_provisioned` runs in the register command, BEFORE `take`.**
- **Only a chart some list on screen showed can be opened** (`AppState::shown`; since R3 every member of a row).
  **EVERY CHART COMMAND NAMES THE CHART ON SCREEN** (`AppState::displayed_patient`; 2c's Critical — a sign-off once
  signed patient B on a review of A's list): each has a `*_impl` pinned by a not-on-screen test; `open_patient` is
  private to `funnel` — **never let a new chart command resolve `open_patient()` alone.** A registration never
  writes, or switches charts, behind an open chart (`register_impl` + `open_after_registering`).
- **A click within 800 ms of a prompt landing is "show me", never "register"** (`PROMPT_READ_GUARD_MS`, SOFT POLICY
  in `funnel.js`, #677). **The standing probe matches all four `ActorStanding` arms** (`provisioning_sentence` → `standing_sentence`) (a `Retired` key meets db/004's
  resurrection refusal, #152), paired with the key it probed (#670).
- **Every sentence and its retry advice lives in `funnel/view.rs`** (`Retry::{Now, AfterOperator, Never}`; person-row
  wording in `funnel/rows.rs`). A refusal (`P0001`, `DeliberateRefusal`) is not an outage; the remainder is #655.
- **`TokenStore::settle` ends a `take`**; a success INVALIDATES; `discard` does NOT clear `in_flight`. **A dropped
  `register` future still latches the store (#669) and `register` is cancellation-unsafe (#649): never race it
  against a timeout or `select!`.** The step-3 trigger is advisory, never a gate.
- **Nothing provisions an actor on a write path** (`enrolment_is_never_a_write_side_effect.rs` — **when it goes red,
  do not add your call site to `ALLOWED`**). `resolve_matcher_actor` still enrols (#663); a SUPERSEDED key's class is
  undecided (#666, then #664). **`init` must not `?` its enrolment.**
- **`cairn-gui-live` holds the DB-backed ports.** The P0001 rule has three homes (#652); the `gui` job's step-level
  `CAIRN_ALLOW_DB_SKIP=1` is load-bearing and its deletion invisible (#656); derived truncate lists miss identity
  tables (#658). **`--mock` holds ONE `MockData`** (half of #668); its matching is NOT db/046's and it has no link
  concept — every mock row is a person of one (#722).

## The eighteen traps

**Eighteen traps. Each is a step a next session takes in good faith.** The full argument for each is in its ADR,
its test's header or ROADMAP; this keeps the rule, the pin and the tempting wrong fix.

1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY** (`keystore::adopt_derived_unwrap_secret`); anywhere
   else re-creates the #495 coupling. `unwrap_secret_is_not_derived.rs` sweeps every shipping tree and asserts each
   allow-list entry is live: **when it fails, delete the entry; never add one.** It and `is_a_test_gate_attribute`
   (pgrx's `#[cfg(any(test, feature = "pg_test"))]`) move **together**.
2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6): a database
   recreated under an existing key file needs `cairn-node establish-unwrap-key` before its first sealed write. Never
   make a red fixture green by weakening `ensure_unwrap_key`.
3. **`cairn-sync`'s ONE derived fallback** (no `<key>.unwrap` AND the derived key equals the registered one → start,
   warn every startup). An absent file may fall back; a present-but-unusable one never may — **never merge those two
   arms.** Retiring the fallback is **#514**.
4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It registers a secret
   from the NEW seed and the singleton registrar then refuses the real key for good; recover the export, or restore
   again into a fresh database.
5. **⇒ `Secret32` DOES NOT SEPARATE ONE SECRET ROLE FROM ANOTHER (#511)** — `Secret32::from_bytes(sk.to_bytes())`
   compiles. `secret32_conversions_are_named.rs` pins exactly **two** seed→unwrap-secret sites (the ADR-0066
   migration, trap 3's fallback); a third is the defect returning. `unwrap_secret_is_the_signing_seed` and
   `secret_opens_the_carried_custody` are not made redundant by the types.
6. **`init` refuses a database that already has custody registered** (it reads `node_unwrap_key` first) and names
   `establish-unwrap-key` as the remedy — on a restored node read trap 4 first.
7. **⇒ A BODY SHREDDED AFTER A CAPTURE KEEPS ITS DEK ON THAT MEDIUM. NOT A LEAK — DO NOT "FIX" IT.** A backup
   restores the state at capture time; invalidating old backups is rotation policy (principle 9; ADR-0067 decision
   2). **Never filter old segments** (`medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`).
   Operator half owed: **#589**.
8. **⇒ A PEN ROW WHOSE DEK "BELONGS TO ANOTHER NODE" IS STILL RETAINED (#578).** *"Did not open with the key we
   have now"* is not *"not ours"* (the right `<key>.unwrap` may be on a USB stick); the escape is db/021's human
   `acked`. Pinned by `requeue_releases_custody.rs::a_penned_dek_from_another_node_is_kept_until_a_human_decides_otherwise`
   and `requeue_retains_unlanded_custody.rs`. **Look-alike fix:** keying the guard on the opened `dek`, not the
   row's `dek_wrapped`, passes the headline test and skips the check whenever the key did not open. Keep the Rust
   check although the floor also catches it — it says WHY.
9. **RETIRED (#584, ADR-0070); live residue: never remove or move the `cairn_project_late_custody` calls** — in both
   doors AFTER the substitution guard, in db/020 BEFORE the `cairn.remote_apply` clear (wrapping db/005's call in
   `cairn.remote_apply = 'on'` turns a strict refusal into a flag). Pinned in `late_custody_reaches_the_chart.rs`,
   each placement with its own positive control. (#597's notice is still misleading.)
10. **⇒ WHEN `late_custody_guards.rs` FIRES, THE GUARD IS RIGHT.** Every `event_clear` writer (`apply_remote_event`,
    `submit_event`) calls `cairn_project_late_custody`; every applier reading custody is `heal_safe = TRUE`. **Wrong
    fixes:** a custody reader `heal_safe = false` (a silent owed rebuild — **#610**), flipping a non-idempotent one
    to `TRUE`, exempting a writer. A third writer is a DECISION; a custody read in a helper, `MERGE` or dynamic
    `EXECUTE` is invisible to the guard — review by hand.
11. **⇒ `restore` EXITS 3 FOR AN INCOMPLETE RECOVERY; A TEST EXPECTING 1 IS THE BUG (#594, ADR-0071).** Exit 1 =
    BLOCKED, checked FIRST. **Wrong fixes:** `success()` for a torn medium; a wrong recovery code exiting 1 (its end
    state is the no-registry state, 3); "simplifying" `past_chain_break` to the medium's clinical total (M8). The
    precedence's ONE test is `restore_cli_surface.rs::without_the_flag_a_piped_restore_still_inherits_no_custody`
    — never weaken it to `!success()`; `restore_exit_vocabulary.rs` pins the value,
    `exit_incomplete_matches_cairn_nodes_restore` the agreement. ⚠️ **Write `Some(n)`, never `!status.success()`.**
    ⚠️ An ACKED pen row counts in `penned` and `requeue` will not clear it; do not subtract it. Residuals **#611**,
    **#616**.
12. **⇒ THE SUBSTITUTION REFUSAL HAS ONE HOME (`cairn_refuse_substitution`, db/053), AND A DOOR CALLS IT.** `IS
    DISTINCT FROM` ("cannot tell" is a refusal); a fourth inline copy is #608's `<>` fail-open returning
    (`substitution_guard_is_single_source.rs`). db/005/db/020 read under `GET DIAGNOSTICS`; **db/009 reads
    unconditionally — do NOT "tidy" it into a `ROW_COUNT` check**, its guard AFTER the `IF/ELSE` (M7); a db/009
    refusal aborts the WHOLE restore, correctly. A DEFERRED clinical record is REPORTED, never an exit-3 cause. All
    five event-log doors: `substitution_guard_covers_every_writer.rs` over `pg_proc`. Residuals #608, #605, #569, #622.
13. **⇒ THE NODE PLANE'S SUBSTITUTION REFUSAL LIVES IN EACH DOOR'S TAIL; THE PULLER ASKS THE TABLE, NOT THE ERROR
    (#619, ADR-0073).** An early `RETURN` bypasses the guard (the genesis arm, safe only for want of an `ON
    CONFLICT`) and the catalogue guard will NOT notice a new one. **Wrong fixes:** a dedicated SQLSTATE (P0001 is a
    contract, #228); matching the sentence; penning only when the GUARD raised
    (`a_rival_refused_by_an_earlier_check_is_still_penned`); a skip for the failed-lookup freeze
    (`node_substitution_lookup_freezes.rs`, M10); `offered` over the whole frame (#268's alarm fatigue, M11). A
    sixth writer gets the call. Residuals #620, #622, #605.
14. **⇒ A DOOR THAT LETS POSTGRES RAISE ON CALLER-SUPPLIED BYTES BREAKS THE P0001 CONTRACT (#621, ADR-0074).** Every
    signed-bytes field goes through a P0001 helper (`cairn_uuid_or_raise` on `pg_input_is_valid`,
    `cairn_hlc_nonneg_or_raise`, `cairn_node_role_or_raise`); `node_door_input_guards.rs` fails on a bare `::uuid`.
    **Wrong moves:** a REGEX narrower than the cast (M9); re-inlining the role list into the CHECK (M10); `USING
    ERRCODE`; deleting the CHECKs (the raw-SQL floor). ⚠️ **The puller's default for an UNKNOWN SQLSTATE is pen;
    "tightening" it to freeze reinstates #621.** The local classes (`08 40 42 53 55 57 58`, no SQLSTATE) equal
    `cairn-sync`'s (`sqlstate_classes_agree.rs`; merging the copies is #626); **`XX001`/`XX002` are LOCAL**. Such a
    pen row leaves only by applying or an ack. **The role CHECK stays `NOT VALID`** (a validating pair re-scans on
    every connect). Known exception: `cairn_body`'s `22P05` on a NUL (**#628**). Residuals #626, #629, #605, #268,
    #625, #631, #632, #633, #634.
15. **⇒ THREE THINGS IN `db/046`'s PASS 3 LOOK LIKE NOISE AND ARE EACH WORTH HUNDREDS OF MS ON A PI (#639).** (a)
    **`OFFSET 0` is an optimisation fence** — removing it changes no result, so nothing fails. (b) **The lateral is
    `UNION ALL` on purpose.** (c) **The parts-branch skip must test the same `lower(normalize(pn.value, NFC) COLLATE
    pg_unicode_fast)` it splits** — U+0130 `İ` lowercases to `i` + U+0307; dropping `lower` LOSES A TOKEN.
    `the_subset_argument_holds_for_every_unicode_code_point` checks all 1,114,111 code points;
    `the_subset_probe_still_describes_the_query_db046_runs` pins the composed expression. **A probe list is a sample,
    not an argument; re-measure with `scripts/measure_patient_search.py`, do not reason.** Residuals #641, #640,
    #643, #637.
16. **⇒ A GUARD AND THE THING IT GUARDS MUST BE ASKED ABOUT THE SAME STRING** — pin the COMPOSED expression as a
    literal (`include_str!` + `contains`), not its pieces.
17. **⇒ EVERY NAME `lower()` NAMES ITS COLLATION: `COLLATE pg_unicode_fast` (#775).** A bare `lower()` follows the
    cluster (ICU locally: full mapping; CI's libc `C.UTF-8`: simple — `İ`→`i`, `Σ`→`σ`), so a stored token and a
    typed one could case-map differently and a chart went unfindable. The COLLATE also moves the regex classes
    (`\s` = Unicode White_Space: NBSP now splits). Search side and sweep side (matcher `blocking_sql.py`) must move
    TOGETHER — the db/046 DRIFT invariant (sweep-paired ⊆ search-found) — and a matcher-side change bumps
    `__version__` (blocking SQL is not in the digest). Pins: `db046_names_the_collation_at_exactly_its_four_lowercasing_sites`,
    the drift fixtures, `test_eval_generator_sync.py`. A libc test DB is still the way to reproduce CI for anything
    else locale-shaped: `CREATE DATABASE cairn_test_libc TEMPLATE template0 LOCALE_PROVIDER libc LOCALE
    'en_US.UTF-8' ENCODING UTF8;` + `CREATE EXTENSION cairn_pgx;`.
18. **⇒ A TARGETED `cargo test --test X` REPORTING `ok` IS NOT PROOF — ONLY THE SWEEP IS (#661).** A stale test
    binary (shared `target/`, a running rust-analyzer) reported `ok` twice against source it contradicts — silent
    and green. **Gate on the sweep**; `CARGO_TARGET_DIR=/tmp/…` when an IDE is open; re-run every CI gate in CI's
    order AFTER the final edit.
