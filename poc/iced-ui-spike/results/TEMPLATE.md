# Spike 0004 results — <operator> / <hardware> / <date>

- **Host:** <CPU, RAM, OS, display server, iced version>
- **Renderer:** <wgpu | tiny-skia (software)>
- **Fonts:** <Noto/FreeSerif/WenQuanYi… — what covers Arabic / Devanagari / Han>
- **Screen reader:** <Orca x.y | NVDA x.y | none>

| Claim | Pass threshold | Result | Evidence |
|---|---|---|---|
| **A1** a11y tree exists + correct | every focusable control has role + label; matches live AT tree | PASS / FAIL | `--dump-a11y` output + Accerciser note |
| **A2** keyboard-only + screen reader | form completable with no pointer, every stop announced | PASS / FAIL | screen-reader transcript / notes |
| **I1** complex-script shaping | Latin/Arabic/Devanagari/Han shape, no tofu, ≥ floor | PASS / SKIP / FAIL | `cargo test --features shaping` output |
| **I2** RTL/bidi correct | Arabic RTL + caret correct | PASS / FAIL | screenshot |
| **I3** CJK IME | Han composed + committed correctly | PASS / FAIL | screenshot |
| **L2** Pi latency | p95 keystroke→paint ≤ 100 ms (record actual) | PASS / FAIL — p95 = __ ms | `[latency]` line |

## Notes / surprises / issues filed

<free text — and a link to any GitHub issue filed per house rule 5 for a finding that can't be fixed here>
