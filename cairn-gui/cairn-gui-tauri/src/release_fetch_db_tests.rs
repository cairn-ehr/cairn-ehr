//! DB-gated walks for the window as courier (`release_fetch.rs`, #909 slice C2) on
//! `CAIRN_TEST_PG`, as the window's own login. Run locally (#905: CI never runs these).
//!
//! A one-signer channel is minted at runtime (house rule 6) and a release is taken in through the
//! node's own intake, so the doors serve exactly what a node would.
use super::*;
use cairn_event::SigningKey;
use cairn_release::{assemble, genesis, make_genesis, Artifact, Signer};

/// The target the fixture's release is built for (the platform of today's terminals).
fn pi() -> Target {
    Target {
        os: "linux".into(),
        arch: "aarch64".into(),
        platform: "debian-12".into(),
    }
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| (i as u8).wrapping_mul(29) ^ seed))
}

fn signer(sk: &SigningKey) -> Signer {
    let pubkey = sk.verifying_key().to_bytes();
    Signer {
        kid: pubkey[..4].to_vec(),
        pubkey,
        custody_note: "test".into(),
    }
}

/// The serial guard (bind it), and a window state over the test database holding release 1,
/// whose one program is `program`. `None` without `CAIRN_TEST_PG`.
async fn holding(program: &[u8]) -> Option<(tokio_postgres::Client, AppState)> {
    let Ok(cs) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let mut owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    owner
        .batch_execute("TRUNCATE release_artifact, release_bundle, release_root, release_incident")
        .await
        .unwrap();
    let (root_key, release_key) = (key(2), key(12));
    let g = make_genesis(
        "official",
        vec![signer(&root_key)],
        1,
        vec![signer(&release_key)],
        1,
        &[&root_key],
    );
    let window = Artifact {
        kind: "window".into(),
        digest: [0; 32],
        byte_len: 0,
        os: Some(pi().os),
        arch: Some(pi().arch),
        platform: Some(pi().platform),
        generation_line: None,
    };
    let bundle = assemble(
        &[genesis(&g).unwrap()],
        1,
        vec![(window, program.to_vec())],
        &[&release_key],
    );
    cairn_node::release_intake::take_in(&mut owner, &bundle, Some(g))
        .await
        .unwrap();
    let login = crate::enrol::tests::window_login(&owner, &cs).await;
    let mut state = AppState::mock(None);
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        cairn_node::db::connect(&login).await.unwrap(),
    )));
    Some((guard, state))
}

/// The bytes the staged artifact holds so far.
fn staged_len(dir: &Path) -> u64 {
    std::fs::metadata(dir.join("incoming").join(ARTIFACT)).map_or(0, |m| m.len())
}

/// The whole round: the offer door names release 1, and a fetch stages its signed meta (the
/// roots after the pin, the manifest, its signatures, no inclusion proof), the very bytes, and
/// `complete` naming the release.
#[tokio::test]
async fn a_fetch_stages_what_the_node_offers() {
    let program: Vec<u8> = (0..(CHUNK as usize + 99))
        .map(|i| (i % 251) as u8)
        .collect();
    let Some((_guard, state)) = holding(&program).await else {
        return;
    };
    let offers = offers(&state, &pi()).await.expect("the door answers");
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].byte_len, program.len() as u64);
    let dir = std::env::temp_dir().join(format!("cairn-courier-{}", uuid::Uuid::now_v7()));
    fetch(&state, offers[0].clone(), &dir, 0).await;
    let incoming = dir.join("incoming");
    assert_eq!(std::fs::read(incoming.join(ARTIFACT)).unwrap(), program);
    assert_eq!(staged_release(&dir), Some(1));
    let meta: cairn_release::ReleaseMeta =
        cairn_release::decode(&std::fs::read(incoming.join(META)).unwrap()).unwrap();
    assert_eq!(meta.root_chain.len(), 1, "the roots after pin 0");
    assert_eq!(meta.manifest, offers[0].manifest);
    assert_eq!(meta.release_signatures, offers[0].release_signatures);
    assert_eq!(meta.inclusion_proof, None);
    // Pinned at version 1 already: no roots to climb.
    fetch(&state, offers[0].clone(), &dir, 1).await;
    let meta: cairn_release::ReleaseMeta =
        cairn_release::decode(&std::fs::read(incoming.join(META)).unwrap()).unwrap();
    assert!(meta.root_chain.is_empty());
    // A target the node holds nothing for is offered nothing.
    let none = Target {
        platform: "debian-13".into(),
        ..pi()
    };
    assert_eq!(offers_len(&state, &none).await, 0);
}

async fn offers_len(state: &AppState, target: &Target) -> usize {
    offers(state, target).await.expect("the door answers").len()
}

/// The fetch rides the window's ONE connection, one chunk per lock (plan-time call): a press
/// made mid-fetch waits behind at most one chunk, the fetch waits while the press holds the
/// connection, and it resumes and finishes once the press lets go.
#[tokio::test]
async fn a_press_waits_at_most_one_chunk() {
    let program: Vec<u8> = (0..(CHUNK as usize * 6)).map(|i| (i % 253) as u8).collect();
    let Some((_guard, state)) = holding(&program).await else {
        return;
    };
    let offer = offers(&state, &pi()).await.unwrap().remove(0);
    let dir = std::env::temp_dir().join(format!("cairn-courier-{}", uuid::Uuid::now_v7()));
    let total = program.len() as u64;
    let press = async {
        // Mid-fetch: at least one chunk is down and the fetch is still going.
        while staged_len(&dir) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let before = staged_len(&dir);
        let conn = state.db.as_ref().unwrap().lock().await.unwrap();
        let at_press = staged_len(&dir);
        assert!(at_press - before <= CHUNK, "{before} -> {at_press}");
        assert!(
            at_press < total,
            "the press came only after the whole fetch"
        );
        // While the press holds the connection, the fetch takes no chunk.
        conn.query_one("SELECT 1", &[]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            staged_len(&dir),
            at_press,
            "a chunk was read under the press"
        );
    };
    tokio::join!(fetch(&state, offer, &dir, 0), press);
    assert_eq!(
        std::fs::read(dir.join("incoming").join(ARTIFACT)).unwrap(),
        program
    );
    assert_eq!(staged_release(&dir), Some(1));
}

/// Rulings R18/R19: fetches of one release stop after [`MAX_ATTEMPTS`] this session, failed ones
/// included. Here the node's file is shorter than its offer says, so every attempt fails, and
/// none leaves a `complete`.
#[tokio::test]
async fn a_download_that_keeps_failing_is_given_up() {
    let Some((_guard, state)) = holding(b"a short program").await else {
        return;
    };
    let mut offer = offers(&state, &pi()).await.unwrap().remove(0);
    offer.byte_len += 1;
    let dir = std::env::temp_dir().join(format!("cairn-courier-{}", uuid::Uuid::now_v7()));
    for attempt in 1..=MAX_ATTEMPTS {
        assert!(state.release.given_up().is_empty(), "attempt {attempt}");
        fetch(&state, offer.clone(), &dir, 0).await;
    }
    assert_eq!(state.release.given_up(), vec![1]);
    assert_eq!(staged_release(&dir), None);
}

/// Ruling R19: a fetch that SUCCEEDS counts too (the installer may refuse what it staged, and
/// clear it): after three, the release is given up.
#[tokio::test]
async fn a_successful_fetch_counts_as_an_attempt() {
    let Some((_guard, state)) = holding(b"a program").await else {
        return;
    };
    let offer = offers(&state, &pi()).await.unwrap().remove(0);
    let dir = std::env::temp_dir().join(format!("cairn-courier-{}", uuid::Uuid::now_v7()));
    for attempt in 1..=MAX_ATTEMPTS {
        assert!(state.release.given_up().is_empty(), "attempt {attempt}");
        fetch(&state, offer.clone(), &dir, 0).await;
        assert_eq!(staged_release(&dir), Some(1), "attempt {attempt} staged");
        // The installer refuses it and clears `incoming/`.
        std::fs::remove_dir_all(dir.join("incoming")).unwrap();
    }
    assert_eq!(state.release.given_up(), vec![1]);
}
