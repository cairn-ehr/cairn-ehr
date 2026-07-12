//! The iced-free part of the Tab contract: a stable addressable id and the
//! accessibility-contract accessor every tab implements. The full Tab trait with
//! iced view()/update() lives in the shell crate behind the `gui` feature (Task
//! 7/8), keeping this crate iced-free. Cross-pane routing in slice 1 goes through
//! the shell's own message + Workspace::open_in_opposite (Task 6); the spec §4
//! Intent/Outcome vocabulary arrives when tabs become independent TEA sub-apps.
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
