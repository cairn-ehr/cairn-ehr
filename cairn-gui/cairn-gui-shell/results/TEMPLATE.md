# Cairn GUI shell — Spike 0004 (widened) results — <operator>/<hardware>/<date>

> Procedure: see [RUNBOOK.md](RUNBOOK.md). This template records only the passes the **read-only slice-1
> shell** can exercise. The editable passes (**I2 RTL caret, I3 CJK IME, instrumented L2 keystroke→paint**)
> run against `poc/iced-ui-spike` and are filed under *its* `results/` — they are **not** shell passes until
> the shell gains an editable field (note-editor slice).

- Host / renderer (wgpu | tiny-skia) / fonts / screen reader:

| Claim | Threshold | Result | Evidence |
|---|---|---|---|
| A1 shell a11y tree correct | chrome (identity/panes/tabs/divider) + tab fields all have role+label; matches live AT | PASS/FAIL | `--dump-a11y` + AT inspector |
| A2 keyboard-only shell | traverse identity/tab-strips/divider/fields + open cross-ref into other pane, no pointer | PASS/FAIL | screen-reader transcript |
| I1 display shaping | fixture name `Amina أمينة अमीना 阿明娜` renders all 4 scripts, no tofu | PASS/SKIP/FAIL | screenshot |
| L2 responsiveness (subjective) | divider drag / tab switch feels within paper-parity floor on Pi | PASS/FAIL/NOTE | note |
| I2 / I3 / L2 (instrumented) | RTL caret · CJK IME · keystroke→paint p95 ≤ 100 ms | **N/A on shell** — run `poc/iced-ui-spike` | see RUNBOOK §2 |

## A1/A2 divergences observed (the point of the spike — record honestly)

<where the live AT tree fell short of the --dump-a11y expected tree: tab-strip role, pane/divider labels,
keyboard reachability of the divider, focus order — with the specific failure and any issue filed>

## Notes / surprises / issues filed

<free text — link any GitHub issue filed per house rule #5 for a FAIL that can't be fixed here>
