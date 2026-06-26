//! ADR-0026 slice B — backup-as-cold-peer: the DB/IO orchestration of an export plus
//! backup-health surfacing. The PURE medium container format (framing, the CAIRNB2
//! self-marker, signature verification) lives in [`crate::medium`]; this module is the thin
//! DB-touching layer on top.
//!
//! WHY THIS EXISTS: the spec designed *intentional* key-death in detail (crypto-shred) but
//! left *accidental* data-death — a node's disk simply dying — undesigned. For a genuinely-solo
//! clinic (no parent to re-provision from) replication provides zero durability, so a backup is
//! the only safety net. ADR-0026's insight: a backup is just another replication peer — the
//! medium holds a NORMAL Cairn event set and restore is set-union apply through the existing
//! verify-on-apply path. This module reads the signed `node_event` set, writes it to a local
//! medium (with a self-marker so restore can tell which node it belongs to — see `medium`),
//! and surfaces backup health (point 7: a node running without a net must say so).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// The medium format is defined once in `crate::medium`. Re-export the surface other modules
// and tests already reach for via `backup::…`, so the format move stays source-compatible.
pub use crate::medium::{
    parse_medium, verify_event, verify_events, verify_medium_bytes, BackupError, SelfMarker,
    VerifyReport,
};

// ---------------------------------------------------------------------------
// Backup health (node-local operational state — NOT a clinical event, never signed,
// never replicated). Lives in a local sidecar JSON, not the DB: see the slice-B design
// note (smaller safety-critical surface — no SECURITY DEFINER door — and it fails SAFE,
// degrading to "never / running without a net" when absent or unreadable).
// ---------------------------------------------------------------------------

/// A record of the last successful backup. Written only AFTER the medium is durable and
/// self-verified, so it can never over-claim a backup the node does not actually hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupHealth {
    pub version: u8,
    /// Unix seconds at which the backup completed (operational wall-clock, not the HLC).
    pub last_backup_unix: i64,
    /// Where the medium was written (for the operator to locate it).
    pub medium_path: String,
    /// How many signed events the medium holds.
    pub event_count: u64,
    /// Size of the medium image in bytes.
    pub medium_bytes: u64,
}

/// The sidecar path for backup health: a sibling of the key file named
/// `backup-status.json`. Node-local, discoverable from what `status` already has (the
/// key path). Pure.
pub fn health_path_for(key_path: &Path) -> PathBuf {
    key_path.with_file_name("backup-status.json")
}

/// Render a coarse "time ago" for a `status` line. Pure. Negative input (a clock that
/// went backwards between backup and read) is reported as "just now" rather than a
/// nonsense negative age — honest degradation, never a confusing display.
pub fn humanize_ago(secs: i64) -> String {
    if secs <= 0 {
        return "just now".to_string();
    }
    let s = secs as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// The `status` backup-health line. Pure (time injected). Absent health → the honest
/// "running without a net" warning; present → freshness + size + location.
pub fn describe_health(now_unix: i64, health: &Option<BackupHealth>) -> String {
    match health {
        None => "never — running without a net".to_string(),
        Some(h) => format!(
            "{} ago ({} events, {} bytes -> {})",
            humanize_ago(now_unix - h.last_backup_unix),
            h.event_count,
            h.medium_bytes,
            h.medium_path,
        ),
    }
}

/// Read the backup-health sidecar. Returns `None` on ANY error (absent, unreadable,
/// malformed) — the fail-safe reading, so `status` degrades to "never / running without
/// a net" rather than asserting a freshness it cannot vouch for. A lying or missing
/// sidecar can only UNDER-claim; it can never cause data loss, because the load-bearing
/// guarantee lives in the self-verifying medium, not here.
pub fn read_health(path: &Path) -> Option<BackupHealth> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Atomically write the backup-health sidecar (owner-only). Atomic so a torn write can
/// never corrupt the freshness reading; the flip from old→new is a single rename.
pub fn write_health(path: &Path, health: &BackupHealth) -> Result<(), BackupError> {
    let json = serde_json::to_vec_pretty(health)
        .map_err(|e| BackupError::Decode(format!("serializing backup health: {e}")))?;
    crate::fsio::atomic_write(path, &json, Some(0o600))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// DB / IO glue (thin — the only DB-touching part of this module).
// ---------------------------------------------------------------------------

/// Which self-marker `backup_to` actually wrote into the medium, so the caller can warn an
/// operator when a medium is only operator-error-safe (unsigned) rather than tamper-evident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrittenMarker {
    /// No marker — the node is not yet enrolled, so there is no identity to attest.
    None,
    /// The self node-id without a signature (the signing key was not available at backup).
    Unsigned,
    /// A signed self-attestation — tampering can only withhold it, never misdirect (medium docs).
    Signed,
}

/// What one backup did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub event_count: usize,
    pub medium_bytes: usize,
    pub marker: WrittenMarker,
}

/// Read this node's signed `node_event` set, in local `seq` order. A plain `SELECT` —
/// any role with read access works (the runtime `cairn_node` role has `GRANT SELECT ON
/// node_event`); no signing key and no validated door are needed to back up.
pub async fn read_event_set(db: &tokio_postgres::Client) -> anyhow::Result<Vec<Vec<u8>>> {
    use anyhow::Context;
    let rows = db
        .query("SELECT signed_bytes FROM node_event ORDER BY seq", &[])
        .await
        .context("reading node_event set for backup")?;
    Ok(rows.iter().map(|r| r.get::<_, Vec<u8>>(0)).collect())
}

/// This node's own genesis node-id (hex), from `local_node`, or `None` if not yet enrolled.
/// The authoritative answer to "whose backup is this?" — recorded into the medium's marker
/// while we are still live (set-union sync cannot erase what we write into the container).
async fn read_self_node_id(db: &tokio_postgres::Client) -> anyhow::Result<Option<String>> {
    use anyhow::Context;
    let row = db
        .query_opt(
            "SELECT encode(node_id,'hex') AS id FROM local_node WHERE id",
            &[],
        )
        .await
        .context("reading local_node id for the backup self-marker")?;
    Ok(row.map(|r| r.get::<_, String>("id")))
}

/// Choose the self-marker to embed: signed when the node's key is supplied AND the node is
/// enrolled; unsigned when enrolled but no key was available; none when not yet enrolled. The
/// signed attestation is bound to `events` (the exact set being backed up) so it cannot be
/// replayed onto another medium.
fn choose_marker(
    self_id: Option<String>,
    marker_key: Option<(&cairn_event::SigningKey, &str)>,
    events: &[Vec<u8>],
) -> Option<SelfMarker> {
    let id = self_id?;
    match marker_key {
        Some((sk, key_id)) => Some(SelfMarker::Signed(crate::medium::build_self_attestation(
            sk, key_id, &id, events,
        ))),
        None => Some(SelfMarker::Unsigned(id)),
    }
}

/// Back up the node's event set to `medium_path`, then record health at `health_path`.
///
/// `marker_key` is the node's signing key (+ key-id): when present, the medium carries a
/// SIGNED self-attestation (tamper can only withhold it on restore, never misdirect — see
/// [`crate::medium`]); when `None`, an UNSIGNED self-marker is written instead (still closes
/// the operator-typo footgun, just not tamper-evident). An unsigned marker NEVER blocks a
/// backup — the caller decides whether the key is available and warns accordingly.
///
/// Ordering is deliberately fail-safe and verify-BEFORE-write:
///   1. serialize the medium and self-verify the image IN MEMORY — if the event set fails its
///      own signature check we BAIL before touching disk, so the previous good medium at
///      `medium_path` is left completely untouched;
///   2. write the verified image atomically (a crash here never destroys the previous good
///      medium either — the rename either lands whole or not at all);
///   3. re-read and self-verify the on-disk bytes — a defence-in-depth tripwire for an fs bug
///      between write and rename; still BAIL WITHOUT touching health if it fails;
///   4. only then update the health sidecar.
///
/// A crash between (3) and (4) leaves health UNDER-reporting (older / "never"), which is the
/// correct direction for a safety-net indicator — it must never over-claim.
///
/// `now_unix` is injected (operational wall-clock) so the function stays deterministic and
/// testable; the CLI passes `SystemTime::now()`.
pub async fn backup_to(
    db: &tokio_postgres::Client,
    medium_path: &Path,
    health_path: &Path,
    now_unix: i64,
    marker_key: Option<(&cairn_event::SigningKey, &str)>,
) -> anyhow::Result<BackupReport> {
    use anyhow::Context;
    let events = read_event_set(db).await?;
    let self_id = read_self_node_id(db).await?;
    let marker = choose_marker(self_id, marker_key, &events);
    let written = match &marker {
        None => WrittenMarker::None,
        Some(SelfMarker::Unsigned(_)) => WrittenMarker::Unsigned,
        Some(SelfMarker::Signed(_)) => WrittenMarker::Signed,
    };

    // Verify the event set BEFORE it can overwrite the live medium: a set that fails its own
    // signature check is rejected here, with the previous good medium still intact on disk.
    let medium = crate::medium::serialize_and_verify_container(marker.as_ref(), &events)
        .context("self-verifying the backup image before writing (previous medium untouched)")?;

    crate::fsio::atomic_write(medium_path, &medium, Some(0o600))
        .with_context(|| format!("writing backup medium to {}", medium_path.display()))?;

    // Read-after-write: the on-disk bytes must still parse AND verify (catches an fs/rename
    // bug), or we refuse to advance health (never tell the operator a broken backup is good).
    let readback = std::fs::read(medium_path)
        .with_context(|| format!("re-reading backup medium {}", medium_path.display()))?;
    let report = verify_medium_bytes(&readback)
        .with_context(|| format!("verifying freshly-written medium {}", medium_path.display()))?;
    if !report.all_intact() {
        anyhow::bail!(
            "backup medium failed self-verification after write ({} of {} events intact, \
             first bad at index {:?}); health NOT advanced",
            report.intact,
            report.total,
            report.first_bad
        );
    }

    let health = BackupHealth {
        version: 1,
        last_backup_unix: now_unix,
        medium_path: medium_path.display().to_string(),
        event_count: events.len() as u64,
        medium_bytes: medium.len() as u64,
    };
    write_health(health_path, &health).context("writing backup-health sidecar")?;

    Ok(BackupReport {
        event_count: events.len(),
        medium_bytes: medium.len(),
        marker: written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn humanize_ago_buckets_are_coarse_and_safe() {
        assert_eq!(
            humanize_ago(-5),
            "just now",
            "a backwards clock must not show a negative age"
        );
        assert_eq!(humanize_ago(0), "just now");
        assert_eq!(humanize_ago(42), "42s");
        assert_eq!(humanize_ago(120), "2m");
        assert_eq!(humanize_ago(7200), "2h");
        assert_eq!(humanize_ago(172_800), "2d");
    }

    #[test]
    fn describe_health_warns_when_absent_and_summarizes_when_present() {
        assert_eq!(
            describe_health(1000, &None),
            "never — running without a net"
        );
        let h = BackupHealth {
            version: 1,
            last_backup_unix: 1000,
            medium_path: "/mnt/backup/cairn.medium".into(),
            event_count: 7,
            medium_bytes: 2048,
        };
        let line = describe_health(1000 + 3600, &Some(h));
        assert!(line.starts_with("1h ago"), "freshness first: got {line:?}");
        assert!(line.contains("7 events"));
        assert!(line.contains("/mnt/backup/cairn.medium"));
    }

    #[test]
    fn health_sidecar_roundtrips_and_absent_reads_as_none() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("backup-status.json");
        assert_eq!(
            read_health(&p),
            None,
            "a missing sidecar reads as None (fail-safe)"
        );
        let h = BackupHealth {
            version: 1,
            last_backup_unix: 12_345,
            medium_path: "/mnt/x".into(),
            event_count: 3,
            medium_bytes: 999,
        };
        write_health(&p, &h).unwrap();
        assert_eq!(
            read_health(&p),
            Some(h),
            "a written sidecar reads back exactly"
        );
    }

    #[test]
    fn malformed_sidecar_reads_as_none_not_a_crash() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("backup-status.json");
        std::fs::write(&p, b"{ this is not valid json").unwrap();
        assert_eq!(
            read_health(&p),
            None,
            "a corrupt sidecar must fail safe to None"
        );
    }

    #[test]
    fn health_path_is_a_sibling_of_the_key() {
        let p = health_path_for(Path::new("/var/lib/cairn/node.key"));
        assert_eq!(p, Path::new("/var/lib/cairn/backup-status.json"));
    }

    #[test]
    fn choose_marker_picks_signed_unsigned_or_none() {
        let events: Vec<Vec<u8>> = vec![];
        // No identity yet → no marker (nothing to attest).
        assert_eq!(choose_marker(None, None, &events), None);
        // Enrolled but no key available → unsigned marker carrying the self id.
        assert_eq!(
            choose_marker(Some("abcd".into()), None, &events),
            Some(SelfMarker::Unsigned("abcd".into()))
        );
        // Enrolled + key → a signed attestation (a Signed variant; its bytes are exercised in
        // the medium module's verification tests).
        let (sk, _) = cairn_event::generate_key().unwrap();
        let kid = hex::encode(sk.verifying_key().to_bytes());
        assert!(matches!(
            choose_marker(Some("abcd".into()), Some((&sk, &kid)), &events),
            Some(SelfMarker::Signed(_))
        ));
    }
}
