# Clinician Reference GUI — Shell Slice 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first runnable slice of the Cairn clinician reference GUI — a two-pane splittable shell with cross-pane routing, a pinned identity card, a Note tab and a Demographics tab, running on a mock data port — which also serves as the vehicle to run (widened) Spike 0004.

**Architecture:** A standalone Cargo workspace `cairn-gui/`, detached from the node's dependency tree (iced/winit/cosmic-text must never reach `cairn-node`). The bulk of the logic — the tab contract, the accessibility model, the `ClinicalData` port, the manifest loader/merge, and the pane/tab/routing/freshness state — lives in **iced-free crates that are unit-tested headlessly in CI**. iced touches only the shell binary and the thin `view()` bodies, behind a `gui` feature.

**Tech Stack:** Rust (edition 2021, rust ≥ 1.96), iced 0.14 (`pane_grid` for the resizable split, `iced::application` entry, `Task` for async), `cargo test` for the pure core, operator-run passes for a11y/IME/latency.

## Global Constraints

- **License:** AGPL-3.0-only; every dependency must be AGPL-compatible — verify before adding (house rule 1). iced (MIT) and cosmic-text (MIT) are already cleared in eco-eval 0004.
- **Rust floor:** rust-version = "1.96" (matches root workspace / rust-toolchain.toml).
- **Standalone workspace:** `cairn-gui/` is NOT a member of the root `/Cargo.toml`; it is its own workspace root and is added to the root's `exclude` list. iced/winit/cosmic-text never enter the node tree (ADR-0021 / §9.5).
- **iced containment:** only `cairn-gui-shell` and the `view()` bodies in the per-tab crates may depend on iced, and only behind a `gui` feature. `cairn-gui-tab`, `cairn-gui-data`, `cairn-gui-manifest`, and the shell's pane/routing/freshness logic are iced-free and CI-testable with plain `cargo test`.
- **The UI never signs or writes events.** All data access is reads through the `ClinicalData` port; slice 1 uses the mock impl only (no node, per spec §8).
- **Every focusable control carries a role + non-empty accessible label** — enforced by CI tests over the semantic contract (spec §4, §9).
- **No fixed pixels** for layout dimensions; sizing is relative / ratio-based (spec §2).
- **TDD, DRY, YAGNI, frequent commits.** Failing test first, minimal code, commit per task.
- **Slice-1 simplification (recorded):** the manifest is loaded from an **in-memory/TOML fixture source** behind a `ManifestSource` trait; the node-local **Postgres** source (spec §7) is a later slice. The self-repair + site/role⊕user merge logic is fully implemented and tested now; only the storage backend is deferred. This is consistent with §8 (mock, no node).

---

### Task 1: Scaffold the standalone `cairn-gui/` workspace

**Files:**
- Create: `cairn-gui/Cargo.toml` (workspace root)
- Create: `cairn-gui/cairn-gui-tab/Cargo.toml`, `cairn-gui/cairn-gui-tab/src/lib.rs`
- Create: `cairn-gui/cairn-gui-data/Cargo.toml`, `cairn-gui/cairn-gui-data/src/lib.rs`
- Create: `cairn-gui/cairn-gui-manifest/Cargo.toml`, `cairn-gui/cairn-gui-manifest/src/lib.rs`
- Modify: `/Cargo.toml` (add `cairn-gui` to `exclude`)

**Interfaces:**
- Produces: three iced-free library crates (`cairn_gui_tab`, `cairn_gui_data`, `cairn_gui_manifest`) that compile and run an empty test suite.

- [ ] **Step 1: Create the workspace root `cairn-gui/Cargo.toml`**

```toml
# Standalone workspace: iced/winit/cosmic-text must never enter the cairn-node
# dependency tree (ADR-0021 / §9.5). Detached from the root /Cargo.toml, which
# lists this dir in its `exclude`.
[workspace]
resolver = "2"
members = [
    "cairn-gui-tab",
    "cairn-gui-data",
    "cairn-gui-manifest",
]

[workspace.package]
edition = "2021"
rust-version = "1.96"
license = "AGPL-3.0-only"
repository = "https://github.com/cairn-ehr/cairn-ehr"
publish = false
```

- [ ] **Step 2: Create the three crate manifests**

`cairn-gui/cairn-gui-tab/Cargo.toml`:
```toml
[package]
name = "cairn-gui-tab"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false
```
Repeat identically for `cairn-gui-data` (name `cairn-gui-data`) and `cairn-gui-manifest` (name `cairn-gui-manifest`).

- [ ] **Step 3: Create placeholder lib roots with a smoke test**

In each crate's `src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {
        assert_eq!(2 + 2, 4);
    }
}
```

- [ ] **Step 4: Add `cairn-gui` to the root workspace `exclude`**

Modify `/Cargo.toml` line 8:
```toml
exclude = ["extensions/cairn_pgx", "cairn-gui"]
```

- [ ] **Step 5: Verify the workspace builds and the node tree is unaffected**

Run: `cd cairn-gui && cargo test`
Expected: PASS (3 crates, `crate_compiles` passes).

Run: `cd .. && cargo tree -p cairn-node 2>/dev/null | grep -i iced || echo "no iced in node tree"`
Expected: `no iced in node tree`

- [ ] **Step 6: Commit**

```bash
git add cairn-gui Cargo.toml
git commit -m "feat(gui): scaffold standalone cairn-gui workspace (iced-free core crates)"
```

---

### Task 2: The semantic / accessibility contract (`cairn-gui-tab`)

Port the proven `FormModel`/`Field`/`Role` contract from `poc/iced-ui-spike/src/form.rs` forward as the shared, mandatory accessibility contract every tab and shell-chrome element declares.

**Files:**
- Create: `cairn-gui/cairn-gui-tab/src/semantics.rs`
- Modify: `cairn-gui/cairn-gui-tab/src/lib.rs`

**Interfaces:**
- Produces: `Role` (enum: `Heading | TextInput | Button | List | ListItem | Tab | Pane | Divider`), `Field { id: String, role: Role, label: String }`, `SemanticNode { title: String, fields: Vec<Field> }`, `Role::is_focusable() -> bool`, `SemanticNode::assert_complete() -> Result<(), String>`.

- [ ] **Step 1: Write the failing test**

`cairn-gui/cairn-gui-tab/src/semantics.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_focusable_field_must_have_a_nonempty_label() {
        let node = SemanticNode {
            title: "test".into(),
            fields: vec![Field { id: "a".into(), role: Role::Button, label: "".into() }],
        };
        assert!(node.assert_complete().is_err(), "empty label on a focusable control must fail");
    }

    #[test]
    fn complete_node_passes() {
        let node = SemanticNode {
            title: "test".into(),
            fields: vec![Field { id: "a".into(), role: Role::Button, label: "Save".into() }],
        };
        assert!(node.assert_complete().is_ok());
    }

    #[test]
    fn duplicate_ids_fail() {
        let node = SemanticNode {
            title: "t".into(),
            fields: vec![
                Field { id: "a".into(), role: Role::Button, label: "One".into() },
                Field { id: "a".into(), role: Role::Button, label: "Two".into() },
            ],
        };
        assert!(node.assert_complete().is_err(), "duplicate ids must fail");
    }

    #[test]
    fn structural_roles_are_not_focusable() {
        assert!(!Role::Heading.is_focusable());
        assert!(!Role::List.is_focusable());
        assert!(Role::TextInput.is_focusable());
        assert!(Role::Tab.is_focusable());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd cairn-gui && cargo test -p cairn-gui-tab semantics`
Expected: FAIL (cannot find `SemanticNode`, `Field`, `Role`).

- [ ] **Step 3: Write the implementation**

At the top of `cairn-gui/cairn-gui-tab/src/semantics.rs`:
```rust
//! The pure accessibility contract shared by every tab and every piece of shell
//! chrome. It is *data*: declared here, unit-tested for completeness in CI, and
//! rendered by the iced layer. A divergence between "what we declared" and "what
//! a screen reader announces" is then visible (the shell's `--dump-a11y` prints
//! this tree; the operator confirms it against Orca/NVDA). Zero GUI dependency.

/// Accessibility role, mapped to what AccessKit / a screen reader announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Heading,
    TextInput,
    Button,
    List,
    ListItem,
    /// A tab in a pane's tab strip (shell chrome).
    Tab,
    /// A workspace pane container (shell chrome).
    Pane,
    /// The draggable divider between panes (shell chrome).
    Divider,
}

impl Role {
    /// Focusable = a keyboard tab stop. Structural roles (headings, lists, panes)
    /// are announced but not stops. A1 requires labels on focusable controls;
    /// A2 requires each be keyboard-reachable.
    pub fn is_focusable(self) -> bool {
        matches!(self, Role::TextInput | Role::Button | Role::Tab | Role::Divider)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Heading => "heading",
            Role::TextInput => "text_input",
            Role::Button => "button",
            Role::List => "list",
            Role::ListItem => "list_item",
            Role::Tab => "tab",
            Role::Pane => "pane",
            Role::Divider => "divider",
        }
    }
}

/// One control in an accessibility contract.
#[derive(Debug, Clone)]
pub struct Field {
    pub id: String,
    pub role: Role,
    pub label: String,
}

/// A tab's (or shell-chrome region's) accessibility tree.
#[derive(Debug, Clone)]
pub struct SemanticNode {
    pub title: String,
    pub fields: Vec<Field>,
}

impl SemanticNode {
    /// CI-checkable form of accessibility claim A1: every focusable control has a
    /// non-empty label, and ids are unique. Returns Err(description) on the first
    /// violation so a failing test names the offender.
    pub fn assert_complete(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for f in &self.fields {
            if !seen.insert(&f.id) {
                return Err(format!("duplicate field id: {}", f.id));
            }
            if f.role.is_focusable() && f.label.trim().is_empty() {
                return Err(format!("focusable control {} has an empty label", f.id));
            }
        }
        Ok(())
    }
}
```

Add to `cairn-gui/cairn-gui-tab/src/lib.rs`:
```rust
pub mod semantics;
pub use semantics::{Field, Role, SemanticNode};
```
(Remove the placeholder `crate_compiles` test.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd cairn-gui && cargo test -p cairn-gui-tab`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add cairn-gui/cairn-gui-tab
git commit -m "feat(gui): accessibility/semantic contract with CI-checked completeness"
```

---

### Task 3: Context, Capabilities, and the `Tab` contract types (`cairn-gui-tab`)

**Files:**
- Create: `cairn-gui/cairn-gui-tab/src/context.rs`
- Create: `cairn-gui/cairn-gui-tab/src/tab.rs`
- Modify: `cairn-gui/cairn-gui-tab/src/lib.rs`

**Interfaces:**
- Consumes: `SemanticNode` (Task 2).
- Produces:
  - `TabId(pub String)` — stable, addressable tab identity.
  - `Capability` enum: `ViewDemographics | ViewNote | ViewMeds | ViewResults` (extend later).
  - `Capabilities { granted: HashSet<Capability> }` with `allows(&self, Capability) -> bool` and `clinician_all() -> Capabilities` (the stub resolver).
  - `PatientRef { uuid: String, display_name: String }`, `UserRef { actor_id: String, display_name: String }`.
  - `Context { patient: Option<PatientRef>, user: UserRef, capabilities: Capabilities }`.
  - `Intent` enum: `OpenTab(TabId)` (open a tab, resolving to the opposite pane).
  - `Outcome<M> { pub follow_up: Option<M>, pub intents: Vec<Intent> }` with `Outcome::none()`, `Outcome::message(M)`, `Outcome::intent(Intent)`.

- [ ] **Step 1: Write the failing test**

`cairn-gui/cairn-gui-tab/src/context.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clinician_stub_grants_all_known_capabilities() {
        let caps = Capabilities::clinician_all();
        for cap in [
            Capability::ViewDemographics,
            Capability::ViewNote,
            Capability::ViewMeds,
            Capability::ViewResults,
        ] {
            assert!(caps.allows(cap), "clinician stub should grant {cap:?}");
        }
    }

    #[test]
    fn empty_capabilities_deny() {
        let caps = Capabilities::default();
        assert!(!caps.allows(Capability::ViewMeds));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd cairn-gui && cargo test -p cairn-gui-tab context`
Expected: FAIL (types not defined).

- [ ] **Step 3: Write `context.rs`**

```rust
//! The read-only context the shell hands to every tab. `capabilities` is the
//! ENTIRE seam to the future role/policy subsystem (spec §6): today a stub grants
//! a clinician everything; later the resolver is replaced and nothing else changes.
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    ViewDemographics,
    ViewNote,
    ViewMeds,
    ViewResults,
}

#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    granted: HashSet<Capability>,
}

impl Capabilities {
    /// The slice-1 stub resolver: a clinician sees everything. The real resolver
    /// (subsystem B) replaces only this function.
    pub fn clinician_all() -> Self {
        Self {
            granted: [
                Capability::ViewDemographics,
                Capability::ViewNote,
                Capability::ViewMeds,
                Capability::ViewResults,
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn allows(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatientRef {
    pub uuid: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRef {
    pub actor_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct Context {
    pub patient: Option<PatientRef>,
    pub user: UserRef,
    pub capabilities: Capabilities,
}
```

- [ ] **Step 4: Write `tab.rs` (the trait + routing types, no test needed yet — exercised in Task 6/7)**

```rust
//! The Tab contract. A tab is a self-contained view that declares its
//! accessibility contract, lazily loads its own data, and may ask the shell to
//! route (open something in the other pane). The Message/Task/Element associated
//! types are the ONLY iced surface — deliberately small (spec §4).
use crate::context::Context;
use crate::semantics::SemanticNode;

/// Stable, addressable identity for a tab kind (deep links target this).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TabId(pub String);

/// A shell routing request emitted by a tab's update().
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Open this tab; the shell resolves it into the OPPOSITE pane (spec §5).
    OpenTab(TabId),
}

/// What a tab's update() returns: an optional follow-up message and/or shell intents.
#[derive(Debug, Clone)]
pub struct Outcome<M> {
    pub follow_up: Option<M>,
    pub intents: Vec<Intent>,
}

impl<M> Outcome<M> {
    pub fn none() -> Self {
        Self { follow_up: None, intents: Vec::new() }
    }
    pub fn message(m: M) -> Self {
        Self { follow_up: Some(m), intents: Vec::new() }
    }
    pub fn intent(i: Intent) -> Self {
        Self { follow_up: None, intents: vec![i] }
    }
}

/// The accessibility contract accessor every tab must implement, iced-free so it
/// is CI-testable. (The full Tab trait with iced view()/update() lives in the
/// shell crate behind the `gui` feature — see Task 7/8 — to keep this crate
/// iced-free.)
pub trait Semantic {
    fn tab_id(&self) -> TabId;
    fn title(&self) -> String;
    fn semantics(&self, ctx: &Context) -> SemanticNode;
}
```

- [ ] **Step 5: Wire modules in `lib.rs`**

```rust
pub mod semantics;
pub mod context;
pub mod tab;
pub use semantics::{Field, Role, SemanticNode};
pub use context::{Capabilities, Capability, Context, PatientRef, UserRef};
pub use tab::{Intent, Outcome, Semantic, TabId};
```

- [ ] **Step 6: Run tests and commit**

Run: `cd cairn-gui && cargo test -p cairn-gui-tab`
Expected: PASS.
```bash
git add cairn-gui/cairn-gui-tab
git commit -m "feat(gui): Context/Capabilities stub + Tab contract types (routing seam)"
```

---

### Task 4: The `ClinicalData` port + mock impl (`cairn-gui-data`)

**Files:**
- Create: `cairn-gui/cairn-gui-data/src/port.rs`
- Create: `cairn-gui/cairn-gui-data/src/mock.rs`
- Modify: `cairn-gui/cairn-gui-data/Cargo.toml` (dep on `cairn-gui-tab`), `cairn-gui/cairn-gui-data/src/lib.rs`

**Interfaces:**
- Consumes: `PatientRef` (Task 3).
- Produces:
  - `Demographics { patient: PatientRef, sex: String, birth_date: String, identifiers: Vec<(String, String)> }`.
  - `NoteRef { id: String, one_line: String }` — a cross-reference summary (the "see X-ray report" link payload).
  - `DataError` enum: `NotFound | Unavailable(String)`.
  - `trait ClinicalData { fn demographics(&self, patient_uuid: &str) -> Result<Demographics, DataError>; fn note_refs(&self, patient_uuid: &str) -> Result<Vec<NoteRef>, DataError>; }`
  - `MockData::with_fixtures() -> MockData` implementing `ClinicalData` (one fixture patient, multi-script name, two identifiers, one cross-reference).

- [ ] **Step 1: Add the dependency**

`cairn-gui/cairn-gui-data/Cargo.toml`:
```toml
[dependencies]
cairn-gui-tab = { path = "../cairn-gui-tab" }
```

- [ ] **Step 2: Write the failing test**

`cairn-gui/cairn-gui-data/src/mock.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::ClinicalData;

    const FIXTURE_UUID: &str = "00000000-0000-0000-0000-0000000000aa";

    #[test]
    fn mock_returns_fixture_demographics() {
        let data = MockData::with_fixtures();
        let d = data.demographics(FIXTURE_UUID).expect("fixture patient exists");
        assert_eq!(d.patient.uuid, FIXTURE_UUID);
        assert!(!d.identifiers.is_empty(), "fixture has at least one identifier");
    }

    #[test]
    fn mock_unknown_patient_is_not_found() {
        let data = MockData::with_fixtures();
        assert!(matches!(data.demographics("no-such"), Err(crate::port::DataError::NotFound)));
    }

    #[test]
    fn mock_has_a_cross_reference_note() {
        let data = MockData::with_fixtures();
        let refs = data.note_refs(FIXTURE_UUID).unwrap();
        assert!(!refs.is_empty(), "fixture provides a cross-reference for the note→pane demo");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd cairn-gui && cargo test -p cairn-gui-data`
Expected: FAIL (types not defined).

- [ ] **Step 4: Write `port.rs`**

```rust
//! The single read-port the UI uses. Real impl → node's native-API client (later
//! slice); mock impl → fixtures (this slice). The UI never writes/signs (§9.6).
use cairn_gui_tab::PatientRef;

#[derive(Debug, Clone)]
pub struct Demographics {
    pub patient: PatientRef,
    pub sex: String,
    pub birth_date: String,
    /// (system, value), e.g. ("MRN", "12345").
    pub identifiers: Vec<(String, String)>,
}

/// A one-line cross-reference summary — the payload behind a "see X-ray report"
/// link that opens the target in the other pane (spec §5).
#[derive(Debug, Clone)]
pub struct NoteRef {
    pub id: String,
    pub one_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    NotFound,
    Unavailable(String),
}

pub trait ClinicalData {
    fn demographics(&self, patient_uuid: &str) -> Result<Demographics, DataError>;
    fn note_refs(&self, patient_uuid: &str) -> Result<Vec<NoteRef>, DataError>;
}
```

- [ ] **Step 5: Write `mock.rs` implementation (above the test module)**

```rust
//! Fixture-backed ClinicalData for slice 1 (no node). One patient with a
//! multi-script name to also feed the Spike 0004 shaping check.
use crate::port::{ClinicalData, DataError, Demographics, NoteRef};
use cairn_gui_tab::PatientRef;

const FIXTURE_UUID: &str = "00000000-0000-0000-0000-0000000000aa";

pub struct MockData {
    demographics: Demographics,
    note_refs: Vec<NoteRef>,
}

impl MockData {
    pub fn with_fixtures() -> Self {
        let patient = PatientRef {
            uuid: FIXTURE_UUID.to_string(),
            // Latin / Arabic / Devanagari / Han in one label feeds the IME/shaping pass.
            display_name: "Amina أمينة अमीना 阿明娜".to_string(),
        };
        Self {
            demographics: Demographics {
                patient,
                sex: "female".to_string(),
                birth_date: "1984-03-02".to_string(),
                identifiers: vec![
                    ("MRN".to_string(), "12345".to_string()),
                    ("National".to_string(), "QLD-998877".to_string()),
                ],
            },
            note_refs: vec![NoteRef {
                id: "xray-2026-07-01".to_string(),
                one_line: "Chest X-ray 2026-07-01 — no acute abnormality".to_string(),
            }],
        }
    }
}

impl ClinicalData for MockData {
    fn demographics(&self, patient_uuid: &str) -> Result<Demographics, DataError> {
        if patient_uuid == self.demographics.patient.uuid {
            Ok(self.demographics.clone())
        } else {
            Err(DataError::NotFound)
        }
    }

    fn note_refs(&self, patient_uuid: &str) -> Result<Vec<NoteRef>, DataError> {
        if patient_uuid == self.demographics.patient.uuid {
            Ok(self.note_refs.clone())
        } else {
            Err(DataError::NotFound)
        }
    }
}
```

`cairn-gui/cairn-gui-data/src/lib.rs`:
```rust
pub mod port;
pub mod mock;
pub use port::{ClinicalData, DataError, Demographics, NoteRef};
pub use mock::MockData;
```

- [ ] **Step 6: Run tests and commit**

Run: `cd cairn-gui && cargo test -p cairn-gui-data`
Expected: PASS (3 tests).
```bash
git add cairn-gui/cairn-gui-data
git commit -m "feat(gui): ClinicalData port + fixture-backed mock impl"
```

---

### Task 5: Manifest loader with self-repair and site/role⊕user merge (`cairn-gui-manifest`)

**Files:**
- Create: `cairn-gui/cairn-gui-manifest/src/model.rs`
- Create: `cairn-gui/cairn-gui-manifest/src/merge.rs`
- Modify: `cairn-gui/cairn-gui-manifest/Cargo.toml`, `cairn-gui/cairn-gui-manifest/src/lib.rs`

**Interfaces:**
- Consumes: `TabId` (Task 3).
- Produces:
  - `SiteManifest { offered: Vec<TabId>, rail: Vec<TabId>, default_left: TabId, default_right: TabId }`.
  - `UserPrefs { divider_ratio: f32, left_tabs: Vec<TabId>, right_tabs: Vec<TabId>, active_left: Option<TabId>, active_right: Option<TabId> }` with `UserPrefs::default()`.
  - `EffectiveManifest { rail: Vec<TabId>, divider_ratio: f32, left_tabs: Vec<TabId>, right_tabs: Vec<TabId>, active_left: TabId, active_right: TabId }`.
  - `fn merge(site: &SiteManifest, user: &UserPrefs) -> EffectiveManifest` — user may only arrange/choose among `site.offered`; anything not offered is dropped; a clamped `divider_ratio` in `[0.1, 0.9]`; falls back to site defaults when user lists are empty/invalid.
  - `fn repair_ratio(r: f32) -> f32`.

- [ ] **Step 1: Write the failing tests**

`cairn-gui/cairn-gui-manifest/src/merge.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SiteManifest, UserPrefs};
    use cairn_gui_tab::TabId;

    fn site() -> SiteManifest {
        SiteManifest {
            offered: vec![TabId("note".into()), TabId("demographics".into())],
            rail: vec![TabId("note".into()), TabId("demographics".into())],
            default_left: TabId("note".into()),
            default_right: TabId("demographics".into()),
        }
    }

    #[test]
    fn user_cannot_surface_a_tab_the_site_does_not_offer() {
        let mut prefs = UserPrefs::default();
        prefs.left_tabs = vec![TabId("billing".into())]; // not offered
        let eff = merge(&site(), &prefs);
        assert!(!eff.left_tabs.contains(&TabId("billing".into())),
            "unoffered tab must be dropped (soft policy stays within soft policy)");
    }

    #[test]
    fn empty_user_prefs_fall_back_to_site_defaults() {
        let eff = merge(&site(), &UserPrefs::default());
        assert_eq!(eff.active_left, TabId("note".into()));
        assert_eq!(eff.active_right, TabId("demographics".into()));
    }

    #[test]
    fn divider_ratio_is_clamped() {
        assert_eq!(repair_ratio(0.0), 0.1);
        assert_eq!(repair_ratio(5.0), 0.9);
        assert_eq!(repair_ratio(f32::NAN), 0.5);
        assert!((repair_ratio(0.4) - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn user_may_reorder_offered_tabs() {
        let mut prefs = UserPrefs::default();
        prefs.left_tabs = vec![TabId("demographics".into()), TabId("note".into())];
        let eff = merge(&site(), &prefs);
        assert_eq!(eff.left_tabs, vec![TabId("demographics".into()), TabId("note".into())]);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd cairn-gui && cargo test -p cairn-gui-manifest`
Expected: FAIL (types not defined).

- [ ] **Step 3: Add dep + write `model.rs`**

`cairn-gui/cairn-gui-manifest/Cargo.toml`:
```toml
[dependencies]
cairn-gui-tab = { path = "../cairn-gui-tab" }
```

`cairn-gui/cairn-gui-manifest/src/model.rs`:
```rust
//! Manifest data model. Two layers (spec §7): the site/role layer (what tabs are
//! OFFERED — the seam to subsystem B) and the per-user layer (personal layout,
//! remember-last-state). Storage is deferred to a later slice (Postgres); this
//! slice merges/repairs in memory.
use cairn_gui_tab::TabId;

#[derive(Debug, Clone)]
pub struct SiteManifest {
    pub offered: Vec<TabId>,
    pub rail: Vec<TabId>,
    pub default_left: TabId,
    pub default_right: TabId,
}

#[derive(Debug, Clone)]
pub struct UserPrefs {
    pub divider_ratio: f32,
    pub left_tabs: Vec<TabId>,
    pub right_tabs: Vec<TabId>,
    pub active_left: Option<TabId>,
    pub active_right: Option<TabId>,
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self {
            divider_ratio: 0.5,
            left_tabs: Vec::new(),
            right_tabs: Vec::new(),
            active_left: None,
            active_right: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EffectiveManifest {
    pub rail: Vec<TabId>,
    pub divider_ratio: f32,
    pub left_tabs: Vec<TabId>,
    pub right_tabs: Vec<TabId>,
    pub active_left: TabId,
    pub active_right: TabId,
}
```

- [ ] **Step 4: Write `merge.rs` (above the test module)**

```rust
//! The self-repairing merge: the user layer may only arrange/choose among what the
//! site layer offers; invalid or missing values fall back to site defaults. This
//! keeps soft policy within soft policy (the hard access gate is the DB floor).
use crate::model::{EffectiveManifest, SiteManifest, UserPrefs};
use cairn_gui_tab::TabId;

/// Clamp a divider ratio into a sane, both-panes-visible range; NaN → 0.5.
pub fn repair_ratio(r: f32) -> f32 {
    if r.is_nan() {
        0.5
    } else {
        r.clamp(0.1, 0.9)
    }
}

/// Keep only tabs the site offers, preserving the user's order; if the result is
/// empty, fall back to the given site default (a single tab).
fn filter_to_offered(user: &[TabId], offered: &[TabId], fallback: &TabId) -> Vec<TabId> {
    let filtered: Vec<TabId> = user.iter().filter(|t| offered.contains(t)).cloned().collect();
    if filtered.is_empty() {
        vec![fallback.clone()]
    } else {
        filtered
    }
}

pub fn merge(site: &SiteManifest, user: &UserPrefs) -> EffectiveManifest {
    let left_tabs = filter_to_offered(&user.left_tabs, &site.offered, &site.default_left);
    let right_tabs = filter_to_offered(&user.right_tabs, &site.offered, &site.default_right);

    // Active tab: honour the user's choice only if it survived filtering, else the
    // first tab in that pane.
    let active_left = user
        .active_left
        .clone()
        .filter(|t| left_tabs.contains(t))
        .unwrap_or_else(|| left_tabs[0].clone());
    let active_right = user
        .active_right
        .clone()
        .filter(|t| right_tabs.contains(t))
        .unwrap_or_else(|| right_tabs[0].clone());

    // Rail is site-controlled but never shows an unoffered tab.
    let rail = site.rail.iter().filter(|t| site.offered.contains(t)).cloned().collect();

    EffectiveManifest {
        rail,
        divider_ratio: repair_ratio(user.divider_ratio),
        left_tabs,
        right_tabs,
        active_left,
        active_right,
    }
}
```

`cairn-gui/cairn-gui-manifest/src/lib.rs`:
```rust
pub mod model;
pub mod merge;
pub use model::{EffectiveManifest, SiteManifest, UserPrefs};
pub use merge::{merge, repair_ratio};
```

- [ ] **Step 5: Run tests and commit**

Run: `cd cairn-gui && cargo test -p cairn-gui-manifest`
Expected: PASS (4 tests).
```bash
git add cairn-gui/cairn-gui-manifest
git commit -m "feat(gui): manifest model + self-repairing site/role⊕user merge"
```

---

### Task 6: Pane/tab/routing/freshness state machine (`cairn-gui-shell`, iced-free module)

The safety-relevant shell logic — which pane holds which tabs, resolving `OpenTab` into the *opposite* pane, and the freshness/staleness rules — is a **pure state machine**, tested headlessly. iced consumes it in Task 8.

**Files:**
- Create: `cairn-gui/cairn-gui-shell/Cargo.toml`
- Create: `cairn-gui/cairn-gui-shell/src/workspace.rs`
- Create: `cairn-gui/cairn-gui-shell/src/freshness.rs`
- Create: `cairn-gui/cairn-gui-shell/src/lib.rs`
- Modify: `cairn-gui/Cargo.toml` (add member)

**Interfaces:**
- Consumes: `TabId` (Task 3), `EffectiveManifest` (Task 5).
- Produces:
  - `enum Side { Left, Right }` with `Side::opposite(self) -> Side`.
  - `Workspace` holding per-side `Vec<TabId>` + active `TabId`, built via `Workspace::from_manifest(&EffectiveManifest)`.
  - `Workspace::open_in_opposite(&mut self, from: Side, tab: TabId) -> Side` — the cross-pane routing rule (spec §5): opens `tab` in the opposite pane, adds it if absent, makes it active, returns the pane it landed in.
  - `Workspace::activate(&mut self, side: Side, tab: &TabId)`.
  - `enum Freshness { Fresh, Stale }`; `struct Loaded { at_tick: u64 }`; `fn freshness(loaded: &Loaded, now_tick: u64, ttl: u64) -> Freshness` — pure age rule. On-screen data is never auto-swapped; the shell uses this only to raise the stale FLAG (spec §6).

- [ ] **Step 1: Create the crate + register it**

`cairn-gui/cairn-gui-shell/Cargo.toml`:
```toml
[package]
name = "cairn-gui-shell"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
cairn-gui-tab = { path = "../cairn-gui-tab" }
cairn-gui-data = { path = "../cairn-gui-data" }
cairn-gui-manifest = { path = "../cairn-gui-manifest" }
# iced is added behind the `gui` feature in Task 8.

[features]
default = []
gui = []
```

Add `"cairn-gui-shell"` to `members` in `cairn-gui/Cargo.toml`.

- [ ] **Step 2: Write the failing tests**

`cairn-gui/cairn-gui-shell/src/workspace.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_gui_manifest::EffectiveManifest;
    use cairn_gui_tab::TabId;

    fn eff() -> EffectiveManifest {
        EffectiveManifest {
            rail: vec![TabId("note".into()), TabId("demographics".into())],
            divider_ratio: 0.5,
            left_tabs: vec![TabId("note".into())],
            right_tabs: vec![TabId("demographics".into())],
            active_left: TabId("note".into()),
            active_right: TabId("demographics".into()),
        }
    }

    #[test]
    fn open_in_opposite_from_left_lands_on_right_and_activates() {
        let mut ws = Workspace::from_manifest(&eff());
        let landed = ws.open_in_opposite(Side::Left, TabId("xray".into()));
        assert_eq!(landed, Side::Right);
        assert_eq!(ws.active(Side::Right), &TabId("xray".into()));
        assert!(ws.tabs(Side::Right).contains(&TabId("xray".into())));
    }

    #[test]
    fn open_in_opposite_does_not_duplicate_an_existing_tab() {
        let mut ws = Workspace::from_manifest(&eff());
        ws.open_in_opposite(Side::Left, TabId("demographics".into())); // already on right
        let count = ws.tabs(Side::Right).iter().filter(|t| **t == TabId("demographics".into())).count();
        assert_eq!(count, 1, "no duplicate tab");
    }

    #[test]
    fn side_opposite() {
        assert_eq!(Side::Left.opposite(), Side::Right);
        assert_eq!(Side::Right.opposite(), Side::Left);
    }
}
```

`cairn-gui/cairn-gui-shell/src/freshness.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_ttl_is_fresh() {
        assert_eq!(freshness(&Loaded { at_tick: 100 }, 150, 60), Freshness::Fresh);
    }

    #[test]
    fn beyond_ttl_is_stale() {
        assert_eq!(freshness(&Loaded { at_tick: 100 }, 200, 60), Freshness::Stale);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cd cairn-gui && cargo test -p cairn-gui-shell`
Expected: FAIL (types not defined).

- [ ] **Step 4: Implement `workspace.rs` and `freshness.rs`**

`workspace.rs` (above the test module):
```rust
//! Pure pane/tab model + the cross-pane routing rule. No iced. This is where the
//! "open the reference in the OTHER pane, leaving the note in place" behaviour
//! (spec §5) lives, so it is unit-tested in isolation.
use cairn_gui_manifest::EffectiveManifest;
use cairn_gui_tab::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

#[derive(Debug, Clone)]
struct Pane {
    tabs: Vec<TabId>,
    active: TabId,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    left: Pane,
    right: Pane,
    pub divider_ratio: f32,
}

impl Workspace {
    pub fn from_manifest(m: &EffectiveManifest) -> Self {
        Self {
            left: Pane { tabs: m.left_tabs.clone(), active: m.active_left.clone() },
            right: Pane { tabs: m.right_tabs.clone(), active: m.active_right.clone() },
            divider_ratio: m.divider_ratio,
        }
    }

    fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    pub fn tabs(&self, side: Side) -> &[TabId] {
        &self.pane(side).tabs
    }

    pub fn active(&self, side: Side) -> &TabId {
        &self.pane(side).active
    }

    pub fn activate(&mut self, side: Side, tab: &TabId) {
        let p = self.pane_mut(side);
        if p.tabs.contains(tab) {
            p.active = tab.clone();
        }
    }

    /// Cross-pane routing (spec §5): open `tab` in the pane OPPOSITE `from`, adding
    /// it if absent, and make it active. Returns the pane it landed in. Leaving the
    /// originating pane untouched is the whole point (the note stays put).
    pub fn open_in_opposite(&mut self, from: Side, tab: TabId) -> Side {
        let target = from.opposite();
        let p = self.pane_mut(target);
        if !p.tabs.contains(&tab) {
            p.tabs.push(tab.clone());
        }
        p.active = tab;
        target
    }
}
```

`freshness.rs` (above the test module):
```rust
//! Pure age rule for the stale FLAG. Deliberately minimal: the shell NEVER
//! silently swaps on-screen data (spec §6) — it uses this only to decide whether
//! to show the "stale — Refresh?" affordance on already-visible content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, Copy)]
pub struct Loaded {
    pub at_tick: u64,
}

pub fn freshness(loaded: &Loaded, now_tick: u64, ttl: u64) -> Freshness {
    if now_tick.saturating_sub(loaded.at_tick) > ttl {
        Freshness::Stale
    } else {
        Freshness::Fresh
    }
}
```

`cairn-gui/cairn-gui-shell/src/lib.rs`:
```rust
pub mod workspace;
pub mod freshness;
pub use workspace::{Side, Workspace};
pub use freshness::{freshness, Freshness, Loaded};
```

- [ ] **Step 5: Run tests and commit**

Run: `cd cairn-gui && cargo test -p cairn-gui-shell`
Expected: PASS (5 tests).
```bash
git add cairn-gui/cairn-gui-shell cairn-gui/Cargo.toml
git commit -m "feat(gui): pure pane/routing state machine + freshness flag rule"
```

---

### Task 7: The two tabs — Demographics & Note (semantics + a11y dump)

Each tab lives in its own crate (spec §3: one crate per tab). Slice 1 keeps them **iced-free at the contract level**: each implements `Semantic` (id/title/semantics) and a pure `load()` that turns mock-port data into display state. The iced `view()` is added in Task 8 behind the shell's `gui` feature.

**Files:**
- Create: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-demographics/{Cargo.toml, src/lib.rs}`
- Create: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-note/{Cargo.toml, src/lib.rs}`
- Modify: `cairn-gui/Cargo.toml` (add both members)

**Interfaces:**
- Consumes: `Context`, `Semantic`, `SemanticNode`, `Field`, `Role`, `TabId` (Tasks 2–3); `ClinicalData`, `Demographics`, `NoteRef` (Task 4).
- Produces:
  - `DemographicsTab { state: Option<Demographics> }` with `DemographicsTab::new()`, `fn load(&mut self, ctx: &Context, data: &dyn ClinicalData)`, and `impl Semantic`.
  - `NoteTab { refs: Vec<NoteRef> }` with `NoteTab::new()`, `fn load(&mut self, ctx: &Context, data: &dyn ClinicalData)`, `impl Semantic`. Note exposes each cross-reference as a focusable `Button` field whose id encodes the target (`open:<ref.id>`), so activating it maps to `Intent::OpenTab`.

- [ ] **Step 1: Write failing tests (demographics crate)**

`cairn-gui/cairn-gui-tabs/cairn-gui-tab-demographics/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_gui_data::MockData;
    use cairn_gui_tab::{Capabilities, Context, Semantic, UserRef};

    fn ctx() -> Context {
        Context {
            patient: Some(cairn_gui_tab::PatientRef {
                uuid: "00000000-0000-0000-0000-0000000000aa".into(),
                display_name: "Amina أمينة अमीना 阿明娜".into(),
            }),
            user: UserRef { actor_id: "clin-1".into(), display_name: "Dr Vega".into() },
            capabilities: Capabilities::clinician_all(),
        }
    }

    #[test]
    fn semantics_are_complete_after_load() {
        let mut tab = DemographicsTab::new();
        tab.load(&ctx(), &MockData::with_fixtures());
        let node = tab.semantics(&ctx());
        node.assert_complete().expect("every focusable control labelled");
        assert_eq!(tab.tab_id(), cairn_gui_tab::TabId("demographics".into()));
    }

    #[test]
    fn semantics_include_each_identifier() {
        let mut tab = DemographicsTab::new();
        tab.load(&ctx(), &MockData::with_fixtures());
        let node = tab.semantics(&ctx());
        let labels: Vec<String> = node.fields.iter().map(|f| f.label.clone()).collect();
        assert!(labels.iter().any(|l| l.contains("MRN")), "MRN identifier surfaced");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd cairn-gui && cargo test -p cairn-gui-tab-demographics`
Expected: FAIL (crate/types missing).

- [ ] **Step 3: Create the demographics crate**

`cairn-gui/cairn-gui-tabs/cairn-gui-tab-demographics/Cargo.toml`:
```toml
[package]
name = "cairn-gui-tab-demographics"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
cairn-gui-tab = { path = "../../cairn-gui-tab" }
cairn-gui-data = { path = "../../cairn-gui-data" }
```

`src/lib.rs` (above the test module):
```rust
//! Demographics tab — renders the fixture patient's identity + identifiers.
//! Contract-level (iced-free) here; the iced view() is added in the shell crate.
use cairn_gui_data::{ClinicalData, Demographics};
use cairn_gui_tab::{Context, Field, Role, Semantic, SemanticNode, TabId};

#[derive(Default)]
pub struct DemographicsTab {
    pub state: Option<Demographics>,
}

impl DemographicsTab {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lazy load: pull this patient's demographics from the port. Called eagerly
    /// when visible, or by the background scheduler when hidden.
    pub fn load(&mut self, ctx: &Context, data: &dyn ClinicalData) {
        if let Some(p) = &ctx.patient {
            self.state = data.demographics(&p.uuid).ok();
        }
    }
}

impl Semantic for DemographicsTab {
    fn tab_id(&self) -> TabId {
        TabId("demographics".into())
    }
    fn title(&self) -> String {
        "Demographics".into()
    }
    fn semantics(&self, _ctx: &Context) -> SemanticNode {
        let mut fields = vec![Field {
            id: "demographics.heading".into(),
            role: Role::Heading,
            label: "Patient demographics".into(),
        }];
        if let Some(d) = &self.state {
            fields.push(Field { id: "demographics.name".into(), role: Role::TextInput, label: format!("Name: {}", d.patient.display_name) });
            fields.push(Field { id: "demographics.sex".into(), role: Role::TextInput, label: format!("Sex: {}", d.sex) });
            fields.push(Field { id: "demographics.dob".into(), role: Role::TextInput, label: format!("Date of birth: {}", d.birth_date) });
            for (i, (system, value)) in d.identifiers.iter().enumerate() {
                fields.push(Field {
                    id: format!("demographics.id.{i}"),
                    role: Role::TextInput,
                    label: format!("{system}: {value}"),
                });
            }
        }
        SemanticNode { title: "Demographics".into(), fields }
    }
}
```

- [ ] **Step 4: Write the Note tab test + impl**

`cairn-gui/cairn-gui-tabs/cairn-gui-tab-note/src/lib.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_gui_data::MockData;
    use cairn_gui_tab::{Capabilities, Context, Semantic, UserRef};

    fn ctx() -> Context {
        Context {
            patient: Some(cairn_gui_tab::PatientRef {
                uuid: "00000000-0000-0000-0000-0000000000aa".into(),
                display_name: "Amina".into(),
            }),
            user: UserRef { actor_id: "clin-1".into(), display_name: "Dr Vega".into() },
            capabilities: Capabilities::clinician_all(),
        }
    }

    #[test]
    fn cross_reference_is_a_focusable_button_encoding_its_target() {
        let mut tab = NoteTab::new();
        tab.load(&ctx(), &MockData::with_fixtures());
        let node = tab.semantics(&ctx());
        node.assert_complete().unwrap();
        let btn = node.fields.iter().find(|f| f.id.starts_with("open:")).expect("a cross-ref button");
        assert_eq!(btn.role, cairn_gui_tab::Role::Button);
        assert!(btn.label.contains("X-ray"), "one-line summary is the button label");
    }

    #[test]
    fn intent_target_can_be_parsed_from_field_id() {
        assert_eq!(NoteTab::target_of("open:xray-2026-07-01"), Some("xray-2026-07-01".to_string()));
        assert_eq!(NoteTab::target_of("note.body"), None);
    }
}
```

`src/lib.rs` impl (above tests):
```rust
//! Progress-note tab. Slice 1 shows the note heading plus each cross-reference as
//! a focusable button whose id encodes the target id (`open:<id>`). Activating it
//! becomes Intent::OpenTab, which the shell routes to the OTHER pane (spec §5).
//! Rich editing (markdown-source + live preview) is a later slice — no WYSIWYG
//! widget exists in iced out of the box.
use cairn_gui_data::{ClinicalData, NoteRef};
use cairn_gui_tab::{Context, Field, Role, Semantic, SemanticNode, TabId};

#[derive(Default)]
pub struct NoteTab {
    pub refs: Vec<NoteRef>,
}

impl NoteTab {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&mut self, ctx: &Context, data: &dyn ClinicalData) {
        if let Some(p) = &ctx.patient {
            self.refs = data.note_refs(&p.uuid).unwrap_or_default();
        }
    }

    /// Parse the target tab/blob id out of a cross-reference field id.
    pub fn target_of(field_id: &str) -> Option<String> {
        field_id.strip_prefix("open:").map(|s| s.to_string())
    }
}

impl Semantic for NoteTab {
    fn tab_id(&self) -> TabId {
        TabId("note".into())
    }
    fn title(&self) -> String {
        "Current note".into()
    }
    fn semantics(&self, _ctx: &Context) -> SemanticNode {
        let mut fields = vec![Field {
            id: "note.heading".into(),
            role: Role::Heading,
            label: "Progress note".into(),
        }];
        for r in &self.refs {
            fields.push(Field {
                id: format!("open:{}", r.id),
                role: Role::Button,
                label: r.one_line.clone(),
            });
        }
        SemanticNode { title: "Current note".into(), fields }
    }
}
```

`cairn-gui/cairn-gui-tabs/cairn-gui-tab-note/Cargo.toml`: same as demographics with `name = "cairn-gui-tab-note"`.

- [ ] **Step 5: Register both crates + run**

Add `"cairn-gui-tabs/cairn-gui-tab-demographics"` and `"cairn-gui-tabs/cairn-gui-tab-note"` to `members` in `cairn-gui/Cargo.toml`.

Run: `cd cairn-gui && cargo test -p cairn-gui-tab-demographics -p cairn-gui-tab-note`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add cairn-gui/cairn-gui-tabs cairn-gui/Cargo.toml
git commit -m "feat(gui): Demographics + Note tabs (a11y contract, cross-ref buttons)"
```

---

### Task 8: The iced shell binary (two-pane `pane_grid`, tab strips, routing, persistent card)

This is the only heavily iced-dependent task. It wires the pure state from Tasks 3–7 into an iced `application`, using **`pane_grid`** for the resizable two-pane split (its `.on_resize` divider is exactly the user-apportioned splitter), per-pane tab strips, the pinned identity card, and `OpenTab` routing.

> **iced 0.14 API note (not a placeholder):** the render/update bodies below target iced 0.14 (`iced::application(boot, update, view)`, `Task<Message>`, `iced::widget::pane_grid`). Pre-1.0 churn means exact method names may need a touch-up against `https://docs.rs/iced/0.14` — the spike crate already documents this expectation. Verify each iced symbol compiles; the *structure* and the pure calls into `Workspace`/`Semantic`/`MockData` are fixed.

**Files:**
- Create: `cairn-gui/cairn-gui-shell/src/app.rs`
- Create: `cairn-gui/cairn-gui-shell/src/bin/gui.rs`
- Modify: `cairn-gui/cairn-gui-shell/Cargo.toml` (iced dep + bin behind `gui`)

**Interfaces:**
- Consumes: `Workspace`, `Side` (Task 6); `Semantic`, `Context`, `Intent`, `TabId` (Tasks 2–3); the two tab structs (Task 7); `MockData` (Task 4); `SiteManifest`, `UserPrefs`, `merge` (Task 5).
- Produces: `run_gui() -> iced::Result` (called by `main`), and `--dump-a11y` handling (Task 9 extends it).

- [ ] **Step 1: Add iced + a `gui`-gated bin**

`cairn-gui/cairn-gui-shell/Cargo.toml`:
```toml
[features]
default = []
gui = ["dep:iced"]

[dependencies]
cairn-gui-tab = { path = "../cairn-gui-tab" }
cairn-gui-data = { path = "../cairn-gui-data" }
cairn-gui-manifest = { path = "../cairn-gui-manifest" }
cairn-gui-tab-demographics = { path = "../cairn-gui-tabs/cairn-gui-tab-demographics" }
cairn-gui-tab-note = { path = "../cairn-gui-tabs/cairn-gui-tab-note" }
iced = { version = "0.14", optional = true, features = ["tiny-skia", "wgpu"] }

[[bin]]
name = "cairn-gui"
path = "src/bin/gui.rs"
required-features = ["gui"]
```

- [ ] **Step 2: Write `app.rs` — model, update, view**

```rust
//! The iced application: two panes via pane_grid (resizable divider = the user
//! splitter), per-pane tab strips, a pinned identity card, and OpenTab routing
//! into the opposite pane. All non-iced decisions delegate to the pure Workspace.
#![cfg(feature = "gui")]
use cairn_gui_data::{ClinicalData, MockData};
use cairn_gui_manifest::{merge, EffectiveManifest, SiteManifest, UserPrefs};
use cairn_gui_tab::{Capabilities, Context, Intent, PatientRef, Semantic, TabId, UserRef};
use cairn_gui_tab_demographics::DemographicsTab;
use cairn_gui_tab_note::NoteTab;
use crate::workspace::{Side, Workspace};

use iced::widget::{button, column, container, pane_grid, row, scrollable, text};
use iced::{Element, Length, Task};

#[derive(Debug, Clone)]
pub enum Message {
    Resized(pane_grid::ResizeEvent),
    SelectTab(Side, TabId),
    Activate(Side, TabId),
    OpenRef(Side, String), // Side = originating pane, String = target id
}

struct PaneState {
    side: Side,
}

pub struct App {
    ctx: Context,
    data: MockData,
    ws: Workspace,
    note: NoteTab,
    demographics: DemographicsTab,
    panes: pane_grid::State<PaneState>,
}

fn default_site() -> SiteManifest {
    SiteManifest {
        offered: vec![TabId("note".into()), TabId("demographics".into())],
        rail: vec![TabId("note".into()), TabId("demographics".into())],
        default_left: TabId("note".into()),
        default_right: TabId("demographics".into()),
    }
}

fn effective() -> EffectiveManifest {
    merge(&default_site(), &UserPrefs::default())
}

impl App {
    pub fn boot() -> (Self, Task<Message>) {
        let ctx = Context {
            patient: Some(PatientRef {
                uuid: "00000000-0000-0000-0000-0000000000aa".into(),
                display_name: "Amina أمينة अमीना 阿明娜".into(),
            }),
            user: UserRef { actor_id: "clin-1".into(), display_name: "Dr Vega".into() },
            capabilities: Capabilities::clinician_all(),
        };
        let data = MockData::with_fixtures();

        // Eager load for both visible panes' initial tabs (slice 1: both visible).
        let mut note = NoteTab::new();
        note.load(&ctx, &data);
        let mut demographics = DemographicsTab::new();
        demographics.load(&ctx, &data);

        let eff = effective();
        let ws = Workspace::from_manifest(&eff);

        // pane_grid: one horizontal split, ratio from the manifest.
        let (mut panes, left) = pane_grid::State::new(PaneState { side: Side::Left });
        let split = panes.split(pane_grid::Axis::Vertical, left, PaneState { side: Side::Right });
        if let Some((_split_id, _)) = split {
            // set ratio; API: panes.resize(split_id, ratio) — verify method name on 0.14
        }

        (Self { ctx, data, ws, note, demographics, panes }, Task::none())
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Resized(ev) => {
                self.panes.resize(ev.split, ev.ratio);
                self.ws.divider_ratio = ev.ratio;
            }
            Message::Activate(side, tab) => self.ws.activate(side, &tab),
            Message::SelectTab(side, tab) => self.ws.activate(side, &tab),
            Message::OpenRef(from, target_id) => {
                // The cross-reference: open the referenced view in the OTHER pane.
                // Slice 1 has one referenceable kind; map its id to a tab.
                let tab = TabId("demographics".into()); // placeholder mapping for slice 1
                let _ = target_id;
                let intent = Intent::OpenTab(tab);
                if let Intent::OpenTab(t) = intent {
                    self.ws.open_in_opposite(from, t);
                }
            }
        }
        Task::none()
    }

    fn tab_view(&self, side: Side, active: &TabId) -> Element<Message> {
        // Render the active tab's semantics as labelled controls. Cross-ref buttons
        // (id "open:<x>") emit OpenRef so routing lands in the opposite pane.
        let semantic = match active.0.as_str() {
            "note" => self.note.semantics(&self.ctx),
            "demographics" => self.demographics.semantics(&self.ctx),
            _ => self.demographics.semantics(&self.ctx),
        };
        let mut col = column![text(semantic.title).size(18)].spacing(6);
        for f in semantic.fields {
            if let Some(target) = f.id.strip_prefix("open:") {
                let target = target.to_string();
                col = col.push(button(text(f.label)).on_press(Message::OpenRef(side, target)));
            } else {
                col = col.push(text(f.label));
            }
        }
        scrollable(col).height(Length::Fill).into()
    }

    fn pane_content(&self, side: Side) -> Element<Message> {
        // A tab strip over the active tab body.
        let mut strip = row![].spacing(4);
        for t in self.ws.tabs(side) {
            let title = match t.0.as_str() {
                "note" => self.note.title(),
                "demographics" => self.demographics.title(),
                other => other.to_string(),
            };
            let is_active = self.ws.active(side) == t;
            let b = button(text(title)).on_press(Message::SelectTab(side, t.clone()));
            strip = strip.push(if is_active { b } else { b });
        }
        let body = self.tab_view(side, self.ws.active(side));
        column![strip, body].spacing(8).padding(8).into()
    }

    pub fn view(&self) -> Element<Message> {
        // Band 1+2: pinned identity card (persistent safety zone).
        let name = self.ctx.patient.as_ref().map(|p| p.display_name.clone()).unwrap_or_default();
        let identity = container(text(format!("Patient: {name}")).size(16)).padding(10);

        // Band 4: two-pane workspace with a draggable divider.
        let grid = pane_grid(&self.panes, |_id, state, _maximized| {
            pane_grid::Content::new(self.pane_content(state.side))
        })
        .on_resize(10, Message::Resized)
        .width(Length::Fill)
        .height(Length::Fill);

        column![identity, grid].into()
    }
}

pub fn run_gui() -> iced::Result {
    iced::application("Cairn — clinician chart", App::update, App::view)
        .run_with(App::boot)
}
```

- [ ] **Step 3: Write the binary entry**

`cairn-gui/cairn-gui-shell/src/bin/gui.rs`:
```rust
//! Operator-run shell binary (feature `gui`). `--dump-a11y` prints the expected
//! accessibility tree (Task 9); no args launches the window.
#[cfg(feature = "gui")]
fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--dump-a11y") {
        cairn_gui_shell::a11y_dump::print_expected_tree();
        return Ok(());
    }
    cairn_gui_shell::app::run_gui()
}

#[cfg(not(feature = "gui"))]
fn main() {
    eprintln!("build with --features gui");
}
```

Add `pub mod app;` (gui-gated) to `cairn-gui/cairn-gui-shell/src/lib.rs`:
```rust
#[cfg(feature = "gui")]
pub mod app;
```

- [ ] **Step 4: Verify it compiles and launches**

Run: `cd cairn-gui && cargo build -p cairn-gui-shell --features gui`
Expected: builds (fix any iced 0.14 symbol drift per the API note — e.g. `pane_grid::State::resize`/`split` return shapes, `ResizeEvent` fields `split`/`ratio`).

Run (operator, needs a display): `cargo run -p cairn-gui-shell --features gui`
Expected: a window with the identity band on top and two resizable panes (Note left, Demographics right); dragging the divider reapportions; clicking the note's "Chest X-ray…" button opens the reference in the right pane.

- [ ] **Step 5: Commit**

```bash
git add cairn-gui/cairn-gui-shell
git commit -m "feat(gui): iced two-pane shell (pane_grid divider, tab strips, cross-pane routing)"
```

---

### Task 9: Widen the accessibility dump + wire the Spike 0004 operator run

Extend the a11y dump to cover **shell chrome** (panes, tab strips, divider) plus each tab's fields, and record the widened Spike 0004 operator procedure/results against this shell.

**Files:**
- Create: `cairn-gui/cairn-gui-shell/src/a11y_dump.rs`
- Create: `cairn-gui/cairn-gui-shell/results/TEMPLATE.md`
- Modify: `cairn-gui/cairn-gui-shell/src/lib.rs`
- Modify: `docs/spikes/0004-iced-reference-ui-viability.md` (note the widened, shell-level scope + that the reference shell is now the vehicle)

**Interfaces:**
- Consumes: `Side`, the two tab structs, `Context`.
- Produces: `a11y_dump::expected_tree(&Context) -> Vec<SemanticNode>` (shell chrome + both tabs) and `print_expected_tree()`.

- [ ] **Step 1: Write the failing test**

`cairn-gui/cairn-gui-shell/src/a11y_dump.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_tree_covers_shell_chrome_and_is_complete() {
        let nodes = expected_tree(&sample_ctx());
        // Must include a shell-chrome node with pane/tab/divider controls...
        let chrome = nodes.iter().find(|n| n.title == "Shell chrome").expect("chrome node present");
        assert!(chrome.fields.iter().any(|f| f.role == cairn_gui_tab::Role::Divider),
            "divider must be in the a11y tree (keyboard-resizable)");
        assert!(chrome.fields.iter().any(|f| f.role == cairn_gui_tab::Role::Tab),
            "tab strip stops must be in the a11y tree");
        // ...and every node must pass the completeness contract.
        for n in &nodes {
            n.assert_complete().unwrap_or_else(|e| panic!("incomplete node {}: {e}", n.title));
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd cairn-gui && cargo test -p cairn-gui-shell a11y`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement `a11y_dump.rs`**

```rust
//! The EXPECTED accessibility tree for the whole shell — chrome (panes, tab
//! strips, the resizable divider) plus each tab's fields. `--dump-a11y` prints
//! this; the operator diffs it against what Orca/NVDA actually announces. This is
//! the widened Spike 0004 A-claim: shell-level a11y, not just a single form.
use cairn_gui_data::MockData;
use cairn_gui_tab::{Capabilities, Context, Field, PatientRef, Role, SemanticNode, Semantic, UserRef};
use cairn_gui_tab_demographics::DemographicsTab;
use cairn_gui_tab_note::NoteTab;

pub fn sample_ctx() -> Context {
    Context {
        patient: Some(PatientRef {
            uuid: "00000000-0000-0000-0000-0000000000aa".into(),
            display_name: "Amina أمينة अमीना 阿明娜".into(),
        }),
        user: UserRef { actor_id: "clin-1".into(), display_name: "Dr Vega".into() },
        capabilities: Capabilities::clinician_all(),
    }
}

fn shell_chrome() -> SemanticNode {
    SemanticNode {
        title: "Shell chrome".into(),
        fields: vec![
            Field { id: "chrome.pane.left".into(), role: Role::Pane, label: "Left pane".into() },
            Field { id: "chrome.pane.right".into(), role: Role::Pane, label: "Right pane".into() },
            Field { id: "chrome.tab.left.note".into(), role: Role::Tab, label: "Current note tab".into() },
            Field { id: "chrome.tab.right.demographics".into(), role: Role::Tab, label: "Demographics tab".into() },
            Field { id: "chrome.divider".into(), role: Role::Divider, label: "Resize panes".into() },
        ],
    }
}

pub fn expected_tree(ctx: &Context) -> Vec<SemanticNode> {
    let data = MockData::with_fixtures();
    let mut note = NoteTab::new();
    note.load(ctx, &data);
    let mut demo = DemographicsTab::new();
    demo.load(ctx, &data);
    vec![shell_chrome(), note.semantics(ctx), demo.semantics(ctx)]
}

pub fn print_expected_tree() {
    for node in expected_tree(&sample_ctx()) {
        println!("# {}", node.title);
        for f in node.fields {
            println!("  [{}] {} — {}", f.role.as_str(), f.id, f.label);
        }
    }
}
```

Add to `cairn-gui/cairn-gui-shell/src/lib.rs`:
```rust
pub mod a11y_dump;
```

- [ ] **Step 4: Run the test + dump**

Run: `cd cairn-gui && cargo test -p cairn-gui-shell a11y`
Expected: PASS.

Run: `cargo run -p cairn-gui-shell --features gui -- --dump-a11y`
Expected: prints three sections (Shell chrome / Current note / Demographics) with roles + labels.

- [ ] **Step 5: Add the results template + widen the spike doc**

`cairn-gui/cairn-gui-shell/results/TEMPLATE.md`:
```markdown
# Cairn GUI shell — Spike 0004 (widened) results — <operator>/<hardware>/<date>

- Host / renderer (wgpu | tiny-skia) / fonts / screen reader:

| Claim | Threshold | Result | Evidence |
|---|---|---|---|
| A1 shell a11y tree correct | chrome (panes/tabs/divider) + tab fields all have role+label; matches live AT | PASS/FAIL | `--dump-a11y` + Accerciser |
| A2 keyboard-only shell | traverse rail/tab-strips/divider/fields + open cross-ref, no pointer | PASS/FAIL | screen-reader transcript |
| I1 shaping | Latin/Arabic/Devanagari/Han no tofu | PASS/SKIP/FAIL | shaping test |
| I2 RTL/bidi + I3 CJK IME | Arabic RTL caret; Han composed via IME | PASS/FAIL | screenshots |
| L2 Pi latency | p95 keystroke→paint ≤ 100 ms | PASS/FAIL (p95=__ms) | latency log |
```

In `docs/spikes/0004-iced-reference-ui-viability.md`, add a dated note under Status: the spike is **widened to shell-level a11y** (pane/tab-strip/divider traversal, not just a single form) and now runs against the **reference shell** (`cairn-gui/cairn-gui-shell`, mock port), per the approved GUI shell design.

- [ ] **Step 6: Commit**

```bash
git add cairn-gui/cairn-gui-shell/src/a11y_dump.rs cairn-gui/cairn-gui-shell/src/lib.rs cairn-gui/cairn-gui-shell/results docs/spikes/0004-iced-reference-ui-viability.md
git commit -m "feat(gui): shell-level a11y dump + widened Spike 0004 operator harness"
```

---

## Self-Review

**Spec coverage:**
- §1 purpose/scope → Tasks 1–9 (clinician shell, mock port, no writes). ✓
- §2 governing decisions: relative sizing (Task 8 `Length::Fill`, ratio), split-as-shell (Task 6/8), tab=single view (Task 7), compile-time+runtime manifest (Task 5), hybrid data (Tasks 4/7), lazy load (Task 7 `load()`; background scheduler is a later slice — see gap note), semantic contract (Task 2). ✓ (partial — see gaps)
- §3 crate architecture → Task 1 (workspace), one crate per tab (Task 7). ✓
- §4 Tab contract → Tasks 2–3, 7. ✓
- §5 shell (panes, divider, routing, persistent zone, rail) → Tasks 6, 8. Rail is represented in the manifest (`EffectiveManifest.rail`) but not yet rendered as an interactive rail widget — see gap note. Partial.
- §6 context/port/capability seam + freshness → Tasks 3, 4, 6. Freshness FLAG logic is implemented (Task 6); the background-prefetch scheduler + capability-gated prefetch are later slices — see gap note.
- §7 manifest → Task 5 (in-memory source; Postgres deferred per global-constraints simplification). ✓ for logic.
- §8 first slice → Tasks 7–8. ✓
- §9 testing + spike convergence → Tasks 2–9 (headless CI everywhere; widened spike Task 9). ✓

**Deliberate gaps (documented, scoped to later slices — NOT placeholders):**
1. **Background prefetch scheduler + capability-gated prefetch (§6).** Slice 1 has both panes visible, so eager load suffices; the low-priority background scheduler and its capability gate arrive with the first *hidden* tab. Recorded here so the next plan picks it up.
2. **Rail as an interactive widget (§5).** The manifest carries `rail`; rendering it as a clickable navigation column is deferred to the slice that adds a third+ tab (with two tabs, the strips suffice). 
3. **Postgres preference storage (§7).** Deferred per the global-constraints simplification; the merge/self-repair logic is complete and tested now.
4. **`OpenRef` id→tab mapping (Task 8).** Slice 1 maps the single fixture cross-reference to the demographics tab as a stand-in; a real reference registry arrives with the results/imaging tabs.

**Placeholder scan:** no "TBD"/"add error handling"/"similar to Task N" — each task carries full code. The two `// placeholder mapping`/`// verify method name` comments in Task 8 are explicitly scoped (single-ref slice-1 mapping; iced 0.14 API drift) and flagged in the API note, not silent gaps.

**Type consistency:** `Semantic` (id/title/semantics), `Workspace::open_in_opposite`, `Capabilities::clinician_all`, `MockData::with_fixtures`, `merge`/`repair_ratio`, `Side::opposite`, `NoteTab::target_of` are used with the same signatures across defining and consuming tasks. ✓
