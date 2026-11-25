//! The node owner's two release verbs (#909 slice C2): `release-intake` and `release-status`.
//!
//! WHY. A terminal fetches its steward-signed update THROUGH its node (db/086 doors). Someone
//! has to put the update there: the owner hands `release-intake` a bundle file; this module
//! verifies it with the same `cairn-release` verifier the terminal's installer uses, then
//! stores it. The node is not the terminal's trust anchor (the installer re-verifies against
//! its OWN pin), so intake is a courtesy gate: it keeps junk and forks out of the store.
//!
//! FLOW of [`take_in`], all inside ONE transaction that first takes the intake advisory lock:
//! load the stored trust roots -> rebuild a VERIFIED history from them (ruling R7: stored roots
//! are never trusted as verified) -> `verify_bundle` -> check every file against the signed
//! manifest -> write roots, bundle, files and prune. A fork is the one refusal that writes: after
//! the rollback it records an incident (own statement, so it survives the refusal), which makes
//! the db/086 offer door go silent and the chunk door refuse.
use cairn_release::{
    advance, check_artifact, decode, genesis, official_root, verify_bundle, BundleFile, Refusal,
    SignedRoot, VerifiedRoot, Who,
};
use std::path::Path;
use tokio_postgres::{Client, GenericClient, Transaction};

/// Why an intake did not happen.
#[derive(Debug, thiserror::Error)]
pub enum IntakeError {
    /// The verifier said no; the message is its plain sentence (`Refusal::sentence`).
    #[error("{}", .0.sentence(Who::Node))]
    Refused(Refusal),
    /// `--genesis-root` named a first trust file other than the one this node already holds.
    #[error("This node already holds a release channel's trust file, so it will not take a different first one.")]
    GenesisAlreadyHeld,
    /// A database failure, already rendered legibly.
    #[error("{0}")]
    Db(String),
}

impl From<tokio_postgres::Error> for IntakeError {
    fn from(e: tokio_postgres::Error) -> Self {
        IntakeError::Db(crate::db_diagnosis::legible_db_error(&e))
    }
}

/// What a successful intake did.
#[derive(Debug, PartialEq, Eq)]
pub struct IntakeOutcome {
    pub release_version: u64,
    pub files: usize,
}

/// One intake at a time: the verdict reads the held roots and newest release, so two intakes
/// racing on them could both pass (R16). Taken inside the intake transaction, released with it.
const INTAKE_LOCK: i64 = 0x0909_C201;

/// The stored roots, ascending by version.
async fn stored_roots(db: &impl GenericClient) -> Result<Vec<SignedRoot>, IntakeError> {
    let rows = db
        .query(
            "SELECT payload, signatures FROM release_root ORDER BY version",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| SignedRoot {
            payload: r.get(0),
            signatures: r.get(1),
        })
        .collect())
}

/// Rebuild a verified history from `stored` (non-empty): genesis of the first, then advance
/// over the rest. Pure. A stored chain that no longer verifies is refused, not trusted.
pub fn rebuild_history(stored: &[SignedRoot]) -> Result<Vec<VerifiedRoot>, Refusal> {
    let first = stored.first().ok_or(Refusal::NoChannel)?;
    advance(&[genesis(first)?], &stored[1..])
}

/// Choose the history to verify against: the stored roots if any, else the supplied genesis,
/// else the compiled-in official root, else `NoChannel`. With roots held, a supplied genesis is
/// refused unless it is the very first file held (same payload, so the same content address):
/// an owner re-running the first intake's command changes nothing (ruling R30).
fn starting_history(
    stored: &[SignedRoot],
    supplied: Option<SignedRoot>,
) -> Result<Vec<VerifiedRoot>, IntakeError> {
    match (stored.first(), supplied) {
        (Some(held), Some(s)) if s.payload != held.payload => Err(IntakeError::GenesisAlreadyHeld),
        (Some(_), _) => rebuild_history(stored).map_err(IntakeError::Refused),
        (None, supplied) => {
            let root = supplied
                .or_else(official_root)
                .ok_or(IntakeError::Refused(Refusal::NoChannel))?;
            Ok(vec![genesis(&root).map_err(IntakeError::Refused)?])
        }
    }
}

/// A `u64` from a signed document as the database's `bigint`; one too large is a damaged file.
fn to_i64(v: u64) -> Result<i64, IntakeError> {
    i64::try_from(v).map_err(|_| {
        IntakeError::Refused(Refusal::Malformed("a number is too large to store".into()))
    })
}

/// Verify `bundle` and store it. All or nothing, except that a fork leaves an incident row.
pub async fn take_in(
    db: &mut Client,
    bundle: &BundleFile,
    supplied_genesis: Option<SignedRoot>,
) -> Result<IntakeOutcome, IntakeError> {
    let tx = db.transaction().await?;
    let mut channel = String::new();
    let result = verify_and_store(&tx, bundle, supplied_genesis, &mut channel).await;
    if result.is_ok() {
        tx.commit().await?;
        return result;
    }
    tx.rollback().await?;
    if let Err(IntakeError::Refused(r @ Refusal::Fork { version })) = &result {
        // After the rollback, in its own committed statement, so the incident outlives the
        // refusal. `channel` was filled from the history the fork was found against.
        db.execute(
            "INSERT INTO release_incident (channel_id, version, detail) VALUES ($1, $2, $3)",
            &[&channel, &to_i64(*version)?, &r.sentence(Who::Node)],
        )
        .await?;
    }
    result
}

/// The body of an intake, inside the transaction that holds [`INTAKE_LOCK`].
async fn verify_and_store(
    tx: &Transaction<'_>,
    bundle: &BundleFile,
    supplied_genesis: Option<SignedRoot>,
    channel: &mut String,
) -> Result<IntakeOutcome, IntakeError> {
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&INTAKE_LOCK])
        .await?;
    let stored = stored_roots(tx).await?;
    let history = starting_history(&stored, supplied_genesis)?;
    *channel = history[0].root().channel_id.clone();
    let last_installed: i64 = tx
        .query_one(
            "SELECT coalesce(max(release_version), 0) FROM release_bundle",
            &[],
        )
        .await?
        .get(0);

    let (new_history, manifest) = verify_bundle(&history, &bundle.meta, last_installed as u64)
        .map_err(IntakeError::Refused)?;
    // Every listed file must be present and exact; a spare file the manifest never named is
    // as wrong as a missing one.
    if bundle.artifacts.len() != manifest.artifacts.len() {
        let index = bundle.artifacts.len().min(manifest.artifacts.len());
        return Err(IntakeError::Refused(Refusal::ArtifactMismatch { index }));
    }
    for (i, bytes) in bundle.artifacts.iter().enumerate() {
        check_artifact(&manifest, i, bytes).map_err(IntakeError::Refused)?;
    }

    for r in new_history.iter().skip(stored.len()) {
        let s = r.signed();
        tx.execute(
            "INSERT INTO release_root (version, channel_id, payload, signatures, root_hash) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &to_i64(r.root().version)?,
                &r.root().channel_id,
                &s.payload,
                &s.signatures,
                &r.hash().to_vec(),
            ],
        )
        .await?;
    }
    let version = to_i64(manifest.release_version)?;
    tx.execute(
        "INSERT INTO release_bundle (release_version, channel_id, manifest, release_signatures, \
         inclusion_proof) VALUES ($1, $2, $3, $4, $5)",
        &[
            &version,
            &manifest.channel_id,
            &bundle.meta.manifest,
            &bundle.meta.release_signatures,
            &bundle.meta.inclusion_proof,
        ],
    )
    .await?;
    for (i, (a, bytes)) in manifest.artifacts.iter().zip(&bundle.artifacts).enumerate() {
        // Keyed by (release, entry): identical bytes in two entries are two rows (R15).
        tx.execute(
            "INSERT INTO release_artifact (digest, release_version, idx, kind, os, arch, platform, \
             generation_line, byte_len, content) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[
                &a.digest.to_vec(),
                &version,
                &(i as i32),
                &a.kind,
                &a.os,
                &a.arch,
                &a.platform,
                &a.generation_line,
                &to_i64(a.byte_len)?,
                bytes,
            ],
        )
        .await?;
    }
    prune(tx).await?;
    Ok(IntakeOutcome {
        release_version: manifest.release_version,
        files: bundle.artifacts.len(),
    })
}

/// Keep the newest two releases per (os, arch, platform) of kind `window`; drop older window
/// files, then any bundle left with no file at all. Two statements: a data-modifying CTE
/// cannot see its own deletes in the same statement.
async fn prune(tx: &Transaction<'_>) -> Result<(), IntakeError> {
    let gone = tx
        .query(
            "WITH ranked AS (SELECT release_version, idx, dense_rank() OVER (PARTITION BY os, arch, platform \
                 ORDER BY release_version DESC) AS r FROM release_artifact WHERE kind = 'window') \
             DELETE FROM release_artifact a USING ranked \
              WHERE a.release_version = ranked.release_version AND a.idx = ranked.idx \
                AND ranked.r > 2 RETURNING a.release_version",
            &[],
        )
        .await?;
    let versions: Vec<i64> = gone.iter().map(|r| r.get(0)).collect();
    tx.execute(
        "DELETE FROM release_bundle b WHERE b.release_version = ANY($1) \
           AND NOT EXISTS (SELECT 1 FROM release_artifact a WHERE a.release_version = b.release_version)",
        &[&versions],
    )
    .await?;
    Ok(())
}

/// The CLI wrapper: read the files, take in, say what happened. Errors carry the plain sentence.
pub async fn run_intake(
    db: &mut Client,
    bundle_path: &Path,
    genesis_path: Option<&Path>,
) -> anyhow::Result<String> {
    let read = |p: &Path| std::fs::read(p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()));
    let bundle: BundleFile =
        decode(&read(bundle_path)?).map_err(|r| anyhow::anyhow!(r.sentence(Who::Node)))?;
    let supplied = match genesis_path {
        Some(p) => Some(
            decode::<SignedRoot>(&read(p)?).map_err(|r| anyhow::anyhow!(r.sentence(Who::Node)))?,
        ),
        None => None,
    };
    let done = take_in(db, &bundle, supplied)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(format!(
        "Release {} taken in ({} file{}); terminals can now fetch it.",
        done.release_version,
        done.files,
        if done.files == 1 { "" } else { "s" }
    ))
}

/// What `release-status` reports.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StatusFacts {
    pub channel: Option<String>,
    /// A trust file is stored but cannot be read back.
    pub root_damaged: bool,
    pub root_version: Option<u64>,
    pub newest_release: Option<u64>,
    pub open_incidents: Vec<String>,
}

/// Practice-manager words, one line per fact. Pure.
pub fn render_status(f: &StatusFacts) -> String {
    let mut lines = vec![match (&f.channel, f.root_version) {
        _ if f.root_damaged => {
            "The trust file stored on this node is damaged; do not rely on it. Ask the Cairn project."
                .to_string()
        }
        (Some(c), Some(v)) => format!("Release channel: {c} (trust file version {v})."),
        _ => "No release channel is set up on this node yet.".to_string(),
    }];
    lines.push(match f.newest_release {
        Some(r) => format!("Newest release held: {r}."),
        None => "No release held yet.".to_string(),
    });
    for d in &f.open_incidents {
        lines.push(format!("Updates are stopped: {d}"));
    }
    lines.join("\n")
}

/// Read the facts and render them.
pub async fn status(db: &Client) -> anyhow::Result<String> {
    let run = async {
        // Re-verified as intake does (R28): a stored chain that decodes but no longer verifies
        // is damaged, never shown as the channel this node follows.
        let roots = stored_roots(db).await?;
        let newest_root = (!roots.is_empty()).then(|| {
            rebuild_history(&roots).map(|h| h.last().expect("non-empty history").root().clone())
        });
        let newest: Option<i64> = db
            .query_one("SELECT max(release_version) FROM release_bundle", &[])
            .await?
            .get(0);
        let open_incidents = db
            .query(
                "SELECT detail FROM release_incident WHERE resolved_at IS NULL ORDER BY id",
                &[],
            )
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        Ok::<_, IntakeError>(StatusFacts {
            root_damaged: matches!(newest_root, Some(Err(_))),
            channel: newest_root
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(|r| r.channel_id.clone()),
            root_version: newest_root.and_then(|r| r.ok()).map(|r| r.version),
            newest_release: newest.map(|v| v as u64),
            open_incidents,
        })
    };
    let facts = run.await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(render_status(&facts))
}

#[cfg(test)]
#[path = "release_intake_tests.rs"]
mod tests;
