//! #898/#899/#909: the window's launch chooses its link (refusing a remote database without the
//! pinned link) BEFORE it connects, and never loads the schema. Both live in the source of
//! `build_live_state` (the window has no DB-backed launch test, #738, and a reordered call or a
//! reintroduced loader passes every behavioural test), so the source is read.
use crate::source_guard::{position, rust_fn, sources};

const MAIN: &str = include_str!("main.rs");

/// The stamp is judged before the window reads anything else, so a window on a database not
/// prepared for it refuses at launch instead of running against the wrong generation.
#[test]
fn the_schema_stamp_is_judged_before_anything_is_read() {
    let body = rust_fn(MAIN, "build_live_state");
    assert!(position(body, "window_verdict(") < position(body, "load_local("));
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

/// #909 slice B: a window reaches a remote node only through the pinned link, so its launch
/// connects only through the `Link` that `terminal_link::prepare` returned, never by the plain
/// door, and `prepare` (which refuses a remote string with no terminal directory) runs first.
/// `terminal_launch.rs`, which makes the first connect, is read too.
#[test]
fn every_launch_connect_goes_through_the_prepared_link() {
    let body = rust_fn(MAIN, "build_live_state");
    assert!(position(body, "terminal_link::prepare(") < position(body, "connect_with_task("));
    for (name, src) in [
        ("build_live_state", body),
        ("terminal_launch.rs", include_str!("terminal_launch.rs")),
    ] {
        // `reopening(` is the clear-text reopen (`Link::Local`); `reopening_via(` carries the link.
        for door in ["db::connect", "tokio_postgres::connect(", "reopening("] {
            assert!(
                !src.contains(door),
                "{name} connects by `{door}`, around the link"
            );
        }
    }
}

/// The window's flags can only ADD the pin (`--terminal-dir`, `--node-fingerprint`), never skip
/// it: no flag turns the pin off, lets a remote string through without it, or sets the TLS mode.
/// Only the Cli's code lines are read (its doc comments may name the pin they add).
#[test]
fn the_window_cli_has_no_flag_that_skips_the_pin() {
    let start = position(MAIN, "struct Cli {");
    let len = MAIN[start..].find("\n}\n").expect("Cli closes");
    let cli: String = MAIN[start..start + len]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase()
        .replace('_', "-");
    assert!(cli.contains("terminal-dir"), "the scan reads the real Cli");
    for word in ["insecure", "pin", "skip", "tls", "ssl", "allow-remote"] {
        assert!(!cli.contains(word), "a Cli flag carries `{word}`");
    }
}
