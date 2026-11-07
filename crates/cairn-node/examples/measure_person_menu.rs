//! **Measurement scaffolding for [#1021](https://github.com/cairn-ehr/cairn-ehr/issues/1021) —
//! not a product surface.**
//!
//! The person menu ("Waiting for you", ADR-0095) has two reads, and both grow with ALL-TIME
//! history, not with what is open (ADR-0095 (h)):
//!
//! - **the bar's read** — one person's menu: every part their keys, and their current
//!   supervisees' keys, ever wrote (`read_menus(Some(staff))`);
//! - **the picker's counts read** — everybody's count: essentially every note part on the node
//!   (`read_menus(None)`).
//!
//! Both run on the window's ONE database connection, so a pick of who's here waits behind a
//! counts read already in flight. RUNBOOK §26's hand-built fixture (about 60 notes) cannot show a
//! cliff that scales with history, so this rig seeds a ward's worth of notes in STAGES and times
//! the reads at each stage, giving a curve rather than one point.
//!
//! # What it times, and why exactly that
//!
//! Each read is timed as the window composes it (`cairn-gui-tauri/src/person_menu.rs`,
//! `whos_here.rs`), so the figure is the one a clinician waits for, Rust classification included:
//!
//! - **bar** = one REPEATABLE READ snapshot holding `read_menus(Some(staff))`, `menu_for` and
//!   `candidates_by_id` for the rows' charts (the window's `fresh`);
//! - **counts** = one snapshot holding `read_menus(None)`, then `counts` (`whos_here_counts_impl`);
//! - **directory** = `whos_here::directory` (the picker's list, read before the counts);
//! - **pick behind counts** = counts + directory + bar, end to end — the worst case RUNBOOK §26's
//!   third timing describes, since the one connection serialises them.
//!
//! The budget is RUNBOOK §26's: a pick to "Waiting for you: N" in ≤ 1 s. A miss is the finding —
//! file it, never adjust the budget.
//!
//! # The ward it models
//!
//! `--staff` clinicians, one of whom (`dr-b`) supervises `--supervisees` of them, the first
//! `--cosign-required` of those under a mandatory co-sign (so the bar's groups 2 and 3 both hold
//! rows). Notes are one-part, signed by their writer through the production `sign_note` (the same
//! draft, spend, seal, sign and floor the window's Sign uses), the writer round-robin over all
//! staff and the chart round-robin over `--charts` registered charts. A 30-bed ward writing about
//! three notes a bed a day is ~33 000 notes a year; a facility's node holds several wards.
//!
//! **Known distortion, stated rather than hidden:** every note is seeded NOW, so the bar's group 3
//! ("your supervisees' notes of the last 7 days", cut by `recorded_at`, the HLC wall) holds every
//! supervisee note rather than one week's. The SQL cost is unaffected (it returns all-time rows
//! either way); the Rust classification and the menu's row count are over-stated.
//!
//! # Vault rows
//!
//! Staff get their vault row by a direct INSERT with a placeholder `sealed` blob, exactly as the
//! note test suites do (`tests/common/note_vouch_kit.rs`): the menu only asks the vault who a key
//! stands for, and a real enrolment costs two Argon2id releases per person. **Run it only on a
//! throwaway measurement node**: those rows can never be released.
//!
//! # Usage
//!
//! ```text
//! export CAIRN_KEY_PASSPHRASE=...
//! cairn-node --conn "$CONN" --key /tmp/menu-node.key init --name menu-rig --address 127.0.0.1:0
//! cargo run --release --example measure_person_menu -- \
//!     --conn "$CONN" --key /tmp/menu-node.key --stages 1000,5000,20000,40000
//! ```
//!
//! Stages are CUMULATIVE note totals; run it once against a fresh node (a re-run adds staff with
//! new keys beside the old ones and measures a different ward).

use anyhow::Context;
use cairn_event::SigningKey;
use cairn_node::medication::snapshot::in_snapshot;
use cairn_node::medication::AuthorParams;
use cairn_node::note::menu_read::read_menus;
use cairn_node::note::sign::{sign_note, SignNoteInput};
use cairn_node::patient::register::register_patient;
use cairn_patient_search::{CandidateList, SearchQuery};
use chrono::Utc;
use clap::Parser;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio_postgres::Client;
use uuid::Uuid;

#[derive(Parser)]
#[command(about = "Seed a ward's notes in stages and time the person menu's reads (#1021)")]
struct Args {
    /// PostgreSQL connection string for the (throwaway) node being measured.
    #[arg(long)]
    conn: String,
    /// The node key written by the `cairn-node init` ceremony.
    #[arg(long)]
    key: PathBuf,
    /// Operational passphrase for the node key.
    #[arg(long, env = "CAIRN_KEY_PASSPHRASE", hide_env_values = true)]
    passphrase: Option<String>,
    /// Cumulative note totals at which to stop seeding and measure, ascending.
    #[arg(long, value_delimiter = ',', default_value = "1000,5000,20000")]
    stages: Vec<usize>,
    /// How many clinicians write notes.
    #[arg(long, default_value_t = 40)]
    staff: usize,
    /// How many of them `dr-b` supervises.
    #[arg(long, default_value_t = 20)]
    supervisees: usize,
    /// How many of those supervisees need a co-sign.
    #[arg(long, default_value_t = 5)]
    cosign_required: usize,
    /// How many charts the notes spread over.
    #[arg(long, default_value_t = 300)]
    charts: usize,
    /// Timed repeats of each read per stage; the median is reported.
    #[arg(long, default_value_t = 5)]
    repeats: usize,
}

/// The supervisor whose bar is timed (the RUNBOOK's name for her).
const SUPERVISOR: &str = "dr-b";
const TERMINAL: &str = "menu-rig-pc";

/// One clinician: their staff id and signing key.
struct Staff {
    staff_id: String,
    sk: SigningKey,
    kid: String,
}

/// Staff id of clinician `i`: the supervisor first, then `ward-001`, `ward-002`, …
fn staff_id(i: usize) -> String {
    if i == 0 {
        SUPERVISOR.into()
    } else {
        format!("ward-{i:03}")
    }
}

/// The median of `xs` (the upper middle for an even count). Pure; `xs` must be non-empty.
fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort();
    xs[xs.len() / 2]
}

/// Enrol clinician `i` as a human actor (a handle of their own, so each is a distinct actor) and
/// give them a placeholder vault row (see the module doc's "Vault rows").
async fn enrol(db: &Client, i: usize) -> anyhow::Result<Staff> {
    let staff_id = staff_id(i);
    let (sk, kid) = cairn_event::generate_key()?;
    let pinned = cairn_node::enroll::build_human_pinned("clinician", None, Some(&staff_id))?;
    cairn_node::enroll::enroll_human_actor(db, &kid, &pinned).await?;
    db.execute(
        "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                   sealed, enrolled_by, contact_email) \
         SELECT $2, $3, 'clinician', actor_id, $1, '\\x00'::bytea, 'console', 'rig@example.org' \
           FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll' LIMIT 1",
        &[&kid, &staff_id, &format!("Clinician {staff_id}")],
    )
    .await
    .with_context(|| format!("vault row for {staff_id}"))?;
    Ok(Staff { staff_id, sk, kid })
}

/// One console act on the supervision record (db/072's console door).
async fn supervision(
    db: &Client,
    op: &str,
    subject: &str,
    sup: Option<&str>,
) -> anyhow::Result<()> {
    db.execute(
        "SELECT cairn_supervision_console_act($1, $2, $3)",
        &[&op, &subject, &sup],
    )
    .await
    .with_context(|| format!("{op} {subject}"))?;
    Ok(())
}

/// The bar's read, as the window's `person_menu::fresh` composes it. Returns the menu's count.
async fn time_bar(db: &mut Client) -> anyhow::Result<(Duration, usize)> {
    let started = Instant::now();
    let now = Utc::now();
    let today = now.date_naive().to_string();
    let count = in_snapshot(db, async |tx| {
        let read = read_menus(tx, Some(SUPERVISOR)).await?;
        let menu = read.menu_for(SUPERVISOR, now).unwrap_or_default();
        cairn_node::patient::candidate_read::candidates_by_id(tx, &menu.charts(), &today).await?;
        Ok(menu.count())
    })
    .await?;
    Ok((started.elapsed(), count))
}

/// The picker's counts read, as `whos_here_counts_impl` composes it. Returns the parts read.
async fn time_counts(db: &mut Client) -> anyhow::Result<(Duration, usize)> {
    let started = Instant::now();
    let read = in_snapshot(db, async |tx| read_menus(tx, None).await).await?;
    let _ = read.counts(Utc::now());
    Ok((started.elapsed(), read.parts.len()))
}

/// The picker's list (`whos_here::directory`).
async fn time_directory(db: &Client) -> anyhow::Result<Duration> {
    let started = Instant::now();
    cairn_node::note::whos_here::directory(db).await?;
    Ok(started.elapsed())
}

/// Time every read `repeats` times and print one row of the curve.
async fn measure(db: &mut Client, repeats: usize) -> anyhow::Result<()> {
    let (mut bar, mut counts, mut dir) = (vec![], vec![], vec![]);
    let (mut bar_n, mut parts_n) = (0, 0);
    for _ in 0..repeats {
        let (t, n) = time_bar(db).await?;
        bar.push(t);
        bar_n = n;
        let (t, n) = time_counts(db).await?;
        counts.push(t);
        parts_n = n;
        dir.push(time_directory(db).await?);
    }
    let notes: i64 = db
        .query_one("SELECT count(*) FROM clinical_note", &[])
        .await?
        .get(0);
    let (bar, counts, dir) = (median(bar), median(counts), median(dir));
    let pick_behind = bar + counts + dir;
    println!(
        "| {notes} | {parts_n} | {bar_n} | {} | {} | {} | {} |",
        ms(bar),
        ms(counts),
        ms(dir),
        ms(pick_behind),
    );
    Ok(())
}

fn ms(d: Duration) -> String {
    format!("{:.0} ms", d.as_secs_f64() * 1000.0)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    anyhow::ensure!(
        args.stages.windows(2).all(|w| w[0] < w[1]),
        "--stages must be ascending cumulative totals"
    );
    anyhow::ensure!(
        args.cosign_required <= args.supervisees && args.supervisees < args.staff,
        "need cosign-required <= supervisees < staff"
    );
    let mut db = cairn_node::db::connect(&args.conn).await?;
    let node_sk = cairn_node::keystore::load(&args.key, args.passphrase.as_deref())?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let origin = cairn_node::identity::load_local(&db).await?.node_id_hex;

    // The ward: staff, dr-b's supervisees (1..=supervisees), the first few needing a co-sign.
    let mut staff = Vec::with_capacity(args.staff);
    for i in 0..args.staff {
        staff.push(enrol(&db, i).await?);
    }
    for s in &staff[1..=args.supervisees] {
        supervision(&db, "link-supervisor", &s.staff_id, Some(SUPERVISOR)).await?;
    }
    for s in &staff[1..=args.cosign_required] {
        supervision(&db, "require-cosign", &s.staff_id, None).await?;
    }

    // The charts (a chart's first event is its registration, #345).
    let displayed = CandidateList::empty();
    let mut charts: Vec<Uuid> = Vec::with_capacity(args.charts);
    for n in 0..args.charts {
        let name = format!("Rig Patient{n}");
        let query = SearchQuery::new(&name, None, &[]);
        charts.push(
            register_patient(
                &mut db,
                &node_sk,
                &node_kid,
                &origin,
                Some(&name),
                &query,
                &displayed,
            )
            .await?,
        );
    }
    println!(
        "ward: {} staff, {} supervised by {SUPERVISOR} ({} need a co-sign), {} charts",
        args.staff, args.supervisees, args.cosign_required, args.charts
    );
    println!("| clinical_note rows | parts in counts read | dr-b's count | bar | counts | directory | pick behind counts |");
    println!("|---|---|---|---|---|---|---|");

    let mut signed = 0usize;
    let started = Instant::now();
    for &stage in &args.stages {
        while signed < stage {
            let s = &staff[signed % staff.len()];
            let input = SignNoteInput {
                draft: Uuid::now_v7(),
                patient: charts[signed % charts.len()],
                terminal: TERMINAL,
                writer: Some(&s.staff_id),
                narrative: "obs stable, plan unchanged",
                stated: None,
                node_origin: &origin,
                addendum_to: None,
                also_vouches: &[],
            };
            let author = AuthorParams {
                human_sk: &s.sk,
                human_kid: &s.kid,
            };
            if let Err(refusal) = sign_note(&mut db, &node_sk, &input, &author).await? {
                anyhow::bail!("note {signed} was refused: {refusal:?}");
            }
            signed += 1;
        }
        eprintln!(
            "  seeded {signed} notes in {:.0} s",
            started.elapsed().as_secs_f64()
        );
        measure(&mut db, args.repeats).await?;
    }
    Ok(())
}
