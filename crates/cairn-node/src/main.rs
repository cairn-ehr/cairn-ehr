use anyhow::Context;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use zeroize::Zeroizing;

/// The single prompt string + no-echo behaviour for the operational passphrase,
/// shared by every command that reads the secret interactively. One copy so the
/// wording and echo policy can never drift between `init`/`seal-key` and the runtime.
///
/// Returns a `Zeroizing<String>` so the secret is wiped from heap memory on drop
/// (issue #46): `rpassword` flushes its own internal buffer, but the copy we hold and
/// pass on to the KDF would otherwise linger in freed memory.
fn prompt_passphrase() -> anyhow::Result<Zeroizing<String>> {
    Ok(Zeroizing::new(rpassword::prompt_password("operational passphrase: ")?))
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
fn load_signing_key(path: &std::path::Path, allow_prompt: bool)
    -> anyhow::Result<cairn_event::SigningKey> {
    use cairn_node::keystore::{load, KeystoreError};
    // Hold the env-provided secret as Zeroizing too, so the copy we lifted out of the
    // environment is wiped on drop (issue #46). We can't scrub the OS env store itself.
    let env_secret = std::env::var("CAIRN_KEY_PASSPHRASE").ok()
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
    #[arg(long, env = "CAIRN_CONN")] conn: String,
    #[arg(long, default_value = "node.key")] key: PathBuf,
    #[command(subcommand)] cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Provision this node: mint a keypair (SEALED by default) and append genesis.
    Init {
        #[arg(long)] name: String,
        #[arg(long)] address: String,
        /// Operational passphrase (else CAIRN_KEY_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")] passphrase: Option<String>,
        /// Write the key UNSEALED (test nodes only — no recovery escrow).
        #[arg(long)] insecure_plaintext: bool,
    },
    /// Seal an existing plaintext key file and mint a fresh recovery code.
    SealKey {
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")] passphrase: Option<String>,
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
        #[arg(long)] role: Option<String>,
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { name, address, passphrase, insecure_plaintext } => {
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let (sk, kid) = if insecure_plaintext {
                eprintln!("WARNING: --insecure-plaintext: signing key written UNSEALED (test use only)");
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
                cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?
            };
            let node_id = cairn_node::identity::provision(&db, &sk, &kid, &name, &address).await?;
            println!("provisioned node {node_id}\nfingerprint {}", cairn_event::short_fingerprint(&kid)?);
        }
        Cmd::SealKey { passphrase } => {
            use cairn_node::keystore::{key_at_rest_state, KeyAtRest};
            // Validate the file is a sealable plaintext key BEFORE minting or printing a
            // recovery code, so we never show an operator a code for an operation that
            // will then be rejected (which would look like a usable escrow but isn't).
            match key_at_rest_state(&cli.key) {
                KeyAtRest::Plaintext => {}
                KeyAtRest::Sealed { .. } =>
                    anyhow::bail!("key at {} is already sealed", cli.key.display()),
                KeyAtRest::Missing =>
                    anyhow::bail!("no key file at {} (run `cairn-node init` first)", cli.key.display()),
                KeyAtRest::Corrupt =>
                    anyhow::bail!("key at {} is neither a plaintext seed nor a sealed bundle; \
                                   refusing to seal", cli.key.display()),
            }
            let op = resolve_passphrase(passphrase)?;
            let code = Zeroizing::new(cairn_node::seal::generate_recovery_code());
            // Show the code BEFORE the in-place overwrite: a crash mid-write must not be
            // able to leave the sole key sealed under a code that was never displayed
            // (silent escrow loss). The shown-once code is the critical output.
            print_recovery_code(&code);
            cairn_node::keystore::seal_existing(&cli.key, &op, &code)?;
            println!("key at {} sealed.", cli.key.display());
        }
        Cmd::Identity => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            println!("node_id     {}\npubkey      {}\nfingerprint {}\naddress     {}",
                id.node_id_hex, id.pubkey_hex, id.fingerprint, id.address);
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
                &db, &sk, &kid, &id.node_id_hex, &bundle, role.as_deref(),
            ).await?;
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
            cairn_node::identity::author_unpeer(
                &db, &sk, &kid, &id.node_id_hex, &node_id,
            ).await?;
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
                println!("              (not provisioned — run `cairn-node init` to enroll this node)");
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
            if let Some(old) = &st.supersedes {
                println!("supersedes    {old}");
            }
        }
        Cmd::Backup { to } => {
            // Reads node_event (any role with SELECT works) and writes a self-verifying
            // medium; no signing key is loaded. Health is recorded only after the medium
            // re-reads and verifies (see backup_to), so it never over-claims.
            let db = cairn_node::db::connect(&cli.conn).await?;
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let health_path = cairn_node::backup::health_path_for(&cli.key);
            let report = cairn_node::backup::backup_to(&db, &to, &health_path, now_unix).await?;
            println!(
                "backed up {} event(s) ({} bytes) to {}",
                report.event_count,
                report.medium_bytes,
                to.display()
            );
            println!("backup health recorded at {}", health_path.display());
        }
        Cmd::VerifyBackup { from } => {
            // Offline, no DB, no key: read the medium and check every signature. A
            // tampered/bit-rotted event fails the SAME check that catches a hostile peer.
            let bytes = std::fs::read(&from)
                .with_context(|| format!("reading backup medium {}", from.display()))?;
            let report = cairn_node::backup::verify_medium_bytes(&bytes)?;
            if report.all_intact() {
                println!("backup OK: {}/{} event(s) verified", report.intact, report.total);
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
        Cmd::Restore { from, superseded_node, passphrase, insecure_plaintext } => {
            // 1. Read + verify the medium offline (no DB needed yet). Bail on tamper.
            let bytes = std::fs::read(&from)
                .with_context(|| format!("reading backup medium {}", from.display()))?;
            let events = cairn_node::backup::parse_medium(&bytes)?;
            let report = cairn_node::backup::verify_events(&events);
            if !report.all_intact() {
                anyhow::bail!(
                    "refusing to restore a medium that fails self-verification: {}/{} intact, \
                     first bad at index {:?}",
                    report.intact, report.total, report.first_bad
                );
            }
            // 2. Resolve the dead node-id (solo auto-detect, else --superseded-node).
            //    resolve_dead_node_id guarantees the id names an enroll on the medium, so
            //    old_genesis_meta always resolves; the bail is a defensive invariant, never a
            //    silent synthetic identity (paper-parity: a restored node keeps its name/address).
            let dead = cairn_node::restore::resolve_dead_node_id(&events, superseded_node.as_deref())?;
            let (name, address) = cairn_node::restore::old_genesis_meta(&events, &dead)
                .ok_or_else(|| anyhow::anyhow!(
                    "internal: resolved dead node {dead} has no enroll on the medium (unreachable)"
                ))?;

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
                eprintln!("WARNING: --insecure-plaintext: new key written UNSEALED (test use only)");
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
                cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?
            };

            // 5. Apply old events through the self-trusting door (db still un-enrolled),
            //    then author the new genesis + supersede.
            let applied = cairn_node::restore::apply_medium(&db, &events).await?;
            let outcome = cairn_node::restore::finalize_identity(
                &db, &sk, &kid, &name, &address, &dead).await?;

            println!("restored {applied} event(s) from {}", from.display());
            println!("new node {}", outcome.new_node_id_hex);
            println!("supersedes {}", outcome.superseded_node_id_hex);
            println!("re-peer with `cairn-node pair-offer` / `pair-accept` (trust resets on restore)");
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
        Cmd::Run { listen, peer, interval_secs } => {
            use cairn_node::sync;
            let sk = load_signing_key(&cli.key, false)?; // unattended: never prompt, fail fast
            sync::run(listen, peer, &cli.conn, &sk, interval_secs).await?;
        }
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
        assert_eq!(secret.as_str(), "op-pass", "a non-empty flag is returned verbatim");
    }
}
