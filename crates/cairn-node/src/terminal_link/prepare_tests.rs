//! [`super::prepare`]: which link a connection string gets, and every refusal before any network.
//! None of these cases reaches the network: the only one that would (no pin, a fingerprint
//! typed) is driven against an in-test gateway in `tests/terminal_link.rs`.

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

/// A pin file in a fresh directory for a key made at runtime; returns the dir and the key's
/// short fingerprint.
fn pinned_dir() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let (_sk, kid) = cairn_event::generate_key().unwrap();
    TerminalFiles::at(dir.path()).write_pin(&kid).unwrap();
    (dir, cairn_event::short_fingerprint(&kid).unwrap())
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
