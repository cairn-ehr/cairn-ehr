//! #909 slice C2, Task 6: `release-intake` / `release-status` against the real db/086 store.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized by `db::test_serial_guard`. Every key is computed at
//! runtime (house rule 6); bundles are built with the steward's own builders and so verify
//! through the very code the node runs.
mod common;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::release_intake::{status, take_in, IntakeError};
use cairn_release::{
    assemble, genesis, make_genesis, rotate, Artifact, BundleFile, Refusal, SignedRoot, Signer,
    TrustRoot, VerifiedRoot,
};
use common::cs;
use tokio_postgres::Client;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| (i as u8).wrapping_mul(31) ^ seed))
}

fn signer(sk: &SigningKey) -> Signer {
    let pubkey = sk.verifying_key().to_bytes();
    Signer {
        kid: pubkey[..4].to_vec(),
        pubkey,
        custody_note: "test".into(),
    }
}

/// A one-signer-per-role channel: (genesis file, its verified history, root key, release key).
fn channel() -> (SignedRoot, Vec<VerifiedRoot>, SigningKey, SigningKey) {
    let (root_key, release_key) = (key(1), key(11));
    let g = make_genesis(
        "official",
        vec![signer(&root_key)],
        1,
        vec![signer(&release_key)],
        1,
        &[&root_key],
    );
    let history = vec![genesis(&g).unwrap()];
    (g, history, root_key, release_key)
}

fn window(os_platform: &str) -> Artifact {
    Artifact {
        kind: "window".into(),
        digest: [0; 32],
        byte_len: 0,
        os: Some("linux".into()),
        arch: Some("aarch64".into()),
        platform: Some(os_platform.into()),
        generation_line: None,
    }
}

fn release(history: &[VerifiedRoot], v: u64, rk: &SigningKey, bytes: &[u8]) -> BundleFile {
    assemble(
        history,
        v,
        vec![(window("debian-12"), bytes.to_vec())],
        &[rk],
    )
}

/// Connected owner with empty release tables; the guard keeps other suites out.
async fn rig() -> Option<(Client, Client)> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect(&base).await.unwrap();
    owner
        .batch_execute("TRUNCATE release_artifact, release_bundle, release_root, release_incident")
        .await
        .unwrap();
    Some((guard, owner))
}

async fn count(c: &Client, table: &str) -> i64 {
    c.query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await
        .unwrap()
        .get(0)
}

async fn offered(c: &Client) -> i64 {
    c.query_one(
        "SELECT count(*) FROM cairn_release_offer('linux','aarch64','debian-12')",
        &[],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_signed_bundle_is_taken_in_and_offered() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    let (g, h, _, rk) = channel();
    let done = take_in(&mut owner, &release(&h, 1, &rk, b"program one"), Some(g))
        .await
        .unwrap();
    assert_eq!((done.release_version, done.files), (1, 1));
    assert_eq!(count(&owner, "release_root").await, 1);
    assert_eq!(offered(&owner).await, 1);
}

#[tokio::test]
async fn genesis_is_refused_once_a_root_is_held() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    let (g, h, _, rk) = channel();
    take_in(&mut owner, &release(&h, 1, &rk, b"one"), Some(g.clone()))
        .await
        .unwrap();
    let err = take_in(&mut owner, &release(&h, 2, &rk, b"two"), Some(g))
        .await
        .unwrap_err();
    assert!(matches!(err, IntakeError::GenesisAlreadyHeld), "{err}");
    assert_eq!(count(&owner, "release_bundle").await, 1);
}

#[tokio::test]
async fn no_genesis_and_no_official_root_is_no_channel() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    let (_, h, _, rk) = channel();
    let err = take_in(&mut owner, &release(&h, 1, &rk, b"one"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, IntakeError::Refused(Refusal::NoChannel)),
        "{err}"
    );
    assert_eq!(count(&owner, "release_root").await, 0);
}

#[tokio::test]
async fn a_fork_records_an_incident_and_freezes_offers() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    let (g, h, rootk, rk) = channel();
    let draft = |note: &str| TrustRoot {
        channel_id: "official".into(),
        version: 0,
        root_signers: vec![signer(&rootk)],
        root_threshold: 1,
        release_signers: vec![Signer {
            custody_note: note.into(),
            ..signer(&rk)
        }],
        release_threshold: 1,
        prev_root_hash: None,
    };
    let (v2a, v2b) = (
        rotate(&h[0], draft("a"), &[&rootk]),
        rotate(&h[0], draft("b"), &[&rootk]),
    );
    let ha = cairn_release::advance(&h, std::slice::from_ref(&v2a)).unwrap();
    take_in(&mut owner, &release(&ha, 1, &rk, b"one"), Some(g))
        .await
        .unwrap();
    assert_eq!(offered(&owner).await, 1);

    let hb = cairn_release::advance(&h, std::slice::from_ref(&v2b)).unwrap();
    let err = take_in(&mut owner, &release(&hb, 2, &rk, b"two"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, IntakeError::Refused(Refusal::Fork { version: 2 })),
        "{err}"
    );
    assert_eq!(count(&owner, "release_incident").await, 1);
    assert_eq!(count(&owner, "release_bundle").await, 1);
    assert_eq!(
        offered(&owner).await,
        0,
        "an open incident freezes the offer door"
    );
    assert!(status(&owner)
        .await
        .unwrap()
        .contains("Updates are stopped"));
}

#[tokio::test]
async fn a_bad_artifact_leaves_nothing_behind() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    let (g, h, _, rk) = channel();
    let mut b = release(&h, 1, &rk, b"program");
    b.artifacts[0] = b"tampered".to_vec();
    let err = take_in(&mut owner, &b, Some(g.clone())).await.unwrap_err();
    assert!(
        matches!(err, IntakeError::Refused(Refusal::ArtifactMismatch { .. })),
        "{err}"
    );
    // A spare, unlisted file is refused too.
    let mut b = release(&h, 1, &rk, b"program");
    b.artifacts.push(b"extra".to_vec());
    take_in(&mut owner, &b, Some(g)).await.unwrap_err();
    for t in [
        "release_root",
        "release_bundle",
        "release_artifact",
        "release_incident",
    ] {
        assert_eq!(count(&owner, t).await, 0, "{t}");
    }
}

#[tokio::test]
async fn the_same_release_twice_is_not_newer() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    let (g, h, _, rk) = channel();
    take_in(&mut owner, &release(&h, 1, &rk, b"one"), Some(g))
        .await
        .unwrap();
    let err = take_in(&mut owner, &release(&h, 1, &rk, b"one"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            IntakeError::Refused(Refusal::NotNewer {
                installed: 1,
                offered: 1
            })
        ),
        "{err}"
    );
}

#[tokio::test]
async fn old_releases_are_pruned_to_two() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    let (g, h, _, rk) = channel();
    take_in(&mut owner, &release(&h, 1, &rk, b"one"), Some(g))
        .await
        .unwrap();
    for (v, bytes) in [(2, b"two".as_slice()), (3, b"three")] {
        take_in(&mut owner, &release(&h, v, &rk, bytes), None)
            .await
            .unwrap();
    }
    let versions: Vec<i64> = owner
        .query("SELECT release_version FROM release_bundle ORDER BY 1", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(versions, [2, 3]);
    assert_eq!(count(&owner, "release_artifact").await, 2);
}

#[tokio::test]
async fn status_names_channel_root_release_and_incident() {
    let Some((_g, mut owner)) = rig().await else {
        return;
    };
    assert!(status(&owner).await.unwrap().contains("No release channel"));
    let (g, h, _, rk) = channel();
    take_in(&mut owner, &release(&h, 4, &rk, b"four"), Some(g))
        .await
        .unwrap();
    let s = status(&owner).await.unwrap();
    assert!(
        s.contains("official") && s.contains("version 1") && s.contains("release held: 4"),
        "{s}"
    );
    owner
        .execute(
            "INSERT INTO release_incident (channel_id, detail) VALUES ('official', 'Two trust files clash.')",
            &[],
        )
        .await
        .unwrap();
    assert!(status(&owner)
        .await
        .unwrap()
        .contains("Updates are stopped: Two trust files clash."));
}
