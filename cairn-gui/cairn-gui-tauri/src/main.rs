//! Cairn's reference med-list window (#288) — the first runnable clinical surface.
//!
//! # What this binary is, and what it deliberately is not
//!
//! It is a Tauri 2 window whose front door is the §5.3/§5.8 search-before-create funnel
//! (slice 2c): a clerk browses for an existing chart and opens it, or — only when nothing fits —
//! registers a new patient, answering the prompt the registration search raises. Either way the
//! chart opens under a persistent identity header, because the paper affordance for "am I on
//! the right chart?" is possession, not a dropdown or a confirmation dialog (principle 3).
//! `--patient <uuid>` still opens straight onto one chart — the timing runbook and the
//! accessibility pass launch that way.
//!
//! It talks to Postgres directly rather than through a native API, which is the ADR-0021
//! privilege gradient working as designed, not a shortcut: the safety floor is IN the
//! database, so a client with raw SQL access still cannot break it. When the native API
//! (ADR-0023, Phase 8) lands, this window is expected to move onto it — the read path it
//! calls is already the single mapping both would share.
//!
//! `--mock` runs the whole window against fixtures with no database at all. That is a
//! shipped mode, not a toy: it is what the operator accessibility pass and the timing
//! runbook use on a laptop.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod funnel;
mod state;

use clap::Parser;
use state::AppState;

#[derive(Parser)]
#[command(
    name = "cairn-med-list",
    about = "Cairn reference UI — find or register a patient, then their medication chart"
)]
struct Cli {
    /// Open straight onto this chart. Without it the window opens on the patient search.
    #[arg(long)]
    patient: Option<uuid::Uuid>,

    /// PostgreSQL connection string for this node's database.
    #[arg(long, env = "CAIRN_CONN", default_value = "host=/tmp dbname=cairn")]
    conn: String,

    /// The NODE's signing key — holds custody of every sealed body it writes (ADR-0052).
    #[arg(long, env = "CAIRN_KEY", default_value = "node.key")]
    key: std::path::PathBuf,

    /// The CLINICIAN's sealed signing key, unsealed in-window by `unlock`. Distinct from
    /// `--key`: the node seals and holds custody, the human authors and vouches
    /// (ADR-0053).
    #[arg(long)]
    attester_key: Option<std::path::PathBuf>,

    /// Run against fixtures with no database. Clinical writes (sign-off, stopping a drug) are
    /// refused in this mode; registering a patient succeeds into an in-memory population that
    /// vanishes with the window.
    #[arg(long)]
    mock: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // A tokio runtime built by hand rather than by `#[tokio::main]`, because Tauri owns the
    // main thread for the event loop.
    //
    // THIS BINDING IS LOAD-BEARING — do not inline it into the `block_on` below. Commands do
    // NOT run on it (Tauri drives those on its own `tauri::async_runtime`); what it hosts is
    // the `tokio::spawn`ed tokio_postgres connection task that `connect_and_load_schema`
    // starts. Drop this runtime and that task stops being polled, so every query for the rest
    // of the session hangs or fails. Keeping it alive until `main` returns is the point.
    let runtime = tokio::runtime::Runtime::new()?;

    let app_state = if cli.mock {
        // Fixture mode carries NO connection and NO node key — not a flag that says so. The
        // one constructor that builds it is `AppState::mock`, so the two cannot drift apart.
        AppState::mock(cli.patient)
    } else {
        runtime.block_on(build_live_state(&cli))?
    };

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::med_list,
            commands::unlock,
            commands::lock_state,
            commands::sign_off,
            commands::cease,
            funnel::commands::funnel_status,
            funnel::commands::form_edited,
            funnel::commands::browse,
            funnel::commands::prompt_search,
            funnel::commands::register,
            funnel::commands::open_chart,
            funnel::commands::close_chart,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("the window could not start: {e}"))
}

/// Connect, load the schema, read this node's identity, and ask whether it may write —
/// everything a writing window needs before it shows anything.
///
/// The node key is loaded UP FRONT and fails the launch if it cannot be: a window that
/// opens and only discovers at sign-off time that it can never seal anything has wasted
/// the clinician's review.
///
/// Whether that key may AUTHOR is probed up front for the same reason (#654 option 2) — but a
/// "no" does NOT fail the launch. Reading a chart needs no actor, so the window opens and says,
/// in the chrome, what an operator must do; the probe matches all four standings, because the
/// remedy for a retired key is not the remedy for a never-enrolled one. A probe that could not
/// run at all is said too, never swallowed.
async fn build_live_state(cli: &Cli) -> anyhow::Result<AppState> {
    let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
    let identity = cairn_node::identity::load_local(&db).await?;
    let node_sk = cairn_node::keystore::load(
        &cli.key,
        std::env::var("CAIRN_KEY_PASSPHRASE").ok().as_deref(),
    )?;
    // ONE connection, shared by the chart commands and the funnel's live port.
    let db = std::sync::Arc::new(tokio::sync::Mutex::new(db));
    let live = cairn_gui_live::LiveData::sharing(db.clone(), node_sk.clone(), &identity);
    let provisioning = match live.standing().await {
        Ok(standing) => funnel::view::standing_sentence(standing, live.node_kid()),
        Err(e) => {
            use cairn_gui_data::port::DataError;
            let why = match e {
                DataError::Unavailable(t)
                | DataError::Refused(t)
                | DataError::NotProvisioned(t) => t,
                DataError::NotFound => "no answer".to_string(),
            };
            Some(format!(
                "Could not check whether this node may write ({why}), so registering may be \
                 refused."
            ))
        }
    };
    Ok(AppState {
        db: Some(db),
        node_sk: Some(node_sk),
        node_origin: identity.node_id_hex,
        chart: tokio::sync::Mutex::new(cli.patient.map(funnel::window::OpenChart::by_id)),
        funnel_backend: funnel::backend::FunnelBackend::Live(live),
        funnel: tokio::sync::Mutex::new(cairn_gui_funnel::FunnelSession::new()),
        shown: tokio::sync::Mutex::new(Default::default()),
        provisioning,
        attester_key_path: cli.attester_key.clone(),
        session: tokio::sync::Mutex::new(None),
    })
}
