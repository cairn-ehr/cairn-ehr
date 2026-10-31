//! The Rust side of the clinicians' key vault (db/061).
//!
//! A clinician's Ed25519 key lives in the database, sealed under their own passphrase. This module is
//! the thin client over the three SQL doors: enrol (a superuser vouches), bootstrap (console only) and
//! release (the passphrase is checked INSIDE Postgres, and the seed comes back for ONE act).
//!
//! House rules held here: the passphrase never appears in an error, a `Debug` impl or a log line; the
//! seed lives in a `Zeroizing` buffer until it is turned into a `SigningKey`; staff-ID normalisation
//! is done by the database (`cairn_vault_staff_id`) and nowhere in Rust.
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use cairn_event::keys::Secret32;
use cairn_event::{generate_key, sign_attestation, EventSigner, SigningKey};
use tokio_postgres::Client;
use zeroize::Zeroizing;

use crate::enroll::build_human_pinned;
use crate::seal;

/// A person to put in the vault. Borrowed fields: the caller owns the strings (and wipes the passphrase).
pub struct NewMember<'a> {
    pub staff_id: &'a str,
    pub display_name: &'a str,
    pub role: &'a str,
    pub phone: Option<&'a str>,
    pub email: Option<&'a str>,
    pub passphrase: &'a str,
    pub hint: Option<&'a str>,
}

/// Mint a key for `new`, seal its seed under their passphrase, and return `(kid, sealed blob)`.
fn mint_sealed(new: &NewMember<'_>) -> anyhow::Result<(String, Vec<u8>)> {
    let (sk, kid) = generate_key().map_err(|e| anyhow!("key generation failed: {e}"))?;
    let seed = Zeroizing::new(sk.to_bytes());
    // shortcut: the vault re-keys by token (slice 2), so the recovery wrap is unused; drop it if slice 2 does not want it.
    let recovery = Zeroizing::new(seal::generate_recovery_code());
    let sealed = seal::seal(&Secret32::from_bytes(*seed), new.passphrase, &recovery)
        .map_err(|e| anyhow!("sealing the key failed: {e}"))?;
    Ok((kid, seal::to_cbor(&sealed)))
}

/// Enrol `new` with a superuser's say-so. The superuser's attestation covers exactly this
/// `(staff_id, kid, role)` request; the address to sign comes from the database so staff-ID
/// normalisation has one home. Returns the new member's key id.
pub async fn enrol_member(
    db: &Client,
    new: &NewMember<'_>,
    superuser_staff_id: &str,
    superuser: &dyn EventSigner,
) -> anyhow::Result<String> {
    let pinned = build_human_pinned(new.role, None, Some(new.staff_id))?;
    let (kid, sealed) = mint_sealed(new)?;
    let addr: Vec<u8> = db
        .query_one(
            "SELECT cairn_vault_enrol_address($1, $2, $3)",
            &[&new.staff_id, &kid, &new.role],
        )
        .await
        .context("computing the enrolment address")?
        .get(0);
    let su_kid = hex::encode(superuser.verifying_key().to_bytes());
    let token = sign_attestation(&addr, &su_kid, "enrolled", superuser)
        .map_err(|e| anyhow!("signing the enrolment failed: {e}"))?;
    db.query_one(
        "SELECT cairn_vault_enrol($1, $2, $3, $4::text::jsonb, $5, $6, $7, $8, $9, $10, $11)",
        &[
            &new.staff_id,
            &new.display_name,
            &new.role,
            &pinned.to_string(),
            &kid,
            &sealed,
            &new.hint,
            &new.phone,
            &new.email,
            &superuser_staff_id,
            &token,
        ],
    )
    .await
    .map_err(|e| anyhow!("enrolment refused: {}", db_text(&e)))?;
    Ok(kid)
}

/// Console-only: enrol a superuser with no superuser to approve it (owner connection).
pub async fn bootstrap_superuser(db: &Client, new: &NewMember<'_>) -> anyhow::Result<String> {
    if new.role != "superuser" {
        bail!(
            "bootstrap enrols a superuser only (role was {:?})",
            new.role
        );
    }
    let pinned = build_human_pinned("superuser", None, Some(new.staff_id))?;
    let (kid, sealed) = mint_sealed(new)?;
    db.query_one(
        "SELECT cairn_vault_bootstrap($1, $2, $3::text::jsonb, $4, $5, $6, $7, $8)",
        &[
            &new.staff_id,
            &new.display_name,
            &pinned.to_string(),
            &kid,
            &sealed,
            &new.hint,
            &new.phone,
            &new.email,
        ],
    )
    .await
    .map_err(|e| anyhow!("bootstrap refused: {}", db_text(&e)))?;
    Ok(kid)
}

/// The DB's own message for a refusal (the SQL never echoes a passphrase or sealed blob).
fn db_text(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| "database unreachable or malformed request".into())
}

/// What the key is wanted for. The vault refuses a superuser for `Clinical` and a clinician for `Enrol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Clinical,
    Enrol,
}

impl Purpose {
    fn as_sql(self) -> &'static str {
        match self {
            Purpose::Clinical => "clinical",
            Purpose::Enrol => "enrol",
        }
    }
}

/// The vault's answer to a release request. Hand-written `Debug`: never prints key material.
// The key is short-lived and the enum is never stored in bulk; boxing would only change the brief's shape.
#[allow(clippy::large_enum_variant)]
pub enum Release {
    Released { key: SigningKey, kid: String },
    Wrong { retry_after: Option<Duration> },
    Waiting { retry_after: Duration },
    Unknown,
    RefusedPurpose,
}

impl std::fmt::Debug for Release {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Release::Released { kid, .. } => f
                .debug_struct("Released")
                .field("key", &"<redacted>")
                .field("kid", kid)
                .finish(),
            Release::Wrong { retry_after } => f
                .debug_struct("Wrong")
                .field("retry_after", retry_after)
                .finish(),
            Release::Waiting { retry_after } => f
                .debug_struct("Waiting")
                .field("retry_after", retry_after)
                .finish(),
            Release::Unknown => f.write_str("Unknown"),
            Release::RefusedPurpose => f.write_str("RefusedPurpose"),
        }
    }
}

/// Ask the vault for `staff_id`'s key for ONE act. A wrong passphrase is an answer, not an error;
/// an outcome string we do not know, or a seed that does not belong to its key id, IS an error.
pub async fn release_for_act(
    db: &Client,
    staff_id: &str,
    passphrase: &str,
    purpose: Purpose,
) -> anyhow::Result<Release> {
    let row = db
        .query_one(
            "SELECT outcome, seed, signing_key_id, retry_after_ms FROM cairn_vault_release($1, $2, $3)",
            &[&staff_id, &passphrase, &purpose.as_sql()],
        )
        .await
        .map_err(|e| anyhow!("vault release failed: {}", db_text(&e)))?;
    release_from_row(
        &row.get::<_, String>(0),
        row.get::<_, Option<Vec<u8>>>(1).as_deref(),
        row.get::<_, Option<String>>(2).as_deref(),
        row.get(3),
    )
}

/// Pure: turn one `cairn_vault_release` row into a `Release`. Holds R6 (the seed must derive the
/// key id it was filed under) and the rule that an unknown outcome string is an error.
fn release_from_row(
    outcome: &str,
    seed: Option<&[u8]>,
    kid: Option<&str>,
    retry_after_ms: Option<i64>,
) -> anyhow::Result<Release> {
    let retry = retry_after_ms.map(|ms| Duration::from_millis(ms.max(0) as u64));
    match outcome {
        "released" => {
            let kid = kid.ok_or_else(|| anyhow!("vault released no key id"))?;
            let arr: [u8; 32] = seed
                .ok_or_else(|| anyhow!("vault released no seed"))?
                .try_into()
                .map_err(|_| anyhow!("vault seed has the wrong length"))?;
            let arr = Zeroizing::new(arr);
            let key = SigningKey::from_bytes(&arr);
            // The vault does not check this pairing; a mismatch would sign under the wrong identity.
            if hex::encode(key.verifying_key().to_bytes()) != kid {
                bail!("the released key does not match its registered key id; refusing to use it");
            }
            Ok(Release::Released {
                key,
                kid: kid.to_string(),
            })
        }
        "wrong" => Ok(Release::Wrong { retry_after: retry }),
        "waiting" => Ok(Release::Waiting {
            retry_after: retry.ok_or_else(|| anyhow!("vault said wait without saying how long"))?,
        }),
        "unknown" => Ok(Release::Unknown),
        "refused-purpose" => Ok(Release::RefusedPurpose),
        other => bail!("vault returned an unknown outcome {other:?}"),
    }
}

/// Is `staff_id` already enrolled? The database normalises the ID (case, spaces) like every other
/// vault door. Asked before a superuser's key is released, so a duplicate is told plainly.
pub async fn staff_id_taken(db: &Client, staff_id: &str) -> anyhow::Result<bool> {
    let row = db
        .query_one("SELECT cairn_vault_staff_id_taken($1)", &[&staff_id])
        .await
        .map_err(|e| anyhow!("staff-id check failed: {}", db_text(&e)))?;
    Ok(row.get(0))
}

/// An Argon2id PHC string for the shared terminal-unlock passphrase (the window checks against it).
pub fn terminal_phc(passphrase: &str) -> anyhow::Result<String> {
    let entropy = seal::rand_bytes::<16>().map_err(|e| anyhow!("entropy unavailable: {e}"))?;
    let encoded = SaltString::encode_b64(&entropy).map_err(|e| anyhow!("encoding failed: {e}"))?;
    Ok(Argon2::default()
        .hash_password(passphrase.as_bytes(), &encoded)
        .map_err(|e| anyhow!("hashing failed: {e}"))?
        .to_string())
}

/// Pure: does `passphrase` match the PHC string? A malformed PHC is simply "no".
pub fn terminal_unlock_ok(phc: &str, passphrase: &str) -> bool {
    PasswordHash::new(phc)
        .map(|h| {
            Argon2::default()
                .verify_password(passphrase.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_unlock_ok_true_and_false() {
        let phc = terminal_phc("shared pass").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(terminal_unlock_ok(&phc, "shared pass"));
        assert!(!terminal_unlock_ok(&phc, "other"));
        assert!(!terminal_unlock_ok("not a phc", "shared pass"));
    }

    fn seed_of(v: u8) -> [u8; 32] {
        std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(v))
    }
    fn kid_of(seed: [u8; 32]) -> String {
        hex::encode(SigningKey::from_bytes(&seed).verifying_key().to_bytes())
    }

    #[test]
    fn release_from_row_maps_every_outcome() {
        let s = seed_of(3);
        let k = kid_of(s);
        assert!(matches!(
            release_from_row("released", Some(&s), Some(&k), None).unwrap(),
            Release::Released { kid, .. } if kid == k
        ));
        assert!(matches!(
            release_from_row("waiting", None, None, Some(1500)).unwrap(),
            Release::Waiting { retry_after } if retry_after == Duration::from_millis(1500)
        ));
        assert!(release_from_row("waiting", None, None, None).is_err());
        assert!(matches!(
            release_from_row("wrong", None, None, None).unwrap(),
            Release::Wrong { retry_after: None }
        ));
        assert!(matches!(
            release_from_row("wrong", None, None, Some(2000)).unwrap(),
            Release::Wrong { retry_after: Some(d) } if d == Duration::from_secs(2)
        ));
        assert!(matches!(
            release_from_row("unknown", None, None, None).unwrap(),
            Release::Unknown
        ));
        assert!(matches!(
            release_from_row("refused-purpose", None, None, None).unwrap(),
            Release::RefusedPurpose
        ));
        assert!(release_from_row("bogus", None, None, None).is_err());
    }

    #[test]
    fn release_from_row_refuses_a_bad_seed() {
        let s = seed_of(3);
        let other = kid_of(seed_of(4));
        let e = release_from_row("released", Some(&s), Some(&other), None).unwrap_err();
        assert!(e.to_string().contains("does not match"));
        assert!(release_from_row("released", Some(&s[..31]), Some(&kid_of(s)), None).is_err());
    }

    #[test]
    fn debug_never_prints_the_key() {
        let (key, kid) = generate_key().unwrap();
        let shown = format!("{:?}", Release::Released { key, kid });
        assert!(shown.contains("redacted"), "{shown}");
    }
}
