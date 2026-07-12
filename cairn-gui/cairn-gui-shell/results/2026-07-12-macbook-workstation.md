# Cairn GUI shell — Spike 0004 (widened) results — Horst / MacBook (macOS) / 2026-07-12

Procedure: [RUNBOOK.md](RUNBOOK.md). This is a **partial workstation run** — no screen reader (Accessibility
Inspector/VoiceOver) yet, no Pi, no IME. It records what a live macOS run of the read-only slice-1 shell showed.

- **Host:** MacBook, macOS (Darwin 25.5), Apple Silicon. iced 0.14.
- **Renderer:** default (wgpu/Metal) — **not** the tiny-skia software path (L2 not exercised).
- **Fonts:** macOS system (ships Arabic / Devanagari / CJK).
- **Screen reader:** none run yet.
- **Evidence:** live window screenshot (in session), `--dump-a11y` output.

| Claim | Threshold | Result | Evidence |
|---|---|---|---|
| A1 shell a11y tree correct | chrome + tab fields all have role+label; matches live AT | **PARTIAL** | `--dump-a11y` expected tree is correct; **live AT tree NOT yet inspected**; render diverges from the dump (below) |
| A2 keyboard-only shell | traverse + open cross-ref, no pointer | **FAIL (current build)** | Tab does nothing — no focus traversal. **Cause: our shell never wires `Tab → focus_next` (iced requires it explicitly); likely our gap, not an iced limit — see follow-up** |
| I1 display shaping | fixture name renders all 4 scripts, no tofu | **PASS** | screenshot: `Amina أمينة अमीना 阿明娜` renders Latin+Arabic+Devanagari+Han, **zero tofu**, in both the identity band and Name field |
| L2 responsiveness (subjective) | divider/tab feels within floor on Pi | **N/A (not on Pi)** | Mac feels instant, but that is not the paper-parity Pi software-render floor |
| I2 / I3 / L2 (instrumented) | RTL caret · CJK IME · keystroke→paint | **N/A on shell** — run `poc/iced-ui-spike` | shell has no editable field |

## Functional checks (not formal spike claims, but load-bearing)

- **Cross-pane routing — PASS (live).** Clicking the note's "Chest X-ray 2026-07-01 …" button opens Demographics
  in the **opposite** pane while the note pane stays put. The headline cross-reference feature works end to end.
- **Divider — works functionally, invisible visually.** Dragging in the gap reapportions the two columns
  (`pane_grid` resize is live), but nothing is *drawn* for the divider, so it doesn't read as a splittable
  workspace. (See A1 render divergences.)

## A1/A2 render divergences observed (the point of the spike — recorded honestly)

The live render diverges from the `--dump-a11y` expected tree in exactly the ways the runbook predicted:

1. **Tab strip renders as plain buttons**, not tabs — dump declares `Role::Tab`, iced draws a `button`.
2. **Divider has no drawn affordance and (almost certainly) no accessible label** — `pane_grid` exposes no
   label; the dump's `chrome.divider` "Resize panes" is aspirational until we draw+label it.
3. **Panes have no accessible container labels** — dump's `chrome.pane.left/right` are targets, not what iced
   emits today.
4. **No visual chrome at all** — no titlebar styling, no safety-zone cards, no borders. This is the
   walking-skeleton render; styling is a deferred slice.

## Verdict so far

- **I (text): looking good.** The hard complex-script bar (I1) passes cleanly on the real product surface. I2/I3
  (editable RTL/IME) still to run on the `poc` harness.
- **A (accessibility): NOT yet decided — do not read the A2 FAIL as a framework veto.** Keyboard traversal is
  non-functional in *this build* because the shell never implemented `Tab → focus_next`. That must be wired
  before A can be judged. Only *after* wiring keyboard focus AND running Accessibility Inspector / VoiceOver do
  we learn whether iced exposes focus + roles/labels to the AT tree — which is the real tip-to-webview question.

## Follow-up actions (house rule #5)

1. **Wire keyboard focus** in `cairn-gui-shell` — a `keyboard::on_key_press` subscription mapping Tab /
   Shift-Tab to `iced::widget::focus_next` / `focus_previous`, so A2 can actually be tested. (Small change;
   blocks the whole A pass.)
2. **Run Accessibility Inspector + VoiceOver** on macOS (and Orca on Linux) once (1) lands — verify each focused
   control is announced with a correct role + label; record where the live tree still diverges from the dump.
3. **Draw + label the divider and tab strip** (styling slice) so the render matches the intended a11y tree
   rather than the dump chasing the render.
4. Run **I2/I3/instrumented-L2** on `poc/iced-ui-spike`; run **L2** on a Pi with the software renderer.
