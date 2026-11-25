//! `cairn-release`: the steward's OFFLINE signing tool (ADR-0055). It reads and writes
//! files and nothing else; every rule lives in `cairn_release::steward` builders and in the
//! verifier. It must never run on a node or in CI (it unseals signing keys).
//!
//! Key files use `cairn-keystore`'s sealed CAIRNK1 format. The passphrase comes from the
//! environment variable named by `--passphrase-env` (no prompt, so it scripts cleanly);
//! this tool never writes a plaintext key. (An environment variable is readable by the same
//! user's other processes; acceptable for a tool that runs only on the steward's offline
//! machine.)

use cairn_event::SigningKey;
use cairn_keystore::{keystore, seal};
use cairn_release::{
    advance, assemble, decode, encode, genesis, make_genesis, rotate, Artifact, BundleFile,
    SignedRoot, Signer, TrustRoot, VerifiedRoot,
};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(about = "Steward's offline signing tool for Cairn releases")]
struct Cli {
    /// Environment variable holding the key-file passphrase (or recovery code).
    #[arg(long, global = true, default_value = "CAIRN_RELEASE_PASSPHRASE")]
    passphrase_env: String,
    #[command(subcommand)]
    cmd: Cmd,
}

/// The signer sets and thresholds that make up a trust root's body.
#[derive(Args)]
struct RootBody {
    /// Root signer public key (64 hex chars); repeat for several.
    #[arg(long = "root-signer", required = true)]
    root_signers: Vec<String>,
    #[arg(long)]
    root_threshold: u32,
    /// Release signer public key (64 hex chars); repeat for several.
    #[arg(long = "release-signer", required = true)]
    release_signers: Vec<String>,
    #[arg(long)]
    release_threshold: u32,
}

#[derive(Subcommand)]
enum Cmd {
    /// Make a sealed key file; prints the public key (hex) to stdout, the recovery code to stderr.
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    /// Write a channel's first (version 1) trust root.
    Genesis {
        #[arg(long)]
        channel: String,
        #[command(flatten)]
        body: RootBody,
        /// Key file of a root signer; repeat to meet the root threshold.
        #[arg(long = "key", required = true)]
        keys: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Write the next trust root, signed by the PREVIOUS root's signers.
    Rotate {
        /// Existing signed roots, genesis first (ascending).
        #[arg(long = "history", required = true)]
        history: Vec<PathBuf>,
        #[command(flatten)]
        body: RootBody,
        #[arg(long = "key", required = true)]
        keys: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Write a signed release bundle holding one window artifact.
    Bundle {
        #[arg(long = "history", required = true)]
        history: Vec<PathBuf>,
        /// Key file of a release signer; repeat to meet the release threshold.
        #[arg(long = "key", required = true)]
        keys: Vec<PathBuf>,
        #[arg(long)]
        artifact: PathBuf,
        /// CI's window.json: {generation_line, release_version, platform}.
        #[arg(long)]
        sidecar: PathBuf,
        /// Must equal the sidecar's release_version (a cross-check against a mixed-up build).
        #[arg(long)]
        release_version: u64,
        #[arg(long)]
        out: PathBuf,
    },
}

/// Shown beside every signer this tool names; one place to change when custody is richer.
const CUSTODY_NOTE: &str = "steward-held key";

/// Public key (hex) -> a `Signer`. The kid is the key's first 4 bytes, a readable label only;
/// verification names keys by their full public key.
fn signer_from_hex(h: &str) -> Result<Signer, String> {
    let pubkey: [u8; 32] = hex::decode(h)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| format!("not a 32-byte hex public key: {h}"))?;
    Ok(Signer {
        kid: pubkey[..4].to_vec(),
        pubkey,
        custody_note: CUSTODY_NOTE.into(),
    })
}

fn signers(hexes: &[String]) -> Result<Vec<Signer>, String> {
    hexes.iter().map(|h| signer_from_hex(h)).collect()
}

fn load_keys(paths: &[PathBuf], pass: &str) -> Result<Vec<SigningKey>, String> {
    paths
        .iter()
        .map(|p| keystore::load(p, Some(pass)).map_err(|e| format!("{}: {e}", p.display())))
        .collect()
}

/// Read the root files in order and verify them into a history (genesis, then advance).
fn load_history(paths: &[PathBuf]) -> Result<Vec<VerifiedRoot>, String> {
    let mut roots = Vec::new();
    for p in paths {
        let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        roots.push(decode::<SignedRoot>(&bytes).map_err(|e| format!("{}: {e:?}", p.display()))?);
    }
    // clap guarantees `--history` is non-empty (`required = true`), so index 0 exists.
    let first = genesis(&roots[0]).map_err(|e| format!("genesis: {e:?}"))?;
    advance(&[first], &roots[1..]).map_err(|e| format!("history: {e:?}"))
}

/// What CI writes next to the window program.
#[derive(serde::Deserialize)]
struct Sidecar {
    generation_line: String,
    release_version: u64,
    platform: String,
}

/// Write a public output file (root or bundle) atomically, mode 0644: a crash never leaves
/// a half-written release file.
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    cairn_keystore::fsio::atomic_write(path, bytes, Some(0o644))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn run(cli: Cli) -> Result<(), String> {
    let pass = std::env::var(&cli.passphrase_env)
        .map_err(|_| format!("environment variable {} is not set", cli.passphrase_env))?;
    match cli.cmd {
        Cmd::Keygen { out } => {
            // A mistyped --out must never replace the steward's only signing key.
            if out.exists() {
                return Err(format!(
                    "{} already exists; refusing to overwrite a key",
                    out.display()
                ));
            }
            let code = seal::generate_recovery_code();
            let (_, kid) =
                keystore::generate_sealed(&out, &pass, &code).map_err(|e| e.to_string())?;
            eprintln!("RECOVERY CODE (shown once; write it on paper and store it apart): {code}");
            println!("{kid}");
        }
        Cmd::Genesis {
            channel,
            body,
            keys,
            out,
        } => {
            let sks = load_keys(&keys, &pass)?;
            let refs: Vec<&SigningKey> = sks.iter().collect();
            let g = make_genesis(
                &channel,
                signers(&body.root_signers)?,
                body.root_threshold,
                signers(&body.release_signers)?,
                body.release_threshold,
                &refs,
            );
            // Refuse to write a root a terminal would reject (too few signatures, bad threshold).
            genesis(&g).map_err(|e| format!("not a valid genesis: {e:?}"))?;
            write(&out, &encode(&g))?;
        }
        Cmd::Rotate {
            history,
            body,
            keys,
            out,
        } => {
            let h = load_history(&history)?;
            let prev = h.last().expect("history is non-empty");
            let next = TrustRoot {
                channel_id: prev.root().channel_id.clone(),
                version: 0,           // set by `rotate`
                prev_root_hash: None, // set by `rotate`
                root_signers: signers(&body.root_signers)?,
                root_threshold: body.root_threshold,
                release_signers: signers(&body.release_signers)?,
                release_threshold: body.release_threshold,
            };
            let sks = load_keys(&keys, &pass)?;
            let refs: Vec<&SigningKey> = sks.iter().collect();
            let signed = rotate(prev, next, &refs);
            // Prove it before writing: it must advance the history it extends.
            advance(&h, std::slice::from_ref(&signed))
                .map_err(|e| format!("not a valid rotation: {e:?}"))?;
            write(&out, &encode(&signed))?;
        }
        Cmd::Bundle {
            history,
            keys,
            artifact,
            sidecar,
            release_version,
            out,
        } => {
            let side: Sidecar = serde_json::from_slice(
                &std::fs::read(&sidecar).map_err(|e| format!("{}: {e}", sidecar.display()))?,
            )
            .map_err(|e| format!("window.json: {e}"))?;
            if side.release_version != release_version {
                return Err(format!(
                    "window.json says release {} but --release-version is {release_version}",
                    side.release_version
                ));
            }
            let h = load_history(&history)?;
            let bytes =
                std::fs::read(&artifact).map_err(|e| format!("{}: {e}", artifact.display()))?;
            let shell = Artifact {
                kind: "window".into(),
                digest: [0; 32], // filled by `assemble`
                byte_len: 0,     // filled by `assemble`
                // Hard-coded: linux/aarch64 is the only target built. A second target needs
                // os/arch fields in window.json.
                os: Some("linux".into()),
                arch: Some("aarch64".into()),
                platform: Some(side.platform),
                generation_line: Some(side.generation_line),
            };
            let sks = load_keys(&keys, &pass)?;
            let refs: Vec<&SigningKey> = sks.iter().collect();
            let bundle: BundleFile = assemble(&h, release_version, vec![(shell, bytes)], &refs);
            // Prove it before writing: a bundle that does not verify is never shipped.
            cairn_release::verify_bundle(&h, &bundle.meta, 0)
                .map_err(|e| format!("not a valid bundle: {e:?}"))?;
            write(&out, &encode(&bundle))?;
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("cairn-release: {e}");
        std::process::exit(1);
    }
}
