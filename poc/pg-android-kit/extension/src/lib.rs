//! Spike 0003 G3 — a deliberately tiny pgrx smoke extension.
//!
//! It is small on purpose: it touches exactly the layers most likely to fault
//! under a foreign libc (bionic) on a cross-compiled `.so` —
//!   1. a plain scalar return (does the fn get called / ABI sane),
//!   2. argument passing,
//!   3. palloc/varlena text handling (PostgreSQL memory + String marshalling),
//!   4. SPI — a call back INTO the executor from inside the extension.
//! If all four return correctly after `CREATE EXTENSION`, the ADR-0002 pgrx
//! escape hatch is viable on the phone tier.
use pgrx::prelude::*;

pgrx::pg_module_magic!();

/// (1) Plain return — no args.
#[pg_extern]
fn cairn_smoke_answer() -> i32 {
    42
}

/// (2) Argument passing.
#[pg_extern]
fn cairn_smoke_add(a: i32, b: i32) -> i32 {
    a + b
}

/// (3) Varlena / palloc — takes text, returns text (exercises detoasting + String).
#[pg_extern]
fn cairn_smoke_echo(s: &str) -> String {
    format!("cairn:{s}")
}

/// (4) SPI — call back into the executor.
#[pg_extern]
fn cairn_smoke_spi() -> i64 {
    Spi::get_one::<i64>("SELECT count(*) FROM (VALUES (1),(2),(3)) AS v(x)")
        .expect("SPI query failed")
        .unwrap_or(-1)
}
