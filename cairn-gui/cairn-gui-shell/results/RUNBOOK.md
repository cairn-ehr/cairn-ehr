# Spike 0004 — operator runbook

**Purpose.** Retire the three EHR-specific bars that iced adoption for the reference UI is *conditional* on
([eco-eval 0004](../../../docs/ecosystem/0004-reference-ui-framework-iced-vs-tauri.md),
[Spike 0004](../../../docs/spikes/0004-iced-reference-ui-viability.md)): **(A) accessibility**,
**(I) complex international text + IME**, **(L) paper-parity input-to-paint latency**. CI already runs the
*pure* checks headlessly; this runbook is the **operator** procedure for the passes that need a display, a
screen reader, an IME, and a Pi.

> [!IMPORTANT]
> **Read this first — the spike spans TWO harnesses, on purpose.**
> The slice-1 `cairn-gui-shell` renders everything **read-only** (`text` + `button`); it has **no editable
> field yet** (the markdown note editor is a later slice). So it **cannot** exercise the passes that require
> *typing into a field*. Those run against the older, editable `poc/iced-ui-spike` harness, which has the four
> multi-script name fields, an `--latency` mode, and its own operator README. Run each pass against the
> harness that can actually exercise it:

| Claim | What it checks | Harness to use | Why |
|---|---|---|---|
| **A1** a11y tree exists + correct | shell chrome (panes / tab strips / divider / identity band) **and** tab fields all have role + label; matches the live AT tree | **`cairn-gui-shell`** | This is the *widened* claim — the real product's shell chrome, which the flat poc form has none of |
| **A2** keyboard-only + screen reader | traverse rail/tab-strips/divider/fields **and open a cross-reference into the other pane**, no pointer | **`cairn-gui-shell`** | Cross-pane routing + tab strips are shell-only |
| **I1** complex-script shaping | Latin / Arabic / Devanagari / Han render with **no tofu / `.notdef`** | **both** — headless proof in `poc` (`cargo test --features shaping`); **display** proof in the shell (the fixture patient name) | The shell proves it on the *real product surface*; the poc proves it headlessly in CI |
| **I2** RTL + bidi caret | Arabic renders RTL with correct caret/cursor **while editing** | **`poc/iced-ui-spike`** | Needs an **editable** field — the shell has none yet |
| **I3** CJK IME input | Han composed via a system IME **commits into a field** | **`poc/iced-ui-spike`** | Needs an **editable** field — the shell has none yet |
| **L2** Pi input-to-paint | `--latency` keystroke→paint p95 ≤ 100 ms on the tiny-skia software renderer | **`poc/iced-ui-spike`** | Needs keystrokes into a field **and** the instrumented `--latency` mode; the shell has neither yet |

> The shell does not yet retire **I2 / I3 / instrumented-L2** — that is honest, not a gap in this run. Re-run
> those against the shell once it gains an editable field (the note-editor slice). Until then the `poc` harness
> is their vehicle, and the shell can additionally be given a **subjective** responsiveness note on the Pi
> (below). A **FAIL** on any bar is *decision feedback* (tip the reference UI to a webview/Tauri L3), never a
> defect to paper over — file it as a GitHub issue (house rule #5).

---

## 0. Prerequisites

- **Fonts** covering all four scripts (or shaping SKIPs / renders tofu — never a silent pass):
  - Linux: `sudo apt install fonts-noto-core fonts-noto-cjk` (Noto Naskh Arabic, Noto Devanagari, Noto CJK).
  - macOS: Arabic/Devanagari/CJK ship with the OS.
  - Windows: install "Noto" family or ensure Segoe UI Historic + a CJK font are present.
- **Screen reader** (claim A):
  - Linux: **Orca** (`orca`), plus **[Accerciser](https://gitlab.gnome.org/GNOME/accerciser)** to inspect the
    AT-SPI2 tree.
  - Windows: **NVDA**.
  - macOS: **VoiceOver** (⌘F5) + **Accessibility Inspector** (ships with Xcode).
- **IME** for I3 (poc harness): ibus-pinyin / fcitx5 (Linux), the OS IME (macOS/Windows).
- **Pi-class hardware** for L2 (a Raspberry Pi 5, or the Bet-B Pi from Spike 0001) with a display.
- A checkout of this repo on each machine. iced's first build is large — allow several minutes.

---

## 1. Passes run against `cairn-gui-shell` (this crate)

Run from the `cairn-gui/` workspace root. The shell launches with the fixture patient
`Amina أمينة अमीना 阿明娜` (Latin + Arabic + Devanagari + Han in one name — that is the I1 display sample).

### A1 — shell accessibility tree exists and is correct

1. **Dump the expected tree** (exits before opening a window; works anywhere):
   ```bash
   cargo run -p cairn-gui-shell --features gui -- --dump-a11y
   ```
   Confirm it prints three sections — **Shell chrome**, **Current note**, **Demographics** — and that the
   chrome section lists the **identity band**, both **panes**, both **tab-strip tabs**, and the **divider**,
   each with a role and a non-empty label.
2. **Launch the shell** and inspect the *live* tree with your AT inspector (Accerciser / Accessibility
   Inspector):
   ```bash
   cargo run -p cairn-gui-shell --features gui
   ```
   Walk the live accessibility tree and confirm every focusable control the dump listed is present with a
   matching role + label. **Expected honest finding:** the dump declares the tab strip as `Role::Tab` but iced
   renders it as a plain button, and `pane_grid` may expose no accessible labels for the panes/divider —
   **record exactly where the live tree falls short of the dump.** That divergence *is* the measurement.
3. **PASS** = every focusable control is reachable and announced with a correct role + label. Record gaps.

### A2 — keyboard-only operation + screen reader

Start Orca / NVDA / VoiceOver. With **no pointer**:
1. Tab through the interface — identity band → left pane tab strip → note body (incl. the "Chest X-ray…"
   cross-reference button) → right pane tab strip → demographics fields → the divider.
2. Activate the **cross-reference button** (Enter/Space) and confirm the referenced view opens in the **other
   pane** while the note pane stays put, and the screen reader announces the change.
3. Switch the active tab in a pane by keyboard; attempt to move the **divider** by keyboard.
4. **PASS** = the whole shell is operable with no pointer and every stop is announced. **Likely FAIL points**
   to record honestly: iced 0.14's keyboard focus traversal and a keyboard-resizable `pane_grid` divider are
   historically weak — if the divider can't be moved by keyboard, or focus skips the tab strips, **write down
   exactly what failed**; that is the load-bearing result of this pass.

### I1 — complex-script shaping (display proof, on the real surface)

1. In the launched shell, read the identity band and the demographics **Name** field: the string
   `Amina أمينة अमीना 阿明娜` must render **all four scripts with no tofu (□) / `.notdef` boxes**.
2. Take a screenshot as evidence.
3. Also record the headless proof from the poc harness (§2, I1) so the claim has both a CI test and a
   product-surface screenshot.
4. **PASS** = all four scripts shaped, no tofu (given the fonts from §0; a missing font is a **SKIP**, never a
   silent pass).

### L2 (subjective, shell) — responsiveness on the Pi

The shell has no `--latency` instrumentation yet (no keystrokes to time), so on the Pi record a **subjective**
note only: launch the shell on the Pi, drag the divider and switch tabs, and note whether interaction→paint
feels within the paper-parity floor. The **instrumented** keystroke→paint number comes from the poc harness
(§2, L2). Add a `--latency` mode to the shell when it gains an editable field, then re-run this as a measured
pass.

---

## 2. Passes run against `poc/iced-ui-spike` (editable harness)

Run from `poc/iced-ui-spike/`. This harness has the editable identifier + four multi-script name fields and an
instrumented latency mode. Its own [README](../../../poc/iced-ui-spike/README.md) carries the detailed scripts;
the summary:

### I1 — shaping, headless (CI-grade proof)
```bash
cargo test --features shaping
```
Confirms Latin/Arabic/Devanagari/Han shape to > 0 clusters with no tofu over the same cosmic-text stack iced
uses (loud **SKIP** if no font, never a silent pass).

### I2 — RTL + bidi caret (editable)
```bash
cargo run --features gui
```
Type into the **Arabic** name field: confirm it renders **right-to-left** with correct caret/cursor behaviour.
Screenshot.

### I3 — CJK IME input (editable)
Clear the **Han** field; with a system IME (ibus-pinyin / fcitx5 / OS IME) compose and commit Han text; confirm
it commits correctly into the field. Screenshot.

### L2 — Pi input-to-paint latency (instrumented)
On the Pi:
```bash
cargo run --features gui -- --latency
```
Type steadily into a field; the harness logs keystroke→paint samples on the **tiny-skia software renderer**.
Record **p95** (target **≤ 100 ms**).

---

## 3. Recording results & exit criteria

1. **Copy the template per harness run** and fill it in:
   - shell passes → copy `cairn-gui/cairn-gui-shell/results/TEMPLATE.md` → `results/<operator>-<host>-<date>.md`.
   - poc passes → file under `poc/iced-ui-spike/results/` (its own `TEMPLATE.md`).
2. **Record actual numbers and honest gaps**, not just PASS/FAIL — especially the A1/A2 divergences (they are
   the point of the spike) and the L2 p95.
3. **File a GitHub issue for every FAIL** (house rule #5) — a FAIL is selection feedback, not a defect to hide.
4. **Exit criteria** ([Spike 0004 §5](../../../docs/spikes/0004-iced-reference-ui-viability.md)):
   - **A + I PASS** → iced clears the EHR bars; adopt it for the reference desktop UI (an L3 citizen). No ADR,
     no spec change — the layering already permits any L3 framework.
   - **A FAIL** → tip the reference desktop UI to a webview/Tauri L3. The contract, `ClinicalData` port, and
     manifest survive that swap unchanged (*many front-ends, one record*).
   - **I FAIL** → depend on the cosmic-text/plushie-iced fixes and re-run, or tip the international surface to a
     webview L3. Culture-neutrality ([ADR-0014](../../../docs/spec/decisions/0014-locale-pluggable-matcher-comparators.md))
     is non-negotiable — a real shaping/IME gap is a hard stop for *this* framework.
   - **L FAIL** → tuning task (renderer config, font caching) or a Pi hardware-floor finding, fed back to the
     Spike 0001 Bet-B thread — not a framework veto on its own.
