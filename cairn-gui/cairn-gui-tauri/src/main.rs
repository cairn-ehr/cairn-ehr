//! Cairn's reference med-list window (#288) — the first runnable clinical surface.
//!
//! # What this binary is, and what it deliberately is not
//!
//! It is a Tauri 2 window open on ONE patient's medication chart, launched with
//! `--patient <uuid>`. There is no patient picker: the §5.3/§5.8 search-before-create
//! funnel is unbuilt, and inventing a throwaway one here would put an untested
//! wrong-chart hazard in front of a clinician (principle 3 — the paper affordance for
//! "am I on the right chart?" is possession, not a dropdown).
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
mod state;

use clap::Parser;
use state::AppState;

#[derive(Parser)]
#[command(
    name = "cairn-med-list",
    about = "Cairn reference UI — one patient's medication chart"
)]
struct Cli {
    /// The chart to open. There is no patient picker in this slice (see the module doc).
    #[arg(long)]
    patient: uuid::Uuid,

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

    /// Run against fixtures with no database. Writes are refused in this mode.
    #[arg(long)]
    mock: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // One multi-thread runtime shared by every command, built here rather than by
    // `#[tokio::main]` because Tauri owns the main thread for the event loop.
    let runtime = tokio::runtime::Runtime::new()?;

    let app_state = if cli.mock {
        // Fixture mode carries NO connection and NO node key — not a flag that says so.
        // A boolean that can disagree with reality is how a "mock" window ends up writing
        // to a real database.
        AppState {
            db: None,
            node_sk: None,
            node_origin: String::new(),
            patient: cli.patient,
            attester_key_path: None,
            session: tokio::sync::Mutex::new(None),
        }
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
        ])
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("the window could not start: {e}"))
}

/// Connect, load the schema, and read this node's identity — everything a writing window
/// needs before it shows a chart.
///
/// The node key is loaded UP FRONT and fails the launch if it cannot be: a window that
/// opens and only discovers at sign-off time that it can never seal anything has wasted
/// the clinician's review.
async fn build_live_state(cli: &Cli) -> anyhow::Result<AppState> {
    let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
    let identity = cairn_node::identity::load_local(&db).await?;
    let node_sk = cairn_node::keystore::load(
        &cli.key,
        std::env::var("CAIRN_KEY_PASSPHRASE").ok().as_deref(),
    )?;
    Ok(AppState {
        db: Some(tokio::sync::Mutex::new(db)),
        node_sk: Some(node_sk),
        node_origin: identity.node_id_hex,
        patient: cli.patient,
        attester_key_path: cli.attester_key.clone(),
        session: tokio::sync::Mutex::new(None),
    })
}
