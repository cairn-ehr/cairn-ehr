# Spike 0004 — iced as the reference-UI framework: viability against an EHR's hard bars

- **Status:** **RUN — verdict reached: iced FAILS the accessibility bar (A).** The widened spike ran
  against the real reference shell (`cairn-gui/cairn-gui-shell`, mock port) on 2026-07-12.
  **Bet A (accessibility) FAILED:** released **iced 0.14 ships no AccessKit / no accessibility tree** (only
  `text_input`/`text_editor` focusable; no `accesskit` in the compiled tree), empirically confirmed with macOS
  Accessibility Inspector (Cairn controls expose no accessible elements — menu-bar-only hierarchy). **Bet I1
  (complex-script shaping) PASSED** on the real surface (Latin/Arabic/Devanagari/Han, no tofu); I2/I3/L2 not
  reached (moot for the decision). Per the exit criteria, **A FAIL → the reference desktop UI tips to a
  webview/Tauri L3** — recorded in [eco-eval 0004 §6](../ecosystem/0004-reference-ui-framework-iced-vs-tauri.md).
  Results in `cairn-gui/cairn-gui-shell/results/2026-07-12-macbook-workstation.md`.
  Original editable-field harness kit at [`poc/iced-ui-spike/`](../../poc/iced-ui-spike/).

  > [!NOTE]
  > **2026-07-12:** A11y scope widened to shell-level (pane/tab-strip/divider traversal, not just a single form) and run against the **reference shell** (`cairn-gui/cairn-gui-shell`, mock port), per the approved GUI shell design. Headless a11y dump implemented via `--dump-a11y` flag (Task 9).
- **Date:** 2026-06-30
- **Motivation:** [Ecosystem eval 0004](../ecosystem/0004-reference-ui-framework-iced-vs-tauri.md)
  concluded that **iced** is the more mission-aligned L3 reference-UI framework
  ([ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)) **conditional** on
  retiring two EHR-specific risks. That conclusion is *reasoning*; this spike turns the load-bearing
  conditions into a *measurement*.
- **Validates:** that an iced L3 client clears the bars an EHR cannot waive — **(A) accessibility**
  (AccessKit exposes a correct, screen-reader-navigable tree for a dense clinical form),
  **(I) complex international text + IME** (Arabic/Devanagari/CJK shape correctly and CJK input works),
  and **(L) the paper-parity latency floor** ([§1.2](../spec/vision.md#12-the-paper-parity-test-normative))
  on the Pi-class software-render path — **without** the UI ever owning serialization/signing or
  touching the wire core ([§9.5](../spec/language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges)).
- **Does not ratify anything, and cannot.** An L3 framework choice is **below the compatibility
  boundary**; passing this spike adopts iced for the *steward's reference desktop UI* only, and changes
  no ADR and no spec ([§9.5](../spec/language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges)
  forbids UI tech from constraining the inter-node path). A **FAIL on (A)** tips the reference UI back to
  a webview/Tauri L3 — *not* a design defect, since L3 is plural.

> [!NOTE]
> Build-prep, not architecture. The numbered spec (§1–§11) and the ADR log describe a *decided* design.
> This spike exercises a **fit-for-purpose** ([§9.1](../spec/language-substrate.md#91-selection-rule-by-defect-blast-radius))
> choice with zero wire blast radius — the lowest-stakes spike to date. Its value is retiring two risks
> *before* a team invests in a framework, not gating the record.

---

## 1. Why this spike, and why now

The eval established *fit*; the two things that could still sink iced for an **EHR specifically** are not
"is iced pleasant" but the bars clinical software may not waive: **a blind clinician must be able to use
it**, and **a patient's name in any script must render and be editable**. Both are cheap to *demonstrate*
and expensive to discover late. The latency floor is folded in because the Pi software-render path is the
one place iced's "ships its own renderer" advantage could instead become a weakness on weak hardware.

| Bet | What stresses it | Character |
|---|---|---|
| **A — accessibility** | a dense clinical form (identifier entry + med list) driven entirely by keyboard + screen reader | **viability** (does the a11y tree exist and is it correct) |
| **I — international text + IME** | patient names in Latin / Arabic (RTL) / Devanagari (complex shaping) / Han (CJK + IME) | **viability** (shaping + input correctness) |
| **L — paper-parity latency** | input-to-paint on the tiny-skia **software** renderer (the Pi path) | **performance** (is it within the [§1.2](../spec/vision.md#12-the-paper-parity-test-normative) floor) |

A and I are **viability** bets like Spike 0002's design-validity bets — a FAIL is decision feedback (tip to
Tauri), not a defect. L is a **performance** bet like Spike 0001's Bet B — a slow result is a tuning task or
a hardware floor, fed back to the Pi-benchmark thread.

---

## 2. What this spike is *not*

- **Not the reference UI.** It is the smallest dense form that exercises the three bars — not a clinical
  product surface, no real workflow, no real patient data.
- **Not a live node integration.** The form authors into an **in-memory stub** of the native-API client, not
  a running `cairn-node`. The point is the *framework's* a11y/text/latency behaviour, not the
  [§9.6](../spec/language-substrate.md#96-the-validated-submit-surface-the-write-path) submit path (Spike 0002
  already exercised that floor). The stub deliberately mirrors the "UI never signs" rule so the harness can't
  drift into owning serialization.
- **Not a Tauri re-build.** The eval's webview comparison is reasoning; we do not build the same form twice.
  If **A** fails, the comparison is already made — the reference UI tips to a webview L3.
- **Not a screen-reader or Pi CI run.** CI has neither assistive tech nor a Pi. CI runs only the *pure* core
  (shaping + latency stats). The a11y-tree / live-IME / Pi passes are **operator-run**, scripted in the
  harness README, results filed under [`poc/iced-ui-spike/results/`](../../poc/iced-ui-spike/).

---

## 3. What gets built

A standalone crate [`poc/iced-ui-spike/`](../../poc/iced-ui-spike/) (outside the root workspace, so iced never
enters the node's dependency tree). Faithful to the [§9.1](../spec/language-substrate.md#91-selection-rule-by-defect-blast-radius)
blast-radius rule, the **pure, testable core has no GUI dependency**; iced sits behind a `gui` feature so
`cargo test` validates the core without compiling the toolkit.

1. **Pure core (library, no iced) — TDD'd, runs in CI:**
   - a **multi-script corpus** with per-sample expectations (script, directionality, expected shaped-cluster
     count > 0 / no-tofu), and a shaping check over **cosmic-text** (the crate iced uses), so "does iced's text
     stack shape Arabic/Devanagari/Han" is answered headlessly given a font;
   - **latency statistics** — pure percentile/summary functions (p50/p95/p99, frame budget) over a sample
     vector, so the live harness only has to *collect* timings, not compute them.
2. **iced GUI binary (`--features gui`) — operator-run:**
   - a dense clinical form: a patient-identifier field, a multi-row medication list, keyboard-only navigation,
     and the four-script name fields;
   - **AccessKit enabled**, plus an `--dump-a11y` mode that serialises the accessibility tree to JSON for
     inspection without a live screen reader;
   - a `--latency` mode that forces the **tiny-skia software renderer** and logs input-to-paint samples for the
     pure summariser.
3. **Operator scripts + results template** in the README: the Orca/NVDA keyboard walk, the IME entry steps, and
   the Pi `--latency` invocation, each with a results stub.

---

## 4. PASS / FAIL

| # | Claim | PASS threshold |
|---|---|---|
| **A1** | A11y tree exists + is correct | `--dump-a11y` yields a tree where **every** focusable control has a role + accessible label; the identifier field and each med row are reachable and announced. Verified by inspection of the dumped tree **and** an operator keyboard+screen-reader walk that can complete the form without sight |
| **A2** | Keyboard-only operable | the whole form (focus, edit, add/remove med row, submit) is operable with **no pointer**, tab-order legible |
| **I1** | Complex-script shaping | the corpus check passes for Latin, **Arabic (RTL)**, **Devanagari (complex)**, **Han** — each shapes to > 0 clusters with **no tofu/`.notdef`**, given an installed Noto-class font (skips with a loud SKIP if the font is absent, never a silent pass) |
| **I2** | RTL + bidi correct | the Arabic field renders right-to-left with correct cursor/caret behaviour (operator-verified screenshot) |
| **I3** | CJK IME input | composing Han text via a system IME commits correctly into the field (operator-verified) |
| **L1** | Latency summariser correct | the pure p50/p95/p99 + frame-budget functions are unit-tested against known vectors (CI) |
| **L2** | Paper-parity floor on the Pi path | on the **tiny-skia software** renderer, median input-to-paint is within the [§1.2](../spec/vision.md#12-the-paper-parity-test-normative) interactive floor on Pi-class hardware (target **p95 ≤ 100 ms** for keystroke-to-paint; recorded, not asserted) |

---

## 5. Exit criteria → what it means

- **A + I PASS** → iced clears the EHR-specific bars; adopt it for the **reference desktop UI** (an L3
  citizen) and proceed to build the real client against the native API. **No ADR, no spec change** — the
  layering already permits any L3 framework; this is a build choice recorded in the eval, not a decision about
  Cairn.
- **A FAIL** → the reference desktop UI tips to a **webview/Tauri L3**; record it in
  [eval 0004](../ecosystem/0004-reference-ui-framework-iced-vs-tauri.md). An iced client may still ship later —
  L3 is plural — but is not the steward's primary. This is the comparison *made*, not a defeat.
- **I FAIL (shaping/IME)** → either depend on the plushie-iced/cosmic-text fixes and re-run, or tip to a
  webview L3 for the international surface. Culture-neutrality
  ([ADR-0014](../spec/decisions/0014-locale-pluggable-matcher-comparators.md)) is non-negotiable, so a real
  shaping gap is a hard stop for *this* framework, not a soft preference.
- **L FAIL** → tuning task (renderer config, font caching) or a Pi hardware-floor finding, fed back to the
  [Spike 0001](0001-walking-skeleton-wan-sync-and-pi-cost.md) Bet-B Pi-benchmark thread; not a framework veto on
  its own.
- Any FAIL is **design/selection feedback, not a defect to paper over** (house rule 5): if it cannot be fixed
  in place, it is filed as a GitHub issue.

---

## 6. Blast-radius (§9) note

The lowest-stakes spike to date — the whole subject is the **fit-for-purpose** L3 layer
([§9.1](../spec/language-substrate.md#91-selection-rule-by-defect-blast-radius)):

- **No safety-critical surface is touched.** The UI never signs, never `INSERT`s into event tables, and is
  isolated from the node's dependency tree (standalone crate). A defect here yields a worse *screen* a human
  sees — never a corrupted record. The native-API stub mirrors "the node canonicalises + signs"
  ([ADR-0015](../spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)) so the harness
  cannot model a UI that owns serialization.
- **Fit-for-purpose (Rust/iced + cosmic-text):** the harness optimises for iteration speed and is allowed to
  ride iced's pre-1.0 churn — exactly the bucket where fast-moving ecosystem tooling belongs.
