//! Tests for `update.rs` (#909 slice C1): the installed program's line, the launch verdict, the
//! poll in fixture mode, and the floor's gate on opening another chart.
use super::*;
use crate::funnel::commands::{open_chart_impl, register_impl};
use crate::funnel::view::tests::sample_candidate;
use crate::source_guard::position;
use std::time::Instant;

/// A runtime-derived hash, so no 32-byte literal sits in a test (house rule 6).
fn hash(seed: u8) -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(seed))
}

#[test]
fn the_printed_line_round_trips() {
    let line = generation_line(85, hash(3));
    assert_eq!(parse_generation_line(&line), Some((85, hash(3))));
    // What `--print-schema-generation` prints arrives with its newline.
    assert_eq!(
        parse_generation_line(&format!("{line}\n")),
        Some((85, hash(3)))
    );
}

/// An older program that does not know the flag prints clap's usage instead; anything that is not
/// exactly `<generation> <64 hex>` is "not installed", the safe answer (Review Focus 4).
#[test]
fn a_garbled_line_is_not_installed() {
    let hex = hex::encode(hash(1));
    for line in [
        String::new(),
        "85".to_string(),
        format!("eighty-five {hex}"),
        format!("85 {}", &hex[..62]),
        format!("85 {hex}ab"),
        format!("85 {hex} extra"),
        format!("85 {}", "zz".repeat(32)),
        "error: unexpected argument '--print-schema-generation' found".to_string(),
    ] {
        assert_eq!(parse_generation_line(&line), None, "{line:?}");
    }
}

/// On Linux a running program whose file was replaced reads as `<path> (deleted)`: the installed
/// program is the file now at that path.
#[test]
fn a_deleted_exe_path_is_the_installed_path() {
    let installed = std::path::PathBuf::from("/opt/cairn/cairn-med-list");
    assert_eq!(
        exe_path("/opt/cairn/cairn-med-list (deleted)".into()),
        installed
    );
    assert_eq!(exe_path(installed.clone()), installed);
}

/// Review Focus 5: `--mock` never polls a database and never restarts.
#[tokio::test]
async fn mock_update_state_is_current() {
    let state = AppState::mock(None);
    assert_eq!(
        update_state_impl(&state).await,
        UpdateView {
            kind: "current",
            reason: None,
            refusal: None,
        }
    );
    assert_eq!(installed_program_impl(&state).await.kind, "same");
}

/// Each verdict reaches the webview as one of three kinds; a refusal learned mid-session reads as
/// below the floor (every doubt toward refusing) with no reason of its own.
#[test]
fn each_verdict_reads_as_its_kind() {
    let view = |v: WindowVerdict| update_view(&v);
    assert_eq!(view(WindowVerdict::Current).kind, "current");
    assert_eq!(view(WindowVerdict::UpdateWaiting).kind, "waiting");
    let below = view(WindowVerdict::BelowFloor {
        reason: Some("allergy reactions".into()),
    });
    assert_eq!(
        (below.kind, below.reason.as_deref()),
        ("below_floor", Some("allergy reactions"))
    );
    let refused = view(WindowVerdict::Refused("prepared by cairn-sync".into()));
    assert_eq!((refused.kind, refused.reason), ("below_floor", None));
    // Final review M2: the alert shows the refusal's own sentence, not the below-floor copy.
    assert_eq!(refused.refusal.as_deref(), Some("prepared by cairn-sync"));
    for other in [WindowVerdict::Current, WindowVerdict::UpdateWaiting] {
        assert_eq!(view(other).refusal, None);
    }
    let fresh = |fit| install_view(fit, StampSource::Fresh).kind;
    assert_eq!(fresh(InstallFit::Same), "same");
    assert_eq!(fresh(InstallFit::Fits), "fits");
    assert_eq!(fresh(InstallFit::DoesNotFit), "does_not_fit");
}

/// At launch below the floor the window opens nothing and says why (design §4.5).
#[test]
fn a_below_floor_launch_names_its_reason() {
    let named = launch_verdict(WindowVerdict::BelowFloor {
        reason: Some("allergy reactions".into()),
    })
    .unwrap_err();
    assert_eq!(
        named,
        "This window is older than the node's database can show correctly: it does not show \
         allergy reactions. Install the window that matches the node."
    );
    let unnamed = launch_verdict(WindowVerdict::BelowFloor { reason: None }).unwrap_err();
    assert_eq!(
        unnamed,
        "This window is older than the node's database can show correctly: it may not show newer \
         records correctly. Install the window that matches the node."
    );
    assert_eq!(
        launch_verdict(WindowVerdict::Refused("not prepared".into())).unwrap_err(),
        "not prepared"
    );
    for fine in [WindowVerdict::Current, WindowVerdict::UpdateWaiting] {
        assert_eq!(launch_verdict(fine.clone()), Ok(fine));
    }
}

/// Review Focus 1 / design §4.5: below the floor the open chart stays open and usable, but no
/// OTHER chart opens and nothing is registered — an ignored alert costs one chart, never a shift.
#[tokio::test]
async fn a_below_floor_window_refuses_another_chart_but_not_its_own() {
    let open = sample_candidate();
    let mut other = sample_candidate();
    other.patient_id = uuid::Uuid::from_u128(6);
    let state = AppState::mock(Some(open.patient_id));
    *state.verdict.lock().unwrap() = WindowVerdict::BelowFloor {
        reason: Some("allergy reactions".into()),
    };
    let show = |c: &cairn_patient_search::Candidate| (c.patient_id, c.clone());
    state.shown.lock().await.extend([show(&open), show(&other)]);

    let refused = open_chart_impl(&state, &other.patient_id.to_string()).await;
    assert_eq!(refused.unwrap_err(), REFUSED_ANOTHER_CHART);
    assert_eq!(
        state.chart.lock().await.as_ref().map(|c| c.patient),
        Some(open.patient_id),
        "the open chart stays open"
    );
    let own = open_chart_impl(&state, &open.patient_id.to_string()).await;
    assert_eq!(own.unwrap().patient_id, open.patient_id.to_string());

    let token = serde_json::from_str("1").expect("a token is a number on the wire");
    let registered = register_impl(&state, token).await.unwrap_err();
    assert_eq!(registered.text, REFUSED_ANOTHER_CHART);
}

/// The gate holds only below the floor: an update waiting changes nothing a clinician can do.
#[tokio::test]
async fn an_update_waiting_opens_any_chart() {
    let mut other = sample_candidate();
    other.patient_id = uuid::Uuid::from_u128(6);
    let state = AppState::mock(Some(sample_candidate().patient_id));
    *state.verdict.lock().unwrap() = WindowVerdict::UpdateWaiting;
    state
        .shown
        .lock()
        .await
        .insert(other.patient_id, other.clone());
    assert!(open_chart_impl(&state, &other.patient_id.to_string())
        .await
        .is_ok());
}

/// Review Focus 4: a hung installed program never blocks the window — it reads as not installed
/// once the timeout passes.
#[cfg(unix)]
#[tokio::test]
async fn a_hung_program_reads_as_not_installed() {
    let started = Instant::now();
    let fit = installed_fit(Some("/bin/sleep".into()), &["10"], None).await;
    assert_eq!(fit, InstallFit::Same);
    assert!(
        started.elapsed() < INSTALLED_CHECK_TIMEOUT + std::time::Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

/// A program that answers is judged against the stamp: one at the database's generation fits.
#[cfg(unix)]
#[tokio::test]
async fn an_installed_program_is_judged_against_the_stamp() {
    let ours = embedded_schema_version();
    let stamp = SchemaStamp {
        version: ours + 1,
        loader_build: "cairn-node 0.0.0".to_string(), // any cairn-node build is trusted
        schema_hash: Some(hash(2).to_vec()),
        window_floor: None,
    };
    let line = generation_line(ours + 1, hash(2));
    let fit = installed_fit(Some("/bin/echo".into()), &[&line], Some(&stamp)).await;
    assert_eq!(fit, InstallFit::Fits);
    // A missing program is not installed either.
    let missing = installed_fit(
        Some("/nonexistent/cairn-med-list".into()),
        &[],
        Some(&stamp),
    );
    assert_eq!(missing.await, InstallFit::Same);
}

/// A stamp at this window's own generation, stamped by any cairn-node build.
fn our_stamp() -> SchemaStamp {
    SchemaStamp {
        version: embedded_schema_version(),
        loader_build: "cairn-node 0.0.0".to_string(),
        schema_hash: Some(hash(4).to_vec()),
        window_floor: None,
    }
}

/// Fix round 1: a poll that could not read (the connection lost and not yet back, a failed read)
/// keeps what the window last knew, verdict and stamp both; one that read replaces both.
#[test]
fn a_lost_connection_keeps_the_verdict() {
    let state = AppState::mock(None);
    let below = WindowVerdict::BelowFloor {
        reason: Some("allergy reactions".into()),
    };
    *state.verdict.lock().unwrap() = below.clone();
    *state.last_stamp.lock().unwrap() = Some(our_stamp());
    assert_eq!(poll_answer(&state, None).kind, "below_floor");
    assert_eq!(*state.verdict.lock().unwrap(), below);
    assert_eq!(*state.last_stamp.lock().unwrap(), Some(our_stamp()));

    let mut newer = our_stamp();
    newer.version += 1;
    assert_eq!(poll_answer(&state, Some(Some(our_stamp()))).kind, "current");
    assert_eq!(*state.verdict.lock().unwrap(), WindowVerdict::Current);
    assert_eq!(
        poll_answer(&state, Some(Some(newer.clone()))).kind,
        "below_floor"
    );
    assert_eq!(*state.last_stamp.lock().unwrap(), Some(newer));
}

/// Fix round 1: the install check judges against a fresh stamp when it can read one, else the
/// last one read, and says which it used.
#[test]
fn the_install_check_falls_back_to_the_cached_stamp() {
    let state = AppState::mock(None);
    assert_eq!(install_stamp(&state, None), (None, StampSource::Unknown));
    *state.last_stamp.lock().unwrap() = Some(our_stamp());
    assert_eq!(
        install_stamp(&state, None),
        (Some(our_stamp()), StampSource::Cached)
    );
    let mut fresh = our_stamp();
    fresh.version += 1;
    assert_eq!(
        install_stamp(&state, Some(Some(fresh.clone()))),
        (Some(fresh), StampSource::Fresh)
    );
    // A fresh read that found no stamp at all is an answer, not a failure to read.
    assert_eq!(
        install_stamp(&state, Some(None)),
        (None, StampSource::Fresh)
    );
}

/// Fix round 2: only a FRESH read restarts. A restart while the node is unreachable relaunches a
/// window that exits at its first connect, so a fit judged on the cached stamp, or a misfit judged
/// on no stamp at all, is `unreachable`: say so, close nothing. A cached stamp that refuses is
/// still `does_not_fit`, and nothing newer installed is `same` whatever the node says.
#[test]
fn only_a_fresh_stamp_restarts() {
    use InstallFit::{DoesNotFit, Fits, Same};
    use StampSource::{Cached, Fresh, Unknown};
    for (fit, source, kind) in [
        (Fits, Fresh, "fits"),
        (Fits, Cached, "unreachable"),
        (Fits, Unknown, "unreachable"),
        (DoesNotFit, Fresh, "does_not_fit"),
        (DoesNotFit, Cached, "does_not_fit"),
        (DoesNotFit, Unknown, "unreachable"),
        (Same, Fresh, "same"),
        (Same, Cached, "same"),
        (Same, Unknown, "same"),
    ] {
        assert_eq!(install_view(fit, source).kind, kind, "{fit:?} {source:?}");
    }
}

/// End to end on the cached stamp: a program that fits it reads as `unreachable`, never `fits`.
#[cfg(unix)]
#[tokio::test]
async fn a_fit_on_the_cached_stamp_is_unreachable() {
    let state = AppState::mock(None);
    let mut newer = our_stamp();
    newer.version += 1;
    *state.last_stamp.lock().unwrap() = Some(newer.clone());
    let line = generation_line(newer.version, hash(4));
    let (stamp, source) = install_stamp(&state, None);
    let fit = installed_fit(Some("/bin/echo".into()), &[&line], stamp.as_ref()).await;
    assert_eq!(fit, InstallFit::Fits);
    assert_eq!(install_view(fit, source).kind, "unreachable");
}

/// Final review I4: `--print-schema-generation` is a contract ACROSS generations — an older window
/// runs the NEW program with it. It is read from the raw arguments before clap parses anything,
/// so a required argument added later, or one clap refuses, can never stop it answering.
#[test]
fn the_print_flag_is_read_before_clap() {
    let args = |xs: &[&str]| xs.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
    assert!(asks_for_generation_line(args(&[
        "cairn-med-list",
        PRINT_FLAG
    ])));
    assert!(asks_for_generation_line(args(&[
        "cairn-med-list",
        "--an-argument-clap-refuses",
        PRINT_FLAG
    ])));
    assert!(!asks_for_generation_line(args(&["cairn-med-list"])));
    assert!(!asks_for_generation_line(args(&[
        "cairn-med-list",
        "--mock"
    ])));
    // The program's own name is never the flag.
    assert!(!asks_for_generation_line(args(&[PRINT_FLAG])));
    let main = include_str!("main.rs");
    let body = &main[position(main, "fn main() -> anyhow::Result<()> {")..];
    assert!(
        position(
            body,
            "update::asks_for_generation_line(std::env::args_os())"
        ) < position(body, "Cli::parse()"),
        "the flag is answered before clap parses"
    );
}

/// Final review M3: a live window whose own path is unknown (macOS reads a path through any
/// symlink as none) cannot check what is installed, and says so; fixture mode never asks.
#[test]
fn an_unknown_path_is_an_unknown_install() {
    let path = std::path::Path::new("/opt/cairn/cairn-med-list");
    assert!(path_unknown(true, None));
    assert!(!path_unknown(true, Some(path)));
    assert!(!path_unknown(false, None));
}

/// `--print-release-version` is answered from the raw arguments before clap, beside the
/// generation flag (#909 slice C2).
#[test]
fn the_release_flag_is_read_before_clap() {
    let args = |xs: &[&str]| xs.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
    assert!(asks_for_release_line(args(&["w", "--bogus", RELEASE_FLAG])));
    assert!(!asks_for_release_line(args(&["w", PRINT_FLAG])));
    assert!(!asks_for_release_line(args(&[RELEASE_FLAG])));
    let main = include_str!("main.rs");
    let body = &main[position(main, "fn main() -> anyhow::Result<()> {")..];
    assert!(
        position(body, "update::asks_for_release_line(std::env::args_os())")
            < position(body, "Cli::parse()"),
        "the release flag is answered before clap parses"
    );
}

#[test]
fn the_release_line_round_trips() {
    assert_eq!(parse_release_line(&release_line(42)), Some(42));
    assert_eq!(parse_release_line("42\n"), Some(42));
    for bad in ["", "x", "-1", "4 2", "error: unexpected argument"] {
        assert_eq!(parse_release_line(bad), None, "{bad:?}");
    }
}

/// Design section 4.5, "run early on Linux": a program renamed over while running still reads
/// as its own path, and the file there is the new one. Unrun on macOS; CI's Linux job runs it.
#[cfg(target_os = "linux")]
#[test]
fn a_program_renamed_over_while_running_is_read_at_its_path() {
    let dir = std::env::temp_dir().join(format!("cairn-swap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prog");
    std::fs::copy("/bin/sleep", &path).unwrap();
    let mut child = std::process::Command::new(&path).arg("30").spawn().unwrap();
    let fresh = dir.join("prog.new");
    std::fs::write(&fresh, b"the new program").unwrap();
    std::fs::rename(&fresh, &path).unwrap();
    let raw = std::fs::read_link(format!("/proc/{}/exe", child.id())).unwrap();
    let read = exe_path(raw);
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(read, path);
    assert_eq!(std::fs::read(&read).unwrap(), b"the new program");
    let _ = std::fs::remove_dir_all(&dir);
}
