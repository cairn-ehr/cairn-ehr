//! Shared rigging for this crate's DB-gated suites.
//!
//! Deliberately small. `crates/cairn-node/tests/common/` is the real kit; this is the minimum
//! that lets a `cairn-gui` test open a schema-loaded database and sign as a registered actor,
//! and it should stay that way — a second full kit here would be a second set of fixtures to
//! keep true, and they would drift.

// Each test binary in this crate uses a different subset of these helpers, which is normal
// for a `tests/common` module: Cargo compiles it separately into every binary.
#![allow(dead_code)]

use cairn_event::SigningKey;
use tokio_postgres::Client;

/// The connection string for the single-node test database, or `None` when this run has no
/// database. The SAME variable the root tree's suites read, so one export rigs both.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Does this environment-variable value mean YES?
///
/// Deliberately narrow, and the narrowness is the point (#450): `CAIRN_ALLOW_DB_SKIP=please`
/// or `=false` must NOT read as permission to skip the suite, or the opt-out becomes a way to
/// turn the gate off by typo.
pub fn is_affirmative(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
