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
//!   failed, so it does not read like a fresh failure. A caller that decides a retry classifies
//!   it through `From` into a `DataError`: an outage they may retry (`Unavailable`), unless the
//!   reopen was REFUSED, which only an operator can mend (`NotProvisioned`): by the server (a
//!   rejected password, a dropped database, #880) or by the client library (a password the
//!   window was not given, #884). A caller that only shows the error shows its text, which
//!   names the remedy either way, and who refused ([`Refusal`]).
//! - **Gesture timing uses [`NodeConnection::lock_if_open`]**, which never reopens: the write
//!   that follows an act must not spend the reopen attempt (and start the backoff) the act's own
//!   re-read needs.
//!
//! A socket that dies SILENTLY (a pulled cable, a powered-off remote host) sends nothing, so the
//! client closes only once `db::connect`'s TCP keepalives give up on it, about a minute idle
//! (the `cairn-db-keepalive` crate). A query sent on it before then would hang until the OS gives
//! up on its retransmits (30 s on Linux, via `tcp_user_timeout`; minutes on macOS), holding the
//! lock, so:
//!
//! - **A hold is cut off after [`NodeConnection::ANSWER_WITHIN`]** (#875 option 2). Every guard
//!   `lock` or `lock_if_open` hands out starts a watchdog; dropping the guard cancels it. The
//!   whole HOLD is timed, not one statement. A guard still held at the limit has its client
//!   marked cut and its connection's TASK aborted, so the stuck query fails `connection closed`,
//!   as after a server hang-up, and the lock is released. A cut-off COMMIT is "not known"
//!   (`cairn_node::db_diagnosis::commit_failure` marks any commit the server did not answer with
//!   an ERROR), and the client is marked cut, so the next `lock` reopens it.
//! - **The reopen after a cut ends the old backend first.** The server never hears of the cut,
//!   so the cut statement may still be running, and a cut COMMIT may still land. A re-read that
//!   ran beside it could say "not recorded" for an act that lands a moment later, and invite a
//!   second entry (PR #892 review). So the reopen terminates that backend and waits until it has
//!   ended, and fails the press if it will not.
//! - **The limit cannot tell a dead link from a node alive but slower than 15 s**; that is why
//!   it sits far above the slowest command measured (a patient search on the Pi, about 0.9 s). A
//!   `fixed` connection has no watchdog: aborting it would kill it for good.
//! - **A cut-off command's error says so** (#893): the stuck query fails with the same bare
//!   `connection closed` as a server restart, so every call site whose error reaches the screen
//!   passes it through [`ConnectionGuard::explain`] while it still holds the guard (`cut_off.rs`;
//!   pinned for every lock by `cairn-gui-tauri`'s `node_lock_explains_tests.rs`).
//!
//! The reopen uses `cairn_node::db::connect_with_task`, not `connect_and_load_schema`: the schema
//! was checked (never loaded) at launch, the window's own login cannot replay it (ADR-0084), and
//! replaying every `db/*.sql` file takes locks a read never needs (#726). It also skips the
//! deferred-event re-adjudication `connect_and_load_schema` runs; since ADR-0084 so does the
//! window's launch, and only the one-off loader verbs still run it (#912).
use cairn_gui_data::port::DataError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};
use tokio::task::AbortHandle;
use tokio_postgres::Client;

mod cut_off;
pub use cut_off::{cause_of, name_cut_in, CutOff, CutWatch};

/// The connection was lost and could not be reopened, either by this press or, inside the
/// backoff, by the press that last tried; the text says why (#876). The text is a lowercase
/// clause ("connection to the node was lost…"): the window's `ErrorView` puts it after
/// "Nothing was done: the ".
#[derive(Debug, Clone)]
pub struct ConnectionLost {
    text: String,
    /// Who refused the reopen, if anyone did. Retrying a refusal cannot help until an operator
    /// acts; `None` is an outage.
    refused: Option<Refusal>,
}

/// Who refused a reopen that only an operator can mend. Kept apart because the text must name
/// the right one: an operator told "the node refused" looks for a rejected login in the
/// server's log, and a client refusal left none (#884 review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The server answered and refused (`server_refused`, #880).
    Server,
    /// The client library gave up on what the server asked for: a password the connection
    /// string lacks, an authentication method it cannot do, a server proof that did not check
    /// out (`cairn_node::db::ConnectFailed::refused_by_client`, #884).
    Client,
}

impl Refusal {
    /// What happened, as a sentence without its full stop. The ONE wording of each: the reopen's
    /// text and the front door's provisioning line both say it.
    pub fn clause(self) -> &'static str {
        match self {
            Refusal::Server => "The node answered and refused the connection",
            Refusal::Client => "This workstation could not complete the sign-in the node asked for",
        }
    }
}

impl std::fmt::Display for ConnectionLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for ConnectionLost {}

impl ConnectionLost {
    /// For OTHER crates' tests only (the window classifies one without a database to lose);
    /// `refused` stands for a server's refusal. Production builds a `ConnectionLost` in
    /// `Held::reopen` alone, through `lost`.
    #[doc(hidden)]
    pub fn for_tests(text: &str, refused: bool) -> Self {
        Self {
            text: text.into(),
            refused: refused.then_some(Refusal::Server),
        }
    }
}

/// The ONE place a lost connection is classified into a `DataError` (`data_error_from` and the
/// window's `ErrorView` conversion both defer to it). An outage,
/// retried by pressing again: the node may be back by then. A REFUSED reopen, by the server
/// (#880) or the client library (#884), fails every press the same way until an operator acts,
/// so it is `NotProvisioned`: a retry-now would hide the remedy.
impl From<ConnectionLost> for DataError {
    fn from(lost: ConnectionLost) -> Self {
        if lost.refused.is_some() {
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
    /// How long a guard may be held before its connection is cut off (#875).
    answer_within: Duration,
}

/// What the lock guards: the client, and when and why the last reopen of it failed. Kept
/// under ONE tokio mutex, so the failure stamp needs no lock of its own.
struct Held {
    client: Client,
    /// What the watchdog cuts this client off with. `None` for a `fixed` connection.
    cutter: Option<Cutter>,
    last_failed: Option<Failed>,
}

impl Held {
    /// Is this client gone: closed, or cut off by a watchdog? The flag answers at once, where
    /// `is_closed()` turns true only once the aborted task has actually been dropped, so a
    /// command taking the lock in between would otherwise run on a dying client (PR #892 review).
    fn is_lost(&self) -> bool {
        self.client.is_closed() || self.cutter.as_ref().is_some_and(Cutter::was_cut)
    }
}

/// One client's connection task, its server backend, and whether a watchdog cut it. Each
/// client gets its own (a reopen makes a fresh one), so a late watchdog of an old client cannot
/// mark a new one.
#[derive(Clone)]
struct Cutter {
    task: AbortHandle,
    backend: Backend,
    cut: Arc<AtomicBool>,
}

impl Cutter {
    fn new(task: AbortHandle, backend: Backend) -> Self {
        Self {
            task,
            backend,
            cut: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Mark the client lost, so the next `lock` reopens it rather than hand it out.
    fn mark_cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
    }

    /// Mark the client lost, THEN abort its task, so no lock can see it open in between. (The
    /// verb; `ConnectionGuard::cut_off` is the question "was it cut?".)
    fn cut(&self) {
        self.mark_cut();
        self.task.abort();
    }

    fn was_cut(&self) -> bool {
        self.cut.load(Ordering::SeqCst)
    }
}

/// Which server backend a client talks to. Aborting the client's task does not stop it: a
/// backend notices its client is gone only when it next writes to the socket, so a statement
/// that was cut off runs on, and a cut COMMIT may still land (PR #892 review). The reopen after
/// a cut therefore ends this backend before anything reads (`Backend::end_on`). The start time
/// is kept beside the pid because the server may give a later backend the same pid.
#[derive(Clone)]
struct Backend {
    pid: i32,
    started: String,
}

impl Backend {
    /// How long a reopen waits for a cut-off backend to end, inside its own
    /// [`NodeConnection::REOPEN_WITHIN`].
    const ENDED_WITHIN_MS: i64 = 3_000;

    /// The backend `client` talks to.
    async fn of(client: &Client) -> Result<Self, tokio_postgres::Error> {
        let row = client
            .query_one(
                "SELECT pid, backend_start::text FROM pg_stat_activity WHERE pid = pg_backend_pid()",
                &[],
            )
            .await?;
        Ok(Self {
            pid: row.get(0),
            started: row.get(1),
        })
    }

    /// End this backend from `client`, a fresh connection to the same node, and wait until it
    /// has ended: whatever it was doing is then decided (committed or rolled back), so the
    /// re-read that follows a cut-off act says what landed. One already gone is fine. An error
    /// if it is still running after `ENDED_WITHIN_MS`, so the press fails rather than read a
    /// state its own earlier act may still change.
    async fn end_on(&self, client: &Client) -> anyhow::Result<()> {
        let ended: bool = client
            .query_one(
                "SELECT CASE WHEN EXISTS (SELECT 1 FROM pg_stat_activity
                                            WHERE pid = $1 AND backend_start::text = $2)
                             THEN pg_terminate_backend($1, $3) ELSE true END",
                &[&self.pid, &self.started, &Self::ENDED_WITHIN_MS],
            )
            .await?
            .get(0);
        anyhow::ensure!(
            ended,
            "the command cut off on the last connection is still running on the node, so what \
             it saved is not known yet"
        );
        Ok(())
    }
}

/// The last failed reopen. `refused` is stored WITH the cause, so a press inside the backoff
/// classifies the same way as the press that tried (#880).
struct Failed {
    at: Instant,
    cause: String,
    refused: Option<Refusal>,
}

impl NodeConnection {
    /// After a failed reopen, how long before the next attempt.
    pub const RETRY_AFTER: Duration = Duration::from_secs(2);

    /// How long one reopen attempt may take before it counts as failed.
    pub const REOPEN_WITHIN: Duration = Duration::from_secs(5);

    /// How long a command may hold the connection before it is cut off (#875 option 2): the
    /// maintainer's ruling, "any longer is unrealistic to expect ongoing engagement". The
    /// window's waiting bar (`waiting.js`'s `NODE_ANSWERS_WITHIN_MS`) is pinned to it.
    pub const ANSWER_WITHIN: Duration = Duration::from_secs(15);

    /// A connection that is reopened, from `conn`, when it is lost, and cut off when a command
    /// holds it past [`ANSWER_WITHIN`](Self::ANSWER_WITHIN). The window's. `task` is the
    /// client's connection task (`cairn_node::db::connect_with_task`). Asks the server which
    /// backend `client` is, so a reopen after a cut can end it (`Backend`).
    pub async fn reopening(
        client: Client,
        task: AbortHandle,
        conn: String,
    ) -> Result<Self, tokio_postgres::Error> {
        let backend = Backend::of(&client).await?;
        Ok(Self::new(
            client,
            Some(Cutter::new(task, backend)),
            Some(conn),
        ))
    }

    /// A connection that is never replaced, nor cut off. For a caller that made the client
    /// itself and may have set session state on it (a `SET`) that a fresh connection would
    /// silently lack.
    pub fn fixed(client: Client) -> Self {
        Self::new(client, None, None)
    }

    /// The same connection with a different hold limit. For tests, which cannot wait 15 s.
    pub fn answer_within(mut self, limit: Duration) -> Self {
        self.answer_within = limit;
        self
    }

    fn new(client: Client, cutter: Option<Cutter>, reopen_from: Option<String>) -> Self {
        Self {
            held: Mutex::new(Held {
                client,
                cutter,
                last_failed: None,
            }),
            reopen_from,
            answer_within: Self::ANSWER_WITHIN,
        }
    }

    /// Take the connection, reopening it first if it has closed (see the module doc). Returns
    /// `Err` when the reopen failed, or was skipped because the last attempt failed within
    /// [`RETRY_AFTER`](Self::RETRY_AFTER); the closed client stays, so a later press tries
    /// again. A [`fixed`](Self::fixed) connection that closed is handed back closed, never an
    /// error.
    pub async fn lock(&self) -> Result<ConnectionGuard<'_>, ConnectionLost> {
        let mut held = self.held.lock().await;
        if held.is_lost() {
            if let Some(conn) = &self.reopen_from {
                held.reopen(conn).await?;
            }
        }
        Ok(self.watched(held))
    }

    /// Take the connection only if it is open, never reopening it (see the module doc).
    pub async fn lock_if_open(&self) -> Option<ConnectionGuard<'_>> {
        let held = self.held.lock().await;
        (!held.is_lost()).then(|| self.watched(held))
    }

    /// Hand the held client out, starting the watchdog that cuts it off if the guard is still
    /// held after `answer_within` (the module doc). The timer starts AFTER any reopen, so the
    /// reopen's own 5 s cap is not counted against the command.
    fn watched<'a>(&self, held: MutexGuard<'a, Held>) -> ConnectionGuard<'a> {
        let watch = held
            .cutter
            .clone()
            .map(|cutter| Watch::start(cutter, self.answer_within));
        ConnectionGuard {
            client: MutexGuard::map(held, |held| &mut held.client),
            watch,
        }
    }
}

/// One hold's watchdog. The hold ends once, by whichever comes first: the watchdog at the limit,
/// or the guard's drop. Each claims `claimed` first, so the two can never both act, and neither
/// can leave a client cut without the mark (PR #892 review).
struct Watch {
    watchdog: AbortHandle,
    claimed: Arc<AtomicBool>,
    cutter: Cutter,
    /// The limit this hold runs under, for the text that says it was cut off (#893).
    limit: Duration,
}

impl Watch {
    fn start(cutter: Cutter, limit: Duration) -> Self {
        let claimed = Arc::new(AtomicBool::new(false));
        let watchdog = tokio::spawn({
            let (claimed, cutter) = (claimed.clone(), cutter.clone());
            async move {
                tokio::time::sleep(limit).await;
                if !claimed.swap(true, Ordering::SeqCst) {
                    cutter.cut();
                    // The whole hold is timed, not one statement: a command of many quick
                    // statements is cut too, so the line does not say the node never answered.
                    eprintln!(
                        "a command held the database connection past {limit:?}; cut it off \
                         (a commit in flight is not known)"
                    );
                }
            }
        })
        .abort_handle();
        Self {
            watchdog,
            claimed,
            cutter,
            limit,
        }
    }

    /// The guard is being dropped. If the watchdog claimed the hold first, it is cutting the
    /// client off, maybe on another thread at this moment: mark the client lost HERE, while the
    /// guard still holds the lock, or the next `lock` could take the dying client.
    fn end(&self) {
        self.watchdog.abort();
        if self.claimed.swap(true, Ordering::SeqCst) {
            self.cutter.mark_cut();
        }
    }
}

/// The window's connection while a command holds it: a `Client` (it derefs to one), watched.
/// Dropping it gives the connection back and cancels the watchdog. A command that finishes at
/// the very instant the watchdog fires loses its connection anyway, but the next `lock` sees it
/// lost and reopens (`Watch::end`).
pub struct ConnectionGuard<'a> {
    client: MappedMutexGuard<'a, Client>,
    watch: Option<Watch>,
}

impl std::ops::Deref for ConnectionGuard<'_> {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.client
    }
}

impl std::ops::DerefMut for ConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Client {
        &mut self.client
    }
}

impl Drop for ConnectionGuard<'_> {
    fn drop(&mut self) {
        // Runs before the fields drop, so before the lock is released.
        if let Some(watch) = &self.watch {
            watch.end();
        }
    }
}

impl Held {
    /// Put a fresh connection in the closed client's place, unless an attempt failed too
    /// recently. On failure the closed client stays and the cause is returned. An attempt is
    /// logged either way (the loss itself by the connection task, or by the watchdog for a cut:
    /// an aborted task logs nothing); a press the backoff skips is not, its error goes to the
    /// caller only.
    async fn reopen(&mut self, conn: &str) -> Result<(), ConnectionLost> {
        let now = Instant::now();
        if let Some(last) = &self.last_failed {
            if !may_retry(Some(last.at), now) {
                let ago = now.saturating_duration_since(last.at);
                return Err(lost(&last.cause, last.refused, Some(ago)));
            }
        }
        let cut_off = self.cutter.as_ref().filter(|c| c.was_cut());
        let attempt = open_after(conn, cut_off.map(|c| &c.backend));
        let (cause, refused) =
            match tokio::time::timeout(NodeConnection::REOPEN_WITHIN, attempt).await {
                Ok(Ok((fresh, cutter))) => {
                    self.client = fresh;
                    self.cutter = Some(cutter);
                    self.last_failed = None;
                    eprintln!("the database connection was lost; reopened it");
                    return Ok(());
                }
                Ok(Err(e)) => {
                    // `db::connect`'s own error keeps the server's code beside its text (#880),
                    // and marks a refusal the client library raised itself, which has none (#884).
                    let refused = e
                        .downcast_ref::<cairn_node::db::ConnectFailed>()
                        .and_then(|f| {
                            if server_refused(f.sqlstate()) {
                                Some(Refusal::Server)
                            } else {
                                f.refused_by_client().then_some(Refusal::Client)
                            }
                        });
                    (format!("{e:#}"), refused)
                }
                Err(_) => (
                    format!("no answer within {:?}", NodeConnection::REOPEN_WITHIN),
                    None,
                ),
            };
        let failure = lost(&cause, refused, None);
        eprintln!("the database {failure}");
        self.last_failed = Some(Failed {
            at: Instant::now(),
            cause,
            refused,
        });
        Err(failure)
    }
}

/// A fresh connection on `conn`, with its cutter. When the client it replaces was CUT OFF, its
/// backend is ended first (`Backend::end_on`), so the first read on the fresh one is final.
/// Inside the reopen's timeout: a fresh client given up on is dropped, which closes it.
async fn open_after(conn: &str, cut_off: Option<&Backend>) -> anyhow::Result<(Client, Cutter)> {
    let (client, task) = cairn_node::db::connect_with_task(conn).await?;
    if let Some(old) = cut_off {
        old.end_on(&client).await?;
    }
    let backend = Backend::of(&client).await?;
    Ok((client, Cutter::new(task, backend)))
}

/// A failed reopen, its text derived from the same `refused` it carries, so the words and the
/// class can never disagree. The only way production builds a [`ConnectionLost`]. Pure.
fn lost(cause: &str, refused: Option<Refusal>, skipped: Option<Duration>) -> ConnectionLost {
    ConnectionLost {
        text: lost_text(cause, refused, skipped),
        refused,
    }
}

/// Did the server ANSWER a connect and refuse it, so that every retry fails the same way until
/// an operator acts (#880)? Class `28` (a rejected password, no `pg_hba.conf` entry), `3D000`
/// (the database is gone), `42501` (no CONNECT privilege) and `55000` (the database does not
/// allow connections). Everything else, no answer at all included, is an outage the next press
/// may get past: `53300` (too many connections) and `57P03` (starting up) are the server saying
/// "not now", not "never". Pure.
///
/// A refusal the CLIENT library raises carries no code (a password the server asks for but the
/// connection string lacks, a SCRAM or SASL failure); `ConnectFailed::refused_by_client` marks
/// those, beside this, at the one call site, as [`Refusal::Client`] (#884).
///
/// Known limits, each erring towards "press again":
/// - Against several hosts, or a name resolving to several addresses, only the LAST attempt's
///   error comes back, so an earlier host's refusal can hide behind a later one's outage.
/// - `CONNECTION LIMIT 0` answers `53300`, the same as a full server.
/// - Erring the other way: class `28` from an LDAP, RADIUS or PAM backend that is itself
///   unreachable clears when it returns, without an operator.
fn server_refused(sqlstate: Option<&str>) -> bool {
    sqlstate
        .is_some_and(|code| code.starts_with("28") || ["3D000", "42501", "55000"].contains(&code))
}

/// What a failed reopen says (#876). `refused` says who refused, if anyone did: the text then
/// says which ([`Refusal::clause`]) and names the operator instead of the next press. `skipped`
/// is how long ago the last attempt failed, when this press was inside the backoff and tried
/// nothing. Nothing retries on a timer, so an outage's text names the press that will. Pure.
fn lost_text(cause: &str, refused: Option<Refusal>, skipped: Option<Duration>) -> String {
    let head = match skipped {
        None => format!("{LOST_HEAD} and could not be reopened"),
        Some(ago) => {
            // In whole tenths, the age rounded down: the two figures add up to the backoff,
            // and the wait inside it is never "0.0 s".
            let ago = ago.as_millis() / 100;
            let wait = (NodeConnection::RETRY_AFTER.as_millis() / 100).saturating_sub(ago);
            let tenths = |t: u128| format!("{}.{}", t / 10, t % 10);
            let next = match refused {
                Some(_) => String::new(),
                None => format!(" (press again in {} s to retry)", tenths(wait)),
            };
            format!(
                "{LOST_HEAD}; reopening it failed {} s ago, so this press tried nothing{next}",
                tenths(ago),
            )
        }
    };
    match refused {
        Some(r) => format!(
            "{head}: {cause}. {}{REMEDY_JOIN}{OPERATOR_REMEDY}",
            r.clause()
        ),
        None => format!("{head}: {cause}"),
    }
}

/// How a refused reopen's text ends. It must say its own remedy, because several panes show the
/// text with no renderer of their own; a renderer that DOES wrap it asks
/// [`says_operator_remedy`] so the clerk is not told twice (#885).
const OPERATOR_REMEDY: &str = "pressing again fails the same way until an operator acts.";

/// Does `text` already end by telling the clerk that only an operator can mend this (a refused
/// reopen's text does; an enrolment refusal or an outage does not)? Pure.
pub fn says_operator_remedy(text: &str) -> bool {
    text.ends_with(OPERATOR_REMEDY)
}

/// How every failed reopen's text begins, and what joins a refusal's clause to its remedy.
/// Shared by [`lost_text`], which writes them, and [`failed_reopen_cause`], which reads them back.
const LOST_HEAD: &str = "connection to the node was lost";
const REMEDY_JOIN: &str = ", so ";

/// A failed reopen's text read back: its bare cause, and who refused it if anyone did; `None`
/// for any text `lost_text` did not write. For a line that is not a press (the front door's
/// provisioning line after a reload, #888), where "pressing again" and "this press tried
/// nothing" would be untrue. The heads `lost_text` writes contain no ": ", so the first one
/// ends the head. Pure.
pub fn failed_reopen_cause(text: &str) -> Option<(&str, Option<Refusal>)> {
    let (_head, rest) = text.strip_prefix(LOST_HEAD)?.split_once(": ")?;
    let Some(body) = rest
        .strip_suffix(OPERATOR_REMEDY)
        .and_then(|r| r.strip_suffix(REMEDY_JOIN))
    else {
        return Some((rest, None));
    };
    [Refusal::Server, Refusal::Client]
        .into_iter()
        .find_map(|r| {
            let cause = body.strip_suffix(r.clause())?.strip_suffix(". ")?;
            Some((cause, Some(r)))
        })
}

/// May a reopen be attempted `now`, given when the last attempt failed? Pure.
fn may_retry(last_failed: Option<Instant>, now: Instant) -> bool {
    last_failed.is_none_or(|t| now.saturating_duration_since(t) >= NodeConnection::RETRY_AFTER)
}

#[cfg(test)]
mod tests;
