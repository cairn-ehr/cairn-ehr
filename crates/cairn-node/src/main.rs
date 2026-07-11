use anyhow::Context;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use uuid::Uuid;
use zeroize::Zeroizing;

/// The single prompt string + no-echo behaviour for the operational passphrase,
/// shared by every command that reads the secret interactively. One copy so the
/// wording and echo policy can never drift between `init`/`seal-key` and the runtime.
///
/// Returns a `Zeroizing<String>` so the secret is wiped from heap memory on drop
/// (issue #46): `rpassword` flushes its own internal buffer, but the copy we hold and
/// pass on to the KDF would otherwise linger in freed memory.
fn prompt_passphrase() -> anyhow::Result<Zeroizing<String>> {
    Ok(Zeroizing::new(rpassword::prompt_password(
        "operational passphrase: ",
    )?))
}

/// Resolve the operational passphrase: from `--passphrase` (which clap also fills from
/// the CAIRN_KEY_PASSPHRASE env var), else an interactive no-echo prompt. Errors if none
/// is available — we never write an unsealed key implicitly (use --insecure-plaintext).
///
/// The result is `Zeroizing<String>` and stays wrapped all the way to the Argon2 call,
/// so the passphrase is zeroed on drop wherever the short-lived CLI arm ends (issue #46).
fn resolve_passphrase(flag: Option<String>) -> anyhow::Result<Zeroizing<String>> {
    if let Some(p) = flag.filter(|s| !s.is_empty()) {
        return Ok(Zeroizing::new(p));
    }
    let p = prompt_passphrase()?;
    if p.is_empty() {
        anyhow::bail!("no passphrase provided (or use --insecure-plaintext)");
    }
    Ok(p)
}

/// Load the signing key for a command. Uses CAIRN_KEY_PASSPHRASE; a plaintext key
/// needs no secret. We attempt the load ONCE and react only to the typed `Sealed`
/// error — there is no separate `key_at_rest_state` read that could race the load
/// (a transient unreadable-file blip would otherwise misclassify and skip the prompt).
///
/// `allow_prompt` decides the sealed-but-no-env-secret case:
///   - interactive commands (`pair-*`, `unpeer`) prompt no-echo on the tty;
///   - the UNATTENDED daemon (`run`/`serve`) must NEVER prompt — it fails fast with a
///     legible error instead, so a headless start can't block forever on a tty that
///     has no human (the availability floor: a stuck daemon serves nothing).
fn load_signing_key(
    path: &std::path::Path,
    allow_prompt: bool,
) -> anyhow::Result<cairn_event::SigningKey> {
    use cairn_node::keystore::{load, KeystoreError};
    // Hold the env-provided secret as Zeroizing too, so the copy we lifted out of the
    // environment is wiped on drop (issue #46). We can't scrub the OS env store itself.
    let env_secret = std::env::var("CAIRN_KEY_PASSPHRASE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Zeroizing::new);
    match load(path, env_secret.as_ref().map(|s| s.as_str())) {
        Ok(sk) => Ok(sk),
        Err(KeystoreError::Sealed) => {
            if !allow_prompt {
                anyhow::bail!(
                    "signing key is sealed but CAIRN_KEY_PASSPHRASE is not set; set it for \
                     unattended `run`/`serve` (the key was sealed at `init`; \
                     re-provision with --insecure-plaintext only for throwaway test nodes)"
                );
            }
            let p = prompt_passphrase()?;
            Ok(load(path, Some(p.as_str()))?)
        }
        Err(e) => Err(e.into()),
    }
}

/// Print a freshly-minted recovery code exactly once, with the honest loss warning.
fn print_recovery_code(code: &str) {
    eprintln!();
    eprintln!("=== RECOVERY CODE — shown ONCE. Write it down; store it OFF-SITE. ===");
    eprintln!("    {code}");
    eprintln!("=== This is the only off-node way to recover this node's signing key. ===");
    eprintln!("=== Lose BOTH this code and the passphrase and the node is permanently ===");
    eprintln!("=== lost — recoverable only by re-provisioning a new identity. ===");
    eprintln!();
}

/// Write the `.lsk` sidecar (the day-one local-state escrow, ADR-0026 slice D). Mints +
/// dual-wraps a long-lived local-state DEK and atomically writes it 0600 beside the key.
///
/// `overwrite` selects the pre-existing-escrow policy:
///   - `false` — the explicit `establish-local-state-key` verb: REFUSE if a sidecar already
///     exists, so an operator can never silently clobber a working escrow.
///   - `true` — the key-minting / re-sealing paths (`init`, `seal-key`, `restore`): the key's
///     escrow secrets were just (re)minted here, so the LSK MUST travel with them. Replace any
///     stale sidecar so the `.lsk` and the signing key always share one coherent secret set.
///     Without this, `seal-key` on a node that already has a `.lsk` (e.g. from a prior
///     `establish-local-state-key` on a still-plaintext key) would reseal the key under a fresh
///     recovery code, then BAIL on the existing sidecar — leaving the LSK wrapped under the OLD
///     code, desynced, with the command erroring after the key is already resealed. Existing
///     exports stay recoverable regardless: every `CAIRNL1` container is self-contained (carries
///     its own wraps), so the old recovery code still unseals already-written exports; only
///     FUTURE exports use the new sidecar.
fn establish_local_state_escrow(
    key_path: &std::path::Path,
    op_pass: &str,
    recovery_code: &str,
    overwrite: bool,
) -> anyhow::Result<()> {
    use cairn_node::localstate::{establish_lsk, lsk_sidecar_path_for, serialize_sidecar};
    let sidecar = lsk_sidecar_path_for(key_path);
    if sidecar.exists() && !overwrite {
        anyhow::bail!("local-state escrow already exists at {}", sidecar.display());
    }
    let replacing = sidecar.exists();
    let wraps = establish_lsk(op_pass, recovery_code)?;
    cairn_node::fsio::atomic_write(&sidecar, &serialize_sidecar(&wraps), Some(0o600))?;
    eprintln!(
        "local-state escrow {} at {}",
        if replacing {
            "re-established (replaced stale sidecar)"
        } else {
            "established"
        },
        sidecar.display()
    );
    Ok(())
}

/// Seal the node's local-state bundle and write the `CAIRNL1` export sibling beside `medium`
/// (ADR-0026 slice D). Returns the export path on success. Kept separate from the `backup`
/// arm so the caller can treat EVERY failure here as a warn-and-skip degradation: the export
/// is OPTIONAL and the event medium is already written (the load-bearing copy), so a missing
/// passphrase (unattended run), a wrong passphrase, or an I/O error must never abort backup.
async fn seal_and_write_local_state_export(
    db: &tokio_postgres::Client,
    wraps: &cairn_node::localstate::LskWraps,
    passphrase: Option<String>,
    medium: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let op = resolve_passphrase(passphrase)?; // op-pass unwraps the LSK
    let bundle = cairn_node::localstate::read_local_state(db).await?;
    let container = cairn_node::localstate::build_export_container(wraps, &op, &bundle)?;
    let export_path = cairn_node::localstate::localstate_path_for(medium);
    cairn_node::fsio::atomic_write(&export_path, &container, Some(0o600))?;
    Ok(export_path)
}

#[derive(Parser)]
#[command(name = "cairn-node", about = "A Cairn federation node")]
struct Cli {
    /// PostgreSQL connection string. `init` needs DDL privileges (it loads the
    /// schema and creates the `cairn_node` role); the RUNTIME commands
    /// (`serve`/`run`/`peers`/…) should connect as an UNPRIVILEGED role so the
    /// in-DB submit/admission gate is unbypassable — create a login role and
    /// `GRANT cairn_node TO <that role>`, then point `--conn`/`CAIRN_CONN` at it.
    /// `status` reports whether the gate actually binds the connected role
    /// (`db_floor ENFORCED` vs `BYPASSABLE`). See `db/007_node_federation.sql`.
    #[arg(long, env = "CAIRN_CONN")]
    conn: String,
    #[arg(long, default_value = "node.key")]
    key: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Provision this node: mint a keypair (SEALED by default) and append genesis.
    Init {
        #[arg(long)]
        name: String,
        #[arg(long)]
        address: String,
        /// Operational passphrase (else CAIRN_KEY_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
        /// Write the key UNSEALED (test nodes only — no recovery escrow).
        #[arg(long)]
        insecure_plaintext: bool,
    },
    /// Seal an existing plaintext key file and mint a fresh recovery code.
    SealKey {
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
    },
    /// Establish the local-state escrow (`.lsk`) for a node provisioned before slice D.
    /// Prompts for the op passphrase AND the recovery code (both needed once). Errors if
    /// an escrow already exists.
    EstablishLocalStateKey {
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
    },
    /// Print this node's identity (node_id, pubkey, fingerprint, address).
    Identity,
    /// Generate a signed pairing offer (base64) for out-of-band exchange.
    PairOffer {
        #[arg(long, default_value = "cairn")]
        nonce: String,
    },
    /// Accept a peer's pairing offer (base64).  Prints the peer fingerprint and
    /// requires a typed YES confirmation before authoring the peer.added event.
    PairAccept {
        offer: String,
        #[arg(long)]
        role: Option<String>,
    },
    /// List all peers (active and revoked).
    Peers,
    /// Revoke trust for a peer node.
    Unpeer { node_id: String },
    /// Provision the unprivileged runtime login role and grant it `cairn_node`, so
    /// the daemon can connect as a role the in-DB floor actually binds (run this once
    /// with DDL privileges, then point `--conn`/`CAIRN_CONN` at `user=<role>`).
    ProvisionRuntimeRole {
        #[arg(long, default_value = "cairn_runtime")]
        role: String,
    },
    /// Print this node's honest assembly state (peers, keystore health, DR escrow stub).
    Status,
    /// Back up this node's signed event set to a local cold-peer medium (ADR-0026 slice
    /// B). Reads `node_event`, writes a self-verifying medium, re-reads + verifies it,
    /// then records backup health beside the key. No signing key needed — the events are
    /// already signed; confidentiality at rest is the medium volume's job.
    Backup {
        /// Path of the backup medium to write (e.g. a mounted encrypted volume).
        #[arg(long)]
        to: PathBuf,
        /// Operational passphrase to seal the local-state export (else CAIRN_KEY_PASSPHRASE,
        /// else prompt). Only used when a local-state escrow (`.lsk`) exists.
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
    },
    /// Verify a backup medium WITHOUT applying it: every event's signature must check.
    /// Pure/offline — needs no DB and no key. Exits non-zero on any tamper/bit-rot, so a
    /// cron job can detect a rotted backup.
    VerifyBackup {
        /// Path of the backup medium to verify.
        #[arg(long)]
        from: PathBuf,
    },
    /// Restore a node from a cold-peer backup medium into a FRESH, un-enrolled database
    /// (ADR-0026 slice C). Verifies the medium, mints a NEW sealed keypair (the old
    /// signing key is never backed up), rehydrates the old event history through the
    /// self-trusting restore door, authors a new genesis, and records a supersede linking
    /// the dead node to the new one. The node then re-peers from empty.
    Restore {
        /// Path of the backup medium to restore (as written by `backup`).
        #[arg(long)]
        from: PathBuf,
        /// For a federated medium with multiple enrolls: the dead node-id (hex) to
        /// supersede — must name an enroll present on the medium. Optional for a solo
        /// node (auto-detected from the sole enroll).
        #[arg(long)]
        superseded_node: Option<String>,
        /// Operational passphrase for the NEW sealed key (else CAIRN_KEY_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
        /// Write the new key UNSEALED (test nodes only — no recovery escrow).
        #[arg(long)]
        insecure_plaintext: bool,
    },
    /// Serve this node's `node_event` log to pinned-mTLS peers (federation sync).
    Serve {
        #[arg(long, default_value = "0.0.0.0:7843")]
        listen: SocketAddr,
    },
    /// Unattended: serve in the background and pull from `peer` on an interval,
    /// surviving link drops (availability over consistency).
    Run {
        #[arg(long, default_value = "0.0.0.0:7843")]
        listen: SocketAddr,
        #[arg(long)]
        peer: SocketAddr,
        #[arg(long, default_value_t = 5)]
        interval_secs: u64,
    },
    /// List the durable node-event quarantine (issue #111): every pulled node_event
    /// this node refused as UNVERIFIABLE, with its reason, re-offer floor seq, and
    /// ack state. One JSON object per line. An unacked row makes the pull loud every
    /// cycle until its cause is fixed (auto-releases) or it is acked.
    Quarantine,
    /// License a permanent exclusion for one quarantined node_event: mark it acked so
    /// it no longer pins the re-offer floor or makes the pull loud. Takes the hex
    /// content digest from `quarantine`.
    AckQuarantine {
        /// The hex `digest` shown by `cairn-node quarantine`.
        digest: String,
    },

    /// Auto-apply every pending `auto_candidate` match proposal (§5.2/§5.7 C2b) as a
    /// matcher-authored, un-attested, recallable identity link. OWNER ceremony: point
    /// `--conn` at a role that may run `enroll_actor` (the per-epoch matcher actor is
    /// enrolled on first sight), NOT the unprivileged runtime role. Re-checks the db/016
    /// veto per pair; a since-vetoed pair is kicked to human `review` instead of linked.
    ApplyAutoCandidates {
        /// Operational passphrase to seal the per-epoch matcher keys (else
        /// CAIRN_KEY_PASSPHRASE, else prompt). Matcher keys are regenerable, so there is no
        /// separate recovery escrow — but they SIGN identity links, so seal them by default
        /// exactly like the node key.
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
        /// Write matcher keys UNSEALED (throwaway/test nodes only — no at-rest protection).
        #[arg(long)]
        insecure_plaintext: bool,
    },

    /// Register an unidentified ("John Doe") patient (§5.4): mint a UUID, author a
    /// system-generated callsign name + the identity-pending marker so the chart renders
    /// *unconfirmed*. Care can proceed against the printed UUID immediately. OWNER
    /// ceremony: enrolls the node key as a `device` registration actor on first use (a
    /// real clinical UI would attach the operating clerk's human actor instead).
    RegisterJohnDoe {
        /// Care context for the callsign (e.g. ED, ward).
        #[arg(long, default_value = "ED")]
        class: String,
        /// Registering-site label for the callsign (defaults to this node's id).
        #[arg(long)]
        site: Option<String>,
        /// Why the chart is identity-pending — §4.1 value-open.
        #[arg(long, default_value = "unidentified patient, no ID")]
        basis: String,
    },

    /// Record clinician-observed identity evidence on an existing chart (§5.4): an
    /// estimated age (-> a year-range dob) and/or an observed sex (-> administrative-sex),
    /// both provenance `clinician-observed`. Supply at least one of --age / --sex.
    AssertObservedEvidence {
        /// The patient UUID to record evidence on.
        patient: Uuid,
        /// Estimated age in years (apparent age).
        #[arg(long)]
        age: Option<u32>,
        /// ± tolerance in years around the estimated age (default 5).
        #[arg(long, default_value_t = 5)]
        tol: u32,
        /// How the age was estimated (required when --age is given).
        #[arg(long)]
        age_basis: Option<String>,
        /// Observed (apparent) sex — an open string.
        #[arg(long)]
        sex: Option<String>,
        /// How the sex was observed (optional).
        #[arg(long)]
        sex_basis: Option<String>,
    },

    /// Record clinician-observed §5.4 identity evidence on an existing chart. One command for
    /// every evidence kind:
    ///   * `--kind photo` — a content-addressed photograph; requires `--file`, `--media-type`,
    ///     and `--descriptor`. The photo becomes a locally-stored (present + self-verified) blob
    ///     referenced by an `identity.evidence.asserted` event.
    ///   * `--kind mark|belongings|ems-context` — a free-text observation; requires
    ///     `--description`. Non-attachment: the observation is the text in the payload.
    ///
    /// The photo and text flags are mutually exclusive (photo flags iff `--kind photo`). OWNER
    /// ceremony: enrolls the node key as a registration actor on first use (a real UI attaches
    /// the operating clerk's *human* actor).
    AssertIdentityEvidence {
        /// The patient UUID to record evidence on.
        patient: Uuid,
        /// The evidence kind: photo | mark | belongings | ems-context (closed set; typo-rejected).
        #[arg(long)]
        kind: String,
        /// Free-text observation for a text kind (mark/belongings/ems-context): required for
        /// those, rejected for `--kind photo`. Non-empty (principle 4).
        #[arg(long)]
        description: Option<String>,
        /// Path to the image file on disk; required for `--kind photo`, rejected otherwise.
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        /// MIME media type of `--file` (e.g. image/jpeg). Caller-supplied — no sniffing. Photo only.
        #[arg(long = "media-type")]
        media_type: Option<String>,
        /// Honest human description of the photo; required for `--kind photo`, rejected otherwise.
        /// Non-empty (principle 4).
        #[arg(long)]
        descriptor: Option<String>,
        /// How/why it was observed; for ems-context, note the relayed source here (optional).
        #[arg(long)]
        basis: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init {
            name,
            address,
            passphrase,
            insecure_plaintext,
        } => {
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let (sk, kid) = if insecure_plaintext {
                eprintln!(
                    "WARNING: --insecure-plaintext: signing key written UNSEALED (test use only)"
                );
                cairn_node::keystore::generate_plaintext(&cli.key)?
            } else {
                let op = resolve_passphrase(passphrase)?;
                // The recovery code is a key-recovering secret too — hold it Zeroizing so
                // it is wiped on drop once sealed/printed (issue #46).
                let code = Zeroizing::new(cairn_node::seal::generate_recovery_code());
                // Show the recovery code BEFORE the key is persisted. If a crash struck
                // between persist and print, the key would be sealed under a code no
                // human ever saw — silently destroying the off-node escrow. Printing
                // first means the worst case is a shown code for an unwritten key (init
                // simply re-runs and mints a fresh one), never a lost escrow.
                print_recovery_code(&code);
                let kp = cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?;
                // Establish the day-one local-state escrow (ADR-0026 slice D): a long-lived
                // local-state DEK dual-wrapped under the SAME two secrets. Must happen here,
                // while both are in hand — it cannot be retrofitted onto state accrued later.
                // `overwrite=true`: the key was just minted, so any stale sidecar belongs to a
                // dead key and must be replaced under these fresh secrets.
                establish_local_state_escrow(&cli.key, &op, &code, true)?;
                kp
            };
            let node_id = cairn_node::identity::provision(&db, &sk, &kid, &name, &address).await?;
            println!(
                "provisioned node {node_id}\nfingerprint {}",
                cairn_event::short_fingerprint(&kid)?
            );
        }
        Cmd::SealKey { passphrase } => {
            use cairn_node::keystore::{key_at_rest_state, KeyAtRest};
            // Validate the file is a sealable plaintext key BEFORE minting or printing a
            // recovery code, so we never show an operator a code for an operation that
            // will then be rejected (which would look like a usable escrow but isn't).
            match key_at_rest_state(&cli.key) {
                KeyAtRest::Plaintext => {}
                KeyAtRest::Sealed { .. } => {
                    anyhow::bail!("key at {} is already sealed", cli.key.display())
                }
                KeyAtRest::Missing => anyhow::bail!(
                    "no key file at {} (run `cairn-node init` first)",
                    cli.key.display()
                ),
                KeyAtRest::Corrupt => anyhow::bail!(
                    "key at {} is neither a plaintext seed nor a sealed bundle; \
                                   refusing to seal",
                    cli.key.display()
                ),
            }
            let op = resolve_passphrase(passphrase)?;
            let code = Zeroizing::new(cairn_node::seal::generate_recovery_code());
            // Show the code BEFORE the in-place overwrite: a crash mid-write must not be
            // able to leave the sole key sealed under a code that was never displayed
            // (silent escrow loss). The shown-once code is the critical output.
            print_recovery_code(&code);
            cairn_node::keystore::seal_existing(&cli.key, &op, &code)?;
            // `overwrite=true`: sealing mints a FRESH recovery code, so the LSK must be
            // re-wrapped under it. A pre-existing `.lsk` (e.g. from an earlier
            // establish-local-state-key on the still-plaintext key) would otherwise stay
            // wrapped under the old code and desync from the just-resealed signing key.
            establish_local_state_escrow(&cli.key, &op, &code, true)?;
            println!("key at {} sealed.", cli.key.display());
        }
        Cmd::EstablishLocalStateKey { passphrase } => {
            let op = resolve_passphrase(passphrase)?;
            // The recovery code is the OFF-NODE secret; the node never stored it, so the
            // operator must type the one shown at `init`/`seal-key`.
            let code = Zeroizing::new(rpassword::prompt_password(
                "recovery code (from init/seal-key): ",
            )?);
            // Reject whitespace-only input, not just empty: `normalize_recovery_code`
            // (inside `establish_lsk`) strips ALL spacing, so a code of "   " would
            // normalize to empty and wrap the LSK under an effectively-empty recovery
            // secret. Trim only for the guard — pass the ORIGINAL `code` on, since
            // normalization already handles spacing/case during the wrap.
            if code.trim().is_empty() {
                anyhow::bail!("no recovery code provided");
            }
            // `overwrite=false`: this is the explicit "set up the escrow" verb, so refuse to
            // silently clobber a working escrow that protects already-written exports.
            establish_local_state_escrow(&cli.key, &op, &code, false)?;
            println!("local-state escrow established.");
        }
        Cmd::Identity => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            println!(
                "node_id     {}\npubkey      {}\nfingerprint {}\naddress     {}",
                id.node_id_hex, id.pubkey_hex, id.fingerprint, id.address
            );
        }
        Cmd::PairOffer { nonce } => {
            let sk = load_signing_key(&cli.key, true)?; // interactive: may prompt
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            let offer = cairn_node::pairing::make_offer(&id, &sk, &nonce)?;
            println!("{offer}");
        }
        Cmd::PairAccept { offer, role } => {
            let bundle = cairn_node::pairing::read_offer(&offer)?;
            eprintln!(
                "Peer fingerprint: {}\nConfirm it matches what the peer displays, then type YES:",
                bundle.fingerprint
            );
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if line.trim() != "YES" {
                anyhow::bail!("pairing aborted: fingerprint not confirmed");
            }
            let sk = load_signing_key(&cli.key, true)?; // interactive: may prompt
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Stamp signer_key_id with the key we actually sign with (the keystore),
            // not the DB row; on key/DB drift the door then gives a legible rejection.
            let kid = hex::encode(sk.verifying_key().to_bytes());
            cairn_node::identity::author_peer(
                &db,
                &sk,
                &kid,
                &id.node_id_hex,
                &bundle,
                role.as_deref(),
            )
            .await?;
            println!("peered with {}", bundle.node_id_hex);
        }
        Cmd::Peers => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let peers = cairn_node::identity::list_peers(&db).await?;
            if peers.is_empty() {
                println!("no peers");
            } else {
                for p in &peers {
                    println!(
                        "{} fp={} role={} status={}",
                        p.peer_node_id_hex,
                        p.fingerprint,
                        p.role.as_deref().unwrap_or("-"),
                        p.status,
                    );
                }
            }
        }
        Cmd::Unpeer { node_id } => {
            let sk = load_signing_key(&cli.key, true)?; // interactive: may prompt
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            cairn_node::identity::author_unpeer(&db, &sk, &kid, &id.node_id_hex, &node_id).await?;
            println!("unpeered {node_id}");
        }
        Cmd::ProvisionRuntimeRole { role } => {
            // DDL: connect with the privileges that loaded the schema (owner/superuser),
            // not the unprivileged runtime role we are about to create.
            let db = cairn_node::db::connect(&cli.conn).await?;
            cairn_node::db::provision_runtime_role(&db, &role).await?;
            println!(
                "runtime role '{role}' provisioned and granted cairn_node\n\
                 point the daemon at it, e.g. CAIRN_CONN=\"… user={role}\" cairn-node … run …\n\
                 (set a password with `ALTER ROLE {role} PASSWORD …` for a networked deployment)"
            );
        }
        Cmd::Status => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let st = cairn_node::identity::status(&db, &cli.key).await?;
            println!("node_id       {}", st.node_id_hex);
            if !st.initialized {
                println!(
                    "              (not provisioned — run `cairn-node init` to enroll this node)"
                );
            }
            println!("peers_active  {}", st.peers_active);
            println!("peers_revoked {}", st.peers_revoked);
            println!("keystore_ok   {}", st.keystore_ok);
            if !st.keystore_ok {
                println!("              (cannot author: keystore unreadable)");
            }
            println!("key_at_rest   {}", st.key_at_rest);
            println!("runtime_role  {}", st.runtime_role);
            if st.db_floor_enforced {
                println!("db_floor      ENFORCED (connected role cannot raw-INSERT node_event)");
            } else {
                println!(
                    "db_floor      BYPASSABLE — '{}' can raw-INSERT node_event; \
                     run runtime as the cairn_node role to enforce the gate",
                    st.runtime_role
                );
            }
            println!("dr_escrow     {}", st.dr_escrow);
            println!("recovery_esc  {}", st.recovery_escrow);
            println!("last_backup   {}", st.last_backup);
            println!("local_state   {}", st.local_state);
            if let Some(old) = &st.supersedes {
                println!("supersedes    {old}");
            }
        }
        Cmd::Backup { to, passphrase } => {
            // Reads node_event (any role with SELECT works) and writes a self-verifying
            // medium. Health is recorded only after the medium re-reads and verifies (see
            // backup_to), so it never over-claims.
            let db = cairn_node::db::connect(&cli.conn).await?;
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let health_path = cairn_node::backup::health_path_for(&cli.key);

            // Load the signing key NON-INTERACTIVELY (flag/env passphrase, or a plaintext key)
            // so the medium's self-marker can be SIGNED (tamper-evident on restore). We never
            // PROMPT here: an unattended/cron backup must not block on a tty, and an unsigned
            // marker is a safe degradation (operator-error-safe, just not tamper-evident) —
            // never a reason to fail the backup. A wrong/absent secret simply yields no key.
            let key_secret: Option<Zeroizing<String>> = passphrase
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    std::env::var("CAIRN_KEY_PASSPHRASE")
                        .ok()
                        .filter(|s| !s.is_empty())
                })
                .map(Zeroizing::new);
            let signing =
                cairn_node::keystore::load(&cli.key, key_secret.as_deref().map(|s| s.as_str()))
                    .ok()
                    .map(|sk| (hex::encode(sk.verifying_key().to_bytes()), sk));
            let marker_key = signing.as_ref().map(|(kid, sk)| (sk, kid.as_str()));

            let report =
                cairn_node::backup::backup_to(&db, &to, &health_path, now_unix, marker_key).await?;
            println!(
                "backed up {} event(s) ({} bytes) to {}",
                report.event_count,
                report.medium_bytes,
                to.display()
            );
            // How trustworthy is this medium's identity marker? An unsigned medium travels
            // flagged for extra care. A signed marker is UNFORGEABLE (no off-medium private key)
            // and bound to its event set; on a sole-enroll medium it is fully tamper-evident, on a
            // federated medium restore will ask for confirmation (a converged peer's medium could
            // be spliced — see crate::medium / restore::Provenance). Store any medium with care.
            match report.marker {
                cairn_node::backup::WrittenMarker::Signed => {
                    println!("self-marker  SIGNED (unforgeable; identity confirmed on restore)")
                }
                cairn_node::backup::WrittenMarker::Unsigned => eprintln!(
                    "WARNING: self-marker UNSIGNED — this medium is operator-error-safe but NOT \
                     tamper-evident; set CAIRN_KEY_PASSPHRASE / --passphrase (or use a plaintext \
                     key) to sign it. Store and handle this medium with extra care."
                ),
                cairn_node::backup::WrittenMarker::None => {
                    println!("self-marker  none (node not yet enrolled — nothing to attest)")
                }
            }
            println!("backup health recorded at {}", health_path.display());
            // ADR-0026 slice D: co-locate the sealed local-state export beside the medium,
            // IF the local-state escrow exists. Degrades honestly (warn, never fail the
            // event backup) when the escrow is absent — the events are the load-bearing copy.
            let sidecar = cairn_node::localstate::lsk_sidecar_path_for(&cli.key);
            match std::fs::read(&sidecar)
                .ok()
                .and_then(|b| cairn_node::localstate::parse_sidecar(&b).ok())
            {
                Some(wraps) => {
                    // The sealed export is OPTIONAL — the event medium + health are ALREADY
                    // written (the load-bearing copy). So ANY failure here (a passphrase an
                    // unattended/cron run cannot supply, a wrong passphrase, an I/O error)
                    // degrades honestly: warn + skip, exactly like the absent-escrow branch
                    // below. It must NEVER abort an already-complete event backup with a
                    // non-zero exit — that would page an operator over a backup that succeeded.
                    match seal_and_write_local_state_export(&db, &wraps, passphrase, &to).await {
                        Ok(export_path) => {
                            println!("local-state exported to {}", export_path.display())
                        }
                        Err(e) => eprintln!(
                            "WARNING: local-state export skipped: {e:#}. Backed up events only \
                             (they are the load-bearing copy and are safe); set \
                             CAIRN_KEY_PASSPHRASE or pass --passphrase to write the sealed export."
                        ),
                    }
                }
                None => eprintln!(
                    "WARNING: no local-state escrow ({} absent) — backed up events only; \
                     run `cairn-node establish-local-state-key` to enable the sealed export",
                    sidecar.display()
                ),
            }
        }
        Cmd::VerifyBackup { from } => {
            // Offline, no DB, no key: read the medium and check every signature. A
            // tampered/bit-rotted event fails the SAME check that catches a hostile peer.
            let bytes = std::fs::read(&from)
                .with_context(|| format!("reading backup medium {}", from.display()))?;
            let report = cairn_node::backup::verify_medium_bytes(&bytes)?;
            if report.all_intact() {
                println!(
                    "backup OK: {}/{} event(s) verified",
                    report.intact, report.total
                );
            } else {
                // Non-zero exit (bail) so a cron/health check detects a bad backup.
                anyhow::bail!(
                    "backup FAILED self-verification: {}/{} event(s) intact, first bad at index {:?}",
                    report.intact,
                    report.total,
                    report.first_bad
                );
            }
        }
        Cmd::Restore {
            from,
            superseded_node,
            passphrase,
            insecure_plaintext,
        } => {
            // 1. Read + verify the medium offline (no DB needed yet). Bail on tamper.
            let bytes = std::fs::read(&from)
                .with_context(|| format!("reading backup medium {}", from.display()))?;
            let container = cairn_node::medium::parse_container(&bytes)?;
            let report = cairn_node::backup::verify_events(&container.events);
            if !report.all_intact() {
                anyhow::bail!(
                    "refusing to restore a medium that fails self-verification: {}/{} intact, \
                     first bad at index {:?}",
                    report.intact,
                    report.total,
                    report.first_bad
                );
            }
            // 2. Resolve this node's OWN genesis on the medium (the dead node to supersede),
            //    from the medium's container-level self-marker — the events alone cannot say
            //    which enroll is self (set-union convergence; issue #53). A SIGNED marker on a
            //    sole-enroll medium is authoritative + tamper-evident; on a federated/converged
            //    (multi-enroll) medium it resolves self but carries a residual peer-medium splice
            //    risk (confirm below); UNSIGNED / no marker is flagged for confirmation too. An
            //    explicit --superseded-node is validated against the marker and rejected
            //    fail-closed if it names a peer or an off-medium id.
            let dead =
                cairn_node::restore::resolve_dead_node(&container, superseded_node.as_deref())?;
            use cairn_node::restore::Provenance;
            match dead.provenance {
                Provenance::Signed =>
                    println!("self-identity confirmed by a signed self-marker (tamper-evident)"),
                Provenance::SignedFederated => eprintln!(
                    "WARNING: this is a FEDERATED medium (carries peers' genesis too). The signed \
                     self-marker resolves self, but a converged peer's medium holds a byte-identical \
                     event set, so a peer's genuine marker could be spliced here — the signature \
                     alone cannot rule that out. Confirm the restored node's name/address printed \
                     below match THIS node before relying on it."),
                Provenance::Unsigned => eprintln!(
                    "WARNING: this medium's self-marker is UNSIGNED (not tamper-evident). Confirm \
                     the restored node's name/address printed below match THIS node before relying on it."),
                Provenance::NoMarker => eprintln!(
                    "WARNING: this medium carries NO self-marker (legacy/pre-enrollment backup). \
                     Self identity was taken from --superseded-node / a sole enroll; confirm the \
                     name/address below match THIS node."),
            }
            let (name, address) =
                cairn_node::restore::old_genesis_meta(&container.events, &dead.node_id_hex)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                        "internal: resolved dead node {} has no enroll on the medium (unreachable)",
                        dead.node_id_hex
                    )
                    })?;

            // 3. Connect to the FRESH db and load the schema (DDL: owner privileges, like init).
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            if cairn_node::identity::load_local_opt(&db).await?.is_some() {
                anyhow::bail!(
                    "target database already has an enrolled node; restore is only into a \
                     fresh, un-enrolled database (the restore door is fenced closed otherwise)"
                );
            }

            // 4. Mint the NEW key (the old signing key was never backed up).
            let (sk, kid) = if insecure_plaintext {
                eprintln!(
                    "WARNING: --insecure-plaintext: new key written UNSEALED (test use only)"
                );
                cairn_node::keystore::generate_plaintext(&cli.key)?
            } else {
                let op = resolve_passphrase(passphrase)?;
                let code = Zeroizing::new(cairn_node::seal::generate_recovery_code());
                // Show the recovery code BEFORE the key is persisted — same rationale as
                // `init`: a crash between persist and print would seal the disaster-recovery
                // node under a code no human ever saw, silently destroying the new escrow.
                // Printing first means the worst case is a shown code for an unwritten key
                // (restore simply re-runs), never a permanently sealed, unrecoverable node.
                print_recovery_code(&code);
                let kp = cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?;
                // The restored node gets its OWN day-one local-state escrow under its NEW
                // secrets (ADR-0026 slice D) — the old `.lsk` was on the dead disk.
                // `overwrite=true`: the key was just minted; replace any stale sidecar.
                establish_local_state_escrow(&cli.key, &op, &code, true)?;
                kp
            };

            // 5. Apply old events through the self-trusting door (db still un-enrolled),
            //    then author the new genesis + supersede.
            let applied = cairn_node::restore::apply_medium(&db, &container.events).await?;
            let outcome = cairn_node::restore::finalize_identity(
                &db,
                &sk,
                &kid,
                &name,
                &address,
                &dead.node_id_hex,
            )
            .await?;

            // ADR-0026 slice D: if a sealed local-state export sits beside the medium,
            // unseal it with the OLD recovery code and apply it (empty/noop today), then
            // give the NEW node its OWN local-state escrow. Absent export => skip (the node
            // restores from events alone — honest degradation).
            let export_path = cairn_node::localstate::localstate_path_for(&from);
            if let Ok(bytes) = std::fs::read(&export_path) {
                // A corrupt/bit-rotted export sibling must NOT bail — by this point the node
                // is ALREADY fully restored (key minted, events applied, identity finalized),
                // and off-site media bit-rot is a likely failure. Local-state is OPTIONAL and
                // the events are the load-bearing copy, so degrade honestly: warn and skip.
                match cairn_node::localstate::parse_container(&bytes) {
                    Ok(sealed) => {
                        eprintln!("Local-state export found. Enter the OLD node's recovery code to unseal it:");
                        let old_code = Zeroizing::new(
                            rpassword::prompt_password("old recovery code: ")?);
                        // Wrong recovery-code guess degrades the same way (warn + skip) — a bad
                        // guess at the OPTIONAL local-state must not kill an otherwise-complete
                        // restore. Only a non-empty bundle this version cannot apply stays loud
                        // (the `?` on apply_local_state below).
                        match cairn_node::localstate::unseal_local_state_rec(&sealed, &old_code) {
                            Some(plaintext) => {
                                let bundle = cairn_node::localstate::from_cbor(&plaintext)?;
                                cairn_node::localstate::apply_local_state(&db, &bundle).await?;
                                println!("local-state restored from {}", export_path.display());
                            }
                            None => eprintln!(
                                "WARNING: could not unseal the local-state export — wrong recovery code? \
                                 Skipping local-state; node restores from events alone."),
                        }
                    }
                    Err(_) => eprintln!(
                        "WARNING: local-state export present at {} but unreadable (corrupt/bit-rotted?) — \
                         skipping local-state; node restores from events alone.", export_path.display()),
                }
            }

            println!("restored {applied} event(s) from {}", from.display());
            // Always echo the adopted identity (name/address) so any self-mis-identification is
            // visible to the operator, whatever the marker provenance — paper-parity.
            println!("restored identity '{name}' ({address})");
            println!("new node {}", outcome.new_node_id_hex);
            println!("supersedes {}", outcome.superseded_node_id_hex);
            println!(
                "re-peer with `cairn-node pair-offer` / `pair-accept` (trust resets on restore)"
            );
        }
        Cmd::Serve { listen } => {
            use cairn_node::sync;
            let sk = load_signing_key(&cli.key, false)?; // unattended: never prompt, fail fast
            let db = cairn_node::db::connect(&cli.conn).await?;
            let trust = sync::trust_store_from_db(&db).await?;
            let (addr, serve_cfg) = sync::bind_serve(listen, &cli.conn, &sk, trust).await?;
            eprintln!("serving node_event sync on {addr}");
            sync::serve(serve_cfg).await?;
        }
        Cmd::Run {
            listen,
            peer,
            interval_secs,
        } => {
            use cairn_node::sync;
            let sk = load_signing_key(&cli.key, false)?; // unattended: never prompt, fail fast
            sync::run(listen, peer, &cli.conn, &sk, interval_secs).await?;
        }
        Cmd::Quarantine => {
            // Read-only inspection: no signing key needed.
            let db = cairn_node::db::connect(&cli.conn).await?;
            let rows = cairn_node::sync::list_node_quarantine(&db).await?;
            if rows.is_empty() {
                println!("no quarantined node_events");
            } else {
                for r in &rows {
                    println!("{r}");
                }
            }
        }
        Cmd::AckQuarantine { digest } => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let n = cairn_node::sync::ack_node_quarantine(&db, &digest).await?;
            if n == 0 {
                anyhow::bail!(
                    "no quarantined node_event has digest {digest} \
                     (list them with `cairn-node quarantine`)"
                );
            }
            println!("acked node_event {digest} ({n} row) — it no longer pins the floor or fails the pull");
        }
        Cmd::ApplyAutoCandidates {
            passphrase,
            insecure_plaintext,
        } => {
            // Owner connection (needs enroll_actor for the per-epoch matcher actor).
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            // Fail fast (legibly) if the DB predates the db/018 identity floor.
            let has_floor: bool = db
                .query_one("SELECT to_regclass('public.patient_link') IS NOT NULL", &[])
                .await?
                .get(0);
            if !has_floor {
                anyhow::bail!(
                    "this database predates db/018 (no patient_link) — run `cairn-node init` \
                     to load the identity floor"
                );
            }
            // The matcher keystore lives beside the node key. Seal the per-epoch matcher
            // keys under the SAME policy as the node key: sealed by default (passphrase from
            // --passphrase / CAIRN_KEY_PASSPHRASE / interactive prompt), plaintext ONLY on an
            // explicit --insecure-plaintext. Reading the secret from the env var alone would
            // silently write plaintext matcher keys beside a node key sealed via --passphrase
            // or a prompt — a silent at-rest downgrade for keys that author identity links.
            let keystore_dir = cli
                .key
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("matcher-keys");
            let secret: Option<Zeroizing<String>> = if insecure_plaintext {
                None
            } else {
                Some(resolve_passphrase(passphrase)?)
            };
            let node_origin = cairn_node::identity::load_local(&db).await?.node_id_hex;
            let s = cairn_node::auto_apply::apply_auto_candidates(
                &mut db,
                &keystore_dir,
                secret.as_ref().map(|z| z.as_str()),
                &node_origin,
            )
            .await?;
            println!(
                "auto-apply: applied {}  vetoed->review {}  skipped {}  errored {}",
                s.applied, s.vetoed_to_review, s.skipped, s.errored
            );
            // Non-zero exit when anything errored, so a systematic failure can't pass as a
            // healthy quiet run in a cron/pipeline (the summary line is still printed above).
            if s.errored > 0 {
                anyhow::bail!(
                    "{} pair(s) errored during auto-apply (see stderr above)",
                    s.errored
                );
            }
        }
        Cmd::RegisterJohnDoe { class, site, basis } => {
            let sk = load_signing_key(&cli.key, true)?; // interactive: may prompt to unseal
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // The callsign's site defaults to this node's id; its date comes from the node's
            // own DB clock (no date dependency — the DB is the integration substrate).
            let site = site.unwrap_or_else(|| id.node_id_hex.clone());
            let date: String = db.query_one("SELECT current_date::text", &[]).await?.get(0);
            // Owner ceremony: make the signing key an enrolled actor so it may author the
            // additive registration events (idempotent — enrolls only on first use).
            ensure_registration_actor(&db, &kid).await?;
            let (pid, call, ordinal) = cairn_node::john_doe::register_john_doe(
                &mut db,
                &sk,
                &kid,
                &id.node_id_hex,
                &class,
                &site,
                &date,
                &basis,
            )
            .await?;
            println!("registered John Doe {pid}\ncallsign {call}\nlocal ref: John Doe #{ordinal} (this node)");
        }
        Cmd::AssertObservedEvidence {
            patient,
            age,
            tol,
            age_basis,
            sex,
            sex_basis,
        } => {
            let sk = load_signing_key(&cli.key, true)?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Observation year comes from the node's own DB clock (the DB is the clock).
            let observed_year: i32 = db
                .query_one("SELECT extract(year FROM current_date)::int", &[])
                .await?
                .get(0);
            ensure_registration_actor(&db, &kid).await?;

            // Clinical sanity bound on the human-entered estimate: a real apparent age and
            // its tolerance are both well under a human lifespan. Rejecting absurd input here
            // (honest reject, principle 4 — never fabricate a range) also keeps the downstream
            // `u32 -> i32` age arithmetic in `birth_year_range_from_age` far from any overflow.
            const MAX_OBSERVED_AGE_YEARS: u32 = 150;
            let age_obs = match (age, age_basis) {
                (Some(age_years), Some(_)) if age_years > MAX_OBSERVED_AGE_YEARS || tol > MAX_OBSERVED_AGE_YEARS =>
                    anyhow::bail!("--age and --tol must each be <= {MAX_OBSERVED_AGE_YEARS} years (implausible estimate)"),
                (Some(age_years), Some(basis)) =>
                    Some(cairn_node::evidence::AgeObservation { age_years, tolerance_years: tol, basis }),
                (Some(_), None) => anyhow::bail!("--age requires --age-basis (§5.4: estimated age WITH basis)"),
                (None, _) => None,
            };
            let sex_obs = sex.map(|value| cairn_node::evidence::SexObservation {
                value,
                basis: sex_basis,
            });
            let ev = cairn_node::evidence::ObservedEvidence {
                age: age_obs,
                sex: sex_obs,
            };

            cairn_node::evidence::assert_observed_evidence(
                &mut db,
                &sk,
                &kid,
                &id.node_id_hex,
                patient,
                &ev,
                observed_year,
            )
            .await?;
            println!("recorded clinician-observed evidence on {patient}");
        }
        Cmd::AssertIdentityEvidence {
            patient,
            kind,
            description,
            file,
            media_type,
            descriptor,
            basis,
        } => {
            use cairn_node::identity_evidence::EvidenceRoute;
            // Resolve the flag combination to ONE evidence shape (pure, tested) before any
            // keystore/DB/file I/O — the single "--file iff --kind photo" gate. The libraries
            // then re-check content (non-empty descriptor/description) as the principle-4 floor.
            let route = cairn_node::identity_evidence::route_identity_evidence(
                &kind,
                file,
                media_type,
                descriptor,
                description,
                basis,
            )?;
            let sk = load_signing_key(&cli.key, true)?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &kid).await?;

            match route {
                EvidenceRoute::Photo {
                    file,
                    media_type,
                    descriptor,
                    basis,
                } => {
                    // Fast-fail on an empty descriptor before reading the file — same rule the
                    // library re-checks (single source of truth: validate_photo_descriptor).
                    cairn_node::photo_evidence::validate_photo_descriptor(&descriptor)?;
                    let bytes = std::fs::read(&file)
                        .map_err(|e| anyhow::anyhow!("reading {}: {e}", file.display()))?;
                    let event_id = cairn_node::photo_evidence::assert_photo_evidence(
                        &mut db,
                        &sk,
                        &kid,
                        &id.node_id_hex,
                        patient,
                        &bytes,
                        &media_type,
                        &descriptor,
                        basis.as_deref(),
                    )
                    .await?;
                    println!("attached photo evidence {event_id} to {patient}");
                }
                EvidenceRoute::Text {
                    kind,
                    description,
                    basis,
                } => {
                    let event_id = cairn_node::identity_evidence::assert_text_evidence(
                        &db,
                        &sk,
                        &kid,
                        &id.node_id_hex,
                        patient,
                        kind,
                        &description,
                        basis.as_deref(),
                    )
                    .await?;
                    println!("recorded {kind} identity evidence {event_id} on {patient}");
                }
            }
        }
    }
    Ok(())
}

/// Ensure the node's signing key is enrolled as an actor that may author the additive §5.4
/// John-Doe registration events. Enrolls a `device` actor ONLY when this key is not already
/// enrolled under ANY kind. An owner ceremony — the runtime `cairn_agent` role deliberately
/// cannot enroll. A real clinical UI would attach the operating clerk's *human* actor
/// instead; this device-key path is the headless-node/CLI convenience.
///
/// The existence check is deliberately kind-AGNOSTIC. `submit_event` resolves a signer to an
/// actor purely by `signing_key_id` (kind matters only for attestation), and if one key maps
/// to MORE than one `actor_current` row it sets `actor_id = NULL` for EVERY event that key
/// authors node-wide (db/005 `array_length(v_actor_ids, 1) = 1`), silently and irreversibly
/// degrading attribution. A kind-scoped `AND kind = 'device'` guard would happily add a
/// second actor to a key already enrolled as (say) a matcher `agent` or a `human`, tripping
/// exactly that dual-mapping. Keying on `signing_key_id` alone means a key already usable for
/// authoring is left untouched — never split into two actors.
async fn ensure_registration_actor(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<()> {
    let already: bool = db
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_current WHERE signing_key_id = $1)",
            &[&kid],
        )
        .await?
        .get(0);
    if !already {
        let pinned =
            serde_json::json!({ "role": "registration-desk", "node_key": kid }).to_string();
        db.execute(
            "SELECT enroll_actor('device', $1::text::jsonb, $2)",
            &[&pinned, &kid],
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_passphrase_from_flag_is_zeroizing() {
        // The flag (also clap-filled from CAIRN_KEY_PASSPHRASE) must come back wrapped in
        // `Zeroizing` so the secret is wiped from heap memory on drop (issue #46). The type
        // annotation IS the assertion: this fails to compile if the secret is a bare String.
        let secret: zeroize::Zeroizing<String> =
            resolve_passphrase(Some("op-pass".to_string())).unwrap();
        assert_eq!(
            secret.as_str(),
            "op-pass",
            "a non-empty flag is returned verbatim"
        );
    }
}
