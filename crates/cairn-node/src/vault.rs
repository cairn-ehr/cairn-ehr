//! The Rust side of the clinicians' key vault (db/061).
//!
//! A clinician's Ed25519 key lives in the database, sealed under their own passphrase. This module is
//! the thin client over the three SQL doors: enrol (a superuser vouches), bootstrap (console only) and
//! release (the passphrase is checked INSIDE Postgres, and the seed comes back for ONE act).
//!
//! House rules held here: the passphrase never appears in an error, a `Debug` impl or a log line; the
//! released seed is copied only into a `Zeroizing` buffer, then into a `SigningKey` (which wipes
//! itself on drop) — the bytes tokio-postgres received and the bytea Postgres built are outside
//! this module's reach and are NOT wiped; staff-ID normalisation is done by the database
//! (`cairn_vault_staff_id`) and nowhere in Rust.
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use cairn_event::keys::Secret32;
use cairn_event::{generate_key, sign_attestation, EventSigner, SigningKey};
use tokio_postgres::Client;
use zeroize::Zeroizing;

use crate::db_diagnosis::LocalDbFault;
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

/// Pure: is `passphrase` a passphrase at all? Spaces alone are not. The ONE rule for both sides of
/// a vault passphrase: every enrolment door refuses a new one that fails it, and the window never
/// sends a typed one that fails it to the vault (an empty guess would count as a wrong one). Two
/// rules here once enrolled a key under "   " that sign-in could then never send (PR #897 review).
pub fn passphrase_is_usable(passphrase: &str) -> bool {
    !passphrase.trim_matches(' ').is_empty()
}

/// Mint a key for `new`, seal its seed under their passphrase, and return `(kid, sealed blob)`.
/// Refuses an unusable passphrase, whichever door called it.
fn mint_sealed(new: &NewMember<'_>) -> anyhow::Result<(String, Vec<u8>)> {
    if !passphrase_is_usable(new.passphrase) {
        bail!("a new passphrase must not be empty or only spaces");
    }
    let (sk, kid) = generate_key().map_err(|e| anyhow!("key generation failed: {e}"))?;
    let seed = Zeroizing::new(sk.to_bytes());
    // shortcut: the vault re-keys by token (slice 2), so the recovery wrap is unused; drop it if slice 2 does not want it.
    let recovery = Zeroizing::new(seal::generate_recovery_code());
    let sealed = seal::seal(&Secret32::from_bytes(*seed), new.passphrase, &recovery)
        .map_err(|e| anyhow!("sealing the key failed: {e}"))?;
    Ok((kid, seal::to_cbor(&sealed)))
}

/// The pinned determinant set both enrolment doors insist on: `{role, handle}`, the handle being
/// the staff ID AS THE DATABASE NORMALISES IT (`normalised`, read back from
/// `cairn_vault_staff_id`, never computed here — R7). The floor rebuilds the same set and refuses
/// any other, so "Boss" and "boss" are one actor. Pure. A blank role is refused here, as
/// `build_human_pinned` refused it before the vault had its own set.
fn vault_pinned(role: &str, normalised: &str) -> anyhow::Result<serde_json::Value> {
    if role.trim().is_empty() {
        bail!("enrolment needs a role");
    }
    Ok(serde_json::json!({ "role": role, "handle": normalised }))
}

/// Enrol `new` with a superuser's say-so. The superuser's attestation covers exactly this
/// `(staff_id, kid, role)` request; the address to sign and the pinned set's handle both come from
/// the database, in one round trip, so staff-ID normalisation has one home. Returns the new
/// member's key id.
pub async fn enrol_member(
    db: &Client,
    new: &NewMember<'_>,
    superuser_staff_id: &str,
    superuser: &dyn EventSigner,
) -> anyhow::Result<String> {
    let (kid, sealed) = mint_sealed(new)?;
    let row = db
        .query_one(
            "SELECT cairn_vault_enrol_address($1, $2, $3), cairn_vault_staff_id($1)",
            &[&new.staff_id, &kid, &new.role],
        )
        .await
        .context("computing the enrolment address")?;
    let addr: Vec<u8> = row.get(0);
    let pinned = vault_pinned(new.role, row.get(1))?;
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
    // A `LocalDbFault`, not text: the window must still find the database error on the chain to
    // tell a refusal (nothing written) from a lost answer (it may have been written).
    .map_err(|e| LocalDbFault::new("enrolment", e))?;
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
    let normalised: String = db
        .query_one("SELECT cairn_vault_staff_id($1)", &[&new.staff_id])
        .await
        .context("normalising the staff ID")?
        .get(0);
    let pinned = vault_pinned("superuser", &normalised)?;
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
    .map_err(|e| LocalDbFault::new("bootstrap", e))?;
    Ok(kid)
}

/// The DB's own message for a refusal (the SQL never echoes a passphrase or sealed blob). For the
/// reads; the two enrolment writes keep the whole error ([`LocalDbFault`]).
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
    Released {
        key: SigningKey,
        kid: String,
    },
    Wrong {
        retry_after: Option<Duration>,
    },
    Waiting {
        retry_after: Duration,
    },
    Unknown,
    RefusedPurpose,
    /// The member's actor is revoked or superseded: its key would sign nothing the floor accepts.
    NotCurrent,
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
            Release::NotCurrent => f.write_str("NotCurrent"),
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
        // Borrowed from the row's buffer, not copied into a Vec the allocator would keep unwiped.
        row.get::<_, Option<&[u8]>>(1),
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
            let seed = seed.ok_or_else(|| anyhow!("vault released no seed"))?;
            if seed.len() != 32 {
                bail!("vault seed has the wrong length");
            }
            // Copied straight into the wiped buffer: a `[u8; 32]` built first and then wrapped
            // would leave its moved-from stack copy behind (#54).
            let mut arr = Zeroizing::new([0u8; 32]);
            arr.copy_from_slice(seed);
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
        "not-current" => Ok(Release::NotCurrent),
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

/// Pure: does `passphrase` match the PHC string? `Ok(false)` is a mismatch and nothing else; a
/// PHC that cannot be parsed or verified is an `Err`, because it is the node's fault and a "no"
/// would tell every staff member their correct passphrase is wrong.
pub fn terminal_unlock_ok(phc: &str, passphrase: &str) -> anyhow::Result<bool> {
    let hash = PasswordHash::new(phc).map_err(|e| anyhow!("the terminal PHC is malformed: {e}"))?;
    // A PHC cut short still parses; the verifier would then call it a mismatch.
    if hash.salt.is_none() || hash.hash.is_none() {
        bail!("the terminal PHC is malformed: it has no salt or no hash");
    }
    match Argon2::default().verify_password(passphrase.as_bytes(), &hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => bail!("the terminal PHC could not be checked: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_unlock_ok_true_and_false() {
        let phc = terminal_phc("shared pass").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(terminal_unlock_ok(&phc, "shared pass").unwrap());
        assert!(!terminal_unlock_ok(&phc, "other").unwrap());
    }

    /// A damaged PHC is the node's fault, not the typist's: an error, never "no" — or every
    /// staff member is told their correct shared passphrase is wrong (PR #897 review).
    #[test]
    fn a_malformed_terminal_phc_is_an_error_not_a_wrong_passphrase() {
        assert!(terminal_unlock_ok("not a phc", "shared pass").is_err());
        let phc = terminal_phc("shared pass").unwrap();
        let truncated = &phc[..phc.len() / 2];
        assert!(terminal_unlock_ok(truncated, "shared pass").is_err());
    }

    /// The one rule every door (window, console, library) applies to a NEW passphrase, and the
    /// rule the window applies to a typed one: spaces alone are no passphrase. Enrolling one
    /// would seal a key its owner can never use — sign-in never sends it to the vault.
    #[test]
    fn a_passphrase_of_spaces_is_not_usable() {
        assert!(!passphrase_is_usable(""));
        assert!(!passphrase_is_usable("   "));
        assert!(passphrase_is_usable(" a "));
        assert!(passphrase_is_usable("correct horse"));
    }

    #[test]
    fn a_key_is_never_sealed_under_a_passphrase_of_spaces() {
        let new = NewMember {
            staff_id: "x",
            display_name: "X",
            role: "nurse",
            phone: None,
            email: None,
            passphrase: "   ",
            hint: None,
        };
        assert!(mint_sealed(&new).is_err());
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
        assert!(matches!(
            release_from_row("not-current", None, None, None).unwrap(),
            Release::NotCurrent
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
