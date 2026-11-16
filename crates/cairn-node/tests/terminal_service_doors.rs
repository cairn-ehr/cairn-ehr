//! #909 slice A — db/083's doors: the registration request queue and the failed-commit fact.
//!
//! WHY THESE TESTS EXIST. #909 takes the node key off the clinician's window. The two acts the
//! window used to sign with it — registering a patient, and the timed commit of an abandoned note
//! draft — move to a node-side `cairn-node terminal-service` process. db/083 is the meeting point:
//! the window (role `cairn_window`) writes a registration REQUEST and later reads or expires it;
//! the service (role `cairn_terminal_service`) claims one, registers, and settles it in the same
//! transaction. The safety properties all live in the doors, so they are proven here, each one as
//! the role that will really call it:
//!
//! * a re-sent request never registers twice (idempotent by id) and cannot be quietly changed;
//! * the service never claims a request past its deadline, nor one another service holds;
//! * the window's expire waits behind a claim, so it can never say "expired" for a request that
//!   is in fact going in (the "no late registration" rule, design D6);
//! * the window cannot reach the service's doors at all (42501), nor the queue table;
//! * settle allows only the two legal transitions out of `pending`;
//! * purge keeps a day of history and a pending row the window may still be reading;
//! * a failed-commit fact (design D7) disappears with its draft.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Each suite connects as a
//! probe LOGIN granted the role under test, exactly as the real processes will; nothing here runs
//! the doors as the schema owner except the fixture writes a door cannot make (an old deadline).
mod common;

use cairn_node::db;
use common::{cs, db_msg, owner_and_window, service_login};
use tokio_postgres::error::SqlState;
use tokio_postgres::Client;
use uuid::Uuid;

/// A fresh connection as the terminal service.
async fn service(owner: &Client, base: &str) -> Client {
    db::connect(&service_login(owner, base).await)
        .await
        .expect("the service's login connects")
}

/// The DB serial guard, the connection string, and three sessions on an empty queue: the owner
/// (fixture writes a door cannot make), the window and the service. `None` = no database: the
/// test self-skips, and says so.
async fn rig() -> Option<(Client, String, Client, Client, Client)> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    owner
        .batch_execute(
            "TRUNCATE registration_request, note_draft_commit_failure, note_draft CASCADE",
        )
        .await
        .unwrap();
    let svc = service(&owner, &base).await;
    Some((guard, base, owner, window, svc))
}

/// The window's submit, with a fixed attested search so a test varies only what it is about.
async fn submit(window: &Client, id: Uuid, name: &str) -> Result<String, tokio_postgres::Error> {
    submit_raw(window, id, Some(name), r#"{"family":"Nguyen"}"#).await
}

/// [`submit`] with the name (NULL allowed) and the query's JSON text chosen by the caller.
async fn submit_raw(
    window: &Client,
    id: Uuid,
    name: Option<&str>,
    query: &str,
) -> Result<String, tokio_postgres::Error> {
    window
        .query_one(
            "SELECT cairn_registration_submit($1::text::uuid, $2, $3::text::jsonb, $4::text::jsonb, $5)",
            &[&id.to_string(), &name, &query, &"[]", &"2026-11-17"],
        )
        .await
        .map(|r| r.get(0))
}

/// The ids one claim returned (zero or one).
async fn claim(c: &Client) -> Vec<Uuid> {
    c.query(
        "SELECT request_id::text FROM cairn_registration_claim()",
        &[],
    )
    .await
    .unwrap()
    .iter()
    .map(uuid_at_0)
    .collect()
}

async fn status_of(c: &Client, id: Uuid) -> String {
    c.query_one(
        "SELECT status FROM cairn_registration_status($1::text::uuid)",
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

async fn expire(c: &Client, id: Uuid) -> String {
    c.query_one(
        "SELECT cairn_registration_expire($1::text::uuid)",
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

async fn settle_registered(c: &Client, id: Uuid) -> Result<u64, tokio_postgres::Error> {
    let (patient, event) = (Uuid::now_v7(), Uuid::now_v7());
    c.execute(
        "SELECT cairn_registration_settle($1::text::uuid, 'registered', $2::text::uuid, $3::text::uuid, NULL)",
        &[&id.to_string(), &patient.to_string(), &event.to_string()],
    )
    .await
}

/// The window's read of one chart's failed-commit facts, sorted.
async fn failed(c: &Client, patient: Uuid) -> Vec<Uuid> {
    let mut v: Vec<Uuid> = c
        .query(
            "SELECT d::text FROM cairn_note_drafts_commit_failed($1::text::uuid) d",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(uuid_at_0)
        .collect();
    v.sort();
    v
}

/// Column 0 of a row, a uuid read as `::text` (this crate's tokio-postgres has no uuid feature).
fn uuid_at_0(r: &tokio_postgres::Row) -> Uuid {
    r.get::<_, String>(0).parse().unwrap()
}

fn code(e: &tokio_postgres::Error) -> Option<&SqlState> {
    e.code()
}

#[tokio::test]
async fn a_resubmitted_request_returns_its_first_status_and_a_changed_one_is_refused() {
    let Some((_g, _base, _owner, window, svc)) = rig().await else {
        return;
    };
    let id = Uuid::now_v7();
    assert_eq!(submit(&window, id, "Ann Nguyen").await.unwrap(), "pending");
    assert_eq!(status_of(&window, id).await, "pending");
    settle_registered(&svc, id).await.unwrap();
    // A re-send (the window lost the reply) answers what already happened; never a second row.
    assert_eq!(
        submit(&window, id, "Ann Nguyen").await.unwrap(),
        "registered"
    );
    let row = window
        .query_one(
            "SELECT status, patient_id::text FROM cairn_registration_status($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "registered");
    assert!(
        row.get::<_, Option<String>>(1).is_some(),
        "registered names its chart"
    );
    // The same id with different content is a caller bug, refused out loud.
    let e = submit(&window, id, "Ann Smith").await.unwrap_err();
    assert!(db_msg(&e).contains("different"), "{}", db_msg(&e));
    // "The same inputs" is NULL-safe: a nameless request re-sent is the same request.
    let nameless = Uuid::now_v7();
    for _ in 0..2 {
        let s = submit_raw(&window, nameless, None, "{}").await.unwrap();
        assert_eq!(s, "pending", "a nameless re-send answers its status");
    }
    // And it compares the search as JSON, not as text: key order is not a different request.
    let reordered = Uuid::now_v7();
    for query in [
        r#"{"family":"Nguyen","given":"Ann"}"#,
        r#"{"given":"Ann","family":"Nguyen"}"#,
    ] {
        let s = submit_raw(&window, reordered, Some("Ann Nguyen"), query)
            .await
            .unwrap();
        assert_eq!(
            s, "pending",
            "a re-send with its keys reordered answers its status"
        );
    }
}

#[tokio::test]
async fn an_unknown_request_is_refused_not_reported_pending() {
    let Some((_g, _base, _owner, window, _svc)) = rig().await else {
        return;
    };
    let id = Uuid::now_v7();
    let e = window
        .query(
            "SELECT * FROM cairn_registration_status($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .unwrap_err();
    // A bare RAISE (P0001), like every floor refusal (#633); the message names the bug.
    assert_eq!(code(&e), Some(&SqlState::RAISE_EXCEPTION), "status: {e:?}");
    assert!(db_msg(&e).contains("no registration request"));
    let e = window
        .query_one(
            "SELECT cairn_registration_expire($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .unwrap_err();
    assert_eq!(code(&e), Some(&SqlState::RAISE_EXCEPTION), "expire: {e:?}");
    assert!(db_msg(&e).contains("no registration request"));
}

#[tokio::test]
async fn claim_skips_an_expired_request_and_a_locked_one() {
    let Some((_g, base, owner, window, a)) = rig().await else {
        return;
    };
    let (b, c) = (service(&owner, &base).await, service(&owner, &base).await);
    let (stale, older, newer) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for id in [stale, older, newer] {
        submit(&window, id, "Ann Nguyen").await.unwrap();
    }
    // A door never moves a deadline back; the owner does, to make one already passed. Its
    // submitted_at is made the OLDEST so "oldest first" would pick it, were it claimable.
    owner
        .execute(
            "UPDATE registration_request SET deadline = now() - interval '1 second', \
                    submitted_at = now() - interval '1 minute' WHERE request_id = $1::text::uuid",
            &[&stale.to_string()],
        )
        .await
        .unwrap();
    owner
        .execute(
            "UPDATE registration_request SET submitted_at = now() - interval '30 seconds' \
              WHERE request_id = $1::text::uuid",
            &[&older.to_string()],
        )
        .await
        .unwrap();
    // The claim IS the row lock, held until the CALLER's transaction ends (design D6).
    a.batch_execute("BEGIN").await.unwrap();
    assert_eq!(claim(&a).await, vec![older], "oldest live request first");
    b.batch_execute("BEGIN").await.unwrap();
    assert_eq!(claim(&b).await, vec![newer], "a held request is skipped");
    c.batch_execute("BEGIN").await.unwrap();
    assert!(
        claim(&c).await.is_empty(),
        "the stale one is never claimed; both live ones are held"
    );
    // A crash (here: a rollback) leaves the request pending and claimable, never half-claimed.
    a.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(claim(&c).await, vec![older]);
    for s in [&b, &c] {
        s.batch_execute("ROLLBACK").await.unwrap();
    }
    assert_eq!(status_of(&window, older).await, "pending");
}

#[tokio::test]
async fn expire_changes_only_a_pending_request() {
    let Some((_g, _base, _owner, window, svc)) = rig().await else {
        return;
    };
    let (waiting, done, refused) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for id in [waiting, done, refused] {
        submit(&window, id, "Ann Nguyen").await.unwrap();
    }
    settle_registered(&svc, done).await.unwrap();
    svc.execute(
        "SELECT cairn_registration_settle($1::text::uuid, 'refused', NULL, NULL, 'a new possible match')",
        &[&refused.to_string()],
    )
    .await
    .unwrap();
    assert_eq!(expire(&window, waiting).await, "expired");
    assert_eq!(
        expire(&window, waiting).await,
        "expired",
        "expiring twice is harmless"
    );
    assert_eq!(expire(&window, done).await, "registered");
    assert_eq!(expire(&window, refused).await, "refused");
    // An expired request is never claimed, and its settle is refused.
    assert!(claim(&svc).await.is_empty());
    assert!(settle_registered(&svc, waiting).await.is_err());
}

#[tokio::test]
async fn expire_waits_behind_a_claim_and_then_reports_registered() {
    let Some((_g, _base, owner, window, svc)) = rig().await else {
        return;
    };
    let id = Uuid::now_v7();
    submit(&window, id, "Ann Nguyen").await.unwrap();
    // Session A (the service): CLAIM only, transaction left open. Nothing else holds the row, so
    // a wait below can only be on the claim's own lock (settling first would let settle's UPDATE
    // be what B waits on, and the test would pass with the claim taking no lock at all).
    svc.batch_execute("BEGIN").await.unwrap();
    assert_eq!(claim(&svc).await, vec![id]);
    // Session B (the window): its expire must queue behind A's claim.
    let b_pid: i32 = window
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let pending = tokio::spawn(async move {
        let s = expire(&window, id).await;
        (s, window)
    });
    // The proof is Postgres's own word that B waits on a lock, not elapsed time; the bounded
    // poll only gives the spawned query time to reach the server.
    let mut blocked = false;
    for _ in 0..200 {
        let row = owner
            .query_one(
                "SELECT coalesce(wait_event_type, '') FROM pg_stat_activity WHERE pid = $1",
                &[&b_pid],
            )
            .await
            .unwrap();
        if row.get::<_, String>(0) == "Lock" {
            blocked = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(blocked, "the window's expire must wait behind the claim");
    assert!(
        !pending.is_finished(),
        "it has not returned while A holds the row"
    );
    // A registers while B waits, then commits: B must read that outcome, never "expired".
    settle_registered(&svc, id).await.unwrap();
    svc.batch_execute("COMMIT").await.unwrap();
    let (s, _window) = pending.await.unwrap();
    assert_eq!(s, "registered", "B reports what actually happened");
}

#[tokio::test]
async fn the_window_cannot_claim_settle_purge_or_write_a_failure() {
    let Some((_g, _base, _owner, window, _svc)) = rig().await else {
        return;
    };
    let id = Uuid::now_v7();
    submit(&window, id, "Ann Nguyen").await.unwrap();
    for sql in [
        "SELECT * FROM cairn_registration_claim()",
        "SELECT cairn_registration_purge()",
        "SELECT * FROM registration_request",
        "SELECT * FROM note_draft_commit_failure",
    ] {
        let e = window.query(sql, &[]).await.unwrap_err();
        assert_eq!(code(&e), Some(&SqlState::INSUFFICIENT_PRIVILEGE), "{sql}");
    }
    let e = settle_registered(&window, id).await.unwrap_err();
    assert_eq!(code(&e), Some(&SqlState::INSUFFICIENT_PRIVILEGE), "settle");
    let e = window
        .execute(
            "SELECT cairn_note_draft_commit_failed($1::text::uuid, true)",
            &[&id.to_string()],
        )
        .await
        .unwrap_err();
    assert_eq!(
        code(&e),
        Some(&SqlState::INSUFFICIENT_PRIVILEGE),
        "commit_failed"
    );
}

#[tokio::test]
async fn settle_refuses_an_illegal_transition() {
    let Some((_g, _base, _owner, window, svc)) = rig().await else {
        return;
    };
    let id = Uuid::now_v7();
    submit(&window, id, "Ann Nguyen").await.unwrap();
    let patient = Uuid::now_v7();
    for (sql, why) in [
        (
            "SELECT cairn_registration_settle($1::text::uuid, 'registered', NULL, NULL, NULL)",
            "registered without a chart or event",
        ),
        (
            "SELECT cairn_registration_settle($1::text::uuid, 'refused', NULL, NULL, NULL)",
            "refused without a reason",
        ),
        (
            "SELECT cairn_registration_settle($1::text::uuid, 'expired', NULL, NULL, NULL)",
            "expired is the window's word, never settle's",
        ),
    ] {
        assert!(svc.execute(sql, &[&id.to_string()]).await.is_err(), "{why}");
    }
    let e = svc
        .execute(
            "SELECT cairn_registration_settle($1::text::uuid, 'registered', $2::text::uuid, NULL, NULL)",
            &[&id.to_string(), &patient.to_string()],
        )
        .await
        .unwrap_err();
    assert!(db_msg(&e).contains("registered"), "{}", db_msg(&e));
    assert_eq!(
        status_of(&window, id).await,
        "pending",
        "no refused settle wrote"
    );
    settle_registered(&svc, id).await.unwrap();
    // Any settle of a row no longer pending is refused, the same outcome included.
    let e = settle_registered(&svc, id).await.unwrap_err();
    assert!(db_msg(&e).contains("pending"), "{}", db_msg(&e));
    let e = svc
        .execute(
            "SELECT cairn_registration_settle($1::text::uuid, 'refused', NULL, NULL, 'late')",
            &[&id.to_string()],
        )
        .await
        .unwrap_err();
    assert!(db_msg(&e).contains("pending"));
}

#[tokio::test]
async fn purge_deletes_only_old_settled_rows() {
    let Some((_g, _base, owner, window, svc)) = rig().await else {
        return;
    };
    let ids: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();
    for id in &ids {
        submit(&window, *id, "Ann Nguyen").await.unwrap();
    }
    let [old_settled, new_settled, old_pending, new_pending, old_expired] = ids[..] else {
        unreachable!()
    };
    settle_registered(&svc, old_settled).await.unwrap();
    settle_registered(&svc, new_settled).await.unwrap();
    expire(&window, old_expired).await;
    // Age the rows the way 25 hours would; no door moves these clocks.
    owner
        .execute(
            "UPDATE registration_request SET settled_at = now() - interval '25 hours' \
              WHERE request_id = ANY($1::text[]::uuid[])",
            &[&vec![old_settled.to_string(), old_expired.to_string()]],
        )
        .await
        .unwrap();
    owner
        .execute(
            "UPDATE registration_request SET deadline = now() - interval '25 hours' \
              WHERE request_id = $1::text::uuid",
            &[&old_pending.to_string()],
        )
        .await
        .unwrap();
    owner
        .execute(
            "UPDATE registration_request SET deadline = now() - interval '23 hours' \
              WHERE request_id = $1::text::uuid",
            &[&new_pending.to_string()],
        )
        .await
        .unwrap();
    let n: i32 = svc
        .query_one("SELECT cairn_registration_purge()", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 3);
    let left: Vec<Uuid> = owner
        .query(
            "SELECT request_id::text FROM registration_request ORDER BY request_id",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(uuid_at_0)
        .collect();
    let mut want = vec![new_settled, new_pending];
    want.sort();
    assert_eq!(left, want);
}

#[tokio::test]
async fn a_failure_row_goes_when_its_draft_goes() {
    let Some((_g, _base, owner, window, svc)) = rig().await else {
        return;
    };
    let (patient, other) = (Uuid::now_v7(), Uuid::now_v7());
    let (draft, kept, elsewhere) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for (d, p) in [(draft, patient), (kept, patient), (elsewhere, other)] {
        owner
            .execute(
                "INSERT INTO note_draft (draft_id, patient_id, terminal_id, recording_started, last_keystroke) \
                 VALUES ($1::text::uuid, $2::text::uuid, 'pc-1', now(), now())",
                &[&d.to_string(), &p.to_string()],
            )
            .await
            .unwrap();
    }
    for d in [draft, kept, elsewhere] {
        svc.execute(
            "SELECT cairn_note_draft_commit_failed($1::text::uuid, true)",
            &[&d.to_string()],
        )
        .await
        .unwrap();
    }
    let mut both = vec![draft, kept];
    both.sort();
    assert_eq!(
        failed(&window, patient).await,
        both,
        "one chart's facts only"
    );
    // A later success clears the fact.
    svc.execute(
        "SELECT cairn_note_draft_commit_failed($1::text::uuid, false)",
        &[&kept.to_string()],
    )
    .await
    .unwrap();
    assert_eq!(failed(&window, patient).await, vec![draft]);
    // A draft that is gone (spent, discarded, committed) takes its fact with it.
    owner
        .execute(
            "DELETE FROM note_draft WHERE draft_id = $1::text::uuid",
            &[&draft.to_string()],
        )
        .await
        .unwrap();
    assert!(failed(&window, patient).await.is_empty());
    // A failure reported for a draft already gone is a no-op, not an error.
    svc.execute(
        "SELECT cairn_note_draft_commit_failed($1::text::uuid, true)",
        &[&draft.to_string()],
    )
    .await
    .unwrap();
    assert!(failed(&window, patient).await.is_empty());
}
