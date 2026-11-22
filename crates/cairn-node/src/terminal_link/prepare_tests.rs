//! [`super::prepare`]: which link a connection string gets, and every refusal before any network.
//! Only the first-launch arm (no pin, a fingerprint typed) dials; it is driven here through
//! [`super::pinned`] with a loopback string and an in-test gateway, because `prepare` itself would
//! read a loopback string as local.

use super::*;
use files::TerminalFiles;

/// A password that must never appear in a refusal.
const SECRET_WORD: &str = "do-not-echo-me";

fn remote() -> String {
    format!("host=10.1.2.3 port=6432 user=u dbname=d password={SECRET_WORD}")
}

fn local() -> String {
    format!("host=localhost user=u dbname=d password={SECRET_WORD}")
}

/// A refusal's text, checked never to carry the connection string's password.
async fn refusal(conn: &str, dir: Option<&Path>, fp: Option<&str>) -> String {
    let err = match prepare(conn, dir, fp).await {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => format!("{e:#}"),
    };
    assert!(
        !err.contains(SECRET_WORD),
        "a refusal echoed the password: {err}"
    );
    err
}

/// A terminal directory as the first launch leaves it: the terminal's key, then a pin for a node
/// key made at runtime. Returns the dir and the node key's short fingerprint.
fn pinned_dir() -> (tempfile::TempDir, String) {
    let (dir, kid) = pin_without_key();
    TerminalFiles::at(dir.path()).load_or_create_key().unwrap();
    (dir, cairn_event::short_fingerprint(&kid).unwrap())
}

/// A pin with no `terminal.key` beside it: what a restore that skipped the key leaves. Returns
/// the dir and the pinned node key.
fn pin_without_key() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let (_sk, kid) = cairn_event::generate_key().unwrap();
    TerminalFiles::at(dir.path()).write_pin(&kid).unwrap();
    (dir, kid)
}

/// A pin is only ever written after the key it was made with (`pinned`'s first-launch arm), so a
/// pin with no key means the key was LOST. Minting a new one would make a second identity while
/// the old, possibly copied, key stays admitted; the remedy is the operator's (review SF1).
#[tokio::test]
async fn a_pin_whose_key_is_gone_is_refused_and_no_new_key_is_made() {
    let (dir, kid) = pin_without_key();
    let fp = cairn_event::short_fingerprint(&kid).unwrap();
    for typed in [None, Some(fp.as_str())] {
        let e = refusal(&remote(), Some(dir.path()), typed).await;
        assert!(e.contains("terminal.key"), "names the missing file: {e}");
        assert!(e.contains("revoke"), "names the operator's remedy: {e}");
        assert!(
            !dir.path().join("terminal.key").exists(),
            "no new terminal identity was minted ({typed:?})"
        );
    }
}

#[tokio::test]
async fn a_local_host_is_the_local_link() {
    for conn in [
        local(),
        "host=127.0.0.1 user=u dbname=d".into(),
        "host=/tmp user=u dbname=d".into(),
    ] {
        assert!(matches!(
            prepare(&conn, None, None).await.unwrap(),
            Link::Local
        ));
    }
}

#[tokio::test]
async fn a_local_host_refuses_both_flags_as_meaningless() {
    let dir = tempfile::tempdir().unwrap();
    let e = refusal(&local(), Some(dir.path()), None).await;
    assert!(e.contains("--terminal-dir"), "{e}");
    let e = refusal(&local(), None, Some("ABCD-ABCD-ABCD-ABCD-ABCD")).await;
    assert!(e.contains("--node-fingerprint"), "{e}");
}

#[tokio::test]
async fn a_remote_host_without_a_terminal_dir_is_refused_as_before_and_names_the_flags() {
    let e = refusal(&remote(), None, None).await;
    assert!(
        e.contains("another machine (10.1.2.3)"),
        "require_local's words: {e}"
    );
    assert!(e.contains("--terminal-dir"), "{e}");
    assert!(e.contains("--node-fingerprint"), "{e}");
}

#[tokio::test]
async fn a_remote_host_with_no_pin_and_no_fingerprint_names_the_fingerprint_flag() {
    let dir = tempfile::tempdir().unwrap();
    let e = refusal(&remote(), Some(dir.path()), None).await;
    assert!(e.contains("--node-fingerprint"), "{e}");
}

#[tokio::test]
async fn a_pin_and_a_fingerprint_that_disagree_are_refused_with_the_remedy() {
    let (dir, _fp) = pinned_dir();
    let (_other, other_kid) = cairn_event::generate_key().unwrap();
    let other_fp = cairn_event::short_fingerprint(&other_kid).unwrap();
    let e = refusal(&remote(), Some(dir.path()), Some(&other_fp)).await;
    assert!(e.contains("node.pin"), "the remedy names the file: {e}");
    assert!(e.contains("operator"), "{e}");
}

#[tokio::test]
async fn a_pin_alone_or_with_its_own_fingerprint_is_the_pinned_link() {
    let (dir, fp) = pinned_dir();
    let link = prepare(&remote(), Some(dir.path()), None).await.unwrap();
    assert!(matches!(link, Link::Pinned(_)));
    // Typed lower-case and without dashes: the same fingerprint, normalised.
    let typed = fp.replace('-', "").to_ascii_lowercase();
    let link = prepare(&remote(), Some(dir.path()), Some(&typed))
        .await
        .unwrap();
    assert!(matches!(link, Link::Pinned(_)));
}

#[tokio::test]
async fn more_than_one_host_or_a_hostaddr_is_refused() {
    let (dir, _fp) = pinned_dir();
    for conn in [
        format!("host=10.1.2.3,10.1.2.4 user=u dbname=d password={SECRET_WORD}"),
        format!("host=node hostaddr=10.1.2.3 user=u dbname=d password={SECRET_WORD}"),
        format!("host=localhost hostaddr=10.1.2.3 user=u dbname=d password={SECRET_WORD}"),
    ] {
        refusal(&conn, Some(dir.path()), None).await;
    }
}

#[tokio::test]
async fn an_unparseable_string_is_refused_without_echoing_it() {
    refusal(
        &format!("host='10.1.2.3 password={SECRET_WORD}"),
        None,
        None,
    )
    .await;
}

/// A local string keeps every form it has today, a loopback `hostaddr` included: only a remote
/// one must name its single host plainly.
#[tokio::test]
async fn a_local_hostaddr_is_still_the_local_link() {
    let conn = "host=localhost hostaddr=127.0.0.1 user=u dbname=d";
    assert!(matches!(
        prepare(conn, None, None).await.unwrap(),
        Link::Local
    ));
}

#[test]
fn the_gateway_address_brackets_an_ipv6_host_and_defaults_the_port() {
    let addr = |conn: &str| gateway_addr(&conn.parse().unwrap()).unwrap();
    assert_eq!(addr("host=10.1.2.3 port=6432"), "10.1.2.3:6432");
    assert_eq!(addr("host=node.example"), "node.example:5432");
    assert_eq!(addr("host=fd00::1 port=6432"), "[fd00::1]:6432");
}

/// The window awaits these on its async runtime, whose tasks must be `Send`.
#[test]
fn the_link_futures_are_send() {
    fn is_send<F: Send>(_: F) {}
    is_send(prepare("", None, None));
    is_send(Link::Local.connect_with_task(""));
    let (terminal, _) = cairn_event::generate_key().unwrap();
    let (_, node_kid) = cairn_event::generate_key().unwrap();
    let pinned = Link::Pinned(PinnedTls::new(&terminal, &node_kid).unwrap());
    is_send(pinned.connect_with_task(""));
}

#[tokio::test]
async fn a_malformed_fingerprint_beside_a_pin_is_called_malformed() {
    let (dir, _fp) = pinned_dir();
    let e = refusal(&remote(), Some(dir.path()), Some("ABCD-1234")).await;
    assert!(e.contains("20 hex digits"), "{e}");
    assert!(!e.contains("already pinned"), "{e}");
}

/// A one-connection gateway on loopback with the node's real TLS config, refusing every
/// terminal (none is admitted yet). Returns `host=… port=…` for it.
async fn loopback_gateway(node: &cairn_event::SigningKey) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let trust: crate::transport::TrustStore = std::sync::Arc::new(|_: &str| false);
    let cfg = crate::transport::gateway_server_config(node, trust).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut first8 = [0u8; 8];
        tcp.read_exact(&mut first8).await.unwrap();
        tcp.write_all(b"S").await.unwrap();
        let accepted = tokio_rustls::TlsAcceptor::from(cfg)
            .accept(tcp)
            .into_fallible()
            .await;
        if let Err((_, mut tcp)) = accepted {
            let _ = tcp.shutdown().await;
            let mut rest = Vec::new();
            let drain = tcp.read_to_end(&mut rest);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), drain).await;
        }
    });
    format!("host=127.0.0.1 port={port} user=u dbname=d")
}

#[tokio::test]
async fn the_first_launch_writes_the_pin_only_after_the_node_is_recognised() {
    let (node, node_kid) = cairn_event::generate_key().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg: Config = loopback_gateway(&node).await.parse().unwrap();
    let typed = cairn_event::short_fingerprint(&node_kid).unwrap();
    let link = pinned(&cfg, dir.path(), Some(&typed)).await.unwrap();
    assert!(matches!(link, Link::Pinned(_)));
    let files = TerminalFiles::at(dir.path());
    assert_eq!(files.read_pin().unwrap(), Some(node_kid));
    assert!(dir.path().join("terminal.key").exists());
}

#[tokio::test]
async fn a_wrong_fingerprint_at_first_launch_writes_no_pin() {
    let (node, _) = cairn_event::generate_key().unwrap();
    let (_, other_kid) = cairn_event::generate_key().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg: Config = loopback_gateway(&node).await.parse().unwrap();
    let typed = cairn_event::short_fingerprint(&other_kid).unwrap();
    assert!(pinned(&cfg, dir.path(), Some(&typed)).await.is_err());
    assert_eq!(TerminalFiles::at(dir.path()).read_pin().unwrap(), None);
}
