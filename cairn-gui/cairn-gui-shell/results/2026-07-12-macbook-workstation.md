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

## UPDATE — root cause found: released iced 0.14 ships NO accessibility (AccessKit)

Before wiring keyboard focus, inspected the installed iced 0.14 source. Decisive findings:

- **Only `text_input` and `text_editor` implement the focusable operation** (`iced_widget-0.14.2/src/{text_input,text_editor}.rs`); **`button.rs` has no focus support at all.** The shell is entirely buttons + static text → `focus_next` has nothing to focus, which is why **Tab does nothing**. Not a missing-wiring bug we can fix by adding a subscription — the widgets themselves aren't focus participants.
- **There is no `accesskit` dependency anywhere in the compiled tree** — absent from `iced`, `iced_widget`, `iced_runtime`, `iced_winit`, and from `cairn-gui/Cargo.lock`. AccessKit is the crate that bridges a Rust GUI to NSAccessibility / AT-SPI2 / UIA. **Without it, no accessibility tree is emitted — a screen reader sees a generic blank window.**

**Conclusion: the crates.io iced 0.14 has essentially no accessibility support.** The AccessKit the eco-eval
called "partial in mainline" lives on iced's **git main / the plushie-iced fork**, not in the released crate.
So **A1 and A2 cannot be cleared on released iced 0.14 at all** — this is a *framework* limit, not a defect in
our slice. This is the make-or-break signal the spike existed to find, obtained from dependency facts before any
styling investment.

### Optional empirical confirmation
Run **Accessibility Inspector** (macOS) or Orca (Linux) against the running window — expect it to announce
essentially nothing (a blank/generic window). This confirms the dependency-level finding by observation.

## Decision this forces (for the steward)

Per the four-layer model, the wire core + `ClinicalData` port + manifest we built **survive whichever way this
goes** (*many front-ends, one record*) — the choice is only which framework renders the reference UI:

1. **Depend on iced `git main` / plushie-iced fork** — gets *partial* AccessKit, but rides unreleased,
   churning code (a supply-chain + stability cost on a decades horizon). Would need its own spike to see how
   partial.
2. **Tip the reference desktop UI to a webview/Tauri L3** — inherits the browser's mature a11y tree for free;
   the eco-eval's stated fallback if the A bar fails. The contract/port/manifest port over unchanged.
3. **Proceed on iced now, accept a11y debt, revisit when iced releases AccessKit** — only viable if the reference
   client is explicitly not-yet-for-a11y-critical deployment; risky to bank on an unreleased timeline for an EHR.

## Remaining passes (unchanged)

- Run **I2/I3/instrumented-L2** on `poc/iced-ui-spike` (editable fields). NOTE: I2/I3 shaping/IME may work, but
  those fields **also** won't be announced by a screen reader (same no-AccessKit reason) — so the poc harness
  measures *shaping/IME correctness*, not AT exposure.
- Run **L2** on a Pi with the tiny-skia software renderer.
