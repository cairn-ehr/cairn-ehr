//! `cairn-node`'s binding of the shared DB-gate guard (#442, #450, #481).
//!
//! The guard itself — its argument, its parser, its fail-closed polarity and its fixtures —
//! lives in [`tests/common/db_gate.rs`](common/db_gate.rs), because since #481 it is pulled
//! into `cairn-sync`'s test tree as well. A test binary only runs when its own crate is
//! tested, so a guard that lives in one crate cannot bind `cargo test -p <other>`; sharing one
//! file rather than copying it is what keeps the two bindings from drifting apart (#452).
//!
//! This file is deliberately nothing but the include. Its *name* is the load-bearing part —
//! `cargo test -p cairn-node --test db_gate_actually_ran` still names the guard it always did.
#[path = "common/db_gate.rs"]
mod db_gate;
