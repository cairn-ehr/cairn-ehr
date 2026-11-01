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

/// The stamp is judged before the window reads anything else, so a window on a database not
/// prepared for it refuses at launch instead of running against the wrong generation.
#[test]
fn the_schema_stamp_is_judged_before_anything_is_read() {
    let body = rust_fn(MAIN, "build_live_state");
    assert!(position(body, "window_schema_verdict(") < position(body, "load_local("));
}

#[test]
fn the_window_never_loads_the_schema() {
    // Every non-test source file of this crate and of `cairn-gui-live`, whose connection the
    // window reopens (#807); `*tests.rs` fixtures legitimately load the schema.
    let mut files = vec![];
    for dir in ["/src", "/../cairn-gui-live/src"] {
        sources(
            std::path::Path::new(&format!("{}{dir}", env!("CARGO_MANIFEST_DIR"))),
            &mut files,
        );
    }
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
