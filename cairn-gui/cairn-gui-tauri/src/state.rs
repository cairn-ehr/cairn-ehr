//! The window's session state — above all, custody of the clinician's signing key.
//!
//! # Why the key is held at all
//!
//! Whole-list sign-off must cost ONE human act (#288). If the passphrase were re-entered
//! per sign-off the gesture would cost two, and the paper counterpart says otherwise: on a
//! drug chart your identity is established by presence, not re-proved at every signature.
//! So the key is unsealed once and held.
//!
//! # Why it is re-locked
//!
//! A held key widens the unattended-workstation window. Paper has the same failure — an
//! open chart on a desk — so this is parity rather than regression, but Cairn should not be
//! WORSE than paper. The key is wiped after `IDLE_TIMEOUT` of no activity, and only the
//! 32-byte seed is retained, inside `Zeroizing`, so the wipe is real rather than a dropped
//! reference the allocator may leave behind.
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// How long a held signing key survives without activity.
///
/// A named constant with a test pinning it, so this is a reviewable clinical decision
/// rather than a number buried in a timer.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The clinician's unsealed signing key, held for the session.
pub struct SessionKey {
    /// The Ed25519 seed. `SigningKey` is reconstructed per use so the only long-lived copy
    /// is inside `Zeroizing`, which wipes on drop.
    seed: Zeroizing<[u8; 32]>,
    /// Hex key id, safe to display and to log.
    pub kid: String,
    last_activity: Instant,
}

impl SessionKey {
    pub fn new(sk: cairn_event::SigningKey, now: Instant) -> Self {
        let kid = hex::encode(sk.verifying_key().to_bytes());
        Self {
            seed: Zeroizing::new(sk.to_bytes()),
            kid,
            last_activity: now,
        }
    }

    /// Rebuild the usable key. Kept short-lived at every call site.
    pub fn signing_key(&self) -> cairn_event::SigningKey {
        cairn_event::SigningKey::from_bytes(&self.seed)
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.last_activity) >= IDLE_TIMEOUT
    }

    /// Record activity, pushing the expiry out.
    pub fn touch(&mut self, now: Instant) {
        self.last_activity = now;
    }

    /// A key with deterministic, runtime-derived material for the timing tests. Never a
    /// literal (house rule 6) — and never used outside `cfg(test)`.
    #[cfg(test)]
    pub fn for_test(now: Instant) -> Self {
        let seed: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1));
        Self::new(cairn_event::SigningKey::from_bytes(&seed), now)
    }
}

/// Everything a command needs. One database connection, because a single-patient window
/// has no concurrency worth pooling for, and the mutex makes the borrow rules explicit.
pub struct AppState {
    /// `None` in fixture mode: `--mock` runs with no database at all, which is what makes
    /// the operator accessibility pass and the timing runbook possible on a laptop.
    pub db: Option<tokio::sync::Mutex<tokio_postgres::Client>>,
    /// The NODE's key — holds custody of every sealed body (ADR-0052) regardless of who
    /// signed the content. Distinct from the clinician's key in `session`. `None` in
    /// fixture mode.
    pub node_sk: Option<cairn_event::SigningKey>,
    pub node_origin: String,
    /// The chart this window is open on. Set once at launch (`--patient`); there is no
    /// patient picker in this slice.
    pub patient: uuid::Uuid,
    /// Path to the clinician's sealed key file, unsealed on `unlock`. `None` in fixture
    /// mode, where there is nothing to sign.
    pub attester_key_path: Option<std::path::PathBuf>,
    pub session: tokio::sync::Mutex<Option<SessionKey>>,
}

impl AppState {
    /// True when this window is showing fixtures rather than a real chart.
    ///
    /// Derived from the absence of a connection rather than stored as a separate flag: a
    /// boolean that can disagree with reality is exactly how a "mock" window ends up
    /// writing to a real database, or a real window silently shows fixtures.
    pub fn is_mock(&self) -> bool {
        self.db.is_none()
    }

    /// Take the held key if one is live, expiring and WIPING it first if it has gone idle.
    ///
    /// Returns the reconstructed `SigningKey` and its kid. Checking expiry here rather than
    /// at each call site means there is exactly one place a stale key can leak from.
    pub async fn live_key(&self) -> Option<(cairn_event::SigningKey, String)> {
        let mut guard = self.session.lock().await;
        let now = Instant::now();
        if guard.as_ref().is_some_and(|k| k.is_expired(now)) {
            // Dropping the SessionKey drops its Zeroizing seed, which wipes it.
            *guard = None;
        }
        let key = guard.as_mut()?;
        key.touch(now);
        Some((key.signing_key(), key.kid.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A held key is what makes the whole-list gesture cost ONE act, so how long it is
    /// held is a clinical decision, not a timer detail. Pinned so changing it is visible
    /// in a diff.
    #[test]
    fn the_idle_timeout_is_fifteen_minutes() {
        assert_eq!(IDLE_TIMEOUT, Duration::from_secs(15 * 60));
    }

    #[test]
    fn a_key_is_live_until_the_timeout_elapses() {
        let start = Instant::now();
        let key = SessionKey::for_test(start);
        assert!(!key.is_expired(start));
        assert!(!key.is_expired(start + Duration::from_secs(14 * 60 + 59)));
    }

    #[test]
    fn a_key_expires_at_the_timeout() {
        let start = Instant::now();
        let key = SessionKey::for_test(start);
        assert!(key.is_expired(start + IDLE_TIMEOUT));
        assert!(key.is_expired(start + Duration::from_secs(60 * 60)));
    }

    /// Idle means idle: activity resets the clock, so a clinician working continuously is
    /// never asked to unlock mid-list.
    #[test]
    fn activity_pushes_the_expiry_out() {
        let start = Instant::now();
        let mut key = SessionKey::for_test(start);
        key.touch(start + Duration::from_secs(10 * 60));
        assert!(!key.is_expired(start + Duration::from_secs(20 * 60)));
        assert!(key.is_expired(start + Duration::from_secs(25 * 60)));
    }

    /// The kid must be the key's real public identity, not a label: it is what the node
    /// checks against the enrolled-human registry, and what the window shows the clinician
    /// as "who is signed in".
    #[test]
    fn the_kid_is_the_verifying_key_in_hex() {
        let start = Instant::now();
        let key = SessionKey::for_test(start);
        let expected = hex::encode(key.signing_key().verifying_key().to_bytes());
        assert_eq!(key.kid, expected);
    }

    /// The reconstructed key must be the SAME key every time, or a session's signatures
    /// would not verify against the kid the window reported.
    #[test]
    fn the_reconstructed_key_is_stable() {
        let start = Instant::now();
        let key = SessionKey::for_test(start);
        assert_eq!(
            key.signing_key().to_bytes(),
            key.signing_key().to_bytes(),
            "the held seed must rebuild one key, not a fresh one per call"
        );
    }
}
