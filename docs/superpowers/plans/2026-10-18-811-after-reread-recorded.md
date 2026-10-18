# #811 — a re-read after an interrupted commit says only what it knows

**Goal:** `Retry::AfterReread` covers two cases that the window handles the same way but must word differently:
the record changed under the screen (**nothing was recorded**), or a judgement's commit was interrupted (#713, it
**may have been**). The type says which, and the window's two sentences that were true only of the first case become
true of both. The maintainer chose option (b) of #811 on 2026-10-18.

> [!NOTE]
> **As built.** The Architecture below is what shipped, after two review rounds on the PR. The first draft had
> `Unknown` win over `No` and a JS `MAY_BE_YOURS` sentence; both were reversed, for the reasons given inline.

**Architecture:**
- `Retry::AfterReread(Recorded)`, where `Recorded::{Unknown, No}` (`funnel/view.rs`). On the wire it is
  `{"after_reread": "no" | "unknown"}`. `Recorded` is ordered `Unknown < No`, so the value for several is `max` and
  `No` wins: one stale refusal PROVES the list is out of date, which outranks "may be".
- Producers: `judgement_view::stale` and `DataError::Stale` → `No`; `judgement_error_view`'s commit-unknown arm →
  `Unknown`. `combined_retry` ranks `No` above `Unknown`. `LinkReportView::behind_the_record` becomes
  `Option<Recorded>` (`No` if any failed pair's was), not `bool`.
- **M4.** "A change the re-read names may be your own {act}" is Rust's, in the interrupted act's own sentence, for
  Link and Unlink only (`judgement_error_view`'s `Charts::MayMove`). A confirmation (`confirm_error_view`) and a
  "different people" judgement are `Charts::Kept`: they never move the chart set, so a change named after one is
  certainly another source's. A JS sentence keyed on `"unknown"` could not tell the acts apart.
- Webview (`chart_stale.js`): `behindTheRecord(failure)` returns the `Recorded` value or `null`, and stays the one
  reader of `.after_reread`. `knowListBehind(recorded, lead)` keeps, beside `behindAfter`:
  - `staleAfter`, the revision of the newest stale refusal. It is cleared when a list read after it draws, even
    while an interrupted commit is still behind the list. Only it words the notice "is out of date".
  - `unknownLeads`, each interrupted act's sentence. The read that ends the state says them (`withOwed`), so the
    M4 sentence survives when the act's own re-read is overtaken.
  - **The banner.** Without an open stale refusal, `markListStale` says `LIST_MAY_BE_OUT_OF_DATE`, not "the record
    changed", and the sign-off button says "may be" too.
  - **M5.** Any re-read that names a change also says `READ_AGAIN` ("review it before you act on it"), including
    after a successful own act, and only once per line.

**Tests (TDD):** serialisation of both values; the producers' values; `combined_retry` and `behind_the_record` over
a mix; JS source guards in `reread_webview_tests.rs` (the cause flows through, the new sentences quote the
control's label).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** you hand an order to the ward clerk and the phone rings. Afterwards neither of you is sure
  whether it went into the chart, so you look at the chart. You do not assume a colleague wrote what you find there.
- **Steps:** paper **N = 1** (look at the chart). Architecture-forced **M = 1**: the automatic re-read, then reading
  its line. UI bundling target **K = 1**. No step is added; only the wording changes. M ≤ N.
- **Time + cognitive load:**
  - Budget: as for #797, the re-read lands within the single-chart open budget.
  - Load drops: the line no longer implies a colleague's act that may be the clinician's own.
  - The measurement belongs to RUNBOOK §14's human pass. It cannot be measured headlessly.
