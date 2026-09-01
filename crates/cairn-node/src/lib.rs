pub mod apply_proposal;
pub mod auto_apply;
pub mod backup;
pub mod db;
pub mod db_diagnosis;
pub mod enroll;
pub mod evidence;
pub mod identify;
pub mod identity;
pub mod identity_evidence;
pub mod john_doe;
pub mod localstate;
pub mod localstate_read;
pub mod matcher_actor;
pub mod medication;
pub mod pairing;
pub mod patient;
pub mod photo_evidence;
pub mod restore;
pub mod safety;
pub mod sensitivity;
pub mod shred;
pub mod sync;
pub mod transport;
pub mod ui_timing;

// The at-rest key-file layer now lives in its own crate so `cairn-sync` can load the
// same sealed files this node writes (issue #503) without depending on a node
// application. Re-exported rather than renamed at the call sites: `crate::keystore::…`
// and `cairn_node::keystore::…` appear 221 times across ~30 files here, and churning
// them would bury the behavioural change in rename noise. These are not deprecated
// shims — `cairn-node` genuinely still offers these modules, implemented elsewhere.
pub use cairn_keystore::{fsio, keystore, seal};

// The backup-medium container format now lives in its own crate so `cairn-sync` can
// write the clinical plane onto the same medium this node writes its federation plane
// to (issue #500 slice 2a), without depending on a node application. Re-exported rather
// than renamed at the call sites — `crate::medium::…` and `cairn_node::medium::…` appear
// at 15 sites across backup.rs, restore.rs, main.rs and two test suites, and every one of
// them compiling untouched is what proves the move changed no behaviour. This is not a
// deprecated shim: `cairn-node` genuinely still offers this module, implemented elsewhere.
pub use cairn_medium as medium;
