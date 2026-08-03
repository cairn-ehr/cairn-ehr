# Paper-parity measurement — YYYY-MM-DD, \<host>

> Copy to `YYYY-MM-DD-<host>.md`. Follow [`RUNBOOK.md`](RUNBOOK.md). Record what you
> measured, not what the budget hoped for.

## Rig

| | |
|---|---|
| Host / CPU / RAM | |
| OS + webview version | |
| Build profile | `--release` / `--debug` |
| PostgreSQL version | |
| Database size (`event_log` rows) | |
| Operator | |

## Chart under test

| | |
|---|---|
| Lines displayed | |
| Of which unsigned or stale | |
| Of which already signed by someone else | |
| Reconciled groups (rows with >1 thread) | |
| Groups missing from chart | should be 0 — see runbook §2 |

## Measured — whole gesture, by stopwatch

Excludes finding the patient (`--patient` at launch; §5.3/§5.8 funnel unbuilt).

| Gesture | n | median | p95 | Provisional budget | Inside? |
|---|---|---|---|---|---|
| Chart open → list rendered → unsigned lines signed | | | | ≤ 15 s | |
| Cease one drug | | | | ≤ 5 s | |

## Measured — write cost only, from `ui_gesture_timing`

| gesture_kind | size_bucket | n | p50_ms | p95_ms |
|---|---|---|---|---|
| | | | | |

## Step count against the paper counterpart (§1.2)

Paper counterpart: the inpatient drug chart.

| Act | Paper *N* | Architecture-forced *M* | Observed UI *K* |
|---|---|---|---|
| Review a 5-drug list, sign 3 unsigned/stale lines | 3 signatures | 1 | |
| Cease one drug | 2 (strike + initial/date) | 1 | |

`M > N` would be an architecture defect and must be filed (#217 rule). Record `K` as
observed, including the unlock if it was needed.

## Accessibility pass

Screen reader + version: \_\_\_\_ · Keyboard only: yes / no

| Check | Verdict | Notes |
|---|---|---|
| Line announces drug, dose, status and signatory in one utterance | | |
| Chart warnings announced before the table | | |
| Stop button names its drug | | |
| Reason field names its drug | | |
| Sign-off button announces the real thread count | | |
| Every control Tab-reachable, focus ring visible | | |
| "Will be signed" identifiable without colour | | |
| "Ceased" identifiable without colour | | |

## Verdict

- Observed p95 inside the provisional budget? **yes / no**
- If no: issue filed \_\_\_\_. **Do not adjust the budget to match the measurement.**
- Anything that surprised the operator:
