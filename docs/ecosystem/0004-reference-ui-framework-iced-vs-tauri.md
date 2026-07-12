# Ecosystem eval 0004 — Reference-UI framework: iced vs Tauri (the L3 desktop client)

- **Status:** **Resolved — reference desktop UI is Tauri 2 (for now).** The evaluation recommended
  **iced** *conditional* on [Spike 0004](../spikes/0004-iced-reference-ui-viability.md) retiring the
  accessibility bar. **The spike ran (2026-07-12) and iced FAILED that bar** — released iced 0.14 ships
  **no AccessKit / no accessibility tree at all** (empirically confirmed, [§6](#6-outcome-2026-07-12--iced-fails-the-accessibility-bar--tauri)).
  Per this eval's own contingency ("A FAIL on (A) tips the reference UI back to a webview/Tauri L3"), the
  steward's reference desktop UI **adopts Tauri 2**. This is the eco-eval's job done, not a design defect —
  **L3 is plural** and the wire core, `ClinicalData` port, and policy layer are UI-agnostic, so the swap is
  bearable by construction. **Spec unchanged; no ADR** — an L3 framework choice is below the compatibility
  boundary. Reversible: an iced (or libcosmic) client may still ship later if its accessibility matures.
- **Date:** 2026-06-30 · **Resolved:** 2026-07-12
- **Subjects:** [iced](https://github.com/iced-rs/iced) (MIT, Rust-native, Elm Architecture,
  wgpu/tiny-skia renderer) · [Tauri 2](https://github.com/tauri-apps/tauri) (MIT/Apache-2.0,
  Rust backend + system-webview frontend).
- **Motivation:** the question *"is iced a better match for Cairn's long-running clinical UI than a
  webview stack, with less JavaScript and a smaller dependency tree?"* Neither framework is named
  anywhere in the spec — [§9.5](../spec/language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges)
  deliberately leaves UI tech open ("the wire **transport** … is a later fit-for-purpose choice"). This
  is the *why of fit*, not a decision.

> [!NOTE]
> Optional by construction. A bare Cairn node is a complete EHR with **no** UI — it is an event core
> plus the validated submit surface ([§9.6](../spec/language-substrate.md#96-the-validated-submit-surface-the-write-path)).
> A reference UI is one L3 citizen among many; this eval picks a framework for *that* citizen, not for
> Cairn.

---

## 1. Where this lands in the four-layer model

[ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) /
[§9.5](../spec/language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges)
already did the load-bearing work, and it makes this a **low-stakes, reversible** choice:

- **L3, zero wire blast radius.** A UI is a *pure producer/consumer of signed events over a contract it
  cannot alter* — it "can produce content wrong for its clinic but **never a wire-incompatible event**,"
  because the node (L1) canonicalises + signs ([ADR-0015](../spec/decisions/0015-event-serialization-signatures-and-content-addressing.md))
  and the floor is enforced in-DB. The framework cannot threaten node interoperability, the algebras, or
  the floor. This is squarely the [§9.1](../spec/language-substrate.md#91-selection-rule-by-defect-blast-radius)
  **fit-for-purpose bucket** (a UI defect is caught immediately or is cosmetic) → optimise for iteration
  speed + ecosystem, not compile-time safety.
- **L3 is explicitly plural.** "The reference UI is one citizen." Choosing iced forecloses nothing — a
  webview UI, a TUI, or a mobile client can coexist, each binding the same native API
  ([§9.7](../spec/language-substrate.md#97-the-native-api-contract-capability-description-and-conformance)).
  The choice is **non-exclusive and reversible**, which lowers the bar for trying the more
  mission-aligned option.

So the decision is dominated not by "which is nicer" but by **which clears an EHR's non-negotiable bars
(accessibility, complex international text) while best serving the four governing principles** — and that
is what [Spike 0004](../spikes/0004-iced-reference-ui-viability.md) measures.

## 2. The two candidates (current as of 2026-06)

| | **iced** | **Tauri 2** |
|---|---|---|
| Model | Rust-native; **The Elm Architecture** — all state in one struct, every mutation through one `update(Message)`, `view` derived from state | Rust backend + **system-webview** frontend (any JS framework) over an IPC bridge |
| Renderer | **ships its own** — wgpu (GPU) with a **tiny-skia software fallback** (no-GPU path) | the OS webview: WebKitGTK (Linux), WebView2 (Windows), WKWebView (macOS) |
| License | **MIT** (AGPL-compatible) | **MIT/Apache-2.0** (AGPL-compatible) |
| Maturity | **pre-1.0** (0.14, Dec 2025); breaks API between minor releases. Flagship production user: **System76 COSMIC desktop** (stable Dec 2025, 1.1 shipping) built on iced/libcosmic | **stable (Tauri 2)**; large community; **iOS/Android** support |
| Text shaping | **cosmic-text** (same stack COSMIC ships) | the browser's — gold standard for bidi/complex-script/IME |
| Accessibility | **AccessKit**, *still maturing* — partial in mainline; fuller tree (AT-SPI2/UIA/NSAccessibility) furthest along in the [plushie-iced](https://lib.rs/crates/plushie-iced) fork | inherits the **browser a11y tree** (mature ARIA / screen readers) |

Both licenses are AGPL-compatible, so house rule 1 (supply-chain licensing) gates neither out.

## 3. Why iced is the more mission-aligned match

These are not generic GUI-shootout points; each maps onto a stated Cairn value.

1. **Supply-chain auditability = anti-capture applied to dependencies (the strongest point).** House rule 1
   and [§9.2](../spec/language-substrate.md#92-primary-quality-metric-reviewer-legibility) make the dependency
   tree a first-class mission concern ("shrink the audited surface"). Tauri drags in the **entire npm/JS
   frontend ecosystem** (hundreds of transitive packages, each a license + supply-chain surface) **plus** the
   system webview. iced is **one Cargo workspace, all-Rust**, `cargo-deny`/`cargo-audit`-able, reviewer-legible
   end to end. For a small, review-gated team this is the most directly mission-aligned advantage.
2. **Single language across the workspace.** The node, sync daemon, and pgrx live in Rust. An iced UI keeps
   one toolchain and one review skillset, and — because it binds the native API — can **share an API-client
   crate and generated types with the node**, so the L3↔L2 contract is type-checked at the boundary instead
   of stringly-typed across an IPC/JSON seam. Tauri structurally re-introduces the polyglot seam
   [§9.3](../spec/language-substrate.md#93-integration-boundary) works to avoid, one layer up.
3. **Self-contained rendering — fractal + offline fit.** Tauri renders through the OS webview; WebKitGTK in
   particular is a per-distro inconsistency and an availability/version dependency you don't control — exactly
   what bites on locked-down or Pi-class Linux nodes. iced **ships its own renderer** (wgpu, tiny-skia software
   fallback) → deterministic, identical from workstation down to a headless-GPU Pi. That serves
   *fractal topology* and *offline-first* better than "whatever webview the OS ships."
4. **Statefulness fits a long-running clinical session — and rhymes with the record model.** The Elm
   Architecture's single-struct state + unidirectional `update` is a clean fit for a long-lived encounter
   screen, and there is a genuine conceptual rhyme with Cairn: a UI whose state is a **projection derived from
   a stream of events**, updated unidirectionally, maps naturally onto an append-only/projection record. You
   skip the React/Redux/signals cache-invalidation class of bug. *Precision:* iced rebuilds the widget tree
   from `view()` each frame — the benefit is **unidirectional data flow and no stale-cache bugs**, not literally
   fewer redraws.
5. **Latency floor for paper-parity.** Native rendering with no webview/JS-engine overhead gives lower,
   more predictable input-to-paint latency on weak hardware — directly serving the
   [§1.2](../spec/vision.md#12-the-paper-parity-test-normative) paper-parity floor on Pi-class nodes.

## 4. Where a webview/Tauri stack wins — the real risks (not throwaway)

These are why the recommendation is **conditional**, and they matter *more* for an EHR than for a typical app.

1. **Accessibility — the single biggest risk.** A web UI inherits the browser's mature a11y tree (ARIA,
   screen readers, decades of assistive-tech support) essentially for free. iced's a11y is **AccessKit that is
   still maturing** — partial in mainline 0.14; the fuller tree is furthest along in the *plushie-iced* fork,
   whose API is itself described as "still evolving." For clinical software with real accessibility
   obligations this is a **potential blocker, not a cosmetic gap.** This is the first thing the spike must
   settle.
2. **Complex international text / IME — collides with culture-neutrality.** Cairn is explicitly culture-neutral
   (names/addresses in many scripts, [§5.13](../spec/identity.md) locale-pluggable comparators,
   [ADR-0014](../spec/decisions/0014-locale-pluggable-matcher-comparators.md)). The browser is the gold standard
   for bidi, complex-script shaping (Arabic/Indic), and IME (CJK input). iced via **cosmic-text** has improved
   greatly, but complex-script + IME correctness must be **proven, not assumed** — historically iced's weak
   spot.
3. **Pre-1.0 API churn over a decades horizon.** Tauri 2 is stable; iced breaks API between minor releases — a
   recurring tax on a small team across a long horizon. **Mitigant:** COSMIC (shipping stable, well-resourced)
   is now a serious production anchor and momentum signal, but it does not eliminate the churn cost.
4. **Off-the-shelf widget richness / velocity.** Web has ready-made data grids, date pickers, charting, rich
   text; in iced you build more yourself. **Narrowing caveat:** a clinical UI (flowsheets, med lists,
   interaction overlays, dense keyboard-driven charts) is *mostly bespoke widgets anyway*, where web's
   component-library edge matters less — but baseline velocity still favours web.
5. **Mobile / tablet point-of-care.** Tauri 2 has real iOS/Android support; iced does not. If bedside
   tablet/phone capture is on the roadmap, that is a strong point for a web stack (or a separate mobile L3
   client). **Open question for the steward** — not currently on the stated roadmap.

## 5. Conclusion (fit, not decision)

iced is the **leading candidate for the steward's reference desktop UI** and is more aligned than a webview
stack on the axes Cairn weighs most — supply-chain auditability, single-language reviewer-legibility,
self-contained offline rendering, and a stateful model that rhymes with the event/projection record. The
recommendation is **conditional on [Spike 0004](../spikes/0004-iced-reference-ui-viability.md) retiring the two
EHR-specific risks** — **accessibility** and **complex-text/IME** — plus a **Pi input-to-paint latency**
measurement folded into the existing Bet-B Pi benchmark.

Because L3 is plural and the native-API contract is identical for every UI, this is not exclusive: if the a11y
bar fails, the reference UI tips back to a webview/Tauri L3 **and** an iced client can still ship — *many
front-ends, one record*. No commitment is made to the spec; an ADR would only be warranted if a future decision
constrained *all* deployments' UI tech, which the layering explicitly forbids.

> [!WARNING]
> **Honest scope of the spike.** A live screen-reader pass (Orca/NVDA) and a real-Pi latency run need hardware
> and assistive tech not present in CI. [Spike 0004](../spikes/0004-iced-reference-ui-viability.md) therefore
> ships a **runnable harness** ([`poc/iced-ui-spike`](../../poc/iced-ui-spike/)) whose *pure* claims
> (multi-script shaping, latency statistics) are unit-tested headlessly, while the a11y-tree and live-latency
> passes are scripted for a workstation/Pi operator. The harness is the artifact; the green/red verdict is a
> human run.

## 6. Outcome (2026-07-12) — iced fails the accessibility bar → Tauri

The spike was run against the real reference shell (`cairn-gui/cairn-gui-shell`, a two-pane walking skeleton
on a mock port; results in [`cairn-gui/cairn-gui-shell/results/2026-07-12-macbook-workstation.md`](../../cairn-gui/cairn-gui-shell/results/2026-07-12-macbook-workstation.md)).

**What passed.** **I1 complex-script shaping PASSED** on the real product surface — a single fixture name
`Amina أمينة अमीना 阿明娜` rendered Latin + Arabic (RTL) + Devanagari + Han with **zero tofu**. Cross-pane
routing and the draggable divider worked. So the *text* risk — historically iced's weak spot — is not the
blocker.

**What failed — the decisive finding.** **Released iced 0.14 has no accessibility support whatsoever.**
Inspecting the installed crates: only `text_input`/`text_editor` are focusable (`button.rs` has none), and
**there is no `accesskit` dependency anywhere in the compiled tree** (`iced`, `iced_widget`, `iced_runtime`,
`iced_winit`, nor the lockfile). Confirmed empirically with macOS **Accessibility Inspector** on the live
window: the Cairn controls surface **no** accessible elements (`Children: Empty array`; only the AppKit menu
bar is navigable) — a screen reader gets a menu bar and an empty box. For an EHR with real accessibility
obligations this is the "potential blocker, not a cosmetic gap" §4.1 named.

**Trajectory (why "wait for iced" is not bankable).** The AccessKit tracking issue
[iced #552](https://github.com/iced-rs/iced/issues/552) has been **open since 2020**; as of Dec 2025 the thread
was still debating the approach. There is an active but **draft, unmerged** PR
([#3111](https://github.com/iced-rs/iced/pull/3111), May 2026); an earlier attempt (#3281) was closed without
landing. The one iced-family path with a11y *today* is the **libcosmic / plushie-iced fork** (System76,
actively maintained, ships in COSMIC) — viable but couples to a fork.

**Decision.** The reference desktop UI **pivots to Tauri 2** (webview inherits the browser's mature a11y tree).
Rationale beyond the bar: Cairn UIs are **thin layers over the DB + extensions**, and the policy layer is
**GUI-agnostic**, so re-implementation cost is bearable and no wire/contract decision is affected. The slice-1
work is **not wasted** — its framework-agnostic core (the `Tab`/semantic contract, the `ClinicalData` port, the
manifest merge, the pane/routing/freshness state machine) is plain Rust reusable behind a Tauri backend; only
the iced *rendering* layer is superseded. **Reversible:** re-evaluate iced or libcosmic if/when its
accessibility matures (a small dedicated spike would settle libcosmic's completeness).
