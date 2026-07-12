# Design — Cairn clinician reference GUI: shell + tab-plugin architecture

- **Date:** 2026-07-12
- **Status:** Design approved (brainstorming). Awaiting implementation plan (writing-plans).
- **Layer:** L3 reference UI ([ADR-0021](../../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md),
  [§9.5](../../spec/language-substrate.md)). Zero wire blast radius — this design decides nothing
  about the event core, the algebras, or the DB floor. A framework/UI choice below the compatibility
  boundary; no ADR, no canonical-spec change.
- **Framework:** iced, per [eco-eval 0004](../../ecosystem/0004-reference-ui-framework-iced-vs-tauri.md),
  **conditional** on [Spike 0004](../../spikes/0004-iced-reference-ui-viability.md) retiring
  accessibility / complex-text-IME / Pi-latency. That condition is **not yet met** (results dir holds
  only the template). This design is therefore written to be *mostly framework-agnostic* (the Tab
  contract, Context, data port, manifest, and per-setting bundles all survive a framework swap), and
  the first slice **doubles as the vehicle to run Spike 0004 for real** (§8, §9).

---

## 1. Purpose & scope

Build the **clinician-focused reference desktop UI**: a shell that hosts reusable clinical views
("tabs") composed differently for different clinical settings (GP, specialist, ward, ED) — overlapping
but distinct needs — from **one binary** whose active tab-set is selected at runtime.

**In scope**
- The shell: persistent safety zone, left rail, two-pane splittable workspace, cross-pane routing,
  context provision, manifest loading.
- The `Tab` contract (trait + mandatory semantic/accessibility model).
- The `ClinicalData` port (trait) with a real (native-API-client) impl and a mock (fixtures) impl.
- The runtime **manifest** (which compiled tabs are live, rail entries, default layout).
- A first runnable slice (§8).

**Out of scope**
- Purpose-built single-role apps (e.g. a pure front-desk app: demographics / waiting-room /
  appointments / billing). Those are a separate future product; this UI stays narrowly clinician-focused.
- Subsystem **B** — the actual role→permission *policy*. This design defines only the **interface** B
  plugs into (`Context.capabilities`); the resolver is a stub here.
- Any event write/sign path. The UI **binds the node's native API and never signs or `INSERT`s events**
  ([§9.6](../../spec/language-substrate.md)); the node canonicalises + signs
  ([ADR-0015](../../spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)).

## 2. Governing decisions (settled during brainstorming)

1. **Layout** — titlebar / persistent safety zone / left rail / two-pane workspace. Left rail is
   convention. **All sizing relative to available screen real estate — no fixed pixels.** Clinicians
   run full-screen and want every pixel working; functionality rules.
2. **Split-screen is a shell feature, not a tab feature.** Two panes, each with its own tab strip and
   active tab, a **user-draggable divider**. Either pane holds any view; left defaults to the progress
   note. Enables: note + reference cross-referencing; two reports side by side (change over time);
   discharge-summary next to med/history list (reconciliation).
3. **A tab is a single `view()`** — a self-contained TEA sub-app. Internal two-pane geometry is *not* a
   tab concern; the shell owns all tiling.
4. **Compile-time plugins, runtime selection.** All tab crates are compiled into the one binary; a
   **manifest** decides which are *live* per site/role. (Settled earlier: compile-time plugins are
   sufficient; clinicians prefer one stable interface. No dynamic `.so`/WASM loading — also a security
   non-goal for a clinical app.)
5. **Hybrid data model** — the **shell is the context provider**; **tabs are lazy-loading sub-apps**.
   (Explicitly the easyGP lineage.)
6. **Lazy loading** — visible tabs load immediately; hidden tabs prefetch in the background so the UI
   is near-instant. Prefetch is a *hint*, never authority (rhymes with
   [ADR-0004](../../spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)).
7. **The pure semantic/accessibility contract holds** — carried forward from
   [`poc/iced-ui-spike/src/form.rs`](../../../poc/iced-ui-spike/src/form.rs): every tab (and shell
   chrome) declares its focusable controls + labels, CI-checked.

## 3. Crate architecture

A Cargo workspace, **standalone from the node's tree** so iced never enters `cairn-node`'s dependency
graph (same discipline as the spike crate).

```
cairn-gui/                 (workspace)
├─ cairn-gui-shell/        binary: bands, panes, divider, routing, manifest loader, context provider
├─ cairn-gui-tab/          Tab trait + semantic/a11y contract + Context + shared load/error scaffolding
├─ cairn-gui-data/         ClinicalData port (trait) + real (native-API-client) impl + mock (fixtures) impl
├─ cairn-gui-tabs/         Tab implementations: note, timeline, results, meds, demographics… (one crate,
│                          or split per tab as it grows)
└─ cairn-gui-manifest/     manifest schema + loader (which tabs live, rail entries, default layout)
```

**iced-containment rule.** Only `cairn-gui-shell` and the thin `view()` bodies in `cairn-gui-tabs`
touch iced types. `cairn-gui-tab`'s *contract* (semantic model, `Context`, port handle), all of
`cairn-gui-data`, and all of `cairn-gui-manifest` are **iced-free and headlessly CI-testable**. This
keeps the pre-1.0 churn surface small — a framework migration is a mechanical sweep of the render
bodies, not a redesign.

## 4. The Tab contract

A tab is a TEA sub-app plus a declared, testable semantic contract. Illustrative (not final) shape:

```rust
trait Tab {
    type Message;

    /// Stable, addressable identity — deep links target this.
    fn id(&self) -> TabId;

    /// Label shown in the pane's tab strip.
    fn title(&self) -> String;

    /// The pure accessibility contract (from form.rs's FormModel): every focusable
    /// control with a role + non-empty label. CI asserts completeness.
    fn semantics(&self) -> FormModel;

    /// Lazy fetch hook. Called eagerly for a visible tab; called at low priority by
    /// the background scheduler for hidden tabs. Returns an async Task — never blocks paint.
    fn load(&mut self, ctx: &Context, data: &DataHandle) -> Task<Self::Message>;

    fn update(&mut self, msg: Self::Message, ctx: &Context) -> Outcome<Self::Message>;

    fn view(&self, ctx: &Context) -> Element<Self::Message>;
}
```

- **`semantics()` is mandatory**, not optional — this is how the accessibility bar (the biggest
  framework risk) stays enforced in code rather than left to AccessKit's behaviour.
- **`load()` is the lazy hook.** Visible → immediate; hidden → background scheduler.
- **`Outcome`** lets `update()` return a follow-up message *and/or* a **shell intent**. The
  load-bearing intent is `OpenInOtherPane(Intent)` — how the note's "see X-ray report" link opens the
  report in the opposite pane while the note stays put.

**Known coupling (accepted):** `Message`/`Task`/`Element` associated types leak iced into the trait.
Unavoidable if tabs render iced; kept to the smallest possible surface (see §3 containment rule).

## 5. The shell

Four bands, all relative-sized, full-screen, no fixed pixels:

1. **Titlebar** — node identity, online/offline status, current user.
2. **Persistent safety zone** — identity / meds+allergies / present-illness / urgent-actions cards.
   **These use the same `Tab` contract** as everything else (they consume `Context` + the data port),
   but are flagged **pinned**: never hidden, always live-refreshing. Uniformity buys them the a11y
   contract and the data path for free.
3. **Left rail** — navigation, populated from the manifest; opens/switches tabs.
4. **Two-pane workspace** — each pane owns a tab strip + active tab; a **draggable divider** apportions
   width (user-controlled, persisted per session). **Cross-pane routing:** a tab emits
   `OpenInOtherPane(Intent)`; the shell resolves it to a tab instance in the opposite pane. Left pane
   defaults to the progress note.

**Cross-references** (e.g. "see X-ray report") are links that carry a one-line summary and an `Intent`
resolving to a content-addressed event/blob (eager reference, lazy bytes —
[ADR-0013](../../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)); clicking opens
the target in the other pane.

**Shell-level accessibility is in scope** (see §6 risk): the semantic contract extends beyond a single
form to **shell chrome** — pane focus order, tab-strip traversal, rail navigation, and the divider —
because a blind clinician must traverse all of it by keyboard.

## 6. Context, the data port, and the seam to subsystem B

**`Context`** — read-only, shell-owned, handed to every tab and every pinned safety card:
- current **patient**,
- current **user**,
- resolved **`Capabilities`**.

`Context.capabilities` **is the entire seam to subsystem B.** In this design a stub resolver returns
"clinician sees everything." When B lands, it replaces only the resolver with the real role→permission
mapping; the shell and tabs are unchanged — they only ever read `ctx.capabilities`.

**`ClinicalData` port** — a single trait in `cairn-gui-data` covering every read the UI needs. Real
impl → the node's native-API client (shared types with the node where possible). Mock impl → fixtures.
Tabs receive a `DataHandle` to it via `Context`. This trait is the mock-data seam that lets the whole
shell run with **no node** (§9).

**Lazy loading, freshness, and fairness — the clinical caveats:**
- Visible tab → `load()` immediately. Hidden tabs → a **background scheduler** calls `load()` at low
  priority so switching is instant.
- **Stale-prefetch safety.** Prefetched data carries a `loaded_at`. On becoming visible/focused, a
  **pinned or safety-relevant tab revalidates before its data is trusted**, showing an explicit
  "refreshing" state rather than a confident stale number (meds/allergies must never silently display
  20-minute-old safety data). Cosmetic/static content (an old X-ray) may trust the cache.
- **Fairness / availability floor.** The background scheduler is **preemptible and budget-limited**;
  the visible tab's `load()` always wins. Background prefetch must never starve the active view or
  hammer the node (same instinct as the byte-tier availability floor).
- **Capability-gated prefetch.** A hidden tab the user lacks capability for **must not** background-
  fetch protected data — prefetch consults `ctx.capabilities` too. This is *soft* policy (the hard gate
  is the DB floor), but it stops the UI holding data it shouldn't even have in memory.

## 7. The manifest (runtime tab-set selection)

A **signed config** the shell loads at startup (reloadable per site/role) declaring:
- which compiled tabs are **live**,
- their **rail entries**,
- the **default pane layout** (e.g. "note left, timeline right").

Switching GP→ED is a manifest swap — no rebuild. The manifest is **soft policy** (what is *offered*);
the DB floor remains the **hard gate** (what is *permitted*). One mechanism serves both "this site's
tab-set" and (via B) "this role's tab-set." One artifact to build, sign, and distribute — fits the
single signed distribution plane ([§3.13](../../spec/language-substrate.md)).

## 8. First slice

The first runnable slice **already includes the two-pane split**, so the headline cross-referencing
feature is provable immediately:

- Shell with **two panes + draggable divider**.
- The **persistent identity safety card** (pinned).
- **Left pane:** a minimal **Note** tab (progress note) — enough to host a cross-reference link.
- **Right pane:** a **Demographics/identity** tab (matches the built DB slice
  [`db/010–014`](../../../db/010_demographics.sql)) opened via cross-pane routing from the note.
- Running against the **mock `ClinicalData` port** (no node).
- Manifest drives the rail + which tabs are live.

This proves end-to-end: the `Tab` contract, `Context`, the port, the manifest, the two-pane shell, the
divider, and `OpenInOtherPane` routing — and it is the artifact Spike 0004 runs against (§9).

Later slices: additional tabs (timeline, results, meds), real native-API-client port impl, richer note
editing (markdown-source + live preview — no WYSIWYG widget exists in iced out of the box).

## 9. Testing & Spike 0004 convergence

Because the shell runs on the **mock port with zero node**, it **is** the Spike 0004 vehicle — a11y /
IME / Pi-latency get measured against the **real shell + real tabs on fixtures**, not a throwaway form.

- **Headless CI (iced-free):** semantic-contract completeness for every tab **and shell chrome**;
  manifest loading; `ClinicalData` port contract; cross-pane routing logic; lazy-scheduler + freshness
  rules; capability-gated prefetch.
- **Operator runs (widened Spike 0004):**
  - **Accessibility** — screen-reader (Orca/NVDA) traversal of **pane / tab-strip / rail / divider +
    fields**, not just a single form. (Approved widening — see risk below.)
  - **Complex-script + IME** — Latin / Arabic (RTL) / Devanagari / Han + CJK-IME in a name field.
  - **Pi input-to-paint latency** — tiny-skia software renderer, paper-parity floor.
- Results recorded under [`poc/iced-ui-spike/results/`](../../../poc/iced-ui-spike/results/) (currently
  template-only).

## 10. Risks & mitigations

- **Spike 0004 unmet.** iced adoption is conditional on an a11y/IME/Pi pass that has **not run**.
  Mitigation: this design is mostly framework-agnostic; the first slice is the honest spike vehicle;
  a FAIL on accessibility tips the reference UI to a webview/Tauri L3 (L3 is plural) with the contract,
  Context, port, and manifest intact.
- **Shell-level a11y > form-level a11y (approved scope widening).** The original spike tested one
  form; the real shell adds cross-pane/tab/rail/divider traversal that a blind clinician must operate.
  The spike is widened accordingly. Net-saves work vs. building a throwaway form and the shell
  separately.
- **iced pre-1.0 churn.** Contained to the shell + tab `view()` bodies (§3); contract and data layers
  stay iced-free.
- **Stale prefetch of safety data.** Addressed by the revalidate-on-focus rule for pinned/safety tabs
  (§6).
- **Background prefetch starving the active view.** Addressed by the preemptible, budget-limited
  scheduler (§6).
- **UI tab-hiding mistaken for a security boundary.** It is **soft** policy only; the **hard** gate is
  the in-DB validated submit surface + RLS (principle 12). Documented so no one relies on the manifest
  for real access control.
- **No WYSIWYG rich-text widget in iced.** The Note tab uses markdown-source + live preview (both
  available out of the box); a true WYSIWYG editor is a later optional custom-widget investment. This
  also aligns with principle 11 (the plaintext *is* the legibility twin).

## 11. Open items for the plan

- Exact `Outcome`/`Intent` enum shape and the routing table the shell keeps.
- Manifest file format (TOML/JSON) and signing/verification path.
- Divider size-persistence (per session vs per user) location.
- Whether `cairn-gui-tabs` starts as one crate or one-per-tab.
- The native-API-client crate boundary and how much type-sharing with the node is feasible now.
