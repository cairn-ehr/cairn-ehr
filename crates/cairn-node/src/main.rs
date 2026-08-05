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

/// Load the human attester key for `identify-patient --link`. Mirrors `load_signing_key`
/// but keyed on the SEPARATE attester passphrase (flag / CAIRN_ATTESTER_PASSPHRASE / prompt)
/// so the attester key is distinct from the node's own operational key.
fn load_attester_key(
    path: &std::path::Path,
    passphrase: Option<String>,
) -> anyhow::Result<cairn_event::SigningKey> {
    use cairn_node::keystore::{load, KeystoreError};
    // Hold the secret in Zeroizing so it is wiped on drop (issue #46).
    let secret = passphrase.filter(|s| !s.is_empty()).map(Zeroizing::new);
    match load(path, secret.as_ref().map(|s| s.as_str())) {
        Ok(sk) => Ok(sk),
        Err(KeystoreError::Sealed) => {
            let p = prompt_passphrase()?;
            Ok(load(path, Some(p.as_str()))?)
        }
        Err(e) => Err(e.into()),
    }
}

/// The `--attest-as` flag set, shared by every medication verb (author-time
/// convenience) and the standalone `medication-attest` command (post-hoc sign-off).
/// `--attest-as` present ⇒ a human vouches for the affected thread(s); absent ⇒ the
/// event stays device-additive (no vouch) — the slice-4 responsibility overlay is
/// opt-in everywhere except `medication-attest` itself, where a vouch is the whole
/// point (see `resolve_attester`'s caller-side `.ok_or_else` on that command).
#[derive(clap::Args, Clone)]
struct AttestFlags {
    /// Take clinical responsibility for the affected thread(s): a human key that
    /// signs+attests the attestation. Absent ⇒ device-additive (no vouch). Requires
    /// the affected thread(s) to be present locally — you can only vouch for content
    /// you can see, so if a thread has no local content events the whole verb is
    /// refused and rolled back (offline-first still applies to the plain, un-attested
    /// verb; re-run without --attest-as, or attest post-hoc once the thread has synced).
    #[arg(long)]
    attest_as: Option<std::path::PathBuf>,
    /// Passphrase to unseal --attest-as (else CAIRN_ATTESTER_PASSPHRASE, else prompt).
    /// Shares the env var with `identify-patient --attester-key`: both unseal a human
    /// attester key under the same operator convention.
    #[arg(long, env = "CAIRN_ATTESTER_PASSPHRASE")]
    attest_passphrase: Option<String>,
    /// Optional context recorded on the vouch (e.g. "admission reconciliation").
    #[arg(long)]
    basis: Option<String>,
    /// Optional free-text note on the vouch.
    #[arg(long)]
    note: Option<String>,
}

/// The first 8 characters of a key id, for a display line that has no room for 64.
///
/// Takes CHARACTERS, not bytes. A kid is `hex::encode` output today, so `&kid[..8]` would
/// be equivalent — but a byte slice panics the moment anything non-ASCII reaches it, and a
/// display helper is exactly where an unvalidated string eventually arrives. Truncating on
/// a char boundary cannot panic, whatever it is handed (#338 review finding 6).
///
/// This is an ABBREVIATION for reading, never an identifier to act on: two attesters
/// sharing a prefix would render alike. Anything that must distinguish attesters uses the
/// full kid.
fn short_kid(kid: &str) -> String {
    kid.chars().take(8).collect()
}

/// True when attest CONTEXT flags were given but there is no key to attest with —
/// the "nothing to attest" case. Deliberately ignores the passphrase: it carries an
/// `env` fallback and may be set without intent (see `resolve_attester`).
fn attest_context_without_key(has_attest_as: bool, has_basis: bool, has_note: bool) -> bool {
    !has_attest_as && (has_basis || has_note)
}

/// Resolve `--attest-as` into a loaded human key + verified kid, or `None` when the
/// flag is absent. Runs the `attester_is_enrolled_human` legibility pre-check (the
/// db/005 gate is the real enforcement — this only gives a clean error before any
/// event is authored). Errors if a basis/note is given with no key (nothing to
/// attest — refuse loudly, mirroring `identify-patient`'s cross-flag check for
/// --link/--attester-key). `attest_passphrase` is deliberately EXCLUDED from this
/// guard: it carries `env = "CAIRN_ATTESTER_PASSPHRASE"` (see `AttestFlags`), so it
/// can be `Some` purely from a session-wide exported env var with no intent to
/// attest at all — gating on it would break every plain device-additive verb call
/// (e.g. `medication-assert` with no `--attest-as`) whenever that env var happens to
/// be set. `identify-patient`'s own cross-flag check never gates on its passphrase
/// field either, for the same reason.
async fn resolve_attester(
    client: &tokio_postgres::Client,
    flags: &AttestFlags,
) -> anyhow::Result<Option<(cairn_event::SigningKey, String)>> {
    match &flags.attest_as {
        None => {
            if attest_context_without_key(false, flags.basis.is_some(), flags.note.is_some()) {
                anyhow::bail!("--basis/--note require --attest-as: nothing to attest");
            }
            Ok(None)
        }
        Some(path) => {
            let sk = load_attester_key(path, flags.attest_passphrase.clone())?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            if !cairn_node::identify::attester_is_enrolled_human(client, &kid).await? {
                anyhow::bail!(
                    "--attest-as key is not an enrolled human actor; run `enroll-human` first"
                );
            }
            Ok(Some((sk, kid)))
        }
    }
}

/// Borrow a resolved attester (from `resolve_attester`) plus its context flags into
/// the `AttestParams` the medication orchestrators take, or `None` when no `--attest-as`
/// was given (device-additive). Extracted so the six verb handlers below share one
/// construction instead of repeating the same borrow dance — `AttestParams` borrows
/// from BOTH `resolved` and `flags`, so both must outlive the returned value.
fn attest_params<'a>(
    resolved: &'a Option<(cairn_event::SigningKey, String)>,
    flags: &'a AttestFlags,
) -> Option<cairn_node::medication::AttestParams<'a>> {
    resolved
        .as_ref()
        .map(|(sk, kid)| cairn_node::medication::AttestParams {
            human_sk: sk,
            human_kid: kid,
            basis: flags.basis.as_deref(),
            note: flags.note.as_deref(),
        })
}

/// The `--author-as` flag set (#204 / ADR-0053): the human who AUTHORS the clinical
/// event. Present ⇒ the human's key signs the sealed content event and rides as an
/// `authored` contributor (session ≠ author); absent ⇒ device-additive (the node
/// signs, `recorded`-only), unchanged. Distinct from `--attest-as` (which layers the
/// separate ADR-0049 responsibility overlay); the two compose.
#[derive(clap::Args, Clone)]
struct AuthorFlags {
    /// Author this clinical event as a specific enrolled human: their key signs the
    /// event. Absent ⇒ device-additive (the node signs, no human author).
    #[arg(long)]
    author_as: Option<std::path::PathBuf>,
    /// Passphrase to unseal --author-as (else CAIRN_AUTHOR_PASSPHRASE, else prompt).
    #[arg(long, env = "CAIRN_AUTHOR_PASSPHRASE")]
    author_passphrase: Option<String>,
}

/// Resolve `--author-as` into a loaded human key + verified kid, or `None` when the
/// flag is absent. Runs the same enrolled-human pre-check as `resolve_attester` (the
/// db/005 authorship binding is the real enforcement — this only gives a clean error
/// before any event is authored).
async fn resolve_author(
    client: &tokio_postgres::Client,
    flags: &AuthorFlags,
) -> anyhow::Result<Option<(cairn_event::SigningKey, String)>> {
    match &flags.author_as {
        None => Ok(None),
        Some(path) => {
            let sk = load_attester_key(path, flags.author_passphrase.clone())?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            if !cairn_node::identify::attester_is_enrolled_human(client, &kid).await? {
                anyhow::bail!(
                    "--author-as key is not an enrolled human actor; run `enroll-human` first"
                );
            }
            Ok(Some((sk, kid)))
        }
    }
}

/// Borrow a resolved author into `AuthorParams`, or `None` (device-additive).
fn author_params<'a>(
    resolved: &'a Option<(cairn_event::SigningKey, String)>,
) -> Option<cairn_node::medication::AuthorParams<'a>> {
    resolved
        .as_ref()
        .map(|(sk, kid)| cairn_node::medication::AuthorParams {
            human_sk: sk,
            human_kid: kid,
        })
}

#[cfg(test)]
mod attest_context_tests {
    use super::attest_context_without_key;

    /// --basis with no --attest-as is a genuine "nothing to attest" refusal.
    #[test]
    fn basis_without_key_is_refused() {
        assert!(attest_context_without_key(false, true, false));
    }

    /// --note with no --attest-as is a genuine "nothing to attest" refusal.
    #[test]
    fn note_without_key_is_refused() {
        assert!(attest_context_without_key(false, false, true));
    }

    /// No --attest-as, no --basis, no --note: this is the case where ONLY
    /// CAIRN_ATTESTER_PASSPHRASE happens to be exported in the environment (a
    /// documented operator convention) — the predicate deliberately does not see
    /// the passphrase at all, so a plain device-additive verb call must be allowed.
    #[test]
    fn env_passphrase_alone_is_allowed() {
        assert!(!attest_context_without_key(false, false, false));
    }

    /// --attest-as present with basis and note: a real attestation, never refused.
    #[test]
    fn key_present_is_never_refused() {
        assert!(!attest_context_without_key(true, true, true));
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

    /// Search this node's charts before creating one (§5.8 item 1). Advisory: it ranks
    /// nothing and decides nothing — it shows a human what exists.
    PatientSearch {
        /// Name as typed. Word order does not matter. Each whitespace-separated word
        /// searches BOTH as a whole (edge punctuation trimmed, so "O'Brien-Smith" stays
        /// one token) AND as its alphanumeric parts ("brien", "smith").
        #[arg(long, default_value = "")]
        name: String,
        /// ISO YYYY-MM-DD (or YYYY / YYYY-MM — an exact match on the value as stored).
        #[arg(long)]
        birth_date: Option<String>,
        /// Repeatable `system=value`, e.g. --identifier MRN=12345
        #[arg(long = "identifier")]
        identifiers: Vec<String>,
    },

    /// Register a standard patient, recording the search that preceded it (§5.8).
    ///
    /// The search runs HERE, immediately before the write, and its result is what gets
    /// attested — so the attestation always describes a real search this command ran,
    /// never one an operator retyped.
    PatientRegister {
        /// Name as typed. Searched exactly as `patient-search --name` searches it, and
        /// recorded VERBATIM on the new chart — never re-ordered or re-punctuated.
        #[arg(long, default_value = "")]
        name: String,
        /// ISO YYYY-MM-DD, or YYYY / YYYY-MM when only that much is known — the declared
        /// precision follows the shape you type and is never rounded up.
        #[arg(long)]
        birth_date: Option<String>,
        /// Repeatable `system=value`, e.g. --identifier MRN=12345. Searched AND recorded
        /// on the new chart, so the chart is findable by this identifier afterwards.
        #[arg(long = "identifier")]
        identifiers: Vec<String>,
        /// Proceed even though candidates were displayed. Without it the command STOPS and
        /// prints them: a funnel that auto-proceeds past near-matches is not a funnel.
        #[arg(long)]
        confirm_new: bool,
    },

    /// Enroll a clinician's signing key as a `kind='human'` actor so it may sign+attest an
    /// `identify-patient --link` (and any future human-attested surface). An OWNER ceremony —
    /// point `--conn` at a role that may run `enroll_actor`. The pinned determinant set carries
    /// a person-distinguishing field (`--registration-id` and/or `--handle`, ADR-0044) and NEVER
    /// the key (so `rotate-key` keeps the actor_id stable). `--role` is ALSO part of the actor's
    /// identity: the actor is the (entity, role) pair, so one clinician may hold several
    /// role-actors (e.g. clinician + registrar), each a distinct actor_id linked as one person by
    /// their shared `--registration-id`. Keep `--role` consistent for a given role — a differing
    /// or mistyped role mints a SEPARATE (still linkable) actor, never a silent merge (issue #168).
    /// If `--key` does not exist it is minted: sealed under a shown-once recovery code, or unsealed
    /// with `--insecure-plaintext` (test nodes only). No local-state `.lsk` escrow is attached — a
    /// personal key has none.
    EnrollHuman {
        /// A professional licence/registration number (preferred person-distinguishing determinant).
        #[arg(long)]
        registration_id: Option<String>,
        /// A node-local human-chosen handle (use when there is no registration number).
        #[arg(long)]
        handle: Option<String>,
        /// The actor's role tag — part of the (entity, role) actor identity, not just a label
        /// (one person holds one role-actor per role; keep it consistent, issue #168).
        #[arg(long, default_value = "clinician")]
        role: String,
        /// Passphrase to seal a newly-minted key (else CAIRN_KEY_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
        /// Mint the key UNSEALED if it does not exist (test nodes only).
        #[arg(long)]
        insecure_plaintext: bool,
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
        /// The year the age was observed (defaults to the node's current year). Lets a
        /// clinician record evidence about a PAST observation. Bounded 1900..=current year.
        #[arg(long)]
        observed_year: Option<i32>,
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

    /// Resolve a John-Doe chart (§5.4 finisher 3): record WHO the patient is
    /// (`identity.identify.asserted`, flipping the chart to *confirmed*) and OPTIONALLY
    /// link it to a prior chart so their history joins. The identify is device-additive
    /// (node key). The link MERGES charts — a human attribution — so it requires a
    /// separate human `--attester-key` that signs+attests it; identify + link are atomic.
    IdentifyPatient {
        /// The John-Doe patient UUID being identified.
        patient: Uuid,
        /// §5.7 "method recorded": how identity was established (non-empty).
        #[arg(long)]
        method: String,
        /// Optional prior chart UUID to link this now-identified chart to.
        #[arg(long)]
        link: Option<Uuid>,
        /// Human signing key that vouches for the link. Required when --link is given.
        #[arg(long)]
        attester_key: Option<PathBuf>,
        /// Passphrase to unseal --attester-key (else CAIRN_ATTESTER_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_ATTESTER_PASSPHRASE")]
        attester_passphrase: Option<String>,
    },

    /// Record a medication the patient takes/took (clinical.medication.asserted).
    /// Mints a medication thread id. Only --term is required; it may be vague
    /// ("little white pill"). Everything else is an honest unknown when omitted.
    MedicationAssert {
        /// The patient UUID this medication is recorded against.
        patient: Uuid,
        /// As-asserted substance term (required, may be vague).
        #[arg(long)]
        term: String,
        /// Drug-identity coding system — `drugref-moiety` today (ADR-0059).
        /// Supply all three --coding-* flags together, or none at all.
        #[arg(long)]
        coding_system: Option<String>,
        /// The immortal drug identifier (a drugref moiety_uuid).
        #[arg(long)]
        coding_code: Option<String>,
        /// The INN-preferred label as it reads at coding time.
        #[arg(long)]
        coding_display: Option<String>,
        /// Formulation (tablet, capsule, liquid, patch, …).
        #[arg(long)]
        formulation: Option<String>,
        /// Dose magnitude (decimal, e.g. "40").
        #[arg(long)]
        dose_amount: Option<String>,
        /// Dose unit (mg, mcg, g, mL, units, puffs, drops, %, or a free-text long-tail).
        #[arg(long)]
        dose_unit: Option<String>,
        /// Free-text directions ("one BD", "PRN").
        #[arg(long)]
        sig: Option<String>,
        /// Who the claim came from: patient-reported | clinician-observed | external-record | unknown.
        #[arg(long, default_value = "unknown")]
        info_source: String,
        /// When the patient began taking it (value, e.g. "2024" or a "2020/2024" range).
        #[arg(long)]
        started: Option<String>,
        /// Precision token for --started (year|month|day|year-range).
        #[arg(long)]
        started_precision: Option<String>,
        #[command(flatten)]
        attest: AttestFlags,
        #[command(flatten)]
        author: AuthorFlags,
    },
    /// Cease a medication thread (clinical.medication-cessation.asserted) — makes it
    /// past. Offline-first: does not require the assert to be present locally.
    MedicationCease {
        /// The patient UUID the thread belongs to.
        patient: Uuid,
        /// The medication thread id (printed by `medication-assert`).
        medication_id: Uuid,
        /// When it was stopped (value).
        #[arg(long)]
        stopped: Option<String>,
        /// Precision token for --stopped.
        #[arg(long)]
        stopped_precision: Option<String>,
        /// Optional free-text reason.
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        attest: AttestFlags,
        #[command(flatten)]
        author: AuthorFlags,
    },
    /// Record a dose change on an existing medication thread
    /// (clinical.medication-dose-change.asserted). Additive — the prior dose stays in
    /// the history. Offline-first: does not require the thread to be present locally.
    MedicationChangeDose {
        /// The patient UUID the thread belongs to.
        patient: Uuid,
        /// The medication thread id (printed by `medication-assert`).
        medication_id: Uuid,
        /// New dose magnitude (decimal). Omit if unknown ("upped it, dunno to what").
        #[arg(long)]
        dose_amount: Option<String>,
        /// New dose unit (mg, mcg, mL, …, or free-text).
        #[arg(long)]
        dose_unit: Option<String>,
        /// When the dose changed (value, e.g. "2025-06").
        #[arg(long)]
        effective: Option<String>,
        /// Precision token for --effective (year|month|day|year-range).
        #[arg(long)]
        effective_precision: Option<String>,
        /// Who the claim came from: patient-reported | clinician-observed | external-record | unknown.
        #[arg(long, default_value = "unknown")]
        info_source: String,
        /// Optional free-text reason ("titration", "renal dosing").
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        attest: AttestFlags,
        #[command(flatten)]
        author: AuthorFlags,
    },
    /// Correct a wrongly-recorded dose (clinical.medication-dose-correction.asserted).
    /// The prior value stays in the record (audit); this only wins the current dose.
    MedicationCorrectDose {
        /// The patient UUID the thread belongs to.
        patient: Uuid,
        /// The medication thread id.
        medication_id: Uuid,
        /// The dose event to correct. Defaults to the current dose point of the thread.
        #[arg(long)]
        target: Option<Uuid>,
        /// Set the corrected dose magnitude (with --dose-unit). Omit to leave the dose
        /// unchanged; use --strike dose to set it unknown.
        #[arg(long)]
        dose_amount: Option<String>,
        /// Set the corrected dose unit.
        #[arg(long)]
        dose_unit: Option<String>,
        /// Set the corrected effective date (e.g. 2024-01). Omit to leave it unchanged;
        /// use --strike effective to set it unknown.
        #[arg(long)]
        effective: Option<String>,
        /// Precision token for --effective (year|month|day|year-range).
        #[arg(long)]
        effective_precision: Option<String>,
        /// Set the corrected clinical reason for the dose (e.g. "titration"). Omit to
        /// leave it unchanged; use --strike reason to set it unknown.
        #[arg(long)]
        reason: Option<String>,
        /// Group(s) to set unknown: dose | effective | reason (repeatable).
        #[arg(long)]
        strike: Vec<String>,
        /// Why this correction was made (audit note, e.g. "mis-keyed the date").
        // Named `correction_note`/--correction-note rather than `note`/--note: the
        // flattened AttestFlags below already owns --note (the vouch note on
        // --attest-as), and clap requires unique argument ids within one command.
        #[arg(long)]
        correction_note: Option<String>,
        /// Optional provenance of the correction claim.
        #[arg(long)]
        info_source: Option<String>,
        #[command(flatten)]
        attest: AttestFlags,
        #[command(flatten)]
        author: AuthorFlags,
    },
    /// Code an existing medication thread (clinical.medication-coding.asserted).
    /// Coding is optional and separately authored (ADR-0059 decision 3) — a pharmacist or
    /// coder may code a medication a clinician recorded uncoded, without touching the
    /// clinical claim. Offline-first: does not require the thread to be present locally.
    //
    // Carries --author-as (ADR-0053: a coder's coding must be THEIRS) but deliberately no
    // --attest-as: attestation is clinical responsibility for a thread's CONTENT
    // (ADR-0049), and coding a drug identity is not a sign-off of the medication list.
    MedicationCode {
        /// The patient UUID this medication belongs to.
        patient: Uuid,
        /// The medication thread being coded (printed by `medication-assert`).
        medication_id: Uuid,
        /// Drug-identity coding system — `drugref-moiety` today (ADR-0059).
        /// All three --coding-* flags are required together.
        #[arg(long)]
        coding_system: Option<String>,
        /// The immortal drug identifier (a drugref moiety_uuid).
        #[arg(long)]
        coding_code: Option<String>,
        /// The INN-preferred label as it reads at coding time.
        #[arg(long)]
        coding_display: Option<String>,
        #[command(flatten)]
        author: AuthorFlags,
    },
    /// Correct a medication's coding — replace it, or --strike it back to honest
    /// not-yet-coded (clinical.medication-coding-correction.asserted). Additive: the
    /// corrected claim stays in the record; this only wins the current coding.
    MedicationCodeCorrect {
        /// The patient UUID this medication belongs to.
        patient: Uuid,
        /// The medication thread whose coding is being corrected.
        medication_id: Uuid,
        /// The event whose coding claim this fixes (a prior coding overlay, or the
        /// assertion itself when the coding was inline). Not required to be present
        /// locally — it may replicate later, or never.
        #[arg(long)]
        corrects: Uuid,
        /// The replacement coding system (all three --coding-* flags together).
        #[arg(long)]
        coding_system: Option<String>,
        /// The replacement immortal drug identifier.
        #[arg(long)]
        coding_code: Option<String>,
        /// The replacement INN-preferred label.
        #[arg(long)]
        coding_display: Option<String>,
        /// Strike the coding back to honest not-yet-coded — for when a reviewer
        /// establishes the coding is wrong but cannot say what the substance is.
        /// Mutually exclusive with the --coding-* flags.
        #[arg(long)]
        strike: bool,
        /// Why this correction was made (audit note, e.g. "misread the brand").
        #[arg(long)]
        note: Option<String>,
        #[command(flatten)]
        author: AuthorFlags,
    },
    /// Reconcile two medication threads as the same real drug
    /// (clinical.medication-reconciliation.asserted). Symmetric, reversible, additive —
    /// both threads' histories are preserved; the current list collapses to one row.
    /// Offline-first: neither thread need be present locally.
    MedicationReconcile {
        /// The patient UUID both threads belong to.
        patient: Uuid,
        /// The first medication thread id.
        thread_a: Uuid,
        /// The second medication thread id (must differ from thread_a).
        thread_b: Uuid,
        /// Provenance of the judgment (§4.1). Defaults to "clinician-judgment".
        #[arg(long, default_value = "clinician-judgment")]
        provenance: String,
        /// Optional free-text reason ("brand vs generic", "duplicate on transfer").
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        attest: AttestFlags,
        #[command(flatten)]
        author: AuthorFlags,
    },
    /// Separate two previously-reconciled threads — "actually two different drugs"
    /// (clinical.medication-separation.asserted). The never-erase reversal.
    MedicationSeparate {
        /// The patient UUID both threads belong to.
        patient: Uuid,
        /// The first medication thread id.
        thread_a: Uuid,
        /// The second medication thread id (must differ from thread_a).
        thread_b: Uuid,
        /// Provenance of the judgment (§4.1). Defaults to "clinician-judgment".
        #[arg(long, default_value = "clinician-judgment")]
        provenance: String,
        /// Optional free-text reason.
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        attest: AttestFlags,
        #[command(flatten)]
        author: AuthorFlags,
    },

    /// Take clinical responsibility for an existing medication thread (post-hoc med-rec
    /// sign-off): a human vouches for the thread's CURRENT content-event set without
    /// authoring a new clinical event. Records who vouched and pins the reviewed
    /// commitment, so a later content change (assert/cease/dose-change/dose-correction)
    /// flags the vouch as `stale` — a re-attest clears it. Complements the author-time
    /// `--attest-as` convenience on the six verb subcommands above (same orchestrator
    /// seam, `cairn_node::medication::attestation`).
    MedicationAttest {
        /// The medication_id thread to vouch for.
        medication_id: Uuid,
        /// Patient UUID (the chart the thread belongs to).
        #[arg(long)]
        patient: Uuid,
        #[command(flatten)]
        attest: AttestFlags,
    },

    /// Read a patient's medication list — current drugs and ceased ones, each with the
    /// clinician whose signature it carries. The read path the reference UI uses; a
    /// future native API (ADR-0023) is expected to wrap the same function.
    MedicationList {
        /// The patient UUID whose chart to read.
        patient: Uuid,
        /// Emit JSON instead of the human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Sign off the whole medication list in ONE gesture (#288): attests every thread
    /// whose vouch is absent or stale, in one transaction. Threads already carrying a
    /// current signature keep it — a drug line carries the signature of the person
    /// responsible for that drug.
    MedicationSignOff {
        /// The patient UUID whose chart is being signed off.
        patient: Uuid,
        #[command(flatten)]
        attest: AttestFlags,
    },

    /// Rung-3 audited crypto-shred (ADR-0005 / ADR-0052): irreversibly destroy an
    /// event's custody + derived plaintext, leaving behind a signed, LEGIBLE tombstone
    /// ("existed -> destroyed, basis Z") that outlives every key. Device-additive by
    /// default; `--attest-as` makes a human personally responsible for the erasure
    /// DECISION itself — the tombstone is then authored and signed by that human, not
    /// the node (mirrors `identify-patient --link`'s human-attributed shape).
    Shred {
        /// The event to destroy.
        event: Uuid,
        /// Why (the audited "why" — non-empty; the db/037 floor requires it).
        #[arg(long)]
        basis: String,
        /// Take personal responsibility for this erasure decision: a human key that
        /// authors, signs, and attests the tombstone itself. Absent -> device-additive
        /// (the node signs; no vouch demanded).
        #[arg(long)]
        attest_as: Option<PathBuf>,
        /// Passphrase to unseal --attest-as (else CAIRN_ATTESTER_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_ATTESTER_PASSPHRASE")]
        attest_passphrase: Option<String>,
    },

    /// Replay event_log through the registered projection apply fns (#208 /
    /// ADR-0057): heal a projection after a logic fix, or rebuild after a
    /// wrote-garbage defect. Needs an owner-privileged --conn (like `init`) —
    /// the runtime role deliberately cannot execute it.
    Reproject {
        /// Event-type prefix to replay ('' = everything).
        #[arg(long, default_value = "")]
        prefix: String,
        /// TRUNCATE the in-scope projection tables first (refuses if a table
        /// is also fed by out-of-prefix types). Default is heal (no deletes).
        #[arg(long)]
        rebuild: bool,
    },

    /// List events this node admitted UNINTERPRETED (ADR-0056 decision 1 / #265):
    /// stored verbatim and re-propagated, but holding NO power because this node
    /// has no code classifying their type. A row carrying a reason has since been
    /// re-adjudicated and REFUSED — it stays powerless until that reason resolves
    /// (a missing overlay target arriving, for instance). A healthy node whose code
    /// covers everything it has received lists nothing.
    Deferred,
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
            println!("clock         {}", st.clock_health);
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
            // Fail cheap, before any key I/O or DB round trip — the same discipline
            // `patient-register` applies to `--birth-date`, and the SAME function
            // `register_john_doe` calls internally, so the CLI edge and the library call
            // cannot drift into different opinions of "stated". Without it, `--basis ""`
            // unsealed a key, opened a connection, minted a patient UUID and burned three
            // HLC ticks before the db/045 floor refused the first submit.
            cairn_node::john_doe::validate_basis(&basis)?;
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
        Cmd::PatientSearch {
            name,
            birth_date,
            identifiers,
        } => {
            // Malformed input is refused before any I/O — same discipline as EnrollHuman's
            // pre-mint check: fail cheaply before spending a round trip on a query we'd have
            // to explain was built from a silently-mangled identifier.
            let identifiers = parse_identifier_pairs(&identifiers)?;
            let query =
                cairn_patient_search::SearchQuery::new(&name, birth_date.as_deref(), &identifiers);
            let db = cairn_node::db::connect(&cli.conn).await?;
            // The John-Doe precedent: today's date comes from the node's own DB clock, never
            // the operator's wall clock, so the offline-first age math the shared crate does
            // is anchored the same way everywhere it runs.
            let today: String = db.query_one("SELECT current_date::text", &[]).await?.get(0);
            let list = cairn_node::patient::search::search_patients(&db, &query, &today).await?;
            print_candidates(&list);
        }
        Cmd::PatientRegister {
            name,
            birth_date,
            identifiers,
            confirm_new,
        } => {
            let identifiers = parse_identifier_pairs(&identifiers)?;
            // Fail cheap, before any I/O (same discipline as `parse_identifier_pairs` above,
            // review round 1 #350 Important 1): a malformed `--birth-date` must never reach
            // the search/write path only to be discovered deep inside `register_patient`'s
            // own transaction. Reuses the EXACT function `register_patient` calls internally
            // — one function, so the CLI edge and the library call can never silently drift
            // into two different opinions of "a valid shape".
            if let Some(bd) = birth_date.as_deref() {
                cairn_node::patient::register::dob_precision(bd)?;
            }
            let query =
                cairn_patient_search::SearchQuery::new(&name, birth_date.as_deref(), &identifiers);

            let sk = load_signing_key(&cli.key, true)?; // interactive: may prompt to unseal
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            let today: String = db.query_one("SELECT current_date::text", &[]).await?.get(0);
            // Owner ceremony: make the signing key an enrolled actor so it may author the
            // additive registration event (idempotent — enrolls only on first use), mirroring
            // RegisterJohnDoe.
            ensure_registration_actor(&db, &kid).await?;

            // THE SEARCH RUNS HERE, immediately before the write, over THIS process's own DB
            // connection — never a result an operator retyped from an earlier `patient-search`
            // run. `register_patient` (below) attests exactly this `list`, so what gets signed
            // into the permanent record is provably the search a human actually saw on THIS
            // screen a moment ago, not a claim about a search that may never have happened.
            let list = cairn_node::patient::search::search_patients(&db, &query, &today).await?;
            print_candidates(&list);

            if !list.candidates.is_empty() && !confirm_new {
                // STOP HERE. This is NOT a confirmation dialog — principle 3 explicitly
                // forbids those as a safety mechanism, because a dialog habituates a busy
                // clerk to click through without reading. This is instead the PAPER
                // affordance: a clerk working a physical card index sees the existing cards
                // before they can pull a fresh blank one — the index itself is the barrier,
                // not a prompt asking "are you sure?". Exiting non-zero with the candidates
                // already printed above reproduces that same physical fact for a CLI: nothing
                // is registered, and the only way past is the explicit `--confirm-new` a
                // clerk types after having looked. If a future change turns this into an
                // auto-proceed-past-candidates default, it quietly deletes the funnel this
                // whole task exists to build — please don't "simplify" it away.
                anyhow::bail!(
                    "{} candidate(s) already on file for this search (printed above) — \
                     re-run with --confirm-new to register a new chart anyway, or use one of \
                     the patient_ids shown instead",
                    list.candidates.len()
                );
            }

            // The raw typed name — NOT `query.name_tokens` (`SearchQuery` retains only
            // normalised search tokens, never the raw string; see
            // `patient::register`'s module doc, "signature problem", for why). Passed through
            // unconditionally: `register_patient` itself treats a blank/empty string the same
            // as `None` (principle 4 — an empty `--name` default must never assert an
            // empty-string name), so no pre-filtering is needed here.
            let patient_id = cairn_node::patient::register::register_patient(
                &mut db,
                &sk,
                &kid,
                &id.node_id_hex,
                Some(name.as_str()),
                &query,
                &list,
            )
            .await?;
            println!("registered patient {patient_id}");
        }
        Cmd::EnrollHuman {
            registration_id,
            handle,
            role,
            passphrase,
            insecure_plaintext,
        } => {
            // Validate the determinant BEFORE any key or DB I/O (pre-I/O validation, mirroring
            // identify-patient): refuse an enrollment that would compute a non-distinguishing
            // actor_id, before minting a key or opening a connection.
            let pinned = cairn_node::enroll::build_human_pinned(
                &role,
                registration_id.as_deref(),
                handle.as_deref(),
            )?;

            // Open the DB connection BEFORE any key-minting I/O, so the fresh-key branch below
            // can run a pre-mint collision check (finding 3): without this, a fresh key (and,
            // on the sealed path, its shown-once recovery code) would already be minted before
            // a collision on the determinant could be detected, leaving a stray key + a
            // recovery code for a ceremony that then fails.
            let db = cairn_node::db::connect(&cli.conn).await?;

            // Load the human's personal key, or mint one if the file is absent.
            use cairn_node::keystore::{key_at_rest_state, KeyAtRest};
            let (sk, kid) = match key_at_rest_state(&cli.key) {
                KeyAtRest::Missing => {
                    // Pre-mint collision check. Safe/correct ONLY on this branch: the key is
                    // about to be freshly minted, so it cannot be the same key that is already
                    // bound to this determinant's actor_id — a fresh key can never be the
                    // idempotent case. So if the determinant is already claimed, its actor_id
                    // necessarily belongs to some OTHER (already-enrolled) key, and minting +
                    // sealing a new key here would only be discarded a moment later by
                    // `enroll_human_actor`'s guard 2. Refusing first means that, in the common
                    // (uncontended) case, no key material or recovery code is generated for a
                    // ceremony that cannot succeed. This is a best-effort narrowing, NOT a hard
                    // guarantee: under the accepted TOCTOU (#166) a concurrent enroll can still
                    // slip in between this check and the floor call and fail the ceremony after a
                    // key was minted — recoverable by re-running (the load branch then completes
                    // it idempotently). `enroll_human_actor` remains the real guard for the load
                    // branch below, and re-checks this collision itself as the floor-backed
                    // re-check (legibility, not enforcement — same pattern as guard 2 there).
                    if cairn_node::enroll::determinant_already_claimed(&db, &pinned).await? {
                        anyhow::bail!(
                            "enroll-human: this determinant set is already claimed by an \
                             existing actor — a brand-new key cannot be the idempotent case, so \
                             refusing before minting one. If this is genuinely a new person, add \
                             a distinguishing --registration-id or --handle."
                        );
                    }
                    if insecure_plaintext {
                        eprintln!(
                            "WARNING: --insecure-plaintext: human signing key written UNSEALED \
                             (test use only)"
                        );
                        cairn_node::keystore::generate_plaintext(&cli.key)?
                    } else {
                        let op = resolve_passphrase(passphrase)?;
                        // The recovery code is a key-recovering secret — Zeroizing so it is
                        // wiped on drop (issue #46). Printed BEFORE persist so a crash can never
                        // seal under a code no human saw. No local-state escrow: a personal key
                        // has no node-scoped local state to wrap (design D2).
                        let code = Zeroizing::new(cairn_node::seal::generate_recovery_code());
                        print_recovery_code(&code);
                        cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?
                    }
                }
                _ => {
                    // An existing key file. We must fully load (and, if sealed, UNSEAL) it even
                    // though enrollment binds only the public `kid`: the kid is the ed25519 public
                    // key derived FROM the secret seed, and the sealed-at-rest format stores no
                    // separate cleartext public key to read without unsealing. The unseal doubles
                    // as a possession proof — you cannot enrol a key you cannot open — so it is
                    // not wasted work.
                    let sk = load_signing_key(&cli.key, true)?; // may prompt to unseal
                    let kid = hex::encode(sk.verifying_key().to_bytes());
                    (sk, kid)
                }
            };
            // `sk` is not used again (enrollment binds only the public kid) but was needed to
            // derive it above; drop it explicitly so the secret's lifetime ends here.
            drop(sk);

            match cairn_node::enroll::enroll_human_actor(&db, &kid, &pinned).await? {
                cairn_node::enroll::EnrollHumanOutcome::Enrolled => {
                    println!("enrolled human actor {kid}");
                }
                cairn_node::enroll::EnrollHumanOutcome::AlreadyEnrolled => {
                    println!("human actor {kid} already enrolled (no change)");
                }
            }
        }
        Cmd::AssertObservedEvidence {
            patient,
            age,
            tol,
            age_basis,
            sex,
            sex_basis,
            observed_year,
        } => {
            let sk = load_signing_key(&cli.key, true)?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Default the observation year to the node's own DB clock (the DB is the
            // clock), but let --observed-year override it for a past observation. The
            // pure validator rejects a future or absurdly-historical year (principle 4).
            let current_year: i32 = db
                .query_one("SELECT extract(year FROM current_date)::int", &[])
                .await?
                .get(0);
            let observed_year =
                cairn_node::evidence::resolve_observed_year(observed_year, current_year)?;
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
        Cmd::IdentifyPatient {
            patient,
            method,
            link,
            attester_key,
            attester_passphrase,
        } => {
            // §5.7 requires a recorded identification method; the db/024 floor rejects an
            // empty `method` too, but validate here (before any I/O) so a blank `--method`
            // gets the same clean, pre-authoring message as the cross-flag checks below —
            // not a floor error after the node key has been unsealed and the DB opened.
            cairn_node::identify::validate_identify_method(&method)?;

            // Cross-flag validation (clap cannot express "attester-key iff link"). Reject
            // both mismatches loudly — an attester with nothing to attest is a mistake worth
            // surfacing, not silently ignoring. After this block the only surviving states
            // are (link, attester_key) = (Some, Some) or (None, None) — the matches below
            // rely on that invariant, so their `_ => None` arm is only ever the (None, None) case.
            match (&link, &attester_key) {
                (Some(_), None) => anyhow::bail!(
                    "--link requires --attester-key: linking to a prior chart is a human \
                     attribution that must be attested"
                ),
                (None, Some(_)) => {
                    anyhow::bail!("--attester-key was given without --link: nothing to attest")
                }
                _ => {}
            }

            let node_sk = load_signing_key(&cli.key, true)?; // may prompt to unseal
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Owner ceremony: the node key must be an enrolled actor to author the additive
            // identify (idempotent — enrolls a `device` actor only on first use).
            ensure_registration_actor(&db, &node_kid).await?;

            // Load the human attester key + pre-check human-ness (legibility; the db/005 gate
            // is the real enforcement). Held so the borrows live across identify_patient.
            let attester = match (&link, &attester_key) {
                (Some(_), Some(path)) => {
                    let sk = load_attester_key(path, attester_passphrase)?;
                    let kid = hex::encode(sk.verifying_key().to_bytes());
                    if !cairn_node::identify::attester_is_enrolled_human(&db, &kid).await? {
                        anyhow::bail!(
                            "--attester-key ({kid}) is not an enrolled human actor; a link \
                             must be attested by a human (enroll the clinician first)"
                        );
                    }
                    Some((sk, kid))
                }
                _ => None,
            };
            let link_params = match (&link, &attester) {
                (Some(prior), Some((sk, kid))) => Some(cairn_node::identify::LinkParams {
                    prior: *prior,
                    human_sk: sk,
                    human_kid: kid,
                }),
                _ => None,
            };

            let out = cairn_node::identify::identify_patient(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                &method,
                link_params,
            )
            .await?;
            println!(
                "identified {patient} (chart now confirmed); event {}",
                out.identify_event_id
            );
            if let (Some(prior), Some(link_eid)) = (link, out.link_event_id) {
                println!("linked to {prior}; link event {link_eid}");
            }
        }
        Cmd::MedicationAssert {
            patient,
            term,
            coding_system,
            coding_code,
            coding_display,
            formulation,
            dose_amount,
            dose_unit,
            sig,
            info_source,
            started,
            started_precision,
            attest,
            author,
        } => {
            cairn_node::medication::validate_term(&term)?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::AssertMedicationInput {
                term: &term,
                coding: cairn_node::medication::coding_from_parts(
                    coding_system.as_deref(),
                    coding_code.as_deref(),
                    coding_display.as_deref(),
                )?,
                formulation: formulation.as_deref(),
                dose_amount: dose_amount.as_deref(),
                dose_unit: dose_unit.as_deref(),
                sig: sig.as_deref(),
                info_source: &info_source,
                started: started.as_deref(),
                started_precision: started_precision.as_deref(),
            };
            let resolved = resolve_attester(&db, &attest).await?;
            let params = attest_params(&resolved, &attest);
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let med_id = cairn_node::medication::assert_medication(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                &input,
                a_params.as_ref(),
                params.as_ref(),
            )
            .await?;
            println!("recorded medication for {patient}; thread {med_id}");
        }
        Cmd::MedicationCease {
            patient,
            medication_id,
            stopped,
            stopped_precision,
            reason,
            attest,
            author,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::CeaseMedicationInput {
                stopped: stopped.as_deref(),
                stopped_precision: stopped_precision.as_deref(),
                reason: reason.as_deref(),
            };
            let resolved = resolve_attester(&db, &attest).await?;
            let params = attest_params(&resolved, &attest);
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::cease_medication(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                &input,
                a_params.as_ref(),
                params.as_ref(),
            )
            .await?;
            println!("ceased medication thread {medication_id}; event {event_id}");
        }
        Cmd::MedicationChangeDose {
            patient,
            medication_id,
            dose_amount,
            dose_unit,
            effective,
            effective_precision,
            info_source,
            reason,
            attest,
            author,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::ChangeDoseInput {
                dose_amount: dose_amount.as_deref(),
                dose_unit: dose_unit.as_deref(),
                effective: effective.as_deref(),
                effective_precision: effective_precision.as_deref(),
                info_source: &info_source,
                reason: reason.as_deref(),
            };
            let resolved = resolve_attester(&db, &attest).await?;
            let params = attest_params(&resolved, &attest);
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::change_dose(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                &input,
                a_params.as_ref(),
                params.as_ref(),
            )
            .await?;
            println!("dose change recorded for thread {medication_id}; event {event_id}");
        }
        Cmd::MedicationCorrectDose {
            patient,
            medication_id,
            target,
            dose_amount,
            dose_unit,
            effective,
            effective_precision,
            reason,
            strike,
            correction_note,
            info_source,
            attest,
            author,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let corrects =
                cairn_node::medication::resolve_correction_target(&db, medication_id, target)
                    .await?;
            let strike_refs: Vec<&str> = strike.iter().map(String::as_str).collect();
            let input = cairn_node::medication::CorrectDoseInput {
                dose_amount: dose_amount.as_deref(),
                dose_unit: dose_unit.as_deref(),
                effective: effective.as_deref(),
                effective_precision: effective_precision.as_deref(),
                reason: reason.as_deref(),
                strike: &strike_refs,
                note: correction_note.as_deref(),
                info_source: info_source.as_deref(),
            };
            let resolved = resolve_attester(&db, &attest).await?;
            let params = attest_params(&resolved, &attest);
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::correct_dose(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                corrects,
                &input,
                a_params.as_ref(),
                params.as_ref(),
            )
            .await?;
            println!("dose correction recorded for thread {medication_id} (target {corrects}); event {event_id}");
        }
        Cmd::MedicationCode {
            patient,
            medication_id,
            coding_system,
            coding_code,
            coding_display,
            author,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            // coding_from_parts is all-or-nothing; None means no --coding-* flag was
            // given at all, which for THIS verb is the caller having asked to code
            // nothing — refuse it here rather than at the DB floor.
            let coding = cairn_node::medication::coding_from_parts(
                coding_system.as_deref(),
                coding_code.as_deref(),
                coding_display.as_deref(),
            )?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "coding a medication needs all three --coding-system/--coding-code/--coding-display flags"
                )
            })?;
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::code_medication(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                &cairn_node::medication::CodeMedicationInput { coding },
                a_params.as_ref(),
                None,
            )
            .await?;
            println!("coded thread {medication_id}; event {event_id}");
        }
        Cmd::MedicationCodeCorrect {
            patient,
            medication_id,
            corrects,
            coding_system,
            coding_code,
            coding_display,
            strike,
            note,
            author,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            // None here is exactly what a --strike wants. A command line can still spell
            // both or neither, so this is where those are refused — coding_claim_from_parts
            // collapses the two independent switches into the ONE claim a correction is
            // allowed to make, and nothing downstream can represent anything else.
            let coding = cairn_node::medication::coding_from_parts(
                coding_system.as_deref(),
                coding_code.as_deref(),
                coding_display.as_deref(),
            )?;
            let claim = cairn_node::medication::coding_claim_from_parts(coding, strike)?;
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::correct_medication_coding(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                &cairn_node::medication::CorrectCodingInput {
                    corrects,
                    claim,
                    note: note.as_deref(),
                },
                a_params.as_ref(),
                None,
            )
            .await?;
            println!("corrected coding on thread {medication_id}; event {event_id}");
        }
        Cmd::MedicationReconcile {
            patient,
            thread_a,
            thread_b,
            provenance,
            reason,
            attest,
            author,
        } => {
            cairn_node::medication::validate_distinct_subjects(thread_a, thread_b)?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::ReconcileInput {
                provenance: &provenance,
                reason: reason.as_deref(),
            };
            let resolved = resolve_attester(&db, &attest).await?;
            let params = attest_params(&resolved, &attest);
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::reconcile_medications(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                thread_a,
                thread_b,
                &input,
                a_params.as_ref(),
                params.as_ref(),
            )
            .await?;
            println!("reconciled threads {thread_a} + {thread_b}; event {event_id}");
        }
        Cmd::MedicationSeparate {
            patient,
            thread_a,
            thread_b,
            provenance,
            reason,
            attest,
            author,
        } => {
            cairn_node::medication::validate_distinct_subjects(thread_a, thread_b)?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::ReconcileInput {
                provenance: &provenance,
                reason: reason.as_deref(),
            };
            let resolved = resolve_attester(&db, &attest).await?;
            let params = attest_params(&resolved, &attest);
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::separate_medications(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                thread_a,
                thread_b,
                &input,
                a_params.as_ref(),
                params.as_ref(),
            )
            .await?;
            println!("separated threads {thread_a} + {thread_b}; event {event_id}");
        }
        Cmd::MedicationAttest {
            medication_id,
            patient,
            attest,
        } => {
            // Post-hoc sign-off authors no clinical content EVENT of its own, but the
            // attestation event IS clinical.* and born-sealed (ADR-0052), so its DEK must be
            // wrapped into node custody — which needs the NODE signing key to register the
            // node's unwrap key defensively (idempotent; a no-op when the content path
            // already registered it). No registration-ACTOR ceremony is needed (this key
            // authors no additive content event); just the node key for custody.
            let node_sk = load_signing_key(&cli.key, true)?;
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            let (human_sk, human_kid) = resolve_attester(&db, &attest).await?.ok_or_else(|| {
                anyhow::anyhow!(
                    "medication-attest requires --attest-as: a vouch needs a responsible human"
                )
            })?;
            let params = cairn_node::medication::AttestParams {
                human_sk: &human_sk,
                human_kid: &human_kid,
                basis: attest.basis.as_deref(),
                note: attest.note.as_deref(),
            };
            let event_id = cairn_node::medication::attest_medication_thread(
                &mut db,
                &node_sk,
                &id.node_id_hex,
                &params,
                patient,
                medication_id,
            )
            .await?;
            println!("attested medication thread {medication_id} (event {event_id})");
        }
        Cmd::MedicationList { patient, json } => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let list = cairn_node::medication::read::list_patient_medications(&db, patient).await?;
            if json {
                // The payload is the WHOLE `PatientMedicationList`, not a bare row array. A
                // machine reader must be able to see the incompleteness signal in-band: a
                // script that pipes stdout to `jq` and never reads stderr would otherwise
                // consume a chart the node knows is missing a drug, with a zero exit code
                // and no marker at all. Serializing the struct itself (rather than
                // hand-picking fields into a `json!`) means a field added to the read model
                // reaches machine readers automatically instead of being silently dropped
                // here — the same "surface the uncertainty" rule the Rust return type
                // follows (principle 4).
                println!("{}", serde_json::to_string_pretty(&list)?);
                if !list.groups_missing_from_chart.is_empty() {
                    eprintln!(
                        "warning: {} medication group(s) with locally-known content for this \
                         patient are missing from this chart (issue #334); see \
                         .separation_targets for the threads to pass to `medication-separate`",
                        list.groups_missing_from_chart.len()
                    );
                }
            } else if list.rows.is_empty() {
                // Deliberately explicit: an empty chart is a real clinical state, and
                // silence would read as "the query failed" (issue #331 covers recording
                // "nil medications, reviewed" as an act — not attempted here).
                println!("no medications recorded for {patient}");
                if !list.groups_missing_from_chart.is_empty() {
                    // The #334 case this whole fix exists for: "no medications recorded"
                    // would otherwise be a straightforward LIE when the node actually holds
                    // content for this patient it just cannot display (a cross-patient
                    // reconciliation). Never let the plain empty-chart message stand alone
                    // when this is true.
                    println!(
                        "! but {} medication group(s) with locally-known content for this \
                         patient are missing from this chart entirely (issue #334) — this is \
                         NOT the same as \"no medications\". {}\n    {}",
                        list.groups_missing_from_chart.len(),
                        cairn_node::medication::read::SEPARATION_INSTRUCTION,
                        cairn_node::medication::read::format_hazard_groups(
                            &list.groups_missing_from_chart,
                            &list.separation_targets
                        )
                    );
                }
            } else {
                for row in &list.rows {
                    let name = row.display_name();
                    let dose = match (&row.dose_amount, &row.dose_unit) {
                        (Some(a), Some(u)) => format!(" {a} {u}"),
                        (Some(a), None) => format!(" {a}"),
                        _ => String::new(),
                    };
                    let status = match row.status {
                        cairn_medication_view::MedicationStatus::Active => "current",
                        cairn_medication_view::MedicationStatus::Ceased => "ceased",
                    };
                    // One line per MEMBER THREAD's signature state, not a row-level
                    // summary: a reconciled group can hold several threads at different
                    // signature states, and a summary is exactly what would hide that.
                    let vouches: Vec<String> = row
                        .members
                        .iter()
                        .map(|m| match &m.vouch {
                            cairn_medication_view::VouchState::Absent => "unsigned".to_string(),
                            cairn_medication_view::VouchState::Fresh { by } => {
                                format!("signed by {}", short_kid(by))
                            }
                            cairn_medication_view::VouchState::Stale { by } => {
                                format!("signed by {} (out of date)", short_kid(by))
                            }
                        })
                        .collect();
                    println!("{name}{dose} [{status}] — {}", vouches.join("; "));
                    if row.reconciliation_flagged {
                        println!("    ! possible un-reconciled duplicate");
                    }
                    if row.coding_conflict {
                        println!("    ! two different drug anchors in this group");
                    }
                    if row.cross_patient {
                        // This row displayed at all only because this patient happened to
                        // win the DISTINCT ON tiebreak in medication_group_display — the
                        // group's OTHER patient sees no row for it (issue #334). The dose
                        // shown may be that other patient's, so the line cannot be signed.
                        // The member-thread list is printed with it because the remedy named
                        // here takes thread ids, and `row.members` holds only THIS patient's
                        // half of the group.
                        println!(
                            "    ! this group's member threads span more than one patient — \
                             the dose shown may belong to the other patient, so this line \
                             CANNOT be signed off (issue #334). {}",
                            cairn_node::medication::read::SEPARATION_INSTRUCTION
                        );
                        println!(
                            "      {}",
                            cairn_node::medication::read::format_hazard_groups(
                                &[row.group_id],
                                &list.separation_targets
                            )
                        );
                    }
                }
                if !list.groups_missing_from_chart.is_empty() {
                    println!(
                        "! {} medication group(s) with locally-known content for this patient \
                         are missing from this chart entirely (issue #334) — this list is \
                         INCOMPLETE. {}\n    {}",
                        list.groups_missing_from_chart.len(),
                        cairn_node::medication::read::SEPARATION_INSTRUCTION,
                        cairn_node::medication::read::format_hazard_groups(
                            &list.groups_missing_from_chart,
                            &list.separation_targets
                        )
                    );
                }
            }
        }
        Cmd::MedicationSignOff { patient, attest } => {
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            // A sign-off IS the human vouch (it takes clinical responsibility for the
            // whole list) — there is no device-additive form of it, unlike every other
            // medication verb where --attest-as is an optional overlay. Refuse loudly
            // rather than silently proceeding node-signed-only.
            //
            // Resolved BEFORE the node key is unsealed (#338 review finding 6): unsealing
            // prompts for a passphrase, and an operator who simply forgot `--attest-as`
            // should not be made to type one only to be refused straight afterwards. A
            // wasted step is a paper-parity step (§1.2).
            let (human_sk, human_kid) = resolve_attester(&db, &attest).await?.ok_or_else(|| {
                anyhow::anyhow!(
                    "medication-sign-off requires --attest-as: a sign-off IS the human vouch"
                )
            })?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let id = cairn_node::identity::load_local(&db).await?;
            let params = cairn_node::medication::AttestParams {
                human_sk: &human_sk,
                human_kid: &human_kid,
                basis: attest.basis.as_deref(),
                note: attest.note.as_deref(),
            };

            let out = cairn_node::medication::signoff::sign_off_medication_list(
                &mut db,
                &node_sk,
                &id.node_id_hex,
                &params,
                patient,
            )
            .await?;

            if out.attested.is_empty() {
                // FIX 4 (#288 review) + #338 review finding 2: FOUR clinical states all
                // produce an empty `attested`, and each earns a different sentence. Saying
                // "everything is signed" for any of the other three is a precise untruth —
                // exactly what principle 4 forbids — about the one question the clinician
                // is asking.
                if out.total_rows == 0 && out.groups_missing_from_chart.is_empty() {
                    println!(
                        "no medications are recorded for {patient}; nothing was signed. \
                         Recording \"nil medications, reviewed\" as an act has no home yet \
                         (issue #331)."
                    );
                } else if out.total_rows == 0 {
                    // Guarded on the missing set: "no medications are recorded" is a
                    // straightforward LIE when the node holds content for this patient it
                    // simply cannot display. The detail follows in the block below.
                    println!(
                        "nothing could be signed for {patient}: this chart displays no \
                         medications, but that is NOT the same as having none — see below."
                    );
                } else if out.active_rows == 0 {
                    // A chart of nothing but struck lines. Those lines may well carry NO
                    // signature at all — a ceased drug is never re-signed — so the
                    // "everything already signed" sentence below would be flatly false here.
                    println!(
                        "no CURRENT medications for {patient} ({} ceased line(s) on the \
                         chart); nothing needed a signature. A struck line is never \
                         re-signed, signed or not.",
                        out.total_rows
                    );
                } else if out.withheld.is_empty() {
                    println!(
                        "nothing to sign off for {patient}: every current drug already \
                         carries a current signature"
                    );
                } else {
                    // Guard on `withheld`: without it this branch would claim every drug
                    // is signed while the ONLY outstanding lines were ones the node
                    // refused to sign — a precise untruth about the one thing the
                    // clinician is asking about.
                    println!("nothing was signed off for {patient}.");
                }
            } else {
                println!(
                    "signed off {} medication thread(s) for {patient}",
                    out.attested.len()
                );
                for (thread, event) in out.attested.iter().zip(&out.event_ids) {
                    println!("  {thread} -> attestation {event}");
                }
            }
            if !out.withheld.is_empty() {
                // Printed in EVERY outcome, never folded into the success line. "Signed off
                // 11 medication thread(s)" on a chart with a twelfth outstanding line reads
                // as a finished chart; the whole point of withholding rather than refusing
                // is that the clinician is told precisely which line they still own.
                println!(
                    "! {} medication line(s) still need a signature but were NOT signed: their \
                     group's member threads span more than one patient, so the dose displayed \
                     may belong to the other patient (issue #334). {} Then sign off again.",
                    out.withheld.len(),
                    cairn_node::medication::read::SEPARATION_INSTRUCTION
                );
                // The member threads, not just the group id: they ARE the remedy's
                // arguments, and this patient's own row lists only their half of the group
                // (#338 review finding 1).
                println!(
                    "    {}",
                    cairn_node::medication::read::format_hazard_groups(
                        &out.withheld,
                        &out.separation_targets
                    )
                );
            }
            if !out.groups_missing_from_chart.is_empty() {
                // Also printed in EVERY outcome, success included (#339). Sign-off no
                // longer refuses over an incomplete chart — a defect on one line must not
                // block another — so this report is now the ONLY thing standing between the
                // clinician and a chart that looks finished while the node knows it is not.
                println!(
                    "! {} medication group(s) with locally-known content for this patient \
                     could NOT be displayed on this chart and were therefore NOT signed \
                     (issue #334) — whatever was signed above, this list is INCOMPLETE. {}",
                    out.groups_missing_from_chart.len(),
                    cairn_node::medication::read::SEPARATION_INSTRUCTION
                );
                println!(
                    "    {}",
                    cairn_node::medication::read::format_hazard_groups(
                        &out.groups_missing_from_chart,
                        &out.separation_targets
                    )
                );
            }
            if !out.failed.is_empty() {
                // Distinct from `withheld` and `groups_missing_from_chart`, which are
                // reported, actionable, normal-operation states. A failed line is an
                // ATTEMPTED WRITE THAT ERRORED, so it is the one outstanding-work category
                // that also earns a non-zero exit: a script must not read "sign-off
                // succeeded" from a run where a signature the clinician asked for did not
                // land. Each line rolled back alone (ADR-0060), so the rest are committed.
                eprintln!(
                    "! {} medication line(s) could NOT be signed. Each rolled back on its own \
                     — every other line above is committed and unaffected.",
                    out.failed.len()
                );
                for line in &out.failed {
                    eprintln!("    {} — {}", line.medication_id, line.error);
                }
                std::process::exit(1);
            }
        }
        Cmd::Shred {
            event,
            basis,
            attest_as,
            attest_passphrase,
        } => {
            // Pre-authoring validation before any key is unsealed or connection opened —
            // same discipline as validate_term/validate_identify_method.
            cairn_node::shred::validate_basis(&basis)?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Owner ceremony: the node key must be an enrolled actor to author the
            // device-additive tombstone (idempotent; a no-op once already enrolled).
            // Harmless even on the attested path, where the human — not the node — ends
            // up as the tombstone's signer.
            ensure_registration_actor(&db, &node_kid).await?;
            // Build a throwaway AttestFlags value purely to reuse the existing
            // resolve_attester/attest_params machinery verbatim (same functions every
            // medication verb uses): `basis`/`note` are hardcoded absent because a
            // shred's own required --basis already IS the vouch's "why" — a SEPARATE
            // AttestFlags --basis on this command would collide with it (clap forbids
            // two args with the same id) and would double up on meaning besides.
            let flags = AttestFlags {
                attest_as,
                attest_passphrase,
                basis: None,
                note: None,
            };
            let resolved = resolve_attester(&db, &flags).await?;
            let params = attest_params(&resolved, &flags);
            let shred_event_id = cairn_node::shred::shred_event(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                event,
                &basis,
                params.as_ref(),
            )
            .await?;
            println!("shredded {event}; tombstone event {shred_event_id}");
        }
        Cmd::Reproject { prefix, rebuild } => {
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let rows = db
                .query(
                    "SELECT event_type, events_replayed FROM cairn_reproject($1, $2, 'cli')",
                    &[&prefix, &rebuild],
                )
                .await?;
            let mut total: i64 = 0;
            for r in &rows {
                let ty: String = r.get(0);
                let n: i64 = r.get(1);
                total += n;
                println!("{ty:<55} {n:>10}");
            }
            let log = db
                .query_one(
                    "SELECT elapsed_ms, skipped_fns FROM reproject_log ORDER BY id DESC LIMIT 1",
                    &[],
                )
                .await?;
            let ms: i64 = log.get(0);
            let skipped: Vec<String> = log.get(1);
            println!("replayed {total} events in {ms} ms");
            if !skipped.is_empty() {
                println!("skipped (heal_safe = false — rebuild to heal these): {skipped:?}");
            }
        }
        Cmd::Deferred => {
            // connect_and_load_schema itself re-adjudicates before returning, so this
            // listing already reflects the freshest verdict the node can reach — an
            // operator running it right after a code-plane update sees the post-upgrade
            // state, not the stale one.
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            // admitted_at is cast in SQL rather than formatted in Rust: TIMESTAMPTZ::text
            // renders ISO-8601 with the session offset and costs no new dependency.
            let rows = db
                .query(
                    "SELECT event_id::text, event_type, admitted_at::text, \
                            COALESCE(adjudication_error, '(not yet re-adjudicated)') \
                       FROM event_deferred ORDER BY admitted_at",
                    &[],
                )
                .await?;
            if rows.is_empty() {
                println!("no deferred events — every event this node holds has a classified type");
            }
            for r in &rows {
                let id: String = r.get(0);
                let ty: String = r.get(1);
                let at: String = r.get(2);
                let reason: String = r.get(3);
                println!("{id}  {ty:<40}  {at}  {reason}");
            }
        }
    }
    Ok(())
}

/// Parse the repeatable `--identifier system=value` flag shared by `patient-search` and
/// `patient-register`. Pure (no I/O) so it is directly unit-testable — see the `tests`
/// module below.
///
/// STRICT ON PURPOSE. A silently-skipped malformed pair is worse than a loud error here: the
/// caller feeds the result straight into a `SearchQuery` that `patient-register` also signs
/// into a permanent attestation, so a pair this function quietly dropped would let that
/// attestation claim a search was run against an identifier it never actually searched for.
/// Splitting on only the FIRST `=` (not `split('=')`) deliberately allows a value that itself
/// contains `=` (e.g. a base64-ish fragment) — only the separator between `system` and
/// `value` is special.
///
/// **The blank check is `trim().is_empty()`, matching the persistence side (final review,
/// N2/N3).** It used to be a bare `.is_empty()`, which let `--identifier "MRN=   "` (a
/// whitespace-only value) past the CLI, into a `SearchQuery` that gets SIGNED into the
/// permanent attestation — and then silently dropped by
/// `cairn_node::patient::register::supplied_identifiers`'s own `trim().is_empty()` filter,
/// never persisted. That is the "attested but not persisted" shape this whole slice exists to
/// close, one edge later than the fix that closed the rest of it: refusing loudly here, before
/// anything is signed, is strictly better than a downstream silent drop.
fn parse_identifier_pairs(raw: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    raw.iter()
        .map(|s| {
            let (system, value) = s.split_once('=').ok_or_else(|| {
                anyhow::anyhow!(
                    "malformed --identifier {s:?}: expected `system=value`, e.g. \
                     --identifier MRN=12345"
                )
            })?;
            if system.trim().is_empty() || value.trim().is_empty() {
                anyhow::bail!(
                    "malformed --identifier {s:?}: both `system` and `value` must be \
                     non-empty (expected `system=value`, e.g. --identifier MRN=12345)"
                );
            }
            Ok((system.to_string(), value.to_string()))
        })
        .collect()
}

/// Width of the `name` column in `print_candidates`' fixed-column layout. Both the header's
/// `{:<name_w$}` and `ellipsize`'s ceiling read from here so the two can never drift apart.
const NAME_COLUMN_WIDTH: usize = 28;

/// Shorten `s` to at most `width` CHARACTERS, marking the cut with a trailing `…`.
///
/// Character-based, not byte-based: `&s[..width]` panics on a multi-byte boundary, and a
/// patient name is exactly where multi-byte characters live ("田中 太郎", "José"). Culture-
/// neutral in the §4.2 sense — it never inspects or parses the name, only counts characters.
///
/// The ellipsis is what keeps this honest (principle 4): a silently-clipped name reads as the
/// whole name, and on a wrong-chart-prevention surface "Nguyen Thi Minh" clipped to
/// "Nguyen Thi Min" is a precise untruth a clerk could act on. It costs one character of the
/// budget — the returned string is still at most `width` characters wide, so the column holds.
///
/// Grapheme clusters, not chars, would be the fully correct unit (a combining sequence can
/// span several `char`s and render as one column), as would East-Asian double-width handling.
/// Both need a dependency and neither can make a row WIDER than this bound, so the column
/// alignment this function exists to protect is safe either way; the residual is that a
/// double-width name may render narrower than 28 columns, never wider.
fn ellipsize(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    // `width - 1` leaves room for the ellipsis itself. `width == 0` cannot happen from the
    // one call site (a compile-time constant of 28), but saturating keeps the function total
    // rather than panicking if a future caller passes 0.
    let keep = width.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// Render a `CandidateList` the way `patient-search` and `patient-register` both show it: one
/// line per candidate, in the exact order the list carries them (`patient-register` attests
/// this same order — see `register::build_registration_body`'s doc — so the print order and
/// the attested order must never be allowed to drift apart).
///
/// The `incomplete` reason is printed LAST and on its own line, deliberately AFTER every
/// candidate row rather than as a header above them (ADR-0060 decision 2: partial completion
/// must be reported, never implied). A reason printed first can scroll off the top of a
/// terminal behind a long candidate list and never be seen; printed last, it is the last thing
/// on screen no matter how many rows precede it.
fn print_candidates(list: &cairn_patient_search::CandidateList) {
    if list.candidates.is_empty() {
        println!("no candidates found");
    } else {
        // `{:name_w$}` on both this header and the row below, from ONE constant, so the
        // header can never drift out of step with the width `ellipsize` truncates to.
        println!(
            "{:<36}  {:<name_w$}  {:>4}  {:<12}  {:<12}  locale",
            "patient_id",
            "name",
            "age",
            "trust",
            "last activity",
            name_w = NAME_COLUMN_WIDTH
        );
        for c in &list.candidates {
            let age = c
                .age
                .as_ref()
                .map(|a| a.years.to_string())
                .unwrap_or_else(|| "?".to_string());
            let last_activity = c.last_activity.as_deref().unwrap_or("-");
            let locale = c.locale.as_deref().unwrap_or("-");
            println!(
                "{:<36}  {:<name_w$}  {:>4}  {:<12}  {:<12}  {}",
                c.patient_id,
                // TRUNCATED, not merely padded. Rust's `{:<n}` is a MINIMUM width: one
                // long name would push the age/trust/last-activity columns right on that
                // row alone, so a clerk scanning DOWN the age column to tell two same-named
                // patients apart loses the alignment exactly when a name is unusual — i.e.
                // exactly when this wrong-chart-prevention surface is being leaned on. The
                // ellipsis is what keeps the truncation honest (principle 4: a shortened
                // value must not read as the whole value).
                //
                // Only the NAME needs this. `patient_id` is a UUID (always 36), `age` is a
                // small integer or "?", `trust` is a closed short vocabulary, and
                // `last_activity` is an ISO date — all bounded. `locale` is unbounded too
                // (its own doc warns it is not guaranteed to be suburb-only — issue #347),
                // but it is the LAST column, so a long value can only wrap; it cannot
                // misalign anything, and truncating it would hide the disambiguating detail
                // it exists to show.
                ellipsize(&c.display_name, NAME_COLUMN_WIDTH),
                age,
                c.trust.as_str(),
                last_activity,
                locale,
                name_w = NAME_COLUMN_WIDTH
            );
        }
    }
    // Deliberately last — see the fn doc. Do not hoist this above the loop.
    if list.incomplete {
        let reason = list
            .incomplete_reason
            .as_deref()
            .unwrap_or("(no reason given)");
        println!("! search incomplete: {reason}");
    }
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

    // --- `print_candidates`' name column must not shift on a long name (final review) ---

    #[test]
    fn a_short_name_is_returned_unchanged_and_never_gains_an_ellipsis() {
        assert_eq!(ellipsize("Smith, John", NAME_COLUMN_WIDTH), "Smith, John");
        // Exactly at the boundary: still whole, still no ellipsis.
        let exact: String = std::iter::repeat_n('x', NAME_COLUMN_WIDTH).collect();
        assert_eq!(ellipsize(&exact, NAME_COLUMN_WIDTH), exact);
    }

    #[test]
    fn a_long_name_is_cut_to_the_column_width_and_says_so() {
        let long: String = std::iter::repeat_n('x', NAME_COLUMN_WIDTH + 40).collect();
        let out = ellipsize(&long, NAME_COLUMN_WIDTH);
        assert_eq!(
            out.chars().count(),
            NAME_COLUMN_WIDTH,
            "the whole point: the rendered cell must never exceed the column, or every \
             later column shifts on that row alone"
        );
        assert!(
            out.ends_with('…'),
            "a clipped name must not read as the whole name (principle 4): {out}"
        );
    }

    #[test]
    fn a_multibyte_name_is_cut_on_a_character_boundary_not_a_byte_one() {
        // Byte slicing here would PANIC mid-character. A patient name is exactly where
        // multi-byte characters live, so this is the realistic case, not an exotic one.
        let long: String = std::iter::repeat_n('田', NAME_COLUMN_WIDTH + 5).collect();
        let out = ellipsize(&long, NAME_COLUMN_WIDTH);
        assert_eq!(out.chars().count(), NAME_COLUMN_WIDTH);
        assert!(out.ends_with('…'));
        assert!(out.starts_with('田'));
    }

    #[test]
    fn a_well_formed_identifier_pair_parses() {
        let out = parse_identifier_pairs(&["MRN=12345".to_string()]).unwrap();
        assert_eq!(out, vec![("MRN".to_string(), "12345".to_string())]);
    }

    #[test]
    fn several_identifier_pairs_parse_in_order() {
        let raw = vec!["MRN=12345".to_string(), "NHI=ABC1234".to_string()];
        let out = parse_identifier_pairs(&raw).unwrap();
        assert_eq!(
            out,
            vec![
                ("MRN".to_string(), "12345".to_string()),
                ("NHI".to_string(), "ABC1234".to_string()),
            ]
        );
    }

    #[test]
    fn a_value_may_itself_contain_an_equals_sign() {
        // Only the FIRST `=` is the system/value separator — a base64-ish value carrying its
        // own `=` must survive intact rather than being truncated or rejected.
        let out = parse_identifier_pairs(&["TOKEN=abc=def=".to_string()]).unwrap();
        assert_eq!(out, vec![("TOKEN".to_string(), "abc=def=".to_string())]);
    }

    #[test]
    fn a_pair_with_no_equals_sign_is_rejected_not_dropped() {
        // The load-bearing case (#344 brief): a malformed pair must be a loud, legible error
        // naming the expected form — never silently skipped. A silent skip would let
        // `patient-register` attest a search that never actually ran against this identifier.
        let err = parse_identifier_pairs(&["MRN12345".to_string()]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("MRN12345") && msg.contains("system=value"),
            "error must name the offending input and the expected form: {msg}"
        );
    }

    #[test]
    fn an_empty_system_is_rejected() {
        let err = parse_identifier_pairs(&["=12345".to_string()]).unwrap_err();
        assert!(err.to_string().contains("system=value"));
    }

    #[test]
    fn an_empty_value_is_rejected() {
        let err = parse_identifier_pairs(&["MRN=".to_string()]).unwrap_err();
        assert!(err.to_string().contains("system=value"));
    }

    #[test]
    fn a_whitespace_only_value_is_rejected_at_the_cli_edge() {
        // The N2 regression: a bare `.is_empty()` check let `--identifier "MRN=   "` (no
        // real value, only spaces) past the CLI and into a `SearchQuery` that gets SIGNED
        // into the permanent attestation, only to be silently dropped later by
        // `supplied_identifiers`'s own `trim().is_empty()` filter — attested but never
        // persisted. Refusing here, loudly, before anything is signed, is the fix.
        let err = parse_identifier_pairs(&["MRN=   ".to_string()]).unwrap_err();
        assert!(err.to_string().contains("system=value"));
    }

    #[test]
    fn a_whitespace_only_system_is_rejected_at_the_cli_edge() {
        let err = parse_identifier_pairs(&["   =12345".to_string()]).unwrap_err();
        assert!(err.to_string().contains("system=value"));
    }

    #[test]
    fn one_malformed_pair_among_valid_ones_fails_the_whole_parse() {
        // Strict, not best-effort: a single bad pair must not let the OTHER, well-formed
        // pairs quietly proceed while it is dropped — the whole command refuses instead.
        let raw = vec!["MRN=12345".to_string(), "bad-pair".to_string()];
        assert!(parse_identifier_pairs(&raw).is_err());
    }

    #[test]
    fn an_empty_list_of_identifiers_parses_to_empty() {
        assert_eq!(parse_identifier_pairs(&[]).unwrap(), Vec::new());
    }
}
