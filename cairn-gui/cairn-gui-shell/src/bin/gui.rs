//! Operator-run shell binary (feature `gui`). Slice 1: launches the two-pane
//! window. `--dump-a11y` (Task 9) will add the headless accessibility-tree dump
//! used by CI to catch a11y regressions without a display; that flag is not yet
//! wired here.
#[cfg(feature = "gui")]
fn main() -> iced::Result {
    cairn_gui_shell::app::run_gui()
}

#[cfg(not(feature = "gui"))]
fn main() {
    eprintln!("build with --features gui");
}
