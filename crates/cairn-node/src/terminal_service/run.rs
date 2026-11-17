//! The terminal service's loop: serve registration requests, commit overdue note drafts, purge
//! old requests, forever (#909, design D1).
//!
//! WHY A SCHEDULE. Three jobs share one connection at three paces (requests every 250 ms so a
//! clerk's registration feels instant; commits every minute; purge hourly). [`due`] is the pure
//! decision of which are due, so the pacing is unit-tested without a clock or a database.
//! No job's error ends the loop (an unattended service that stops serves nothing); a lost
//! connection is reopened after a 1 s back-off. Log lines carry ids and primary messages only.
use std::collections::HashSet;
use std::time::{Duration, Instant};

use cairn_event::SigningKey;
use tokio_postgres::Client;
use uuid::Uuid;

use super::commit::{commit_tick, primary_message};
use super::registration::{serve_one, Served};

const REQUEST_TICK: Duration = Duration::from_millis(250);
/// How often overdue drafts are committed (the window's old tick, moved here).
pub const COMMIT_TICK: Duration = Duration::from_secs(60);
const PURGE_TICK: Duration = Duration::from_secs(3600);
const RECONNECT_BACKOFF: Duration = Duration::from_secs(1);
/// A burst of requests is drained in one tick, but never so many that a commit tick starves.
const MAX_SERVED_PER_TICK: usize = 50;

/// When each job last ran. `None` means never (requests only: the first tick serves requests,
/// the slower jobs wait their first interval).
pub struct Schedule {
    last_requests: Option<Instant>,
    last_commit: Instant,
    last_purge: Instant,
}

/// Which jobs are due now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Due {
    pub requests: bool,
    pub commits: bool,
    pub purge: bool,
}

impl Schedule {
    pub fn new(start: Instant) -> Self {
        Schedule {
            last_requests: None,
            last_commit: start,
            last_purge: start,
        }
    }

    /// Record that the due jobs ran at `now`.
    pub fn mark(&mut self, d: &Due, now: Instant) {
        if d.requests {
            self.last_requests = Some(now);
        }
        if d.commits {
            self.last_commit = now;
        }
        if d.purge {
            self.last_purge = now;
        }
    }
}

/// Which jobs are due at `now`. **Pure.**
pub fn due(now: Instant, last: &Schedule) -> Due {
    let elapsed = |t: Instant| now.saturating_duration_since(t);
    Due {
        requests: last
            .last_requests
            .is_none_or(|t| elapsed(t) >= REQUEST_TICK),
        commits: elapsed(last.last_commit) >= COMMIT_TICK,
        purge: elapsed(last.last_purge) >= PURGE_TICK,
    }
}

/// Serve requests until none is waiting (or the cap). A refusal is logged WITHOUT its reason,
/// which can quote a typed date of birth.
async fn serve_requests(client: &mut Client, sk: &SigningKey, origin: &str) {
    for _ in 0..MAX_SERVED_PER_TICK {
        match serve_one(client, sk, origin).await {
            Ok(Served::Idle) => return,
            Ok(Served::Registered { request, patient }) => {
                eprintln!("request {request} registered as patient {patient}")
            }
            Ok(Served::Refused { request, .. }) => eprintln!("request {request} refused"),
            Err(e) => {
                eprintln!(
                    "serving a registration request failed: {}",
                    primary_message(&e)
                );
                return;
            }
        }
    }
}

async fn purge(client: &Client) {
    match client
        .query_one("SELECT cairn_registration_purge()", &[])
        .await
    {
        Ok(r) => {
            let n: i32 = r.get(0);
            if n > 0 {
                eprintln!("purged {n} old registration requests");
            }
        }
        Err(e) => eprintln!(
            "purging registration requests failed: {}",
            primary_message(&e.into())
        ),
    }
}

/// One iteration's work: each due job once. No job's error escapes (it is logged).
pub async fn run_jobs(
    client: &mut Client,
    sk: &SigningKey,
    origin: &str,
    d: &Due,
    said: &mut HashSet<Uuid>,
) {
    if d.requests {
        serve_requests(client, sk, origin).await;
    }
    if d.commits {
        if let Err(e) = commit_tick(client, sk, origin, said).await {
            eprintln!("the commit tick failed: {}", primary_message(&e));
        }
    }
    if d.purge {
        purge(client).await;
    }
}

/// Run the service forever on `conn`, reopening it after a loss. Never returns. The reconnect
/// and back-off path is not covered by a test (it needs a killed backend); `run_jobs` is.
pub async fn run(conn: &str, sk: &SigningKey, origin: &str) -> anyhow::Result<()> {
    let mut said: HashSet<Uuid> = HashSet::new();
    let mut client = crate::db::connect(conn).await?;
    let mut schedule = Schedule::new(Instant::now());
    loop {
        if client.is_closed() {
            tokio::time::sleep(RECONNECT_BACKOFF).await;
            match crate::db::connect(conn).await {
                Ok(c) => client = c,
                Err(e) => {
                    eprintln!("reconnecting failed: {}", primary_message(&e));
                    continue;
                }
            }
        }
        let now = Instant::now();
        let d = due(now, &schedule);
        run_jobs(&mut client, sk, origin, &d, &mut said).await;
        schedule.mark(&d, now);
        tokio::time::sleep(REQUEST_TICK).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn due_table() {
        let t0 = Instant::now();
        let s = Schedule::new(t0);
        let d = due(t0, &s);
        assert!(
            d.requests && !d.commits && !d.purge,
            "first tick: requests only"
        );
        let mut s = s;
        s.mark(&d, t0);
        assert_eq!(due(t0 + Duration::from_millis(100), &s), Due::default());
        assert!(due(t0 + Duration::from_millis(250), &s).requests);
        let d = due(t0 + Duration::from_secs(60), &s);
        assert!(d.commits && !d.purge);
        let d = due(t0 + Duration::from_secs(3600), &s);
        assert!(d.commits && d.purge);
    }
}
