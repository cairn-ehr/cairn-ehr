//! #898/#899: the window's launch refuses a remote database BEFORE it connects, and never loads
//! the schema. Both live in the source of `build_live_state` (the window has no DB-backed launch
//! test, #738, and a reordered call or a reintroduced loader passes every behavioural test), so
//! the source is read.
use crate::source_guard::{position, rust_fn, sources};

const MAIN: &str = include_str!("main.rs");

#[test]
fn the_remote_refusal_comes_before_any_connect() {
    let body = rust_fn(MAIN, "build_live_state");
    assert!(position(body, "require_local(") < position(body, "db::connect("));
}

#[test]
fn the_window_never_loads_the_schema() {
    // Every non-test source file under src/ (`*tests.rs` fixtures legitimately load the schema).
    let mut files = vec![];
    sources(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
        &mut files,
    );
    assert!(files.len() > 20, "the walk found the crate's sources");
    for (name, src) in files {
        // main.rs's comment names the loader to explain its absence; code lines must not.
        let code: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect();
        assert!(!code.contains("connect_and_load_schema"), "{name}");
    }
}
