//! DB-gated: the clinicians' key vault floor (db/061).
//!
//! The vault replaces a key FILE: each clinician's Ed25519 seed sits in `vault_member`, sealed under
//! their passphrase, and the ONLY way out is `cairn_vault_release`, which checks the passphrase inside
//! Postgres and hands back the seed for one act. These tests pin the floor's promises:
//!
//! * the released seed is the enrolled key, whatever case/whitespace the staff ID is typed in;
//! * a wrong passphrase is an ANSWER (`outcome = 'wrong'`), never a raised error (an error can carry
//!   the passphrase into the server log, and would roll back the failure counter);
//! * failures 1–3 are free, the 4th starts a back-off, and a try during the back-off (`waiting`)
//!   neither unseals nor counts — a back-off, never a lock-out;
//! * a superuser's key is never released for a clinical act (spec §5.1), and a clinician's key never
//!   enrols anyone;
//! * enrolment needs a superuser's attestation over THIS request;
//! * the runtime role cannot read the wrapped keys, yet can still use the release door.
//!
//! Every member is built through the real doors (`cairn_vault_bootstrap`, `cairn_vault_enrol`) with
//! blobs from `seal::seal` and keys from runtime-computed seeds (house rule 6: no key literals).
use cairn_event::keys::Secret32;
use cairn_event::{sign_attestation, SigningKey};
use cairn_node::{db, seal};
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A deterministic 32-byte seed built at runtime; `variant` gives each person their own.
fn test_seed(variant: u8) -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(variant))
}

/// The hex verifying-key id of a seed — what the registry calls `signing_key_id`.
fn kid_of(seed: [u8; 32]) -> String {
    hex::encode(SigningKey::from_bytes(&seed).verifying_key().to_bytes())
}

/// Seal `seed` under `passphrase`, exactly as the enrolment surface will.
fn sealed_blob(seed: [u8; 32], passphrase: &str) -> Vec<u8> {
    let sealed = seal::seal(
        &Secret32::from_bytes(seed),
        passphrase,
        &seal::generate_recovery_code(),
    )
    .unwrap();
    seal::to_cbor(&sealed)
}

/// One serialised, freshly-loaded connection with an empty vault and actor registry.
/// The returned guard must be held for the whole test.
async fn fresh() -> (Client, Client) {
    let base = cs().expect("guarded by caller");
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE vault_attempt, vault_member, vault_terminal, vault_policy, event_log, actor_event, patient_chart CASCADE",
    )
    .await
    .unwrap();
    (guard, c)
}

const SU_PASS: &str = "boss passphrase";
const CLIN_PASS: &str = "nurse passphrase";

/// The pinned determinant set the vault's doors accept for `(role, normalised staff ID)`: the role
/// and the staff ID as the actor's `handle`, nothing else (final review item 2).
fn pinned(role: &str, handle: &str) -> String {
    format!(r#"{{"role":"{role}","handle":"{handle}"}}"#)
}

/// Call `cairn_vault_bootstrap` for `staff_id` with `pinned`, sealed under the superuser passphrase.
async fn bootstrap(
    c: &Client,
    staff_id: &str,
    pinned: &str,
    seed: [u8; 32],
) -> Result<Vec<u8>, tokio_postgres::Error> {
    c.query_one(
        "SELECT cairn_vault_bootstrap($1, $2, $3::text::jsonb, $4, $5, $6, $7, $8)",
        &[
            &staff_id,
            &"Site Superuser",
            &pinned,
            &kid_of(seed),
            &sealed_blob(seed, SU_PASS),
            &Some("the usual"),
            &Some("0400 000 000"),
            &None::<&str>,
        ],
    )
    .await
    .map(|r| r.get(0))
}

/// Bootstrap the site's first superuser, `boss`, at the "console". Returns its key id.
async fn bootstrap_boss(c: &Client) -> String {
    let seed = test_seed(1);
    bootstrap(c, "boss", &pinned("superuser", "boss"), seed)
        .await
        .unwrap();
    kid_of(seed)
}

/// The superuser's attestation token over the enrolment `(staff_id, kid, role)`.
async fn enrol_token(
    c: &Client,
    staff_id: &str,
    kid: &str,
    role: &str,
    signer: [u8; 32],
) -> Vec<u8> {
    let addr: Vec<u8> = c
        .query_one(
            "SELECT cairn_vault_enrol_address($1, $2, $3)",
            &[&staff_id, &kid, &role],
        )
        .await
        .unwrap()
        .get(0);
    sign_attestation(
        &addr,
        &kid_of(signer),
        "superuser",
        &SigningKey::from_bytes(&signer),
    )
    .unwrap()
}

/// Call `cairn_vault_enrol` for a clinician whose pinned role is `role` (`staff_id` must already
/// be normalised: it is the pinned handle as typed).
async fn enrol(
    c: &Client,
    staff_id: &str,
    seed: [u8; 32],
    role: &str,
    superuser_staff_id: &str,
    token: &[u8],
) -> Result<Vec<u8>, tokio_postgres::Error> {
    enrol_pinned(
        c,
        staff_id,
        seed,
        role,
        &pinned(role, staff_id),
        superuser_staff_id,
        token,
    )
    .await
}

/// [`enrol`] with a caller-chosen pinned set, for the tests that the floor refuses a wrong one.
async fn enrol_pinned(
    c: &Client,
    staff_id: &str,
    seed: [u8; 32],
    role: &str,
    pinned: &str,
    superuser_staff_id: &str,
    token: &[u8],
) -> Result<Vec<u8>, tokio_postgres::Error> {
    c.query_one(
        "SELECT cairn_vault_enrol($1, $2, $3, $4::text::jsonb, $5, $6, $7, $8, $9, $10, $11)",
        &[
            &staff_id,
            &"J Smith",
            &role,
            &pinned,
            &kid_of(seed),
            &sealed_blob(seed, CLIN_PASS),
            &None::<&str>,
            &None::<&str>,
            &Some("j@example.org"),
            &superuser_staff_id,
            &token,
        ],
    )
    .await
    .map(|r| r.get(0))
}

/// Bootstrap `boss` and enrol clinician `jsmith`. Returns (boss kid, jsmith kid).
async fn boss_and_jsmith(c: &Client) -> (String, String) {
    let boss = bootstrap_boss(c).await;
    let seed = test_seed(2);
    let token = enrol_token(c, "jsmith", &kid_of(seed), "clinician", test_seed(1)).await;
    enrol(c, "jsmith", seed, "clinician", "boss", &token)
        .await
        .expect("a superuser's token over this request enrols");
    (boss, kid_of(seed))
}

/// One release outcome, as plain Rust values.
#[derive(Debug)]
struct Release {
    outcome: String,
    seed: Option<Vec<u8>>,
    signing_key_id: Option<String>,
    retry_after_ms: Option<i64>,
}

/// Call the release door. A raised error here fails the "never raises" contract outright.
async fn release(c: &Client, staff_id: &str, passphrase: &str, purpose: &str) -> Release {
    let row = c
        .query_one(
            "SELECT outcome, seed, signing_key_id, retry_after_ms FROM cairn_vault_release($1, $2, $3)",
            &[&staff_id, &passphrase, &purpose],
        )
        .await
        .expect("cairn_vault_release must never raise");
    Release {
        outcome: row.get(0),
        seed: row.get(1),
        signing_key_id: row.get(2),
        retry_after_ms: row.get(3),
    }
}

/// Failures since the last success, as the release door itself computes them (from the member's
/// non-transactional attempt sequence and the transactional success mark).
async fn failures(c: &Client, staff_id: &str) -> i64 {
    c.query_one(
        "SELECT failures FROM cairn_vault_attempt_state($1)",
        &[&staff_id],
    )
    .await
    .unwrap()
    .get(0)
}

/// The test seam for time: move a member's last-attempt clock to `offset_ms` from now (negative
/// = in the past). The clock is a sequence, so this is `setval` on it, as the owner.
async fn set_last_attempt(c: &Client, staff_id: &str, offset_ms: i64) {
    c.query_one(
        "SELECT setval(format('public.%I', cairn_vault_sequence_name(m.attempt_series_id, 'clock'))::regclass,
                       (extract(epoch FROM clock_timestamp()) * 1000)::bigint + $2)
           FROM vault_member m WHERE m.staff_id = $1",
        &[&staff_id, &offset_ms],
    )
    .await
    .unwrap();
}

/// The other test seam: set a member's failures since the last success to `n`, by moving the
/// attempt counter (a sequence, so `setval`, as the owner) to the success mark plus `n`.
async fn set_failures(c: &Client, staff_id: &str, n: i64) {
    c.query_one(
        "SELECT setval(format('public.%I', cairn_vault_sequence_name(m.attempt_series_id, 'attempts'))::regclass,
                       a.success_mark + $2, true)
           FROM vault_member m JOIN vault_attempt a ON a.staff_id = m.staff_id
          WHERE m.staff_id = $1",
        &[&staff_id, &n],
    )
    .await
    .unwrap();
}

fn db_message(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

#[tokio::test]
async fn release_returns_the_enrolled_key() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let (_, jsmith) = boss_and_jsmith(&c).await;
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    let seed: [u8; 32] = r
        .seed
        .expect("a released row carries the seed")
        .try_into()
        .unwrap();
    assert_eq!(kid_of(seed), jsmith);
    assert_eq!(r.signing_key_id.as_deref(), Some(jsmith.as_str()));
    assert_eq!(r.retry_after_ms, None);
}

#[tokio::test]
async fn release_normalises_the_staff_id() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let (_, jsmith) = boss_and_jsmith(&c).await;
    let r = release(&c, " JSmith ", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    assert_eq!(r.signing_key_id.as_deref(), Some(jsmith.as_str()));
}

#[tokio::test]
async fn a_wrong_passphrase_is_an_answer_not_an_error() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(r.outcome, "wrong");
    assert_eq!(r.seed, None);
    assert_eq!(failures(&c, "jsmith").await, 1);
    // An unknown staff ID is an answer too, and counts nothing (there is no member to count against).
    let r = release(&c, "nobody", "not it", "clinical").await;
    assert_eq!(r.outcome, "unknown");
    assert_eq!(r.seed, None);
}

#[tokio::test]
async fn the_fourth_failure_starts_a_back_off() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    for n in 1..=3 {
        let r = release(&c, "jsmith", "not it", "clinical").await;
        assert_eq!(r.outcome, "wrong", "failure {n}");
        assert_eq!(
            r.retry_after_ms, None,
            "failures 1-3 are free (failure {n})"
        );
    }
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(r.outcome, "wrong");
    assert!(
        r.retry_after_ms.is_some_and(|ms| ms > 0),
        "the 4th failure names a wait: {r:?}"
    );
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(r.outcome, "waiting");
    assert!(r.retry_after_ms.is_some_and(|ms| ms > 0), "{r:?}");
    assert_eq!(
        failures(&c, "jsmith").await,
        4,
        "a waiting try is not a failure"
    );
}

#[tokio::test]
async fn waiting_does_not_count_a_failure() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    // Ten failures (a 64 s back-off), the last one just now: the test cannot race the wait.
    set_failures(&c, "jsmith", 10).await;
    set_last_attempt(&c, "jsmith", 0).await;
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "waiting", "the right passphrase waits too");
    assert_eq!(r.seed, None, "waiting never unseals");
    assert!(r.retry_after_ms.is_some_and(|ms| ms > 0));
    assert_eq!(failures(&c, "jsmith").await, 10);

    set_last_attempt(&c, "jsmith", -65_000).await;
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    assert_eq!(
        failures(&c, "jsmith").await,
        0,
        "a success resets the count"
    );
    // Really reset: the next wrong guess is a free one again.
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(r.outcome, "wrong");
    assert_eq!(r.retry_after_ms, None, "{r:?}");
}

/// Final review item 3: the last attempt is stamped with the wall clock, so if the clock is then
/// stepped BACK (an NTP correction, a dead RTC battery after a power cut) the last attempt looks
/// like it is in the future. The time since it is unknown, and an unknown wait must not become
/// a lock-out — the door checks the guess (and restamps the clock at now).
#[tokio::test]
async fn a_clock_stepped_back_never_locks_out() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    set_failures(&c, "jsmith", 10).await;
    set_last_attempt(&c, "jsmith", 3_600_000).await;
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released", "{r:?}");
}

/// The other half of the clock-step rule (PR #897 review): the guess the step let through RESTAMPS
/// the clock at now, so the back-off resumes from it at once. A "monotonic" tidy-up that kept the
/// future stamp would make every guess free for the whole size of the step — years, after a dead
/// clock battery.
#[tokio::test]
async fn a_clock_stepped_back_frees_one_guess_and_no_more() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    set_failures(&c, "jsmith", 10).await;
    set_last_attempt(&c, "jsmith", 3_600_000).await;
    let first = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(
        first.outcome, "wrong",
        "the step frees one guess: {first:?}"
    );
    let second = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(second.outcome, "waiting", "and only one: {second:?}");
}

#[tokio::test]
async fn a_superuser_key_is_not_released_for_a_clinical_act() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    let r = release(&c, "boss", SU_PASS, "clinical").await;
    assert_eq!(r.outcome, "refused-purpose");
    assert_eq!(r.seed, None);
    // Refused before any guess is counted: a mis-pressed act never starts a back-off.
    assert_eq!(failures(&c, "boss").await, 0);
    // The same key IS released for the act a superuser exists for.
    assert_eq!(
        release(&c, "boss", SU_PASS, "enrol").await.outcome,
        "released"
    );
}

#[tokio::test]
async fn a_clinician_key_cannot_enrol() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    let r = release(&c, "jsmith", CLIN_PASS, "enrol").await;
    assert_eq!(r.outcome, "refused-purpose");
    assert_eq!(r.seed, None);
    assert_eq!(failures(&c, "jsmith").await, 0);
    // A purpose outside the vocabulary is refused the same way, never raised.
    assert_eq!(
        release(&c, "jsmith", CLIN_PASS, "nonsense").await.outcome,
        "refused-purpose"
    );
}

#[tokio::test]
async fn enrol_needs_a_superuser_token_over_this_request() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;

    let seed = test_seed(3);
    let kid = kid_of(seed);

    // A genuine superuser token, but over a DIFFERENT staff ID: refused, and the message never
    // echoes the token.
    let other = enrol_token(&c, "someone-else", &kid, "clinician", test_seed(1)).await;
    let e = enrol(&c, "anurse", seed, "clinician", "boss", &other)
        .await
        .expect_err("a token over another request must not enrol");
    let msg = db_message(&e);
    assert!(msg.contains("attestation"), "{msg}");
    assert!(!msg.contains(&hex::encode(&other)[..16]), "{msg}");

    // A token signed by a clinician, naming the clinician as the enroller: refused.
    let by_clinician = enrol_token(&c, "anurse", &kid, "clinician", test_seed(2)).await;
    let e = enrol(&c, "anurse", seed, "clinician", "jsmith", &by_clinician)
        .await
        .expect_err("a clinician cannot enrol");
    assert!(db_message(&e).contains("superuser"), "{}", db_message(&e));

    // The right token enrols, and the registry holds the new key as a human actor.
    let good = enrol_token(&c, "anurse", &kid, "clinician", test_seed(1)).await;
    let actor_id = enrol(&c, "anurse", seed, "clinician", "boss", &good)
        .await
        .unwrap();
    let kind: String = c
        .query_one(
            "SELECT kind FROM actor_current WHERE signing_key_id = $1 AND actor_id = $2",
            &[&kid, &actor_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(kind, "human");

    // M1: the approval is kept on the member row, and re-verifies; a console bootstrap has none.
    let ok: bool = c
        .query_one(
            "SELECT cairn_attestation_ok(m.enrol_token,
                        cairn_vault_enrol_address(m.staff_id, m.signing_key_id, m.role),
                        decode(su.signing_key_id, 'hex'))
               FROM vault_member m, vault_member su
              WHERE m.staff_id = 'anurse' AND su.staff_id = m.enrolled_by",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        ok,
        "the stored enrol token must re-verify against the enrolling superuser"
    );
    let boss_token: Option<Vec<u8>> = c
        .query_one(
            "SELECT enrol_token FROM vault_member WHERE staff_id = 'boss'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        boss_token, None,
        "a console bootstrap is approved by no superuser"
    );
}

#[tokio::test]
async fn the_runtime_role_cannot_read_the_vault() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let (_, jsmith) = boss_and_jsmith(&c).await;
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();

    let e = c
        .query("SELECT sealed FROM vault_member", &[])
        .await
        .expect_err("the runtime role must not read wrapped keys");
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );

    // Nor may it call the unwrapping primitive directly and skip the back-off.
    let e = c
        .query("SELECT cairn_vault_unseal('\\x00'::bytea, 'x')", &[])
        .await
        .expect_err("cairn_vault_release is the only door to cairn_vault_unseal");
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );

    // Nor reset its own back-off by moving the attempt clock (the sequences carry no grant).
    c.batch_execute("RESET ROLE").await.unwrap();
    let clock: String = c
        .query_one(
            "SELECT format('public.%I', cairn_vault_sequence_name(attempt_series_id, 'clock')) FROM vault_member WHERE staff_id = 'jsmith'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let e = c
        .query(&format!("SELECT setval('{clock}'::regclass, 1)"), &[])
        .await
        .expect_err("the runtime role must not move a member's attempt clock");
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );

    // The door itself still works under that role.
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    assert_eq!(r.signing_key_id.as_deref(), Some(jsmith.as_str()));
    c.batch_execute("RESET ROLE").await.unwrap();
}

/// Postgres's own CHECK error carries "Failing row contains (…)", every column printed — the
/// sealed blob included. The vault's insert re-raises with the constraint's name only.
#[tokio::test]
async fn a_refused_enrolment_never_echoes_the_sealed_blob() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let seed = test_seed(4);
    let blob = sealed_blob(seed, SU_PASS);
    let e = c
        .query_one(
            "SELECT cairn_vault_bootstrap($1, $2, $3::text::jsonb, $4, $5, $6, $7, $8)",
            &[
                &"boss",
                &"Site Superuser",
                &pinned("superuser", "boss"),
                &kid_of(seed),
                &blob,
                &None::<&str>,
                &None::<&str>, // no phone and no email: vault_member_needs_a_contact
                &None::<&str>,
            ],
        )
        .await
        .expect_err("a member nobody can reach must be refused");
    let d = e.as_db_error().expect("a database refusal");
    let said = format!("{} {:?} {:?}", d.message(), d.detail(), d.hint());
    assert!(said.contains("vault_member_needs_a_contact"), "{said}");
    assert!(!said.contains(&hex::encode(&blob)[..16]), "{said}");
    assert_eq!(d.detail(), None, "{said}");
}

/// THE BYPASS (fix round 1, Important #1): a caller who reads the outcome and then ROLLS BACK must
/// not undo the count, or any runtime client could guess at full speed. Four rolled-back wrong
/// guesses still start the back-off.
#[tokio::test]
async fn a_rolled_back_guess_still_counts() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    for n in 1..=4 {
        c.batch_execute("BEGIN").await.unwrap();
        let r = release(&c, "jsmith", "not it", "clinical").await;
        assert_eq!(r.outcome, "wrong", "guess {n}");
        c.batch_execute("ROLLBACK").await.unwrap();
    }
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(
        r.outcome, "waiting",
        "a rollback must not undo a counted guess: {r:?}"
    );
}

/// The same bypass in autocommit: a statement that reads the outcome and then ERRORS rolls its own
/// work back. Four such statements still start the back-off.
#[tokio::test]
async fn an_erroring_statement_still_counts() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    for n in 1..=4 {
        let e = c
            .query(
                "SELECT 1 / (r.outcome <> 'wrong')::int FROM cairn_vault_release($1, $2, $3) r",
                &[&"jsmith", &"not it", &"clinical"],
            )
            .await
            .expect_err("a wrong outcome divides by zero");
        assert_eq!(
            e.code(),
            Some(&tokio_postgres::error::SqlState::DIVISION_BY_ZERO),
            "guess {n}"
        );
    }
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(
        r.outcome, "waiting",
        "an erroring statement must not undo a counted guess: {r:?}"
    );
}

/// Two terminals at once. Terminal 1 guesses inside an open transaction (holding the attempt row's
/// lock); terminal 2 guesses meanwhile and is made to wait for that lock; terminal 1 then rolls
/// back. Both guesses must be accounted for: terminal 1's is counted although rolled back, and
/// terminal 2's is either counted too (`wrong`) or answered `waiting` — never silently free.
///
/// (The brief said "failures 5". That holds only if terminal 1's unseal outlasts the 1-second
/// first back-off; otherwise terminal 2 is rightly answered `waiting`, which does not count. So
/// the invariant is asserted, not a timing-dependent number.)
#[tokio::test]
async fn a_concurrent_guess_after_a_rolled_back_one_still_counts() {
    let Some(base) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    for _ in 0..3 {
        release(&c, "jsmith", "not it", "clinical").await;
    }
    assert_eq!(failures(&c, "jsmith").await, 3);

    c.batch_execute("BEGIN").await.unwrap();
    let first = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(first.outcome, "wrong");

    let c2 = db::connect(&base).await.unwrap();
    let second = tokio::spawn(async move { release(&c2, "jsmith", "not it", "clinical").await });
    // Give terminal 2 time to reach the row lock; if it has not, the test still holds (it then
    // simply runs after the rollback).
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    c.batch_execute("ROLLBACK").await.unwrap();
    let second = second.await.unwrap();
    eprintln!("terminal 2 answered: {second:?}");

    assert!(
        second.outcome == "wrong" || second.outcome == "waiting",
        "{second:?}"
    );
    let second_counted = i64::from(second.outcome == "wrong");
    assert_eq!(
        failures(&c, "jsmith").await,
        3 + 1 + second_counted,
        "terminal 1's rolled-back guess and terminal 2's guess must both be accounted for ({second:?})"
    );
    assert!(second.retry_after_ms.is_some_and(|ms| ms > 0), "{second:?}");
}

/// The row lock itself (PR #897 review: the test above passes with it deleted, because sequences
/// are not transactional and terminal 2 reads the same state either way). The door counts with an
/// absolute `setval(mark + failures + 1)`: without the lock, N terminals reading the state at the
/// same moment each get a guess and all are counted as ONE. So: hold the attempt row's lock as the
/// owner; terminal 2's release must WAIT on it (seen in pg_stat_activity, not timed); move the
/// state while it waits; and after the commit it must answer from the NEW state.
#[tokio::test]
async fn a_second_terminal_waits_for_the_attempt_row_lock() {
    let Some(base) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    c.batch_execute("BEGIN").await.unwrap();
    c.execute(
        "SELECT 1 FROM vault_attempt WHERE staff_id = 'jsmith' FOR UPDATE",
        &[],
    )
    .await
    .unwrap();

    let c2 = db::connect(&base).await.unwrap();
    let pid: i32 = c2
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let second = tokio::spawn(async move { release(&c2, "jsmith", "not it", "clinical").await });
    let observer = db::connect(&base).await.unwrap();
    let mut waiting = false;
    for _ in 0..500 {
        let row = observer
            .query_one(
                "SELECT coalesce(wait_event_type, '') FROM pg_stat_activity WHERE pid = $1",
                &[&pid],
            )
            .await
            .unwrap();
        if row.get::<_, String>(0) == "Lock" {
            waiting = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(waiting, "terminal 2 must wait on the attempt row's lock");
    assert!(
        !second.is_finished(),
        "terminal 2 answered without the lock"
    );

    // While it waits: ten failures, the last one just now.
    set_failures(&c, "jsmith", 10).await;
    set_last_attempt(&c, "jsmith", 0).await;
    c.batch_execute("COMMIT").await.unwrap();
    let second = second.await.unwrap();
    assert_eq!(
        second.outcome, "waiting",
        "terminal 2 must read the state AFTER the lock, not before it: {second:?}"
    );
}

/// M5: a token signed by a CLINICIAN's key but naming the real superuser `boss` as the enroller is
/// refused by the ATTESTATION check (boss is a superuser; the token is just not boss's).
#[tokio::test]
async fn a_clinician_token_naming_the_superuser_is_refused_at_the_attestation() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    let seed = test_seed(3);
    let forged = enrol_token(&c, "anurse", &kid_of(seed), "clinician", test_seed(2)).await;
    let e = enrol(&c, "anurse", seed, "clinician", "boss", &forged)
        .await
        .expect_err("a clinician's token must not enrol, whoever it names");
    let msg = db_message(&e);
    assert!(msg.contains("attestation"), "{msg}");
    assert!(!msg.contains("not an enrolled superuser"), "{msg}");
}

/// Fix round 2: `nextval` WAL-logs 32 values AHEAD, and crash recovery restores the logged value —
/// so after a power cut a recently active member would wake up with up to 32 phantom failures and
/// be made to wait with the RIGHT passphrase. Counting with `setval` logs the exact value. The
/// observable proxy for "nothing pre-logged that a crash could replay" is `log_cnt = 0`.
#[tokio::test]
async fn a_count_is_logged_exactly() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    assert_eq!(
        release(&c, "jsmith", "not it", "clinical").await.outcome,
        "wrong"
    );
    let counter: String = c
        .query_one(
            "SELECT format('public.%I', cairn_vault_sequence_name(attempt_series_id, 'attempts')) FROM vault_member WHERE staff_id = 'jsmith'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let (last, pre_logged): (i64, i64) = {
        let r = c
            .query_one(&format!("SELECT last_value, log_cnt FROM {counter}"), &[])
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };
    assert_eq!(last, 1, "one counted attempt");
    assert_eq!(
        pre_logged, 0,
        "a crash must replay exactly the counted attempts, no more"
    );
}

/// Fix round 2: a member whose attempt row is missing must fail CLOSED. Without the row there is
/// nothing to lock and no success mark, so the door could not back off — it answers `unknown`,
/// unsealing nothing, even for the right passphrase.
#[tokio::test]
async fn a_missing_attempt_row_fails_closed() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    c.execute("DELETE FROM vault_attempt WHERE staff_id = 'jsmith'", &[])
        .await
        .unwrap();
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "unknown", "{r:?}");
    assert_eq!(r.seed, None);
}

/// R11: the window asks "is this staff ID taken?" BEFORE it releases a superuser's key. The
/// predicate normalises like every other door, answers for the runtime role, and reveals only
/// whether an ID is enrolled (the roster is not secret; `cairn_vault_hint` already says as much).
#[tokio::test]
async fn staff_id_taken_normalises_and_answers() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let _ = boss_and_jsmith(&c).await;
    async fn taken(c: &Client, id: &str) -> bool {
        c.query_one("SELECT cairn_vault_staff_id_taken($1)", &[&id])
            .await
            .unwrap()
            .get(0)
    }
    assert!(taken(&c, " JSmith ").await);
    assert!(!taken(&c, "nobody-here").await);
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    assert!(taken(&c, "jsmith").await, "callable by the runtime role");
    assert!(!taken(&c, "nobody-here").await);
    c.batch_execute("RESET ROLE").await.unwrap();
}

/// Final review item 2: the floor BUILDS the pinned set it accepts — `{role, handle}` with the
/// handle the normalised staff ID — and refuses any other, so a raw-SQL caller cannot enrol a
/// vault member under an actor whose determinants say something else. Three ways to get it wrong,
/// at both doors: another handle, an extra determinant, and the handle as typed, not normalised.
#[tokio::test]
async fn both_doors_refuse_a_pinned_set_they_did_not_build() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let wrong = [
        ("another handle", pinned("superuser", "someone-else")),
        (
            "an extra determinant",
            r#"{"role":"superuser","handle":"boss","registration_id":"MED-1"}"#.to_string(),
        ),
        (
            "a typed, not normalised, handle",
            pinned("superuser", " Boss "),
        ),
    ];
    for (what, set) in &wrong {
        let e = bootstrap(&c, " Boss ", set, test_seed(1))
            .await
            .expect_err(what);
        assert!(
            db_message(&e).contains("pinned"),
            "{what}: {}",
            db_message(&e)
        );
    }
    bootstrap(&c, " Boss ", &pinned("superuser", "boss"), test_seed(1))
        .await
        .expect("the set the floor builds is accepted");

    let seed = test_seed(3);
    let token = enrol_token(&c, " ANurse ", &kid_of(seed), "clinician", test_seed(1)).await;
    let wrong = [
        ("another handle", pinned("clinician", "someone-else")),
        (
            "an extra determinant",
            r#"{"role":"clinician","handle":"anurse","registration_id":"MED-1"}"#.to_string(),
        ),
        (
            "a typed, not normalised, handle",
            pinned("clinician", " ANurse "),
        ),
    ];
    for (what, set) in &wrong {
        let e = enrol_pinned(&c, " ANurse ", seed, "clinician", set, "boss", &token)
            .await
            .expect_err(what);
        assert!(
            db_message(&e).contains("pinned"),
            "{what}: {}",
            db_message(&e)
        );
    }
    enrol_pinned(
        &c,
        " ANurse ",
        seed,
        "clinician",
        &pinned("clinician", "anurse"),
        "boss",
        &token,
    )
    .await
    .expect("the set the floor builds is accepted");
}

/// Final review item 2's point: because the handle is the NORMALISED staff ID, a case variant of
/// a staff ID is the same actor. Enrol "Boss", empty the vault (a restore that kept the registry),
/// enrol "boss" with a new key: the registry refuses it as prior history of the SAME actor_id
/// (enroll_actor's #152 check), rather than minting a second actor for one person.
#[tokio::test]
async fn a_case_variant_of_a_staff_id_is_the_same_actor() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let first = bootstrap(&c, "Boss", &pinned("superuser", "boss"), test_seed(1))
        .await
        .unwrap();
    c.batch_execute("DELETE FROM vault_attempt; DELETE FROM vault_member")
        .await
        .unwrap();
    let e = bootstrap(&c, "boss", &pinned("superuser", "boss"), test_seed(5))
        .await
        .expect_err("a second key for the same actor is not a fresh enrolment");
    assert!(
        db_message(&e).contains("prior registration history"),
        "{}",
        db_message(&e)
    );
    let actors: Vec<Vec<u8>> = c
        .query("SELECT DISTINCT actor_id FROM actor_event", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(actors, vec![first], "one person, one actor");
}

/// Revoke every actor enrolled under `kid` (the registry is append-only: a `revoke` event).
async fn revoke(c: &Client, kid: &str) {
    c.execute(
        "INSERT INTO actor_event (actor_id, op) \
         SELECT DISTINCT actor_id, 'revoke' FROM actor_event WHERE signing_key_id = $1",
        &[&kid],
    )
    .await
    .unwrap();
}

/// A revoked superuser enrols nobody, even with a valid token from its own key (PR #897 review:
/// `cairn_vault_enrol`'s `actor_current` check had no test, so a revoked — perhaps compromised —
/// superuser key could have kept enrolling staff with nothing going red).
#[tokio::test]
async fn a_revoked_superuser_cannot_enrol() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let boss = bootstrap_boss(&c).await;
    revoke(&c, &boss).await;
    let seed = test_seed(3);
    let token = enrol_token(&c, "anurse", &kid_of(seed), "nurse", test_seed(1)).await;
    let e = enrol(&c, "anurse", seed, "nurse", "boss", &token)
        .await
        .expect_err("a revoked superuser must not enrol");
    assert!(
        db_message(&e).contains("not a current"),
        "{}",
        db_message(&e)
    );
}

/// A revoked member's key is not released: every act it signed would be refused by the floor, and
/// the clinician should be told why rather than see their act fail (PR #897 review). Answered
/// before any guess is counted, with or without the right passphrase.
#[tokio::test]
async fn a_revoked_member_is_told_so_and_nothing_is_unsealed_or_counted() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let (_boss, jsmith) = boss_and_jsmith(&c).await;
    revoke(&c, &jsmith).await;
    for pass in [CLIN_PASS, "not it"] {
        let r = release(&c, "jsmith", pass, "clinical").await;
        assert_eq!(r.outcome, "not-current", "{r:?}");
        assert_eq!(r.seed, None);
    }
    assert_eq!(failures(&c, "jsmith").await, 0);
}

/// Postgres's own NOT NULL error carries "Failing row contains (…)" too, sealed blob and all; the
/// re-raise must catch it like the CHECK above (PR #897 review: only the CHECK was pinned).
#[tokio::test]
async fn a_not_null_refusal_never_echoes_the_sealed_blob() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let seed = test_seed(4);
    let blob = sealed_blob(seed, SU_PASS);
    let e = c
        .query_one(
            "SELECT cairn_vault_bootstrap($1, $2, $3::text::jsonb, $4, $5, $6, $7, $8)",
            &[
                &"boss",
                &None::<&str>, // no display name
                &pinned("superuser", "boss"),
                &kid_of(seed),
                &blob,
                &None::<&str>,
                &Some("0400 000 000"),
                &None::<&str>,
            ],
        )
        .await
        .expect_err("a member with no name must be refused");
    let d = e.as_db_error().expect("a database refusal");
    let said = format!("{} {:?} {:?}", d.message(), d.detail(), d.hint());
    assert!(!said.contains(&hex::encode(&blob)[..16]), "{said}");
    assert_eq!(d.detail(), None, "{said}");
}

/// A superuser's token binds the ROLE: a token over `(anurse, kid, nurse)` cannot enrol that same
/// key and ID as a superuser (PR #897 review: the address's role binding was pinned only by the
/// SQL mirror, so a door that stopped passing `p_role` would have passed every test).
#[tokio::test]
async fn a_token_for_one_role_cannot_enrol_another() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    bootstrap_boss(&c).await;
    let seed = test_seed(3);
    let token = enrol_token(&c, "anurse", &kid_of(seed), "nurse", test_seed(1)).await;
    let e = enrol(&c, "anurse", seed, "superuser", "boss", &token)
        .await
        .expect_err("a nurse's token must not enrol a superuser");
    assert!(db_message(&e).contains("attestation"), "{}", db_message(&e));
}

/// db/061 refuses to LOAD while the runtime role can call the unthrottled primitive (PR #897
/// review). Its `REVOKE … FROM PUBLIC` is only a WARNING, and a no-op, when the loader does not own
/// the extension's functions, and then `cairn_agent` could skip every count by calling
/// `cairn_vault_unseal` directly. Simulated with a direct grant, which that REVOKE does not reach.
#[tokio::test]
async fn the_schema_refuses_to_load_while_the_runtime_role_can_unseal() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    c.batch_execute("GRANT EXECUTE ON FUNCTION cairn_vault_unseal(bytea, text) TO cairn_agent")
        .await
        .unwrap();
    let loaded = c
        .batch_execute(include_str!("../../../db/061_vault.sql"))
        .await;
    // Restore before asserting, so a failure here cannot leave the grant behind for other suites.
    c.batch_execute("ROLLBACK").await.unwrap();
    c.batch_execute("REVOKE EXECUTE ON FUNCTION cairn_vault_unseal(bytea, text) FROM cairn_agent")
        .await
        .unwrap();
    let e = loaded.expect_err("the load must refuse");
    assert!(
        db_message(&e).contains("can still execute cairn_vault_unseal"),
        "{}",
        db_message(&e)
    );
}
