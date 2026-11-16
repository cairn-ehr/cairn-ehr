//! The bounded prompt — how many candidates a registration may swear it displayed.
//!
//! Its home is now `cairn_patient_search::prompt_bound` (#909), where the reasoning lives: the
//! node's terminal service re-runs a registration's search and must cut it exactly as this
//! window's prompt does, so both use the one function and the one constant. Re-exported here
//! unchanged, so every window caller still names `cairn_gui_funnel::prompt::…`.
pub use cairn_patient_search::prompt_bound::{
    bound_for_prompt, node_reason, PromptCounts, PromptList, PROMPT_CAP,
};
