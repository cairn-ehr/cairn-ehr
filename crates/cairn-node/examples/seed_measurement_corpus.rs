//! **Measurement scaffolding for [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512) —
//! not a product surface.**
//!
//! House rule 7 owes the DR restore ceremony a measured §1.2 figure: *"a restore of a
//! 100 000-event medium completes in ≤ 10 min, one secret, no knowledge of the dead node's
//! config."* Measuring it needs a node holding a corpus at that scale, and there is no
//! honest way to get one: driving `cairn-node medication-assert` a hundred thousand times
//! costs about a fifth of a second per process start, which is six hours of CLI overhead
//! measuring nothing anybody cares about.
//!
//! So this seeds **in-process, through the production orchestrators** —
//! [`cairn_node::patient::register::register_patient`] and
//! [`cairn_node::medication::assert_medication`] — reusing one connection. Every event it
//! writes goes through the same `submit_event` floor, the same born-sealed body construction
//! (ADR-0052) and the same per-write human authorship rewrite (ADR-0053) that the CLI uses.
//! **Hand-built rows would produce a medium that is not a medium**, and a restore timed
//! against one would measure nothing.
//!
//! # Why this is an example and not a subcommand
//!
//! It bulk-writes fabricated patients. That belongs nowhere near an operator's CLI surface,
//! and `cargo run --example` keeps it out of the shipped binary entirely.
//!
//! # What the corpus looks like, and why that shape
//!
//! One registration act per patient (which itself authors a registration + a name + a date of
//! birth — ADR-0061's search-carrying act), then `--meds-per-patient` **born-sealed**
//! medication asserts. The mix matters: demographic events are not sealed, so a corpus made
//! only of registrations would carry no `event_dek` rows and the restore's per-record unwrap
//! and re-wrap — the expensive half, and the whole subject of ADR-0067 — would never run.
//! The default shape is roughly 85% sealed clinical events, which is the direction a real
//! clinic's log skews.
//!
//! Medication asserts are authored by **one enrolled human actor**, deliberately. A restored
//! node resolves every clinical author through `actor_current`, so a node-signed-only corpus
//! would leave ADR-0067 decision 1's actor-registry re-entry barely exercised — the failure
//! mode that showed up as *"signer … is not an enrolled, non-revoked actor"* during 2d.
//!
//! # Usage
//!
//! ```text
//! export CAIRN_KEY_PASSPHRASE=...
//! cairn-node --conn "$CONN" --key /tmp/measure-node.key init --name measure-rig --address 127.0.0.1:0
//! cargo run --release --example seed_measurement_corpus -- \
//!     --conn "$CONN" --key /tmp/measure-node.key --patients 5000 --meds-per-patient 17
//! ```
//!
//! The node must already be provisioned by the real `init` ceremony: this example loads the
//! key that ceremony wrote rather than minting one, so the medium and its sealed export are
//! produced by the same provisioning the operator would have. The node key must be **sealed**:
//! `init --insecure-plaintext` mints no recovery escrow, so `backup` writes no `CAIRNL1`
//! export and a restore from that medium recovers no custody and no actor registry — it would
//! measure the degraded path rather than the one the budget describes.

use cairn_node::medication::{assert_medication, AssertMedicationInput, AuthorParams};
use cairn_node::patient::register::register_patient;
use cairn_patient_search::{CandidateList, SearchQuery};
use clap::Parser;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(about = "Seed a measurement corpus for the #512 DR-restore paper-parity run")]
struct Args {
    /// PostgreSQL connection string for the node being seeded.
    #[arg(long)]
    conn: String,
    /// The node key written by the `cairn-node init` ceremony (sealed — see the module doc).
    #[arg(long)]
    key: PathBuf,
    /// Operational passphrase for the node key. The rig must be provisioned SEALED (a
    /// `--insecure-plaintext` node has no recovery escrow, so `backup` writes no sealed
    /// `CAIRNL1` export and a restore from it measures the degraded, custody-less path).
    #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
    passphrase: Option<String>,
    /// Where to mint the throwaway human clinician's key.
    #[arg(long, default_value = "/tmp/cairn-measure-clinician.key")]
    clinician_key: PathBuf,
    /// How many patients to register.
    #[arg(long, default_value_t = 100)]
    patients: usize,
    /// How many born-sealed medication asserts per patient.
    #[arg(long, default_value_t = 17)]
    meds_per_patient: usize,
    /// Print a progress line every N patients. `0` prints none.
    ///
    /// The zero sentinel is documented because a caller relies on it, and an undocumented
    /// sentinel is a contract only the implementation knows.
    #[arg(long, default_value_t = 250)]
    progress_every: usize,
}

/// A deterministic fabricated surname for patient `n`.
///
/// Pure, so the corpus is reproducible across runs and two seeded databases can be compared.
/// Deliberately drawn from a small pool crossed with the index: a pool alone would make every
/// name token collide and turn `register_patient`'s search pass into a scan over the whole
/// corpus, which would measure the search rather than the seeding.
fn surname(n: usize) -> String {
    const POOL: [&str; 8] = [
        "Nakamura",
        "Okonkwo",
        "Ferreira",
        "Haddad",
        "Lindqvist",
        "Mwangi",
        "Petrov",
        "Silva",
    ];
    format!("{}{}", POOL[n % POOL.len()], n)
}

/// A deterministic fabricated given name for patient `n`.
fn given_name(n: usize) -> String {
    const POOL: [&str; 8] = [
        "Ana", "Tomas", "Yuki", "Amara", "Ines", "Kwame", "Lena", "Rafael",
    ];
    POOL[(n / 8) % POOL.len()].to_string()
}

/// A deterministic ISO birth date for patient `n`, spread across a plausible range.
///
/// Day and month are kept in-range by construction rather than by validation — the floor is
/// deliberately parse-free and culture-neutral, so a shaped-but-impossible date would be
/// accepted and would make the corpus quietly unrealistic.
fn birth_date(n: usize) -> String {
    let year = 1930 + (n % 90);
    let month = 1 + (n % 12);
    let day = 1 + (n % 28);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A deterministic medication term for the `k`th assert on patient `n`.
///
/// Uncoded on purpose: `assert_medication` looks a coding up in the safety-class map, and an
/// empty map is the shipped default, so coding every row would add a lookup per event that
/// yields nothing. The uncoded path is also the honest common case today (ADR-0059 decision 5
/// leaves the coded/uncoded seam open).
fn medication_term(n: usize, k: usize) -> &'static str {
    const POOL: [&str; 10] = [
        "amoxicillin",
        "metformin",
        "atorvastatin",
        "salbutamol",
        "perindopril",
        "sertraline",
        "levothyroxine",
        "amlodipine",
        "pantoprazole",
        "warfarin",
    ];
    POOL[(n + k) % POOL.len()]
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // `connect`, not `connect_and_load_schema`: `init` already loaded the schema, and
    // replaying every migration here would time the migration replay into the seed.
    let mut db = cairn_node::db::connect(&args.conn).await?;
    let node_sk = cairn_node::keystore::load(&args.key, args.passphrase.as_deref())?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let node_origin = cairn_node::identity::load_local(&db).await?.node_id_hex;

    // One enrolled human clinician signs every clinical write (ADR-0053). Minted rather than
    // reused so the rig is self-contained; enrollment is idempotent for the same key and
    // determinant set, so re-running the seeder against a live corpus does not collide.
    let human_sk = if args.clinician_key.exists() {
        cairn_node::keystore::load(&args.clinician_key, None)?
    } else {
        cairn_node::keystore::generate_plaintext(&args.clinician_key)?.0
    };
    let human_kid = hex::encode(human_sk.verifying_key().to_bytes());
    let pinned =
        cairn_node::enroll::build_human_pinned("clinician", None, Some("measure-rig-clinician"))?;
    cairn_node::enroll::enroll_human_actor(&db, &human_kid, &pinned).await?;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };

    // A registration act carries the search that preceded it (ADR-0061). A genuinely empty
    // candidate list is the honest shape for a clerk registering someone the node has never
    // seen, which every patient here is.
    let displayed = CandidateList {
        candidates: Vec::new(),
        incomplete: false,
        incomplete_reason: None,
    };

    let started = Instant::now();
    let mut events = 0usize;
    for n in 0..args.patients {
        let name = format!("{} {}", given_name(n), surname(n));
        let dob = birth_date(n);
        let query = SearchQuery::new(&name, Some(&dob), &[]);
        let patient = register_patient(
            &mut db,
            &node_sk,
            &node_kid,
            &node_origin,
            Some(&name),
            &query,
            &displayed,
        )
        .await?;
        // registration + name + date of birth. This is an EXPECTATION, not an observation;
        // the log is queried at the end and disagreement is an error, not a silent shortfall.
        events += 3;

        for k in 0..args.meds_per_patient {
            let input = AssertMedicationInput {
                term: medication_term(n, k),
                coding: None,
                formulation: None,
                dose_amount: Some("1"),
                dose_unit: Some("tablet"),
                sig: Some("daily"),
                info_source: "patient",
                started: None,
                started_precision: None,
            };
            assert_medication(
                &mut db,
                &node_sk,
                &node_kid,
                &node_origin,
                patient,
                &input,
                Some(&author),
                None,
            )
            .await?;
            events += 1;
        }

        if args.progress_every > 0 && (n + 1) % args.progress_every == 0 {
            let secs = started.elapsed().as_secs_f64();
            println!(
                "  {} patients, {events} events, {secs:.1}s ({:.0} events/s)",
                n + 1,
                events as f64 / secs
            );
        }
    }

    let secs = started.elapsed().as_secs_f64();

    // ASK THE LOG, do not trust the running total. `events` is incremented by a hardcoded
    // `+= 3` per registration, which duplicates `register_patient`'s internal branching —
    // and that branching is CONDITIONAL (it writes a name event only if given a name, a DOB
    // event only if given a date). The moment those conditions change, the seeder would
    // print a corpus larger than the one it wrote, and a short corpus makes the restore it
    // feeds look FASTER than it really is. A count that can be measured should never be
    // assumed, least of all by a rig whose product is a number.
    let actual: i64 = db
        .query_one("SELECT count(*) FROM event_log", &[])
        .await?
        .get(0);
    if actual < events as i64 {
        anyhow::bail!(
            "seeded corpus is SHORT: authored {events} event(s) but the log holds {actual}. \
             A restore measured against this medium would be measuring less than it claims."
        );
    }
    println!(
        "seeded {actual} events across {} patients in {secs:.1}s ({:.0} events/s)",
        args.patients,
        actual as f64 / secs
    );
    Ok(())
}
