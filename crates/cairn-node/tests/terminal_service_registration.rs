//! #909 slice A, Task 4 — the terminal service serves one registration request
//! (`terminal_service::registration::serve_one`).
//!
//! WHY THESE TESTS EXIST. The window no longer holds the node key, so it cannot sign a
//! registration; it writes a REQUEST (db/083) naming the search it ran and the candidates its
//! screen showed, and the node's terminal service registers it. The service is the one place a
//! forged request can be caught: anyone holding the window login could write "I searched and saw
//! no one" by raw SQL, so the service RE-RUNS the search and refuses if it finds a chart the
//! request does not name. These tests prove that, and the queue's failure modes:
//!
//! * an honest request registers once, and its row says which chart it made;
//! * a forged "saw no one" is refused while someone matches, and no chart is made;
//! * a request past its deadline is never served;
//! * a request the service cannot read is refused (never left to loop), and the next is served;
//! * a floor refusal settles `refused` with the floor's words;
//! * two services at once register a request once (the claim is `FOR UPDATE SKIP LOCKED`);
//! * two requests for one new patient, served by two services at once, make one chart (the
//!   registration lock, `REGISTRATION_LOCK`);
//! * a service that fails mid-transaction leaves the request pending, served once later.
//!
//! Each test runs `serve_one` as a login granted `cairn_terminal_service`, the role the real
//! process holds, and writes requests as the window. DB-gated on `$CAIRN_TEST_PG`, serialized
//! via `db::test_serial_guard`.
mod common;

use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use cairn_node::patient::register::register_patient_returning_event;
use cairn_node::patient::search::search_patients;
use cairn_node::terminal_service::registration::{serve_one, Served, NEW_MATCH_REFUSAL};
use cairn_patient_search::{bound_for_prompt, CandidateList, SearchQuery, PROMPT_CAP};
use common::{cs, owner_and_window, service_login, setup};
use std::time::Duration;
use tokio_postgres::Client;
use uuid::Uuid;

const TODAY: &str = "2026-11-17";
const NODE: &str = "n";

/// Everything a test needs: the serial guard, the connection string, the owner (fixtures), the
/// window (writes requests), the service (runs `serve_one`), and the node key the service signs
/// with (enrolled by `setup`).
struct Rig {
    _guard: Client,
    base: String,
    owner: Client,
    window: Client,
    service: Client,
    sk: SigningKey,
    kid: String,
}

/// An empty record and an empty queue. `None` = no database: the test self-skips, and says so.
async fn rig() -> Option<Rig> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    let (sk, kid) = setup(&owner, &["patient_registration", "patient_name"]).await;
    owner
        .batch_execute("TRUNCATE registration_request")
        .await
        .unwrap();
    let service = connect_service(&owner, &base).await;
    Some(Rig {
        _guard: guard,
        base,
        owner,
        window,
        service,
        sk,
        kid,
    })
}

/// A fresh connection as the terminal service.
async fn connect_service(owner: &Client, base: &str) -> Client {
    db::connect(&service_login(owner, base).await)
        .await
        .expect("the service's login connects")
}

/// A name-only search: two tests' charts never meet on a shared date of birth.
fn query(name: &str) -> SearchQuery {
    SearchQuery::new(name, None, &[])
}

/// The window's submit, writing the query and the displayed list with the serde the service
/// reads them back with. Returns the new request's id.
async fn submit(window: &Client, name: &str, q: &SearchQuery, shown: &CandidateList) -> Uuid {
    let id = Uuid::now_v7();
    let status: String = window
        .query_one(
            "SELECT cairn_registration_submit($1::text::uuid, $2, $3::text::jsonb, $4::text::jsonb, $5)",
            &[
                &id.to_string(),
                &name,
                &serde_json::to_string(q).unwrap(),
                &serde_json::to_string(shown).unwrap(),
                &TODAY,
            ],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "pending");
    id
}

/// A chart already on the record, registered the ordinary way (the CLI's path).
async fn seed_chart(r: &mut Rig, name: &str) -> Uuid {
    let q = query(name);
    register_patient_returning_event(
        &mut r.owner,
        &r.sk,
        &r.kid,
        NODE,
        Some(name),
        &q,
        &CandidateList::empty(),
    )
    .await
    .unwrap()
    .value
}

/// `(status, patient_id, refusal)` of one request, as the window reads it.
async fn outcome(window: &Client, id: Uuid) -> (String, Option<Uuid>, Option<String>) {
    let row = window
        .query_one(
            "SELECT status, patient_id::text, refusal FROM cairn_registration_status($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .unwrap();
    let patient: Option<String> = row.get(1);
    (row.get(0), patient.map(|p| p.parse().unwrap()), row.get(2))
}

async fn charts(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM patient_registration", &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn an_honest_request_registers_and_settles_registered() {
    let Some(mut r) = rig().await else { return };
    let namesake = seed_chart(&mut r, "Ann Nguyen").await;
    // The clerk saw the namesake, judged her a different person, and pressed "new patient".
    let q = query("Ann Nguyen");
    let shown = search_patients(&r.owner, &q, TODAY).await.unwrap();
    assert_eq!(shown.displayed_charts(), vec![namesake]);
    let id = submit(&r.window, "Ann Nguyen", &q, &shown).await;

    let served = serve_one(&mut r.service, &r.sk, NODE).await.unwrap();
    let Served::Registered { request, patient } = served else {
        panic!("an honest request registers, got {served:?}");
    };
    assert_eq!(request, id);
    assert_ne!(patient, namesake, "a new chart, not the one shown");
    assert_eq!(
        outcome(&r.window, id).await,
        ("registered".into(), Some(patient), None)
    );
    assert_eq!(charts(&r.owner).await, 2);
    // Settled means served: nothing is left to claim.
    assert_eq!(
        serve_one(&mut r.service, &r.sk, NODE).await.unwrap(),
        Served::Idle
    );
}

/// The window attests only the PROMPT it showed: the closest `PROMPT_CAP` people, never the
/// whole search (ADR-0075). The service must compare the same cut, or the sixth namesake reads
/// as "a new match" on nearly every common-name registration, and searching again never helps.
#[tokio::test]
async fn six_namesakes_register_when_the_clerk_saw_the_bounded_five() {
    let Some(mut r) = rig().await else { return };
    for _ in 0..PROMPT_CAP + 1 {
        seed_chart(&mut r, "Ann Nguyen").await;
    }
    let q = query("Ann Nguyen");
    let found = search_patients(&r.owner, &q, TODAY).await.unwrap();
    assert_eq!(found.people.len(), PROMPT_CAP + 1);
    // Exactly what the window's prompt would have shown and signed.
    let shown = bound_for_prompt(&found).as_list().clone();
    assert_eq!(shown.people.len(), PROMPT_CAP);
    let id = submit(&r.window, "Ann Nguyen", &q, &shown).await;

    let served = serve_one(&mut r.service, &r.sk, NODE).await.unwrap();
    assert!(
        matches!(served, Served::Registered { request, .. } if request == id),
        "the clerk saw the closest five, which is all a prompt shows: {served:?}"
    );
    assert_eq!(charts(&r.owner).await, PROMPT_CAP as i64 + 2);
}

#[tokio::test]
async fn a_forged_saw_no_one_is_refused_when_someone_matches() {
    let Some(mut r) = rig().await else { return };
    seed_chart(&mut r, "Ann Nguyen").await;
    // Raw SQL with the window's login: "I searched for Ann Nguyen and was shown nobody."
    let id = submit(
        &r.window,
        "Ann Nguyen",
        &query("Ann Nguyen"),
        &CandidateList::empty(),
    )
    .await;

    assert_eq!(
        serve_one(&mut r.service, &r.sk, NODE).await.unwrap(),
        Served::Refused {
            request: id,
            reason: NEW_MATCH_REFUSAL.into()
        }
    );
    assert_eq!(
        outcome(&r.window, id).await,
        ("refused".into(), None, Some(NEW_MATCH_REFUSAL.into()))
    );
    assert_eq!(charts(&r.owner).await, 1, "no chart was made");
}

#[tokio::test]
async fn an_expired_request_is_never_served() {
    let Some(mut r) = rig().await else { return };
    let id = submit(
        &r.window,
        "Ann Nguyen",
        &query("Ann Nguyen"),
        &CandidateList::empty(),
    )
    .await;
    // The window waited out its deadline (a door cannot write an old deadline; the owner can).
    r.owner
        .execute(
            "UPDATE registration_request SET deadline = now() - interval '1 second' \
             WHERE request_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap();

    assert_eq!(
        serve_one(&mut r.service, &r.sk, NODE).await.unwrap(),
        Served::Idle
    );
    assert_eq!(outcome(&r.window, id).await.0, "pending");
    assert_eq!(charts(&r.owner).await, 0);
}

#[tokio::test]
async fn a_poison_request_is_refused_and_the_next_one_is_served() {
    let Some(mut r) = rig().await else { return };
    // A query the service cannot read, written ahead of an honest request (oldest is served
    // first), so a poison row that stayed pending would starve everything behind it.
    let poison = Uuid::now_v7();
    r.owner
        .execute(
            "INSERT INTO registration_request (request_id, name, query, displayed, today, \
                                               submitted_at, deadline, status) \
             VALUES ($1::text::uuid, 'Ann Nguyen', '{\"bogus\":1}', '{}', $2, \
                     now() - interval '1 second', now() + interval '10 seconds', 'pending')",
            &[&poison.to_string(), &TODAY],
        )
        .await
        .unwrap();
    let honest = submit(
        &r.window,
        "Ann Nguyen",
        &query("Ann Nguyen"),
        &CandidateList::empty(),
    )
    .await;

    let first = serve_one(&mut r.service, &r.sk, NODE).await.unwrap();
    assert!(
        matches!(&first, Served::Refused { request, .. } if *request == poison),
        "{first:?}"
    );
    assert_eq!(outcome(&r.window, poison).await.0, "refused");
    let second = serve_one(&mut r.service, &r.sk, NODE).await.unwrap();
    assert!(
        matches!(second, Served::Registered { request, .. } if request == honest),
        "{second:?}"
    );
    assert_eq!(charts(&r.owner).await, 1);
}

#[tokio::test]
async fn a_floor_refusal_settles_refused_with_its_words() {
    let Some(mut r) = rig().await else { return };
    // A key the record has never enrolled: the floor refuses its events (a bare RAISE, P0001).
    let (stranger, _) = generate_key().unwrap();
    let id = submit(
        &r.window,
        "Ann Nguyen",
        &query("Ann Nguyen"),
        &CandidateList::empty(),
    )
    .await;
    let served = serve_one(&mut r.service, &stranger, NODE).await.unwrap();
    let Served::Refused { request, reason } = served else {
        panic!("a floor refusal settles refused, got {served:?}");
    };
    assert!(
        reason.contains("not an enrolled"),
        "the floor's own words: {reason}"
    );
    assert_eq!(request, id);
    assert_eq!(outcome(&r.window, id).await.2.as_deref(), Some(&*reason));
    assert_eq!(charts(&r.owner).await, 0);

    // And a refusal decided in Rust before any SQL (a date of birth no precision fits).
    let odd_dob = SearchQuery::new("Ann Nguyen", Some("3/2/1980"), &[]);
    let id = submit(&r.window, "Ann Nguyen", &odd_dob, &CandidateList::empty()).await;
    let served = serve_one(&mut r.service, &r.sk, NODE).await.unwrap();
    assert!(
        matches!(&served, Served::Refused { request, reason }
                 if *request == id && reason.contains("not a recognised shape")),
        "{served:?}"
    );
    assert_eq!(charts(&r.owner).await, 0);
}

#[tokio::test]
async fn two_servers_register_once() {
    let Some(mut r) = rig().await else { return };
    let mut other = connect_service(&r.owner, &r.base).await;
    let id = submit(
        &r.window,
        "Ann Nguyen",
        &query("Ann Nguyen"),
        &CandidateList::empty(),
    )
    .await;

    let (a, b) = tokio::join!(
        serve_one(&mut r.service, &r.sk, NODE),
        serve_one(&mut other, &r.sk, NODE)
    );
    let mut both = [a.unwrap(), b.unwrap()];
    both.sort_by_key(|s| matches!(s, Served::Idle));
    assert!(
        matches!(both, [Served::Registered { request, .. }, Served::Idle] if request == id),
        "one registers, the other finds nothing: {both:?}"
    );
    assert_eq!(charts(&r.owner).await, 1);

    // The same, made deterministic: while one session holds the claim, a second service skips
    // the row at once (it neither waits nor registers it a second time).
    let held = submit(
        &r.window,
        "Bo Tran",
        &query("Bo Tran"),
        &CandidateList::empty(),
    )
    .await;
    let claim = other.transaction().await.unwrap();
    claim
        .query_one(
            "SELECT request_id::text FROM cairn_registration_claim()",
            &[],
        )
        .await
        .unwrap();
    let skipped = tokio::time::timeout(
        Duration::from_secs(5),
        serve_one(&mut r.service, &r.sk, NODE),
    )
    .await
    .expect("a held claim is skipped, never waited on");
    assert_eq!(skipped.unwrap(), Served::Idle);
    claim.rollback().await.unwrap();
    assert!(matches!(
        serve_one(&mut r.service, &r.sk, NODE).await.unwrap(),
        Served::Registered { request, .. } if request == held
    ));
    assert_eq!(charts(&r.owner).await, 2);
}

/// The backend pid of `c`, to watch in `pg_stat_activity`.
async fn pid_of(c: &Client) -> i32 {
    c.query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0)
}

/// Final review F1 (ruling R15). Two clerks register the same new patient at once (two windows,
/// or one press resent), and two services are running. Both clerks searched before either chart
/// existed, so both requests honestly say "saw no one". Without one lock across services, both
/// searches run before either registration commits (the HLC row lock comes only at the first
/// tick), neither sees the other's chart, and the record gets a duplicate. With the
/// registration lock, the second service waits for the first to commit, searches after it, and
/// refuses: "a new possible match has appeared".
///
/// Deterministic, not timing-based: a third session holds the node's HLC row, so the first
/// service to get going parks on it with its search already run, and the test releases it only
/// once BOTH services are parked on a lock (without the registration lock, both park on the
/// HLC row, each search done; with it, the second parks on the registration lock, unsearched).
#[tokio::test]
async fn two_servers_two_requests_for_one_patient_register_one_chart() {
    let Some(r) = rig().await else { return };
    let q = query("Ann Nguyen");
    let first = submit(&r.window, "Ann Nguyen", &q, &CandidateList::empty()).await;
    let second = submit(&r.window, "Ann Nguyen", &q, &CandidateList::empty()).await;

    let mut holder = db::connect(&r.base).await.unwrap();
    let hold = holder.transaction().await.unwrap();
    hold.query_one("SELECT wall FROM node_hlc_tick()", &[])
        .await
        .unwrap();

    let mut services = Vec::new();
    let mut pids = Vec::new();
    for c in [r.service, connect_service(&r.owner, &r.base).await] {
        pids.push(pid_of(&c).await);
        let sk = r.sk.clone();
        services.push(tokio::spawn(async move {
            let mut c = c;
            serve_one(&mut c, &sk, NODE).await
        }));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let parked: i64 = hold
            .query_one(
                "SELECT count(*) FROM pg_stat_activity \
                  WHERE pid = ANY($1) AND wait_event_type = 'Lock'",
                &[&pids],
            )
            .await
            .unwrap()
            .get(0);
        if parked == 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "both services never parked on a lock within 5s ({parked} did)"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    hold.rollback().await.unwrap();

    let mut served = Vec::new();
    for s in services {
        served.push(s.await.unwrap().unwrap());
    }
    served.sort_by_key(|s| matches!(s, Served::Refused { .. }));
    assert!(
        matches!(&served[..], [Served::Registered { .. }, Served::Refused { reason, .. }]
                 if reason == NEW_MATCH_REFUSAL),
        "one registers, the other sees its chart and refuses: {served:?}"
    );
    assert_eq!(charts(&r.owner).await, 1, "one chart, never a duplicate");
    let mut settled = [
        outcome(&r.window, first).await.0,
        outcome(&r.window, second).await.0,
    ];
    settled.sort();
    assert_eq!(settled, ["refused".to_string(), "registered".to_string()]);
}

/// A NOLOGIN role that can claim, settle and search (the search runs as its caller), but not tick the node's clock, so the
/// registration fails on its first statement with a privilege error (42501, not a floor
/// refusal): the shape of a service that breaks mid-transaction. Dropped at the start and the
/// end of the test (a role is cluster-wide; a panicked run must not leave it behind).
const NO_CLOCK: &str = "cairn_test_terminal_no_clock";

async fn drop_no_clock(owner: &Client) {
    owner
        .batch_execute(&format!(
            "DO $$ BEGIN
               IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{NO_CLOCK}') THEN
                 EXECUTE 'DROP OWNED BY {NO_CLOCK}';
                 EXECUTE 'DROP ROLE {NO_CLOCK}';
               END IF;
             END $$;"
        ))
        .await
        .expect("dropping the test role");
}

#[tokio::test]
async fn a_dropped_transaction_leaves_the_request_pending() {
    let Some(mut r) = rig().await else { return };
    drop_no_clock(&r.owner).await;
    r.owner
        .batch_execute(&format!(
            "CREATE ROLE {NO_CLOCK} NOLOGIN;
             GRANT EXECUTE ON FUNCTION cairn_registration_claim(),
                 cairn_registration_settle(uuid, text, uuid, uuid, text),
                 cairn_search_candidates(text[], text, jsonb) TO {NO_CLOCK};
             GRANT SELECT ON patient_identifier, patient_demographic, patient_name TO {NO_CLOCK};"
        ))
        .await
        .unwrap();
    let mut broken = db::connect(&r.base).await.unwrap();
    broken
        .batch_execute(&format!("SET ROLE {NO_CLOCK}"))
        .await
        .unwrap();
    // A name nobody has, so the search returns before any read the role lacks.
    let id = submit(
        &r.window,
        "Ann Nguyen",
        &query("Ann Nguyen"),
        &CandidateList::empty(),
    )
    .await;

    let failed = serve_one(&mut broken, &r.sk, NODE).await;
    let e = failed.expect_err("a non-refusal error is returned");
    // It failed inside the registration (on its savepoint), after the claim and the search.
    assert!(format!("{e:#}").contains("ticking the node HLC"), "{e:#}");
    // Statements run in order on one connection: once this answers, the dropped transaction's
    // ROLLBACK has run and the claim's row lock is gone.
    broken.batch_execute("RESET ROLE").await.unwrap();
    assert_eq!(outcome(&r.window, id).await.0, "pending");
    assert_eq!(charts(&r.owner).await, 0);

    // A later poll by a working service registers it, once.
    assert!(matches!(
        serve_one(&mut r.service, &r.sk, NODE).await.unwrap(),
        Served::Registered { request, .. } if request == id
    ));
    assert_eq!(
        serve_one(&mut r.service, &r.sk, NODE).await.unwrap(),
        Served::Idle
    );
    assert_eq!(charts(&r.owner).await, 1);
    drop(broken);
    drop_no_clock(&r.owner).await;
}
