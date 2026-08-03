//! The rendering-framework-free part of the Tab contract: a stable addressable id and
//! the accessibility-contract accessor every tab implements. There is deliberately no
//! `view()` here — under Tauri 2 the rendering is semantic HTML in the webview, and what
//! a tab owes the shell is its *contract*, not its widgets. Cross-pane routing goes
//! through the shell's `Workspace::open_in_opposite`; the spec §4 Intent/Outcome
//! vocabulary arrives when tabs become independent sub-apps.
use crate::context::Context;
use crate::semantics::SemanticNode;

/// Stable, addressable identity for a tab kind (deep links target this).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TabId(pub String);

/// The accessibility contract accessor every tab must implement, iced-free so it
/// is CI-testable.
pub trait Semantic {
    fn tab_id(&self) -> TabId;
    fn title(&self) -> String;
    fn semantics(&self, ctx: &Context) -> SemanticNode;
}
