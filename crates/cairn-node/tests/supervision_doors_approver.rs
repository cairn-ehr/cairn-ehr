//! DB-gated: the APPROVER rule of db/072's supervision door (`cairn_supervision_approver`), each
//! check on its own. `supervision_doors.rs` drives the matrix; here every token is signed over a
//! FRESH address (the current prior), so a refusal can only come from the check under test:
//!
//! * a token replayed after the state moved (the prior binds it to one state);
//! * a token signed by any key but the approver's own;
//! * an approver whose key is no longer current (repudiated) — even a confirmed one before;
//! * an approver who is not a member at all.
//!
//! Fixtures (runtime-derived keys, the real enrolment doors) are `common/vault_enroller_kit.rs`.
mod common;
#[path = "common/vault_enroller_kit.rs"]
#[macro_use]
mod kit;

use kit::*;
use tokio_postgres::Client;

const LINK: &str = "link-supervisor";
const UNLINK: &str = "unlink-supervisor";
const REQUIRE: &str = "require-cosign";
const ACT: &str = "SELECT cairn_supervision_act($1, $2, $3, $4, $5)";

/// boss (superuser); clinicians jsmith and bwong, enrolled (and confirmed) by boss.
async fn pair(c: &Client) {
    boot(c).await;
    for (s, v) in [("jsmith", JSMITH), ("bwong", BWONG)] {
        enrol(c, s, "clinician", v, "boss", BOSS).await.unwrap();
    }
}

/// The address of `op` on `subject` (and `supervisor`) at the CURRENT prior.
async fn fresh_address(c: &Client, op: &str, subject: &str, supervisor: Option<&str>) -> Vec<u8> {
    bytes(
        c,
        "SELECT cairn_supervision_address($1, $2, $3, cairn_supervision_prior($1, $2, $3))",
        &[&op, &subject, &supervisor],
    )
    .await
}

#[tokio::test]
async fn a_stale_token_is_refused() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    pair(&c).await;
    // A token signed over the state BEFORE a link, replayed after the link and its unlink.
    let (s, p) = ("bwong", Some("jsmith"));
    let token = sign(&fresh_address(&c, LINK, s, p).await, BOSS);
    c.execute(ACT, &[&LINK, &s, &p, &"boss", &token])
        .await
        .unwrap();
    acted(&c, UNLINK, s, p, "boss", BOSS).await;
    refused(
        c.execute(ACT, &[&LINK, &s, &p, &"boss", &token])
            .await
            .map(drop),
        "the approver's attestation is invalid or does not cover this act",
    );
}

#[tokio::test]
async fn a_token_by_another_key_than_the_approvers_is_refused() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    pair(&c).await;
    // A fresh address, approver boss, but signed with jsmith's (valid, confirmed) key.
    let none: Option<&str> = None;
    let wrong = sign(&fresh_address(&c, REQUIRE, "bwong", none).await, JSMITH);
    refused(
        c.execute(ACT, &[&REQUIRE, &"bwong", &none, &"boss", &wrong])
            .await
            .map(drop),
        "the approver's attestation is invalid or does not cover this act",
    );
    // The same fresh address signed by boss's own key is accepted.
    acted(&c, REQUIRE, "bwong", none, "boss", BOSS).await;
}

#[tokio::test]
async fn an_approver_whose_key_is_not_current_is_refused() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    pair(&c).await;
    acted(&c, "grant-assigner", "jsmith", None, "boss", BOSS).await;
    // A superuser repudiates jsmith's key: the member row still names it, but it is current no
    // more — so jsmith approves nothing, over a fresh address, signed with that very key.
    judge(&c, JSMITH, false, "boss", BOSS).await.unwrap();
    refused(
        supervise(&c, REQUIRE, "bwong", None, "jsmith", JSMITH).await,
        "jsmith's key is not current (revoked, superseded, rotated away or repudiated)",
    );
}

#[tokio::test]
async fn a_non_member_cannot_approve() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    pair(&c).await;
    refused(
        supervise(&c, REQUIRE, "bwong", None, "ghost", BOSS).await,
        "ghost is not an enrolled member, so cannot approve",
    );
}
