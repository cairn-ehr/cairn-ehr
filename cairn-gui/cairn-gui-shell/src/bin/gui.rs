//! Operator-run shell binary (feature `gui`). Slice 1: launches the two-pane
//! window. `--dump-a11y` (Task 9) prints the headless accessibility-tree dump
//! used by CI to catch a11y regressions without a display.
#[cfg(feature = "gui")]
fn main() -> iced::Result {
    use std::env;

    // Check for --dump-a11y flag before launching the GUI
    if env::args().any(|arg| arg == "--dump-a11y") {
        cairn_gui_shell::a11y_dump::print_expected_tree();
        return Ok(());
    }

    cairn_gui_shell::app::run_gui()
}

#[cfg(not(feature = "gui"))]
fn main() {
    eprintln!("build with --features gui");
}
