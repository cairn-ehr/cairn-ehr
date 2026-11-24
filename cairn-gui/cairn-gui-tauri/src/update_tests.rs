//! Tests for `update.rs` (#909 slice C1): the installed program's line, the launch verdict, the
//! poll in fixture mode, and the floor's gate on opening another chart.
use super::*;
use crate::funnel::commands::{open_chart_impl, register_impl};
use crate::funnel::view::tests::sample_candidate;
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
            reason: None
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
    assert_eq!(install_view(InstallFit::Same).kind, "same");
    assert_eq!(install_view(InstallFit::Fits).kind, "fits");
    assert_eq!(install_view(InstallFit::DoesNotFit).kind, "does_not_fit");
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
