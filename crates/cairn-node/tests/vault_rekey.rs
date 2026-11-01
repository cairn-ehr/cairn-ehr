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
