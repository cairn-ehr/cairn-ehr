//! Notice a database connection whose peer vanished without a word (#875).
//!
//! A Postgres connection can end in two ways. A LOUD end (a server restart's FATAL, a reset
//! socket) closes the client at once, and the window's `NodeConnection` reopens it on the next
//! press (#807). A SILENT end (a pulled cable, a switch port that flaps, a remote host powered
//! off, a laptop waking from sleep) sends nothing at all, so the client still looks open. Left
//! at tokio-postgres's defaults, nothing probes an idle socket for **two hours**, and a query
//! sent on a dead one waits for the operating system to give up on its retransmits (about 15
//! minutes on Linux). The window holds its one connection lock that whole time, so every other
//! chart and funnel action queues behind the hang.
//!
//! [`detect_dead_peers`] shortens both waits for every caller of `db::connect` (the window, the
//! CLI, `cairn-sync`):
//!
//! - **TCP keepalives** probe a socket idle for [`KEEPALIVE_IDLE`], every [`KEEPALIVE_INTERVAL`],
//!   [`KEEPALIVE_RETRIES`] times. A silent death on an idle connection is noticed within about a
//!   minute on every platform. The client then reads as closed, and the window reopens it.
//! - **`tcp_user_timeout`** ([`TCP_USER_TIMEOUT`]) ends a connection whose sent data goes
//!   unacknowledged that long. It bounds a query sent on an already-dead socket. **Linux only**:
//!   tokio-postgres applies it nowhere else, so on macOS that query still waits for the OS (#875
//!   keeps the window-side bound open; it overlaps #649's cancellation question).
//!
//! Neither touches a live connection running a long query: a live server acknowledges both the
//! probes and the data. Neither applies to a Unix-domain socket, which cannot die silently.
//!
//! **The connection string wins.** An operator who sets any of these keys keeps that value, as
//! libpq users expect; `keepalives=0` still turns probing off.
use std::time::Duration;
use tokio_postgres::Config;

/// An idle connection is probed after this long without traffic.
pub const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
/// The gap between unanswered probes.
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// Unanswered probes before the connection is declared dead (30 s + 3 × 10 s ≈ one minute).
pub const KEEPALIVE_RETRIES: u32 = 3;
/// Sent data unacknowledged this long ends the connection (Linux only, see the module doc).
pub const TCP_USER_TIMEOUT: Duration = Duration::from_secs(30);

/// tokio-postgres's own keepalive idle, which a parsed string that names none still carries.
const LIBRARY_DEFAULT_IDLE: Duration = Duration::from_secs(2 * 60 * 60);

/// Fill in the dead-peer settings the connection string left unset. Pure apart from `cfg`.
pub fn detect_dead_peers(cfg: &mut Config) {
    // The idle getter has no "unset", only the library default. shortcut: an operator who
    // writes keepalives_idle=7200 explicitly gets ours instead; parse the string's keys
    // ourselves if that case ever matters.
    if cfg.get_keepalives_idle() == LIBRARY_DEFAULT_IDLE {
        cfg.keepalives_idle(KEEPALIVE_IDLE);
    }
    if cfg.get_keepalives_interval().is_none() {
        cfg.keepalives_interval(KEEPALIVE_INTERVAL);
    }
    if cfg.get_keepalives_retries().is_none() {
        cfg.keepalives_retries(KEEPALIVE_RETRIES);
    }
    if cfg.get_tcp_user_timeout().is_none() {
        cfg.tcp_user_timeout(TCP_USER_TIMEOUT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(conn: &str) -> Config {
        let mut cfg: Config = conn.parse().expect("a valid connection string");
        detect_dead_peers(&mut cfg);
        cfg
    }

    #[test]
    fn a_bare_connection_string_gets_dead_peer_detection() {
        let cfg = configured("host=127.0.0.1 dbname=cairn");
        assert!(cfg.get_keepalives());
        assert_eq!(cfg.get_keepalives_idle(), KEEPALIVE_IDLE);
        assert_eq!(cfg.get_keepalives_interval(), Some(KEEPALIVE_INTERVAL));
        assert_eq!(cfg.get_keepalives_retries(), Some(KEEPALIVE_RETRIES));
        assert_eq!(cfg.get_tcp_user_timeout(), Some(&TCP_USER_TIMEOUT));
    }

    #[test]
    fn a_value_the_connection_string_sets_is_kept() {
        let cfg = configured(
            "host=127.0.0.1 keepalives_idle=600 keepalives_interval=5 \
             keepalives_retries=9 tcp_user_timeout=90",
        );
        assert_eq!(cfg.get_keepalives_idle(), Duration::from_secs(600));
        assert_eq!(cfg.get_keepalives_interval(), Some(Duration::from_secs(5)));
        assert_eq!(cfg.get_keepalives_retries(), Some(9));
        assert_eq!(cfg.get_tcp_user_timeout(), Some(&Duration::from_secs(90)));
    }

    #[test]
    fn keepalives_turned_off_stay_off() {
        assert!(!configured("host=127.0.0.1 keepalives=0").get_keepalives());
    }

    /// The tests above pin the settings, not that anyone applies them: a client's socket
    /// options cannot be read back from outside. So pin the one call site by source.
    #[test]
    fn db_connect_applies_the_settings() {
        let src = include_str!("db.rs");
        let start = src
            .find("pub async fn connect(conn: &str)")
            .expect("db::connect exists");
        let body = &src[start..start + src[start..].find("\n}\n").expect("its body ends")];
        assert!(
            body.contains("detect_dead_peers(&mut cfg)"),
            "db::connect must apply the dead-peer settings (#875)"
        );
    }
}
