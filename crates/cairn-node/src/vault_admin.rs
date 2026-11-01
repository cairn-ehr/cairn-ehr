//! The Rust side of the vault's slice-2 doors (db/063): re-key, grants, judging a key, change of
//! passphrase, and the two worklists. The window calls only this module; the console verb
//! `vault-rekey-superuser` calls [`rekey_superuser_from_console`].
//!
//! Same shape as `vault::enrol_member`: every address to sign comes FROM THE DATABASE (so staff-ID
//! normalisation has one home), the signature is a `sign_attestation` over it, and every write is
//! wrapped in [`LocalDbFault`] so the window can tell a refusal (nothing written) from a lost
//! answer (it may have been written). Passphrases and tokens never appear in an error.
use anyhow::{anyhow, bail, Context};
use cairn_event::{sign_attestation, EventSigner};
use tokio_postgres::Client;
use zeroize::Zeroizing;

use crate::db_diagnosis::LocalDbFault;
use crate::vault::{
    mint_sealed, passphrase_is_usable, release_for_act, seal_seed, Purpose, Release,
};

/// The role string every approval token carries. The doors verify the signature over the ADDRESS;
/// the role is informational, but the tests and the library must agree on it.
const ATTEST_ROLE: &str = "approved";

/// A person to give a new key. Borrowed fields, as `vault::NewMember`; `None` keeps what is stored.
pub struct Rekey<'a> {
    pub staff_id: &'a str,
    pub role: &'a str,
    pub display_name: Option<&'a str>,
    pub phone: Option<&'a str>,
    pub email: Option<&'a str>,
    pub passphrase: &'a str,
    pub hint: Option<&'a str>,
}

/// A recent re-key of a member's CURRENT key, for the sign-in strip.
pub struct ReplacedNotice {
    /// When the key was replaced, RFC 3339 (UTC).
    pub at: String,
    /// Display name of whoever approved it ("" if that person is no longer in the vault).
    pub approver: String,
}

/// Is `staff_id`'s current key a recent (or still unconfirmed) replacement? `None` otherwise.
pub async fn replaced_notice(
    db: &Client,
    staff_id: &str,
) -> anyhow::Result<Option<ReplacedNotice>> {
    let row = db
        .query_opt(
            "SELECT to_char(replaced_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                    COALESCE(approver_display_name, '') FROM cairn_vault_replaced_notice($1)",
            &[&staff_id],
        )
        .await
        .context("reading the replaced-key notice")?;
    Ok(row.map(|r| ReplacedNotice {
        at: r.get(0),
        approver: r.get(1),
    }))
}

/// When `staff_id`'s signing stops because their key is still unconfirmed under the site's
/// optional expiry (§5.5), as RFC 3339 UTC — or `None` when it never will (no limit set, the key
/// is confirmed, or no such member). The window asks once after a signed act, to warn days ahead.
pub async fn unconfirmed_expires_at(db: &Client, staff_id: &str) -> anyhow::Result<Option<String>> {
    let row = db
        .query_one(
            "SELECT to_char(cairn_vault_unconfirmed_expires_at($1) AT TIME ZONE 'UTC', \
                            'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
            &[&staff_id],
        )
        .await
        .context("reading when an unconfirmed key stops signing")?;
    Ok(row.get(0))
}

/// Sign `addr` as `signer`: the approval token every approver-gated door takes.
fn approve(addr: &[u8], signer: &dyn EventSigner) -> anyhow::Result<Vec<u8>> {
    let kid = hex::encode(signer.verifying_key().to_bytes());
    sign_attestation(addr, &kid, ATTEST_ROLE, signer)
        .map_err(|e| anyhow!("signing the approval failed: {e}"))
}

/// Re-key `r.staff_id` with a superuser's or enroller's say-so; the person keeps their actor.
/// Returns the new key id.
pub async fn rekey_member(
    db: &Client,
    r: &Rekey<'_>,
    approver_staff_id: &str,
    approver: &dyn EventSigner,
) -> anyhow::Result<String> {
    let (kid, sealed) = mint_sealed(r.passphrase)?;
    let addr: Vec<u8> = db
        .query_one(
            "SELECT cairn_vault_rekey_address($1, $2, $3)",
            &[&r.staff_id, &r.role, &kid],
        )
        .await
        .context("computing the re-key address")?
        .get(0);
    let token = approve(&addr, approver)?;
    db.query_one(
        "SELECT cairn_vault_rekey($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        &[
            &r.staff_id,
            &r.role,
            &r.display_name,
            &r.phone,
            &r.email,
            &kid,
            &sealed,
            &r.hint,
            &approver_staff_id,
            &token,
        ],
    )
    .await
    .map_err(|e| LocalDbFault::new("re-key", e))?;
    Ok(kid)
}

/// Console-only (owner connection): re-key a superuser with nobody to approve it.
pub async fn rekey_superuser_from_console(db: &Client, r: &Rekey<'_>) -> anyhow::Result<String> {
    if r.role != "superuser" {
        bail!(
            "the console re-keys a superuser only (role was {:?})",
            r.role
        );
    }
    let (kid, sealed) = mint_sealed(r.passphrase)?;
    db.query_one(
        "SELECT cairn_vault_bootstrap_rekey($1, $2, $3, $4, $5, $6, $7)",
        &[
            &r.staff_id,
            &r.display_name,
            &kid,
            &sealed,
            &r.hint,
            &r.phone,
            &r.email,
        ],
    )
    .await
    .map_err(|e| LocalDbFault::new("console re-key", e))?;
    Ok(kid)
}

/// Grant (`grant`) or withdraw the enroller role, signed by a superuser. The token is bound to the
/// member's current grant state, which the database reports (`cairn_vault_grant_prior`).
pub async fn set_enroller(
    db: &Client,
    staff_id: &str,
    grant: bool,
    su_staff_id: &str,
    su: &dyn EventSigner,
) -> anyhow::Result<()> {
    let prior: i64 = db
        .query_one("SELECT cairn_vault_grant_prior($1)", &[&staff_id])
        .await
        .context("reading the grant state")?
        .get(0);
    let addr: Vec<u8> = db
        .query_one(
            "SELECT cairn_vault_grant_address($1, $2, $3)",
            &[&staff_id, &grant, &prior],
        )
        .await
        .context("computing the grant address")?
        .get(0);
    let token = approve(&addr, su)?;
    db.execute(
        "SELECT cairn_vault_set_enroller($1, $2, $3, $4)",
        &[&staff_id, &grant, &su_staff_id, &token],
    )
    .await
    .map_err(|e| LocalDbFault::new("enroller grant", e))?;
    Ok(())
}

/// Confirm (`confirm`) or repudiate key `kid`, signed by a superuser.
pub async fn judge_key(
    db: &Client,
    kid: &str,
    confirm: bool,
    su_staff_id: &str,
    su: &dyn EventSigner,
) -> anyhow::Result<()> {
    let addr: Vec<u8> = db
        .query_one(
            "SELECT cairn_vault_judge_address($1, $2)",
            &[&kid, &confirm],
        )
        .await
        .context("computing the judgement address")?
        .get(0);
    let token = approve(&addr, su)?;
    db.execute(
        "SELECT cairn_vault_judge_key($1, $2, $3, $4)",
        &[&kid, &confirm, &su_staff_id, &token],
    )
    .await
    .map_err(|e| LocalDbFault::new("key judgement", e))?;
    Ok(())
}

/// The answer to a change of passphrase.
// Short-lived, never stored in bulk; boxing the `Release` would only change the brief's shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Reseal {
    Done,
    /// The vault did not release the key (wrong passphrase, waiting, ...): nothing changed.
    Refused(Release),
}

/// Change my own passphrase. The current one goes through the COUNTED release door (purpose
/// `reseal`), so guessing it here is throttled like a sign-in; the released key then re-seals its
/// own seed under `new` and signs the exact change, and is dropped. The key itself is unchanged.
pub async fn change_passphrase(
    db: &Client,
    staff_id: &str,
    current: &str,
    new: &str,
    hint: Option<&str>,
) -> anyhow::Result<Reseal> {
    if !passphrase_is_usable(new) {
        bail!("a new passphrase must not be empty or only spaces");
    }
    let (key, _kid) = match release_for_act(db, staff_id, current, Purpose::Reseal).await? {
        Release::Released { key, kid } => (key, kid),
        refused => return Ok(Reseal::Refused(refused)),
    };
    let seed = Zeroizing::new(key.to_bytes());
    let sealed = seal_seed(&seed, new)?;
    let addr: Option<Vec<u8>> = db
        .query_one(
            "SELECT cairn_vault_reseal_address($1, $2)",
            &[&staff_id, &sealed],
        )
        .await
        .context("computing the reseal address")?
        .get(0);
    let addr = addr.ok_or_else(|| anyhow!("no such member to re-seal"))?;
    let proof = approve(&addr, &key)?;
    db.execute(
        "SELECT cairn_vault_reseal($1, $2, $3, $4)",
        &[&staff_id, &sealed, &hint, &proof],
    )
    .await
    .map_err(|e| LocalDbFault::new("change of passphrase", e))?;
    Ok(Reseal::Done)
}

/// One key awaiting a superuser's judgement.
pub struct Unconfirmed {
    pub staff_id: String,
    pub display_name: String,
    pub kid: String,
    pub approver_staff_id: String,
    /// RFC 3339 (UTC).
    pub approved_at: String,
    pub doubtful: bool,
    /// When it stops signing under the site's optional expiry (§5.5), RFC 3339 (UTC); `None`
    /// with no limit set.
    pub stops_at: Option<String>,
}

/// The superuser's worklist: current keys still unconfirmed or doubtful, oldest first, each
/// with its expiry date (a companion call per row in the same statement, so
/// `cairn_vault_unconfirmed`'s columns stay as they are).
pub async fn unconfirmed(db: &Client) -> anyhow::Result<Vec<Unconfirmed>> {
    let rows = db
        .query(
            "SELECT staff_id, COALESCE(display_name, ''), signing_key_id, \
                    COALESCE(approver_staff_id, ''), \
                    to_char(approved_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), doubtful, \
                    to_char(cairn_vault_unconfirmed_expires_at(staff_id) AT TIME ZONE 'UTC', \
                            'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
               FROM cairn_vault_unconfirmed()",
            &[],
        )
        .await
        .context("reading the unconfirmed keys")?;
    Ok(rows
        .iter()
        .map(|r| Unconfirmed {
            staff_id: r.get(0),
            display_name: r.get(1),
            kid: r.get(2),
            approver_staff_id: r.get(3),
            approved_at: r.get(4),
            doubtful: r.get(5),
            stops_at: r.get(6),
        })
        .collect())
}

/// One member in the grant panel.
pub struct EnrollerRow {
    pub staff_id: String,
    pub display_name: String,
    pub enroller: bool,
}

/// Every non-superuser member and whether they hold the enroller grant now.
pub async fn enrollers(db: &Client) -> anyhow::Result<Vec<EnrollerRow>> {
    let rows = db
        .query(
            "SELECT staff_id, display_name, enroller FROM cairn_vault_enrollers()",
            &[],
        )
        .await
        .context("reading the enrollers")?;
    Ok(rows
        .iter()
        .map(|r| EnrollerRow {
            staff_id: r.get(0),
            display_name: r.get(1),
            enroller: r.get(2),
        })
        .collect())
}
