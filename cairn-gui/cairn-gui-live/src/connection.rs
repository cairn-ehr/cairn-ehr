//! The window's one node connection, reopened when it is lost (#807).
//!
//! The reference window opens ONE connection at launch and shares it between the chart
//! commands and the funnel (see [`crate::LiveData`]'s doc). Before #807 that connection was a
//! bare `Mutex<Client>`: after a Postgres restart, or any dropped connection, every later read
//! and write failed until the window itself was restarted.
//!
//! # How it works
//!
//! Every command takes the connection through [`NodeConnection::lock`], which returns
//! `Err(`[`ConnectionLost`]`)` when a closed connection cannot be reopened (below); gesture
//! timing alone uses [`NodeConnection::lock_if_open`]. `lock` checks whether the client has
//! CLOSED: its connection task ended because the server hung up (a restart's FATAL) or the
//! socket was reset. Only then does it open a fresh connection and put it in the old one's
//! place, so every holder of the shared `Arc` gets the new one.
//!
//! - **Never on a refusal.** A floor refusal, a constraint, a syntax error: each is an ANSWER on
//!   a live connection, which stays open, so `lock` never reopens for one.
//! - **Never mid-command.** The check runs only when the lock is taken, before the command does
//!   anything; no transaction can be open on a client nobody holds. A write whose commit got no
//!   answer is NOT retried: the next command once the node answers again gets a working
//!   connection, so the re-read that follows it can say what landed.
//! - **Backoff.** A failed reopen is not tried again for [`NodeConnection::RETRY_AFTER`], so a
//!   node that is still down costs one attempt per press, not one per lock a gesture takes. An
//!   attempt that gets no answer gives up after [`NodeConnection::REOPEN_WITHIN`], so an
//!   unreachable host cannot hold the lock for a TCP timeout.
//! - **If reopening fails**, `lock` returns [`ConnectionLost`] carrying the CAUSE (a refused
//!   socket, a rejected password), so the screen says why, not merely "connection closed"
//!   (#876). A press inside the backoff tries nothing, and its error says when the last attempt
//!   failed, so it does not read like a fresh failure. Callers classify it through `From` into a
//!   `DataError`: an outage they may retry (`Unavailable`), unless the server ANSWERED and
//!   refused the reopen (a rejected password, a dropped database), which only an operator can
//!   mend (`NotProvisioned`, #880).
//! - **Gesture timing uses [`NodeConnection::lock_if_open`]**, which never reopens: the write
//!   that follows an act must not spend the reopen attempt (and start the backoff) the act's own
//!   re-read needs.
//!
//! A socket that dies SILENTLY (a pulled cable, a powered-off remote host) sends nothing, so the
//! client closes only once `db::connect`'s TCP keepalives give up on it, about a minute idle
//! (`cairn_node::db_keepalive`). A query sent on it before then hangs, holding the lock: on Linux
//! for at most `tcp_user_timeout` (30 s), elsewhere until the OS gives up (#875, still open).
//!
//! The reopen uses `cairn_node::db::connect`, not `connect_and_load_schema`: the schema was loaded
//! at launch, and replaying every `db/*.sql` file takes locks a read never needs (#726). It also
//! skips the deferred-event re-adjudication `connect_and_load_schema` runs; a window restart, or
//! any other node process connecting, still does that.
use cairn_gui_data::port::DataError;
use std::time::{Duration, Instant};
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};
use tokio_postgres::Client;

/// The connection was lost and could not be reopened, either by this press or, inside the
/// backoff, by the press that last tried; the text says why (#876). The text is a lowercase
/// clause ("connection to the node was lost…"): the window's `ErrorView` puts it after
/// "Nothing was done: the ".
#[derive(Debug, Clone)]
pub struct ConnectionLost {
    text: String,
    /// The server answered the reopen and refused it ([`server_refused`], #880).
    refused: bool,
}

impl std::fmt::Display for ConnectionLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for ConnectionLost {}

/// The ONE place a lost connection is classified (`data_error_from` defers to it). An outage,
/// retried by pressing again: the node may be back by then. A reopen the server REFUSED fails
/// every press the same way until an operator acts, so it is `NotProvisioned`: a retry-now
/// would hide the remedy (#880).
impl From<ConnectionLost> for DataError {
    fn from(lost: ConnectionLost) -> Self {
        if lost.refused {
            DataError::NotProvisioned(lost.text)
        } else {
            DataError::Unavailable(lost.text)
        }
    }
}

/// One node connection, shared through an `Arc` and taken with [`NodeConnection::lock`].
pub struct NodeConnection {
    held: Mutex<Held>,
    /// Where to reopen from. `None` for [`NodeConnection::fixed`]: that one is never replaced.
    reopen_from: Option<String>,
}

/// What the lock guards: the client, and when and why the last reopen of it failed. Kept
/// under ONE tokio mutex, so the failure stamp needs no lock of its own.
struct Held {
    client: Client,
    last_failed: Option<Failed>,
}

/// The last failed reopen. `refused` is stored WITH the cause, so a press inside the backoff
/// classifies the same way as the press that tried (#880).
struct Failed {
    at: Instant,
    cause: String,
    refused: bool,
}

impl NodeConnection {
    /// After a failed reopen, how long before the next attempt.
    pub const RETRY_AFTER: Duration = Duration::from_secs(2);

    /// How long one reopen attempt may take before it counts as failed.
    pub const REOPEN_WITHIN: Duration = Duration::from_secs(5);

    /// A connection that is reopened, from `conn`, when it is lost. The window's.
    pub fn reopening(client: Client, conn: String) -> Self {
        Self::new(client, Some(conn))
    }

    /// A connection that is never replaced. For a caller that made the client itself and may
    /// have set session state on it (a `SET`) that a fresh connection would silently lack.
    pub fn fixed(client: Client) -> Self {
        Self::new(client, None)
    }

    fn new(client: Client, reopen_from: Option<String>) -> Self {
        Self {
            held: Mutex::new(Held {
                client,
                last_failed: None,
            }),
            reopen_from,
        }
    }

    /// Take the connection, reopening it first if it has closed (see the module doc). Returns
    /// `Err` when the reopen failed, or was skipped because the last attempt failed within
    /// [`RETRY_AFTER`](Self::RETRY_AFTER); the closed client stays, so a later press tries
    /// again. A [`fixed`](Self::fixed) connection that closed is handed back closed, never an
    /// error.
    pub async fn lock(&self) -> Result<MappedMutexGuard<'_, Client>, ConnectionLost> {
        let mut held = self.held.lock().await;
        if held.client.is_closed() {
            if let Some(conn) = &self.reopen_from {
                held.reopen(conn).await?;
            }
        }
        Ok(MutexGuard::map(held, |held| &mut held.client))
    }

    /// Take the connection only if it is open, never reopening it (see the module doc).
    pub async fn lock_if_open(&self) -> Option<MappedMutexGuard<'_, Client>> {
        let held = self.held.lock().await;
        (!held.client.is_closed()).then(|| MutexGuard::map(held, |held| &mut held.client))
    }
}

impl Held {
    /// Put a fresh connection in the closed client's place, unless an attempt failed too
    /// recently. On failure the closed client stays and the cause is returned. An attempt is
    /// logged either way (the loss itself by `db::connect`'s connection task); a press the
    /// backoff skips is not, its error goes to the caller only.
    async fn reopen(&mut self, conn: &str) -> Result<(), ConnectionLost> {
        let now = Instant::now();
        if let Some(last) = &self.last_failed {
            if !may_retry(Some(last.at), now) {
                let ago = now.saturating_duration_since(last.at);
                return Err(ConnectionLost {
                    text: lost_text(&last.cause, last.refused, Some(ago)),
                    refused: last.refused,
                });
            }
        }
        let attempt = cairn_node::db::connect(conn);
        let (cause, refused) =
            match tokio::time::timeout(NodeConnection::REOPEN_WITHIN, attempt).await {
                Ok(Ok(fresh)) => {
                    self.client = fresh;
                    self.last_failed = None;
                    eprintln!("the database connection was lost; reopened it");
                    return Ok(());
                }
                Ok(Err(e)) => {
                    // `db::connect`'s own error keeps the server's code beside its text (#880).
                    let code = e
                        .downcast_ref::<cairn_node::db::ConnectFailed>()
                        .and_then(cairn_node::db::ConnectFailed::sqlstate);
                    (format!("{e:#}"), server_refused(code))
                }
                Err(_) => (
                    format!("no answer within {:?}", NodeConnection::REOPEN_WITHIN),
                    false,
                ),
            };
        let text = lost_text(&cause, refused, None);
        eprintln!("the database {text}");
        self.last_failed = Some(Failed {
            at: Instant::now(),
            cause,
            refused,
        });
        Err(ConnectionLost { text, refused })
    }
}

/// Did the server ANSWER a connect and refuse it, so that every retry fails the same way until
/// an operator acts (#880)? Class `28` (a rejected password, no `pg_hba.conf` entry), `3D000`
/// (the database is gone) and `42501` (no CONNECT privilege). Everything else, no answer at all
/// included, is an outage the next press may get past: `53300` (too many connections) and
/// `57P03` (starting up) are the server saying "not now", not "never". Pure.
fn server_refused(sqlstate: Option<&str>) -> bool {
    sqlstate.is_some_and(|code| code.starts_with("28") || code == "3D000" || code == "42501")
}

/// What a failed reopen says (#876). `refused` is [`server_refused`]'s answer: the text then
/// names the operator instead of the next press. `skipped` is how long ago the last attempt
/// failed, when this press was inside the backoff and tried nothing. Nothing retries on a
/// timer, so the text names the press that will. Pure.
fn lost_text(cause: &str, refused: bool, skipped: Option<Duration>) -> String {
    let head = match skipped {
        None => "connection to the node was lost and could not be reopened".to_string(),
        Some(ago) => {
            // In whole tenths, the age rounded down: the two figures add up to the backoff,
            // and the wait inside it is never "0.0 s".
            let ago = ago.as_millis() / 100;
            let wait = (NodeConnection::RETRY_AFTER.as_millis() / 100).saturating_sub(ago);
            let tenths = |t: u128| format!("{}.{}", t / 10, t % 10);
            let next = if refused {
                String::new()
            } else {
                format!(" (press again in {} s to retry)", tenths(wait))
            };
            format!(
                "connection to the node was lost; reopening it failed {} s ago, so this press \
                 tried nothing{next}",
                tenths(ago),
            )
        }
    };
    if refused {
        format!(
            "{head}: {cause}. The node answered and refused the connection, so pressing again \
             fails the same way until an operator acts."
        )
    } else {
        format!("{head}: {cause}")
    }
}

/// May a reopen be attempted `now`, given when the last attempt failed? Pure.
fn may_retry(last_failed: Option<Instant>, now: Instant) -> bool {
    last_failed.is_none_or(|t| now.saturating_duration_since(t) >= NodeConnection::RETRY_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #876: the cause reaches the screen, and a press the backoff skipped says so instead of
    /// reading like a fresh failure.
    #[test]
    fn a_failed_reopen_names_its_cause_and_whether_this_press_tried() {
        let cause = "connecting to the database: connection refused (os error 61)";
        let now = lost_text(cause, false, None);
        assert!(now.contains("could not be reopened"), "{now}");
        assert!(now.ends_with(cause), "the cause, verbatim: {now}");

        // Nothing retries on a timer: the text names the PRESS that will try, never "is tried".
        let skipped = lost_text(cause, false, Some(Duration::from_millis(500)));
        assert!(skipped.contains("failed 0.5 s ago"), "{skipped}");
        assert!(skipped.contains("press again in 1.5 s"), "{skipped}");
        assert!(skipped.ends_with(cause), "{skipped}");

        // Near the end of the backoff the two figures still add up, and the wait is never 0.0.
        let late = lost_text(cause, false, Some(Duration::from_millis(1960)));
        assert!(late.contains("failed 1.9 s ago"), "{late}");
        assert!(late.contains("press again in 0.1 s"), "{late}");
    }

    /// A lost connection the server did not refuse is an outage: `Unavailable`, which the
    /// window offers to retry, with the cause as its whole text.
    #[test]
    fn a_lost_connection_is_an_outage_that_keeps_its_cause() {
        let lost = ConnectionLost {
            text: "the cause".into(),
            refused: false,
        };
        assert!(matches!(DataError::from(lost), DataError::Unavailable(t) if t == "the cause"));
    }

    /// #880: a server that ANSWERED and refused (credentials, `pg_hba.conf`, a dropped database,
    /// no CONNECT privilege) refuses every press the same way; only an operator mends it. A
    /// server that is busy or starting up may take the next press, so those stay outages.
    #[test]
    fn only_a_refusing_server_needs_an_operator() {
        for code in ["28P01", "28000", "3D000", "42501"] {
            assert!(server_refused(Some(code)), "{code}");
        }
        for code in [
            None,
            Some("53300"),
            Some("57P03"),
            Some("08006"),
            Some("XX000"),
        ] {
            assert!(!server_refused(code), "{code:?}");
        }
    }

    /// #880: a refused reopen tells the clerk to get an operator, never to press again, and so
    /// does a press inside the backoff after one. Both routes a window classifies by (`From`, and
    /// `data_error_from` for cease and release, which wrap it in `anyhow`) say operator.
    #[test]
    fn a_refused_reopen_names_the_operator_by_every_route() {
        let cause = "connecting to the database: password authentication failed [28P01]";
        for skipped in [None, Some(Duration::from_millis(500))] {
            let text = lost_text(cause, true, skipped);
            assert!(text.contains(cause), "{text}");
            assert!(text.contains("until an operator acts"), "{text}");
            assert!(!text.contains("press again in"), "{text}");
        }
        let lost = ConnectionLost {
            text: "t".into(),
            refused: true,
        };
        assert!(matches!(DataError::from(lost.clone()), DataError::NotProvisioned(t) if t == "t"));
        let wrapped = anyhow::Error::from(lost);
        assert!(matches!(
            crate::error::data_error_from(&wrapped),
            DataError::NotProvisioned(t) if t == "t"
        ));
        let outage = anyhow::Error::from(ConnectionLost {
            text: "t".into(),
            refused: false,
        });
        assert!(matches!(
            crate::error::data_error_from(&outage),
            DataError::Unavailable(_)
        ));
    }

    #[test]
    fn a_reopen_waits_out_the_backoff_after_a_failure() {
        let retry_after = NodeConnection::RETRY_AFTER;
        let failed = Instant::now();
        assert!(may_retry(None, failed), "no failure yet: try at once");
        assert!(!may_retry(Some(failed), failed), "just failed: wait");
        assert!(!may_retry(
            Some(failed),
            failed + retry_after - Duration::from_millis(1)
        ));
        assert!(may_retry(Some(failed), failed + retry_after));
    }

    /// Every Tauri command holds the guard across `.await`s, so its future must be `Send`.
    /// Pinned here, where the guard is made; otherwise only cairn-gui-tauri's build would
    /// catch a change (a `std` mutex, say) that takes `Send` away.
    #[allow(dead_code)]
    fn the_guard_may_be_held_across_an_await(db: &NodeConnection) -> impl Send + '_ {
        async move {
            let client = db.lock().await;
            std::future::ready(()).await;
            drop(client);
        }
    }
}
