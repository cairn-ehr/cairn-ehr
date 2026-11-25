//! #909 slice C2, Task 5 — db/086: the node's release store and the window's three read-only doors.
//!
//! WHY. A terminal fetches its steward-signed update THROUGH its node; the window (role
//! `cairn_window`) must be able to read the offer, the trust-root chain and the artifact bytes,
//! and nothing else. The tables are owner-only and never synced (a release is not clinical
//! content); the doors are the only way in. They hand over bytes, they never judge them: the
//! terminal's installer verifies against ITS OWN pin.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Digests are fixture
//! bytes derived at runtime (house rule 6).
mod common;

use cairn_node::db;
use common::{cs, owner_and_window};
use tokio_postgres::Client;

/// A deterministic 32-byte value, distinct per `variant` (stands in for a digest).
fn digest(variant: u8) -> Vec<u8> {
    (0..32u8)
        .map(|i| i.wrapping_mul(3).wrapping_add(variant))
        .collect()
}

struct Rig {
    _guard: Client,
    owner: Client,
    window: Client,
}

/// An empty release store. `None` = no database (the test says it skipped).
async fn rig() -> Option<Rig> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    owner
        .batch_execute(
            "TRUNCATE release_incident, release_artifact, release_bundle, release_root CASCADE",
        )
        .await
        .unwrap();
    Some(Rig {
        _guard: guard,
        owner,
        window,
    })
}

/// One bundle for `version` with the given artifacts `(digest variant, kind, platform, content)`.
async fn bundle(owner: &Client, version: i64, arts: &[(u8, &str, &str, &[u8])]) {
    owner
        .execute(
            "INSERT INTO release_bundle VALUES ($1, 'stable', $2, ARRAY[$3::bytea], $4, now())",
            &[
                &version,
                &vec![9u8, version as u8],
                &vec![1u8, 2],
                &vec![7u8],
            ],
        )
        .await
        .unwrap();
    for (i, (v, kind, platform, content)) in arts.iter().enumerate() {
        owner
            .execute(
                "INSERT INTO release_artifact VALUES ($1, $2, $3, $4, 'linux', 'aarch64', $5, \
                 'g85', $6, $7)",
                &[
                    &digest(*v),
                    &version,
                    &(i as i32),
                    kind,
                    platform,
                    &(content.len() as i64),
                    content,
                ],
            )
            .await
            .unwrap();
    }
}

async fn offers(window: &Client, platform: &str) -> Vec<(i64, i32, Vec<u8>, i64)> {
    window
        .query(
            "SELECT release_version, idx, digest, byte_len FROM cairn_release_offer('linux','aarch64',$1)",
            &[&platform],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
        .collect()
}

#[tokio::test]
async fn the_window_role_reads_offers_roots_and_chunks() {
    let Some(r) = rig().await else { return };
    bundle(&r.owner, 3, &[(1, "window", "debian-12", b"0123456789")]).await;
    bundle(&r.owner, 5, &[(2, "window", "debian-12", b"abcdefghij")]).await;
    r.owner
        .execute(
            "INSERT INTO release_root VALUES (1, 'stable', $1, ARRAY[$2::bytea], $3)",
            &[&vec![1u8], &vec![2u8], &vec![3u8]],
        )
        .await
        .unwrap();

    let o = offers(&r.window, "debian-12").await;
    assert_eq!(
        o.iter().map(|x| x.0).collect::<Vec<_>>(),
        vec![5, 3],
        "newest first"
    );
    assert_eq!(o[0].2, digest(2));
    assert_eq!(o[0].3, 10);

    let roots = r
        .window
        .query("SELECT version, payload FROM cairn_release_roots(0)", &[])
        .await
        .unwrap();
    assert_eq!(roots.len(), 1);

    let c: Vec<u8> = r
        .window
        .query_one("SELECT cairn_release_chunk($1, 2, 4)", &[&digest(2)])
        .await
        .unwrap()
        .get(0);
    assert_eq!(c, b"cdef", "offset is 0-based bytes");
    let past: Vec<u8> = r
        .window
        .query_one("SELECT cairn_release_chunk($1, 10, 4)", &[&digest(2)])
        .await
        .unwrap()
        .get(0);
    assert!(past.is_empty(), "past the end is empty");
    assert!(
        r.window
            .query_one("SELECT cairn_release_chunk($1, 0, 4)", &[&digest(99)])
            .await
            .is_err(),
        "an unknown digest is an error, not empty"
    );
}

#[tokio::test]
async fn the_window_role_cannot_read_or_write_the_tables() {
    let Some(r) = rig().await else { return };
    for t in [
        "release_root",
        "release_bundle",
        "release_artifact",
        "release_incident",
    ] {
        assert!(
            r.window
                .query(&format!("SELECT * FROM {t}"), &[])
                .await
                .is_err(),
            "{t} read"
        );
        assert!(
            r.window
                .batch_execute(&format!("DELETE FROM {t}"))
                .await
                .is_err(),
            "{t} write"
        );
    }
}

#[tokio::test]
async fn a_chunk_over_the_cap_is_refused() {
    let Some(r) = rig().await else { return };
    bundle(&r.owner, 1, &[(1, "window", "debian-12", b"x")]).await;
    assert!(r
        .window
        .query_one("SELECT cairn_release_chunk($1, 0, 262145)", &[&digest(1)])
        .await
        .is_err());
    assert!(r
        .window
        .query_one("SELECT cairn_release_chunk($1, 0, 262144)", &[&digest(1)])
        .await
        .is_ok());
    assert!(r
        .window
        .query_one("SELECT cairn_release_chunk($1, -1, 1)", &[&digest(1)])
        .await
        .is_err());
}

#[tokio::test]
async fn an_open_incident_offers_nothing() {
    let Some(r) = rig().await else { return };
    bundle(&r.owner, 1, &[(1, "window", "debian-12", b"x")]).await;
    r.owner
        .execute("INSERT INTO release_incident (channel_id, version, detected_at, detail) VALUES ('stable', 1, now(), 'fork')", &[])
        .await
        .unwrap();
    assert!(
        offers(&r.window, "debian-12").await.is_empty(),
        "frozen while unresolved"
    );
    r.owner
        .batch_execute("UPDATE release_incident SET resolved_at = now()")
        .await
        .unwrap();
    assert_eq!(
        offers(&r.window, "debian-12").await.len(),
        1,
        "offers resume once resolved"
    );
}

#[tokio::test]
async fn offers_skip_other_kinds_and_platforms() {
    let Some(r) = rig().await else { return };
    bundle(
        &r.owner,
        1,
        &[
            (1, "window", "debian-12", b"a"),
            (2, "docs", "debian-12", b"b"),
            (3, "window", "ubuntu-24", b"c"),
        ],
    )
    .await;
    let o = offers(&r.window, "debian-12").await;
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].2, digest(1));
}

#[tokio::test]
async fn roots_after_returns_only_later_versions() {
    let Some(r) = rig().await else { return };
    for v in [1i64, 2, 3] {
        r.owner
            .execute(
                "INSERT INTO release_root VALUES ($1, 'stable', $2, ARRAY[$2::bytea], $2)",
                &[&v, &vec![v as u8]],
            )
            .await
            .unwrap();
    }
    let got: Vec<i64> = r
        .window
        .query("SELECT version FROM cairn_release_roots(1)", &[])
        .await
        .unwrap()
        .iter()
        .map(|x| x.get(0))
        .collect();
    assert_eq!(got, vec![2, 3]);
}

#[tokio::test]
async fn a_chunk_of_a_non_window_artifact_is_refused() {
    let Some(r) = rig().await else { return };
    bundle(&r.owner, 1, &[(1, "docs", "debian-12", b"abc")]).await;
    assert!(
        r.window
            .query_one("SELECT cairn_release_chunk($1, 0, 3)", &[&digest(1)])
            .await
            .is_err(),
        "an unknown kind is stored but never served"
    );
}

#[tokio::test]
async fn a_chunk_during_an_open_incident_is_refused() {
    let Some(r) = rig().await else { return };
    bundle(&r.owner, 1, &[(1, "window", "debian-12", b"abc")]).await;
    r.owner
        .execute("INSERT INTO release_incident (channel_id, version, detail) VALUES ('stable', 1, 'fork')", &[])
        .await
        .unwrap();
    assert!(r
        .window
        .query_one("SELECT cairn_release_chunk($1, 0, 3)", &[&digest(1)])
        .await
        .is_err());
}

#[tokio::test]
async fn a_huge_offset_is_empty_and_bad_arguments_are_refused() {
    let Some(r) = rig().await else { return };
    bundle(&r.owner, 1, &[(1, "window", "debian-12", b"abc")]).await;
    let far: Vec<u8> = r
        .window
        .query_one(
            "SELECT cairn_release_chunk($1, 1099511627776, 4)",
            &[&digest(1)],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        far.is_empty(),
        "an offset of 2^40 is past the end, not an overflow"
    );
    for sql in [
        "SELECT cairn_release_chunk($1, 0, -1)",
        "SELECT cairn_release_chunk($1, 0, NULL)",
        "SELECT cairn_release_chunk($1, NULL, 1)",
    ] {
        assert!(
            r.window.query_one(sql, &[&digest(1)]).await.is_err(),
            "{sql}"
        );
    }
    // NULL "after" is read as 0: the whole chain (documented in db/086).
    r.owner
        .execute(
            "INSERT INTO release_root VALUES (1, 'stable', $1, ARRAY[$1::bytea], $1)",
            &[&vec![1u8]],
        )
        .await
        .unwrap();
    let n = r
        .window
        .query("SELECT 1 FROM cairn_release_roots(NULL)", &[])
        .await
        .unwrap()
        .len();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn public_cannot_execute_the_doors() {
    let Some(r) = rig().await else { return };
    for f in [
        "cairn_release_offer(text,text,text)",
        "cairn_release_roots(bigint)",
        "cairn_release_chunk(bytea,bigint,int)",
    ] {
        let has: bool = r
            .owner
            .query_one(
                "SELECT has_function_privilege('public', $1::text::regprocedure, 'EXECUTE')",
                &[&f],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!has, "PUBLIC can execute {f}");
    }
}
