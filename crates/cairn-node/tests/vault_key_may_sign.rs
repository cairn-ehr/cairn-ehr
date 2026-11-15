//! DB-gated: #1138 item 1 — a HELD key is re-checked each time it is used.
//!
//! The window may keep a released key for the next act (a key-hold terminal, slice 1). Before this
//! check, a held key was used with no database call, so a re-key or a repudiation done at another
//! terminal left it "Signed in for the next act" here. The floor still refused what it signed
//! (`actor_current`), but the window said nothing true about why, and a medication list told its own
//! writer "only they can sign it", because the list's writer key was the NEW one.
//!
//! `cairn_vault_key_may_sign(key)` (db/063) answers the release door's admission question for a key
//! the window already holds, with no passphrase: is it still current, and not past the site's expiry
//! for unconfirmed keys? `cairn_node::vault::key_may_sign` is how the window asks.
#[path = "common/vault_enroller_kit.rs"]
#[macro_use]
mod kit;

use kit::*;
use tokio_postgres::Client;

/// The window's question, asked as the window asks it.
async fn may_sign(c: &Client, v: u8) -> bool {
    cairn_node::vault::key_may_sign(c, &kid_of(v))
        .await
        .expect("the check answers")
}

#[tokio::test]
async fn a_held_key_may_sign_only_while_the_release_door_would_release_it() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    assert!(may_sign(&c, JSMITH).await, "a freshly enrolled key signs");

    // A re-key elsewhere: the old key is rotated away, the new one signs.
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
    assert!(
        !may_sign(&c, JSMITH).await,
        "a rotated-away key no longer signs"
    );
    assert!(may_sign(&c, JSMITH2).await);

    // A repudiation: the key is current in no sense.
    judge(&c, JSMITH2, false, "boss", BOSS).await.unwrap();
    assert!(
        !may_sign(&c, JSMITH2).await,
        "a repudiated key no longer signs"
    );

    // The site's expiry for unconfirmed keys (§5.5), as the release door applies it.
    enrol(&c, "anurse", "nurse", ANURSE, "bwong", BWONG)
        .await
        .unwrap();
    age_key(&c, ANURSE, 31).await;
    assert!(may_sign(&c, ANURSE).await, "no expiry is set by default");
    set_expiry(&c, Some(30)).await;
    assert!(
        !may_sign(&c, ANURSE).await,
        "an expired unconfirmed key no longer signs"
    );

    // A key the registry never enrolled.
    assert!(!may_sign(&c, 250).await);
}

/// The window's login may ask; PUBLIC and the advisory role may not (#1129: the human doors are
/// `cairn_window`'s, never `cairn_agent`'s).
#[tokio::test]
async fn only_the_window_may_ask() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    let grants: (bool, bool, bool) = c
        .query_one(
            "SELECT has_function_privilege('cairn_window', 'cairn_vault_key_may_sign(text)', 'EXECUTE'), \
                    has_function_privilege('cairn_agent', 'cairn_vault_key_may_sign(text)', 'EXECUTE'), \
                    has_function_privilege('public', 'cairn_vault_key_may_sign(text)', 'EXECUTE')",
            &[],
        )
        .await
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .unwrap();
    assert_eq!(grants, (true, false, false));
}
