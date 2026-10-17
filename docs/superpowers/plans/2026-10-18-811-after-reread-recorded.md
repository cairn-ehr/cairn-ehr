# #811 — a re-read after an interrupted commit says only what it knows

**Goal:** `Retry::AfterReread` covers two cases that the window handles the same way but must word differently:
the record changed under the screen (**nothing was recorded**), or a judgement's commit was interrupted (#713, it
**may have been**). The type says which, and the window's two sentences that were true only of the first case become
true of both. The maintainer chose option (b) of #811 on 2026-10-18.

> [!NOTE]
> **As built, after the review on the PR:**
> - "May be your own" is said in Rust, in the interrupted act's own sentence (`judgement_error_view`'s
>   `Charts::MayMove`, Link and Unlink only). It is not a JS `MAY_BE_YOURS`: a confirmation or a "different
>   people" judgement never moves the chart set, so a change named after one is certainly another source's.
> - `Recorded` is ordered `Unknown < No`, so `No` wins. One stale refusal PROVES the list is out of date, and
>   the out-of-date notice is the only thing the value words.

**Architecture:**
- `Retry::AfterReread(Recorded)`, where `Recorded::{No, Unknown}` (`funnel/view.rs`). On the wire it is
  `{"after_reread": "no" | "unknown"}`. `Recorded` is ordered `No < Unknown`, so "the worst of several" is `max`.
- Producers: `judgement_view::stale` and `DataError::Stale` → `No`; `judgement_error_view`'s commit-unknown arm →
  `Unknown`. `combined_retry` ranks `Unknown` above `No`. `LinkReportView::behind_the_record` becomes
  `Option<Recorded>` (the worst failed pair's), not `bool`.
- Webview (`chart_stale.js`): `behindTheRecord(failure)` returns the `Recorded` value or `null`, and stays the one
  place that spells `after_reread`. `knowListBehind(recorded)` keeps the worst cause beside `behindAfter`.
  `drewRead` hands that cause to `sayAfterReread`.
  - **M4.** After the re-read of an `Unknown` act, a named change gets `MAY_BE_YOURS`
    ("That change may be your own, if it was recorded").
  - **The banner.** For `Unknown`, `markListStale` says `LIST_MAY_BE_OUT_OF_DATE`, not "the record changed".
  - **M5.** Any re-read that names a change now also says `READ_AGAIN` ("review it before you act on it"),
    including after a successful own act.

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
