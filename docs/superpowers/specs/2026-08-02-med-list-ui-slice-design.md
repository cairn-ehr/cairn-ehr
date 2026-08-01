# Design — the med-list UI slice (Tauri 2): Cairn's first runnable clinical surface

**Date:** 2026-08-02
**Status:** approved in brainstorming, ready for a plan
**Layer:** L3 (reference UI) + a new read path at the node library tier
**Discharges:** [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) (whole-list sign-off = one human gesture)
**Governing:** [ADR-0021](../../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) (four layers, UI pluralism) ·
[ADR-0049](../../spec/decisions/0049-commitment-based-sign-off-currency.md) (commitment-based sign-off currency) ·
[ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) (per-write human authorship) ·
[ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md) (born-sealed bodies) ·
§1.2 paper-parity · principle 3 (no confirmation dialogs) · principle 4 (acknowledged uncertainty)

---

## 1. Purpose & scope

A Tauri 2 desktop window that shows **one patient's current medication list** and supports exactly
two writes: **whole-list sign-off** (one gesture) and **per-row cease**.

This is a bigger step than "a UI slice", because two things underneath it do not exist yet:

1. **Cairn has no clinical read path in Rust at all.** The `cairn-node` CLI carries nine medication
   verbs, all writes. `patient_medication_current`, `medication_group_display` and
   `medication_group_attestation` are read *only in tests*. This slice builds the first one.
2. **There is no Tauri shell.** [PR #174](https://github.com/cairn-ehr/cairn-ehr/pull/174) merged an
   **iced** shell that then failed the accessibility bar
   ([spike 0004](../../spikes/0004-iced-reference-ui-viability.md)); its rendering layer is a
   superseded spike artifact. What survives is framework-agnostic Rust — the `Tab`/semantic contract,
   the `ClinicalData` port, the manifest merge, and the pane/routing/freshness state machine.

**In scope:** the read path, the sign-off bundling orchestrator, the pure med-list view model, the
Tauri window, per-row cease, session key custody, and node-local gesture-timing capture.

**Out of scope, deliberately:** patient search, any other clinical stream, the native API (Phase 8),
prescribing, dose editing. Section 8 states every gap.

## 2. Governing decisions (settled during brainstorming)

- **The read path is a shared module in `cairn-node`, not SQL in the GUI.** One implementation; the
  future Phase 8 API wraps the same function rather than re-deriving it. Reading the projections
  directly is the ADR-0021 *privilege gradient* ("via the API vs. DB directly"), taken knowingly and
  recorded here rather than silently.
- **The signing key is unsealed once per session and re-locked on idle.** The paper counterpart of
  identity is physical presence, re-proved at neither the second signature nor the tenth. This is
  what makes the whole-list gesture cost exactly one act.
- **Sign-off attests only threads whose vouch is absent or stale.** The paper artifact is the
  **drug chart**, where each drug line carries the signature of the person responsible for *that*
  drug. A thread already holding a non-stale vouch keeps its existing signatory untouched. (This
  corrected an earlier draft that modelled sign-off on an admission med-rec *form*, which would have
  overwritten other clinicians' responsibility with the current user's.)
- **Rust computes the view model; the webview renders it.** Every clinical display decision — is this
  vouch stale, will this gesture sign this row — is a pure Rust function under `cargo test`. Native
  semantic HTML supplies the accessibility tree that iced could not.
- **The iced rendering layer is retired in this slice.** Its pure state machine stays.
- **Gesture timing is captured as node-local aggregates only.** See §7.

## 3. Crate layout

```
crates/cairn-node/src/medication/
    read.rs      NEW  first clinical read path — pure row->struct over the projections
    signoff.rs   NEW  whole-list bundling orchestrator
crates/cairn-node/src/main.rs
                 +medication-list --json      the read path as a public CLI surface
                 +medication-sign-off         whole-list bundling, CLI parity with the UI

cairn-gui/                      standalone workspace; gains a ONE-WAY dep on cairn-node
    cairn-gui-tab/              KEEP    semantics / Tab / Context
    cairn-gui-data/             EXTEND  ClinicalData += medications(); new MedWrite port
    cairn-gui-manifest/         KEEP
    cairn-gui-shell/            TRIM    keep the pure state machine; drop iced + the `gui` feature
    cairn-gui-tabs/
        cairn-gui-tab-medications/  NEW  the pure view model
    cairn-gui-tauri/            NEW  Tauri backend + src-ui/ (vanilla TypeScript)
```

`cairn-gui-data` stays dependency-free (trait + types + mock) so every tab crate can depend on it
without pulling a database driver. The cairn-node-backed adapter lives in `cairn-gui-tauri` — a thin
mapping; extracting it into its own crate is trivial when a second client needs it, and doing so now
would be speculative.

The dependency `cairn-gui → cairn-node` is **one-way**. Nothing in the GUI tree may ever become a
dependency of `cairn-node`; that direction is what the workspace `exclude` protects.

## 4. Data flow

```
Postgres projections (all exist today; all GRANTed to cairn_agent)
    patient_medication_current              one row per GROUP
    medication_thread_group                 group -> member threads
    medication_thread_attestation           per THREAD: attester_kid, stale
    patient_medication_reconciliation_flag  un-reconciled duplicates
    medication_group_coding_conflict        two anchors in one group
         |
    cairn_node::medication::read::list_patient_medications(db, patient)
         |                                   -> Vec<MedicationRow>
    cairn-gui-tauri  (own tokio-postgres connection; Tauri command `med_list`)
         |
    cairn_gui_tab_medications::build_view(rows, now) -> MedListView    PURE, cargo-tested
         |     rows + vouch badges + sign_off_targets + SemanticNode
    webview  renders MedListView as semantic HTML (<table>, <button>, <h2>)
```

**A displayed row is a group, attestation is per thread.** `patient_medication_current` emits one row
per *group* (reconciled duplicates collapse), while `medication_thread_attestation` is keyed per
*thread*. `MedicationRow` therefore carries its member threads and each member's vouch state, and the
row's badge is a rollup. This is the single most error-prone join in the slice and gets explicit tests.

**Writes** travel the same path in reverse:

```
[Sign off list]  -> Tauri command sign_off(patient)
     -> cairn_node::medication::signoff::sign_off_medication_list(...)
          targets  = threads with absent-or-stale attestation
          mint N HLCs -> ONE transaction -> N x attest_thread_in_tx -> commit
[cease] on a row -> Tauri command cease(patient, medication_id, reason?)
     -> existing cease_medication carrying AuthorParams (ADR-0053 human authorship)
```

The bundling shape is the one `reconciliation.rs` already uses for two threads, generalised to N.
Putting it in `cairn-node` rather than the GUI means the CLI gets whole-list sign-off too — the
reference UI uses no privileged path.

**Session key custody:**

```
window open -> one passphrase unlock -> load_attester_key
            -> Zeroizing<SigningKey> held in Tauri app state
            -> 15 min idle -> wiped; UI renders a locked state, both gestures disabled
```

The idle timeout is **15 minutes**, held as one named constant with a test that pins it, so it is a
reviewable decision rather than a number buried in a timer. Idle means no gesture and no keystroke in
the window; it is not a session length cap.

## 5. The #288 core — sign-off targeting

One pure function carries the whole clinical contract:

```rust
/// Which threads a single sign-off gesture attests.
///
/// Paper drug-chart semantics: each drug line carries the signature of the person
/// responsible for THAT drug, so a thread already holding a non-stale vouch keeps its
/// existing signatory untouched. Returns thread ids (not group ids) because ADR-0049
/// attestation is per-thread.
pub fn sign_off_targets(rows: &[MedicationRow]) -> Vec<Uuid>
```

Targets = every member thread of every **active** group whose attestation is **absent or stale**.

Each row renders `VouchState::{None, Fresh { by }, Stale { by }}`. The clinician can therefore see
which lines the gesture will sign **before** signing — the paper affordance of looking at the
unsigned lines. This is explicitly *not* a confirmation dialog (principle 3): nothing is interposed
between the intent and the act; the state is simply visible, ambient, and always was.

**Staleness is not recomputed in the UI.** `medication_thread_attestation.stale` is computed in the
database from the ADR-0049 set-commitment compare. The view model reads it; it never re-derives it.
A second implementation of staleness would be a second answer to a safety question.

## 6. Paper-parity benchmark (§1.2)

**Paper counterpart:** the **inpatient drug chart** — review the list, sign the lines that lack the
responsible clinician's signature, and strike a drug that is being stopped.

**Steps:**

| Act | Paper *N* | Architecture-forced *M* | UI bundling target *K* |
|---|---|---|---|
| Review a 5-drug list, sign 3 unsigned/stale lines | 3 | **1** | **1** |
| Cease one drug | 2 (strike + initial/date) | **1** | **1** |

`M ≤ N` on both limbs, so there is no architecture defect to file under the #217 rule. The N
per-thread attestations a 3-target sign-off authors are a cryptographic artifact of ADR-0049's
set-commitment model, not human acts: `attest_thread_in_tx` takes an already-unsealed key by
reference, so one unseal and one transaction cover all N. This is exactly the argument #288 made, now
discharged in code.

**Time + cognitive load:** provisional budget — chart open → list rendered → unsigned lines signed
**≤ 15 s** for a 5-drug list; one cease **≤ 5 s**. These are seeded figures, **not** measured ones,
and they are explicitly superseded by the observed p95 that §7's capture produces on each premise.
The benchmark's cognitive-load limb is the visible per-row vouch state: the clinician never has to
hold in their head which lines they have already signed.

Measurement excludes finding the patient (see gap 1) and says so in every recorded run.

## 7. Gesture timing — self-learning, aggregate-only

A guessed budget is a magic number. The honest version measures what the gesture actually costs on
*this* premise and lets the observed figure replace the seed.

**The hazard, and why the shape below is the design rather than a caveat.** Per-clinician gesture
timings are a productivity-surveillance dataset. Captured naively — user, gesture, duration,
timestamp — that table is precisely what a hostile administrator or an acquiring vendor would use to
rank clinicians by speed, sitting inside a node the clinician cannot audit. Shipping a ready-made
monitoring substrate as a side effect of a paper-parity benchmark would be an anti-capture project
arming its own opponents. It is also a clinical-safety hazard: clinicians who know they are timed
rush the review step the sign-off exists to force.

The mechanism is therefore built so the misuse is **structurally unavailable**, not merely
discouraged:

```
ui_gesture_timing            node-local; NEVER synced; NEVER the clinical event stream
    gesture_kind   'signoff' | 'cease'
    size_bucket    '1-3' | '4-8' | '9+'
    n              sample count
    p50_ms, p95_ms running estimates, updated at capture time
    PRIMARY KEY (gesture_kind, size_bucket)
```

- **No `user_id`, no `patient_id`, no per-sample rows, no timestamps.** There is nothing to
  re-identify because the identifying columns never exist. This is the same category rule the shell
  design applies to the manifest: UI-tier data must never ride the append-only signed clinical
  stream, and here it additionally must never become a person-level record at all.
- **Site-scoped, not person-scoped.** The premise gets its real number; no individual is rankable.
- The aggregation is a **pure function** (`fold_sample(prev, duration) -> next`) under `cargo test`;
  the table is a node-local preference-tier table, written through the same kind of loader the
  manifest uses.

The recorded p95 is what a later revision of §6 commits to as the real budget.

## 8. Deliberate gaps — stated, not hidden

1. **No patient picker.** The window launches with `--patient <uuid>`; the §5.3/§5.8
   search-before-create funnel is unbuilt. The benchmark measures from *chart open* and records that
   exclusion in every run.
2. **A nil list cannot be signed off.** "No regular medications, reviewed" is a clinically meaningful
   act on paper with no record-layer home here: zero threads means zero attestations. This is
   ADR-0049's deferred "list reviewed at T" summary event. Filed as
   [#331](https://github.com/cairn-ehr/cairn-ehr/issues/331), not built.
3. **The pane/routing/freshness state machine is kept but unwired.** This slice renders a single-tab
   window. It is tested, documented code awaiting the next slice — not dead code.
4. **No native API.** The DB-direct read is the ADR-0021 privilege gradient, recorded in §2.
5. **DOM accessibility is verified by an operator, not by CI.** The TS renderer is a pure function of
   `SemanticNode`, so Rust owns the contract; the live screen-reader pass is a recorded operator step
   as in Spike 0004. Automating it needs a JavaScript toolchain in CI — a supply-chain and
   license-audit decision of its own, filed as
   [#332](https://github.com/cairn-ehr/cairn-ehr/issues/332) rather than smuggled into this slice.
6. **Dose editing, prescribing, and reconciliation from the UI are absent.** Read, sign off, cease.

## 9. Testing

**Pure — no database, no window** (`cairn-gui-tab-medications`):
- vouch-badge derivation for absent / fresh / stale, including the group rollup over member threads;
- `sign_off_targets`, including the load-bearing case **a fresh vouch by another clinician is
  skipped**, and the case where one member of a reconciled group is stale and another is fresh;
- ceased rows are excluded from targeting;
- the empty list yields an empty target set and a disabled gesture (gap 2's honest surface);
- `SemanticNode::assert_complete()` over the rendered contract — every focusable control labelled;
- `fold_sample` aggregation, including the first sample and bucket boundaries.

**DB-gated** (`CAIRN_TEST_PG`, existing serialized harness, in `crates/cairn-node/tests/`):
- `read.rs` against real projections: a plain thread, a reconciled group, a group with a struck
  coding, a ceased thread, and a group carrying a reconciliation flag;
- `signoff.rs`: N attestations land in **one transaction**, all-or-nothing on failure; a thread with
  a fresh vouch is left untouched; the reviewed_commitment pinned matches the thread's state at
  sign-off time.

**Not automated, recorded instead:** the live screen-reader pass and the timing runs, both written to
`cairn-gui/cairn-gui-tauri/results/` following the Spike 0004 pattern.

## 10. Risks

- **The group/thread join is the defect-prone seam.** A row is a group; a vouch is a thread. Getting
  this wrong shows a green "signed" badge over an unsigned thread — a silent clinical falsehood.
  Mitigated by dedicated tests on mixed-freshness groups.
- **Session key held in memory widens the unattended-workstation window.** Paper has the same failure
  (an unattended open chart), so this is parity, not regression; the 15-minute idle re-lock (§4) is
  the mitigation, pinned by a test so changing it is a visible decision.
- **Retiring iced touches a merged artifact.** The spike's *findings* live in
  [spike 0004](../../spikes/0004-iced-reference-ui-viability.md) and eco-eval 0004, which are
  untouched; only the rendering code goes. Expected side benefit: the `wayland-scanner` →
  `quick-xml` advisory chain behind
  [#252](https://github.com/cairn-ehr/cairn-ehr/issues/252) should disappear — **to be verified with
  `cargo deny`, not assumed.**
- **Scope.** This is the largest single slice in the clinical build so far. If it cannot land whole,
  the read path plus `sign_off_medication_list` plus their CLI verbs are independently valuable and
  ship first; the window follows. Any remainder is stated in HANDOVER, never quietly dropped.
