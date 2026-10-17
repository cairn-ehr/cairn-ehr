# Durable rules — what a tidy-up must not undo

Moved here from `docs/HANDOVER.md` on 2026-10-15 (R7) so the handover stays a short, current
scaffold. These are long-lived: each rule is a step a later session could undo in good faith, with
its pin (the test that fails if it is undone) and, where one exists, the tempting wrong fix. The
full argument for each is in its ADR, its test's header, the design page
(`docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`) or ROADMAP.
**Read the section for any code you are about to touch.** When a slice adds rules, add its block
here, newest first, and leave a one-line pointer in HANDOVER.

## Per-slice rules (the repair path, the funnel)

**⇒ #811 — A RE-READ AFTER AN INTERRUPTED COMMIT SAYS ONLY WHAT IT KNOWS — do not undo** (pins: `funnel/view_tests.rs`'s
wire test, `judgement_view_tests.rs`, `duplicates/verdict_tests.rs`, and the source guards in
`reread_webview_tests.rs` / `read_again_webview_tests.rs`):
- **`Retry::AfterReread(Recorded)`: `No` = a stale refusal, nothing recorded; `Unknown` = an interrupted commit
  (#713), it may have been.** It words only the out-of-date notice (`LIST_OUT_OF_DATE` vs `LIST_MAY_BE_OUT_OF_DATE`).
  **Tempting wrong fix:** collapsing it back to a unit variant because the window "does the same" for both. It does
  the same, but must not SAY the same.
- **`No` wins** (`Recorded` is ordered `Unknown < No`; `combined_retry`, `behind_the_record`, and `behindRecorded`
  in `chart_stale.js`, which never goes from `"no"` back to `"unknown"`): one stale refusal PROVES the list is out of
  date. **Tempting wrong fix:** letting `Unknown` win "because it is worse". The banner then says "whether your last
  act was recorded is not known" right after a Stop whose own line said "nothing was done".
- **"A change the re-read names may be your own" is Rust's, in the interrupted act's OWN sentence, for Link and
  Unlink only** (`judgement_error_view(act, Charts::MayMove, e)`). A confirmation and a "different people" judgement
  are `Charts::Kept`: they never move membership, so a change named after one is certainly another source's.
  **Tempting wrong fix:** a JS sentence keyed on `"unknown"`. It cannot tell the acts apart, so it excuses a
  colleague's link as the clinician's own.
- **`forgetBehind` is called only by `drewRead` and `holdCharts`** (counted). One more call in an own-act path would
  end the stale state silently.
- **Any re-read that names a change says `READ_AGAIN` ("review it before you act on it")**, including after the
  clinician's own successful act (M5). A colleague's change is news to review either way.

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
- **`markListStale` keeps the rows and disables sign-off** (`#chart-stale` says `LIST_OUT_OF_DATE`). **Tempting wrong
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
  in `funnel.js`, #677). **The launch probe matches all four `ActorStanding` arms** (a `Retired` key meets db/004's
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
