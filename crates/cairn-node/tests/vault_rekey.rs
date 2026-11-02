//! DB-gated: GUI sign-in slice 2's re-key and reseal doors (db/063; spec 2026-11-01-sign-in-slice-2,
//! §5.3, §5.4, §5.8). Grants, enrolment and judging are `vault_enrollers.rs`; the shared fixtures
//! are `common/vault_enroller_kit.rs`.
//!
//! * a re-key keeps the ACTOR and retires the old key; the old passphrase then answers
//!   `'replaced'` (counted exactly like `'wrong'`), so the real owner learns of it;
//! * after a restore (no member row, registry intact) the re-key doors re-create the member on the
//!   same actor — by a superuser or enroller in person, or by the console for a superuser;
//! * a change of passphrase needs the member's OWN signed proof over the exact new blob, and every
//!   role may release its key for that purpose.
#[path = "common/vault_enroller_kit.rs"]
#[macro_use]
mod kit;

use kit::*;

#[tokio::test]
async fn rekey_keeps_the_actor_and_retires_the_old_key() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let actor = actor_of(&c, "jsmith").await;
    for _ in 0..3 {
        assert_eq!(release(&c, "jsmith", "not it", "clinical").await.0, "wrong");
    }
    let back = rekey(
        &c,
        "jsmith",
        "clinician",
        None,
        None,
        JSMITH2,
        "bwong",
        BWONG,
    )
    .await
    .unwrap();
    assert_eq!(back, actor, "a re-key keeps the actor");
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(JSMITH2)));
    let failures = "SELECT failures FROM cairn_vault_attempt_state('jsmith')";
    let f: i64 = c.query_one(failures, &[]).await.unwrap().get(0);
    assert_eq!(f, 0, "a re-key resets the failure count");

    // The real owner, typing the passphrase they know, is told — counted like a wrong guess.
    let (o, seed, retry) = release(&c, "jsmith", &pass_of(JSMITH), "clinical").await;
    assert_eq!((o.as_str(), seed, retry), ("replaced", None, None));
    let f: i64 = c.query_one(failures, &[]).await.unwrap().get(0);
    assert_eq!(f, 1, "'replaced' counts like 'wrong'");
    let notice = c
        .query_one("SELECT replaced_at IS NOT NULL, approver_display_name FROM cairn_vault_replaced_notice(' JSmith ')", &[])
        .await
        .unwrap();
    assert!(notice.get::<_, bool>(0));
    assert_eq!(notice.get::<_, String>(1), display("bwong"));
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "clinical").await.0,
        "released"
    );
    assert!(c
        .query("SELECT * FROM cairn_vault_replaced_notice('bwong')", &[])
        .await
        .unwrap()
        .is_empty());

    // Counted EXACTLY like 'wrong': the 4th 'replaced' in a row starts the same back-off as the
    // 4th 'wrong' of a member never re-keyed (who gets a bare 'wrong').
    let mut replaced_retry = None;
    for _ in 0..4 {
        let (o, _, retry) = release(&c, "jsmith", &pass_of(JSMITH), "clinical").await;
        assert_eq!(o, "replaced");
        replaced_retry = retry;
    }
    let mut wrong_retry = None;
    for _ in 0..4 {
        let (o, _, retry) = release(&c, "bwong", "not it", "clinical").await;
        assert_eq!(o, "wrong");
        wrong_retry = retry;
    }
    assert_eq!(
        replaced_retry,
        Some(1000),
        "the 4th failure starts a 1 s back-off"
    );
    assert_eq!(replaced_retry, wrong_retry);
}

/// §5.8, the console's half: after a restore no superuser can sign, so the owner-only console door
/// re-keys the first superuser on their EXISTING actor, approved by 'console' and confirmed at once.
#[tokio::test]
async fn bootstrap_rekey_restores_a_superuser() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    let actor = actor_of(&c, "boss").await;
    enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    c.batch_execute("DELETE FROM vault_attempt; DELETE FROM vault_member;")
        .await
        .unwrap();
    let door = "SELECT cairn_vault_bootstrap_rekey($1, 'Dr boss', $2, $3, NULL, '0400', NULL)";

    let back: Vec<u8> = c
        .query_one(door, &[&"boss", &kid_of(BOSS2), &blob(BOSS2)])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        back, actor,
        "the console re-key keeps the superuser's actor"
    );
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(BOSS2)));
    assert_eq!(status(&c, BOSS2).await, "confirmed");
    let rows: Vec<String> = c
        .query(
            "SELECT op || ':' || approved_by FROM actor_event WHERE signing_key_id = $1 ORDER BY seq",
            &[&kid_of(BOSS2)],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(rows, ["rotate-key:console", "confirm:console"]);
    assert_eq!(
        release(&c, "boss", &pass_of(BOSS2), "enrol").await.0,
        "released"
    );

    // A clinician's handle is no superuser actor: the console door refuses it.
    let e = c
        .query_one(door, &[&"jsmith", &kid_of(JSMITH2), &blob(JSMITH2)])
        .await
        .expect_err("the console re-keys superusers only");
    assert!(msg(&e).contains("registry"), "{}", msg(&e));
}

/// Review Focus 2: an impostor's re-key, repudiated, leaves the actor with no key — not revoked —
/// and a proper re-key restores it.
#[tokio::test]
async fn rekey_after_repudiation_restores_the_actor() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let actor = actor_of(&c, "jsmith").await;
    rekey(
        &c,
        "jsmith",
        "clinician",
        None,
        None,
        JSMITH2,
        "bwong",
        BWONG,
    )
    .await
    .unwrap();
    judge(&c, JSMITH2, false, "boss", BOSS).await.unwrap();
    assert_eq!(current_key(&c, &actor).await, None);
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "clinical").await.0,
        "not-current"
    );
    let e = judge(&c, JSMITH2, true, "boss", BOSS).await.unwrap_err();
    assert!(msg(&e).contains("final"), "{}", msg(&e));

    // #926: a repudiation is a superuser's judgement, so an ENROLLER may not undo it a minute
    // later with a fresh key for the same handle. The refused attempt wrote nothing (JSMITH3 is
    // still a new key for the superuser's re-key below).
    let e = rekey(
        &c,
        "jsmith",
        "clinician",
        None,
        None,
        JSMITH3,
        "bwong",
        BWONG,
    )
    .await
    .unwrap_err();
    assert!(msg(&e).contains("only a superuser"), "{}", msg(&e));
    assert_eq!(current_key(&c, &actor).await, None);

    assert_eq!(
        rekey(&c, "jsmith", "clinician", None, None, JSMITH3, "boss", BOSS)
            .await
            .unwrap(),
        actor
    );
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(JSMITH3)));
    assert_eq!(status(&c, JSMITH3).await, "confirmed");
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH3), "clinical").await.0,
        "released"
    );
}

/// §5.8: after a restore the registry is back and the vault is empty; the re-key door re-creates
/// the member on the SAME actor.
#[tokio::test]
async fn rekey_without_a_member_row_recreates_it() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    let actor = enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'jsmith'; DELETE FROM vault_member WHERE staff_id = 'jsmith'",
    )
    .await
    .unwrap();
    let e = rekey(
        &c,
        "jsmith",
        "clinician",
        None,
        Some("j@example.org"),
        JSMITH2,
        "boss",
        BOSS,
    )
    .await
    .unwrap_err();
    assert!(msg(&e).contains("display name"), "{}", msg(&e));
    let e = rekey(
        &c,
        "nobody",
        "clinician",
        Some("N"),
        Some("n@x"),
        JSMITH2,
        "boss",
        BOSS,
    )
    .await
    .unwrap_err();
    assert!(msg(&e).contains("registry"), "{}", msg(&e));

    let back = rekey(
        &c,
        "jsmith",
        "clinician",
        Some("Dr J Smith"),
        Some("j@example.org"),
        JSMITH2,
        "boss",
        BOSS,
    )
    .await
    .unwrap();
    assert_eq!(back, actor);
    assert_eq!(actor_of(&c, "jsmith").await, actor);
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(JSMITH2)));
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "clinical").await.0,
        "released"
    );
}

/// Final review I4: the no-member-row branch re-creates custody on an EXISTING actor from nothing
/// but its `{role, handle}` shape — which a CLI-enrolled human (enroll.rs) shares. So only a
/// superuser may take it (an enroller would otherwise take over any such actor), and only for a
/// person's actor (never a device or agent that happens to carry that pinned shape).
#[tokio::test]
async fn rekey_without_a_member_row_needs_a_superuser_and_a_person() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'jsmith'; DELETE FROM vault_member WHERE staff_id = 'jsmith'",
    )
    .await
    .unwrap();
    let e = rekey(
        &c,
        "jsmith",
        "clinician",
        Some("Dr J Smith"),
        Some("j@example.org"),
        JSMITH2,
        "bwong",
        BWONG,
    )
    .await
    .expect_err("an enroller may not re-create a member row");
    assert!(msg(&e).contains("superuser"), "{}", msg(&e));

    // A device carrying the pinned shape of a person is no person: refused even for a superuser.
    c.execute(
        "SELECT enroll_actor('device', $1::text::jsonb, $2)",
        &[&pinned("clinician", "dev1"), &kid_of(ANURSE)],
    )
    .await
    .unwrap();
    let e = rekey(
        &c,
        "dev1",
        "clinician",
        Some("Device"),
        Some("d@example.org"),
        BNURSE,
        "boss",
        BOSS,
    )
    .await
    .expect_err("a non-human actor is never re-keyed into the vault");
    assert!(msg(&e).contains("person"), "{}", msg(&e));
}

#[tokio::test]
async fn reseal_needs_the_members_own_proof() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let fresh_blob = blob_under(JSMITH, "a new passphrase");
    let a = bytes(
        &c,
        "SELECT cairn_vault_reseal_address('jsmith', $1)",
        &[&fresh_blob],
    )
    .await;
    let reseal = "SELECT cairn_vault_reseal('jsmith', $1, 'new hint', $2)";

    let e = c
        .execute(reseal, &[&fresh_blob, &sign(&a, BOSS)])
        .await
        .unwrap_err();
    assert!(msg(&e).contains("proof"), "{}", msg(&e));
    let other = bytes(
        &c,
        "SELECT cairn_vault_reseal_address('jsmith', $1)",
        &[&blob(JSMITH)],
    )
    .await;
    let e = c
        .execute(reseal, &[&fresh_blob, &sign(&other, JSMITH)])
        .await
        .unwrap_err();
    assert!(msg(&e).contains("proof"), "{}", msg(&e));
    assert!(
        !msg(&e).contains(&hex::encode(&fresh_blob)[..16]),
        "never echo the blob"
    );

    c.execute(reseal, &[&fresh_blob, &sign(&a, JSMITH)])
        .await
        .unwrap();
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH), "clinical").await.0,
        "wrong"
    );
    let (o, seed, _) = release(&c, "jsmith", "a new passphrase", "clinical").await;
    assert_eq!(
        (o.as_str(), seed),
        ("released", Some(test_seed(JSMITH).to_vec()))
    );
    let hint: Option<String> = c
        .query_one("SELECT cairn_vault_hint('jsmith')", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(hint.as_deref(), Some("new hint"));
    // The proof binds the blob it replaced: replaying it now (the stored blob moved on) fails.
    let e = c
        .execute(reseal, &[&fresh_blob, &sign(&a, JSMITH)])
        .await
        .unwrap_err();
    assert!(msg(&e).contains("proof"), "{}", msg(&e));
}

/// The purpose map: every role may release its key to re-seal it; a superuser or an admin never
/// for a clinical act; a clinician for an enrolment only while an enroller.
#[tokio::test]
async fn reseal_releases_the_key_for_reseal_only() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    enrol(&c, "aadmin", "admin", ADMIN, "boss", BOSS)
        .await
        .unwrap();
    for (s, v, released, refused) in [
        ("boss", BOSS, ["enrol", "reseal"], "clinical"),
        ("aadmin", ADMIN, ["enrol", "reseal"], "clinical"),
        ("jsmith", JSMITH, ["clinical", "reseal"], "enrol"),
    ] {
        for p in released {
            assert_eq!(
                release(&c, s, &pass_of(v), p).await.0,
                "released",
                "{s} {p}"
            );
        }
        assert_eq!(
            release(&c, s, &pass_of(v), refused).await.0,
            "refused-purpose",
            "{s} {refused}"
        );
    }
}

/// A judgement acts only on the key the person signs with NOW (the floor, so two terminals
/// cannot race it). The superuser's panel reads its lists once; a re-key in between leaves the
/// row naming the OLD key. Repudiating that would hit the genuine person's rotated-away key and
/// leave the new one (an impostor's, in the case the button exists for) live and unjudged.
#[tokio::test]
async fn a_rotated_away_key_cannot_be_judged() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    for (new, approver, by) in [(JSMITH2, "bwong", BWONG), (JSMITH3, "bwong", BWONG)] {
        rekey(&c, "jsmith", "clinician", None, None, new, approver, by)
            .await
            .unwrap();
    }
    // JSMITH (confirmed) and JSMITH2 (unconfirmed) are both rotated away; JSMITH3 is current.
    for (old, confirm) in [(JSMITH, false), (JSMITH2, false), (JSMITH2, true)] {
        let e = judge(&c, old, confirm, "boss", BOSS).await.unwrap_err();
        assert!(msg(&e).contains("not the current key"), "{}", msg(&e));
    }
    assert_eq!(status(&c, JSMITH).await, "confirmed");
    assert_eq!(status(&c, JSMITH2).await, "unconfirmed");
    assert_eq!(status(&c, JSMITH3).await, "unconfirmed");
    judge(&c, JSMITH3, false, "boss", BOSS).await.unwrap();
    assert_eq!(status(&c, JSMITH3).await, "repudiated");
}

/// #926: a superuser's key is never repudiated from the window — not their own, not another
/// superuser's — so a stolen superuser key cannot strip every other superuser and leave itself
/// the only one. The owner's console does it (`vault-repudiate-superuser-key`), and the remedy is
/// the console re-key, as for a locked-out superuser.
#[tokio::test]
async fn a_superusers_key_is_repudiated_only_at_the_console() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let actor = actor_of(&c, "boss").await;
    enrol(&c, "su2", "superuser", SU2, "boss", BOSS)
        .await
        .unwrap();
    let su2 = actor_of(&c, "su2").await;
    // Own key, a fellow superuser's either way round: all refused, both keys still current.
    for (key, su, by) in [
        (BOSS, "boss", BOSS),
        (SU2, "boss", BOSS),
        (BOSS, "su2", SU2),
    ] {
        let e = judge(&c, key, false, su, by).await.unwrap_err();
        assert!(
            msg(&e).contains("vault-repudiate-superuser-key"),
            "{}",
            msg(&e)
        );
    }
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(BOSS)));
    assert_eq!(current_key(&c, &su2).await, Some(kid_of(SU2)));

    let door = "SELECT cairn_vault_bootstrap_repudiate($1)";
    let kid: String = c.query_one(door, &[&" Boss "]).await.unwrap().get(0);
    assert_eq!(kid, kid_of(BOSS), "the console names the key it repudiated");
    assert_eq!(status(&c, BOSS).await, "repudiated");
    assert_eq!(current_key(&c, &actor).await, None);
    assert_eq!(
        release(&c, "boss", &pass_of(BOSS), "enrol").await.0,
        "not-current"
    );
    let rows: Vec<String> = c
        .query(
            "SELECT approved_by FROM actor_event WHERE op = 'repudiate' AND signing_key_id = $1",
            &[&kid_of(BOSS)],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(rows, ["console"]);

    // Nothing current is left to repudiate; a clinician is no superuser actor.
    let e = c.query_one(door, &[&"boss"]).await.unwrap_err();
    assert!(msg(&e).contains("no current key"), "{}", msg(&e));
    let e = c.query_one(door, &[&"jsmith"]).await.unwrap_err();
    assert!(msg(&e).contains("no current key"), "{}", msg(&e));

    // The remedy: the console re-key, on the same actor.
    c.query_one(
        "SELECT cairn_vault_bootstrap_rekey('boss', NULL, $1, $2, NULL, NULL, NULL)",
        &[&kid_of(BOSS2), &blob(BOSS2)],
    )
    .await
    .unwrap();
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(BOSS2)));
    // The door repudiates the CURRENT key, never the one it repudiated before.
    let kid: String = c.query_one(door, &[&"boss"]).await.unwrap().get(0);
    assert_eq!(kid, kid_of(BOSS2));
}

/// Waits until backend `pid` is blocked on a lock (the `vault_floor.rs` concurrency pattern).
async fn wait_until_blocked(pid: i32) {
    let observer = cairn_node::db::connect(&cs().unwrap()).await.unwrap();
    for _ in 0..500 {
        let row = observer
            .query_one(
                "SELECT coalesce(wait_event_type, '') FROM pg_stat_activity WHERE pid = $1",
                &[&pid],
            )
            .await
            .unwrap();
        if row.get::<_, String>(0) == "Lock" {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("backend {pid} never waited on a lock");
}

/// An enroller's re-key on a second terminal, with its backend pid, so a test can hold a lock on
/// the first terminal and watch the re-key wait for it.
async fn enroller_rekey_elsewhere(
    new: u8,
) -> (
    i32,
    tokio::task::JoinHandle<Result<Vec<u8>, tokio_postgres::Error>>,
) {
    let c2 = cairn_node::db::connect(&cs().unwrap()).await.unwrap();
    let pid: i32 = c2
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let task = tokio::spawn(async move {
        rekey(&c2, "jsmith", "clinician", None, None, new, "bwong", BWONG).await
    });
    (pid, task)
}

/// #926 race: a repudiation committing WHILE an enroller's re-key waits is seen by that re-key,
/// because the re-key reads the confirmation only once it holds the actor lock. The tempting
/// wrong fix — reading the status before taking the lock — passes every other test and fails this.
#[tokio::test]
async fn a_repudiation_landing_mid_rekey_is_seen() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let actor = actor_of(&c, "jsmith").await;
    c.batch_execute("BEGIN").await.unwrap();
    judge(&c, JSMITH, false, "boss", BOSS).await.unwrap(); // holds the actor lock, uncommitted
    let (pid, second) = enroller_rekey_elsewhere(JSMITH2).await;
    wait_until_blocked(pid).await;
    c.batch_execute("COMMIT").await.unwrap();
    let e = second.await.unwrap().unwrap_err();
    assert!(msg(&e).contains("only a superuser"), "{}", msg(&e));
    assert_eq!(current_key(&c, &actor).await, None);
}

/// #928 review: an enroller's re-key takes the member's ROW lock before the actor lock — the order
/// a superuser's re-key takes them (`cairn_vault_install_key`'s `FOR UPDATE`, then
/// `cairn_actor_append`). In the opposite order the two deadlock when they re-key one person at the
/// same moment. Here terminal 1 holds the row, as a superuser's re-key would; the enroller's re-key
/// must wait for it holding nothing, so terminal 1 can still take the actor lock.
#[tokio::test]
async fn an_enrollers_rekey_takes_the_row_lock_first() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let actor = actor_of(&c, "jsmith").await;
    c.batch_execute("BEGIN").await.unwrap();
    c.execute(
        "SELECT 1 FROM vault_member WHERE staff_id = 'jsmith' FOR UPDATE",
        &[],
    )
    .await
    .unwrap();
    let (pid, second) = enroller_rekey_elsewhere(JSMITH2).await;
    wait_until_blocked(pid).await;
    let free: bool = c
        .query_one(
            "SELECT pg_try_advisory_xact_lock(hashtextextended(encode($1, 'hex'), 0))",
            &[&actor],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        free,
        "the enroller's re-key holds the actor lock while it waits for the row — a deadlock"
    );
    c.batch_execute("COMMIT").await.unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(JSMITH2)));
}
