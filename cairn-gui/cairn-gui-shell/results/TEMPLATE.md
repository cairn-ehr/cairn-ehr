# Cairn GUI shell — Spike 0004 (widened) results — <operator>/<hardware>/<date>

- Host / renderer (wgpu | tiny-skia) / fonts / screen reader:

| Claim | Threshold | Result | Evidence |
|---|---|---|---|
| A1 shell a11y tree correct | chrome (panes/tabs/divider) + tab fields all have role+label; matches live AT | PASS/FAIL | `--dump-a11y` + Accerciser |
| A2 keyboard-only shell | traverse rail/tab-strips/divider/fields + open cross-ref, no pointer | PASS/FAIL | screen-reader transcript |
| I1 shaping | Latin/Arabic/Devanagari/Han no tofu | PASS/SKIP/FAIL | shaping test |
| I2 RTL/bidi + I3 CJK IME | Arabic RTL caret; Han composed via IME | PASS/FAIL | screenshots |
| L2 Pi latency | p95 keystroke→paint ≤ 100 ms | PASS/FAIL (p95=__ms) | latency log |
