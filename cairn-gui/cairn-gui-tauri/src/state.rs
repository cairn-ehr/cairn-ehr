//! The window's session state — above all, custody of a clinician's signing key between acts.
//!
//! # Where the key comes from
//!
//! Each clinical act carries the clinician's staff ID and passphrase, and the node's vault
//! releases their key for that act (`AppState::signer_for_act`, `crate::signer`). Nothing is
//! unsealed at launch and nothing is held by default.
//!
//! # Why a key may be held at all
//!
//! On a PERSONAL workstation, whole-list sign-off must cost ONE human act (#288), and the paper
//! counterpart agrees: on a drug chart your identity is established by presence, not re-proved
//! at every signature. So a terminal launched with `--key-hold-seconds N` keeps the last
//! signer's key for up to N seconds of no activity (at most `IDLE_TIMEOUT`). A SHARED terminal
//! holds nothing (the default, 0): the next person can never write as the last one.
//!
//! # Why it is re-locked
//!
//! A held key widens the unattended-workstation window. Paper has the same failure — an
//! open chart on a desk — so this is parity rather than regression, but Cairn should not be
//! WORSE than paper. The key is wiped once the hold runs out, and only the 32-byte seed is
//! retained, inside `Zeroizing`, so the wipe is real rather than a dropped reference the
//! allocator may leave behind.
//!
//! # What "activity" means, and what it deliberately does not
//!
//! Only a CLINICAL ACT counts as activity — signing off, stopping a drug. Merely *asking*
//! whether the key is still held does not, because the window asks that on a timer with no
//! human present (`main.js` polls `lock_state` every 10 s so the lock is ambient rather
//! than discovered when a signature is refused). Routing that poll through the touching
//! accessor is what made the timeout unreachable in the first review of this slice: the
//! idle clock was reset by the window rather than by the clinician, and the key never
//! re-locked at all. Hence the two accessors below — `key_status` reads, `signer_for_act` uses.
use crate::signer::{decide, from_release, Credential, HeldDecision, KeyRefusal};
use cairn_node::vault::Purpose;
use std::time::{Duration, Instant, SystemTime};
use zeroize::Zeroizing;

/// The LONGEST a held signing key may survive without activity: the cap on
/// `--key-hold-seconds` (`crate::signer::parse_key_hold`).
///
/// A named constant with a test pinning it, so this is a reviewable clinical decision
/// rather than a number buried in a timer.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// One reading of BOTH clocks, taken together.
///
/// Two clocks, because neither alone bounds an unattended workstation honestly:
///
/// - `Instant` is monotonic and immune to a clock step, but on macOS (and every other
///   `CLOCK_UPTIME_RAW`-backed platform) it does **not advance while the machine sleeps**.
///   A laptop closed at 17:00 and opened at 08:00 has an `Instant` that barely moved — and
///   a lid-closed laptop in a ward corridor is exactly the scenario the timeout is for.
/// - `SystemTime` counts that sleep, but can be stepped by NTP or by hand.
///
/// So expiry is the OR of the two, and a wall clock that has gone BACKWARDS expires the key
/// rather than extending it. The bias is deliberate and one-directional: this pair can make
/// a key re-lock earlier than 15 minutes, and can never make it fail to re-lock. An early
/// re-lock costs one passphrase entry; a missed one costs a signed medication chart.
#[derive(Debug, Clone, Copy)]
pub struct Now {
    monotonic: Instant,
    wall: SystemTime,
}

impl Now {
    /// Read both clocks. The only constructor outside tests: sampling them together is what
    /// makes comparing them to a stored pair meaningful.
    pub fn read() -> Self {
        Self {
            monotonic: Instant::now(),
            wall: SystemTime::now(),
        }
    }

    /// This reading, advanced on both clocks — the normal passage of time, for tests that
    /// need to cross a 15-minute boundary without waiting 15 minutes.
    #[cfg(test)]
    pub(crate) fn plus(self, elapsed: Duration) -> Self {
        Self {
            monotonic: self.monotonic + elapsed,
            wall: self.wall + elapsed,
        }
    }

    /// This reading with the WALL clock alone advanced: what a machine that slept looks
    /// like, since the monotonic clock does not count suspended time.
    #[cfg(test)]
    pub(crate) fn slept(self, elapsed: Duration) -> Self {
        Self {
            monotonic: self.monotonic,
            wall: self.wall + elapsed,
        }
    }

    /// This reading with the wall clock stepped BACKWARDS — NTP correcting a bad RTC, or a
    /// hand-set clock.
    #[cfg(test)]
    fn wall_stepped_back(self, by: Duration) -> Self {
        Self {
            monotonic: self.monotonic,
            wall: self.wall - by,
        }
    }
}

/// A clinician's released signing key, held between acts on a terminal with a key hold.
pub struct SessionKey {
    /// The Ed25519 seed. `SigningKey` is reconstructed per use so the only long-lived copy
    /// is inside `Zeroizing`, which wipes on drop.
    seed: Zeroizing<[u8; 32]>,
    /// Hex key id, safe to display and to log.
    pub kid: String,
    /// The staff ID it was released for, as typed: the window shows it as "who signs next".
    pub staff_id: String,
    /// When a clinical act last used this key. Both clocks — see [`Now`].
    last_activity: Now,
}

impl SessionKey {
    /// `staff_id` is trimmed of spaces for display, and only of spaces: case and Unicode
    /// normalisation live in SQL alone (`cairn_vault_staff_id`, R7).
    pub fn new(sk: cairn_event::SigningKey, staff_id: String, now: Now) -> Self {
        let kid = hex::encode(sk.verifying_key().to_bytes());
        Self {
            seed: Zeroizing::new(sk.to_bytes()),
            kid,
            staff_id: staff_id.trim_matches(' ').to_string(),
            last_activity: now,
        }
    }

    /// Rebuild the usable key. Kept short-lived at every call site.
    pub fn signing_key(&self) -> cairn_event::SigningKey {
        cairn_event::SigningKey::from_bytes(&self.seed)
    }

    /// Has this key been idle for `hold` or longer? See [`Now`] for why both clocks are
    /// consulted and why the answer is biased towards "yes". A zero hold is always expired.
    pub fn is_expired(&self, now: Now, hold: Duration) -> bool {
        // Saturates to zero rather than panicking if the readings arrive out of order.
        if now
            .monotonic
            .duration_since(self.last_activity.monotonic)
            .ge(&hold)
        {
            return true;
        }
        match now.wall.duration_since(self.last_activity.wall) {
            Ok(elapsed) => elapsed >= hold,
            // The wall clock moved backwards, so we cannot say how long this key has been
            // idle. Principle 4 applied to a security control: an unknown idle time is not
            // evidence of recent activity, so it re-locks.
            Err(_) => true,
        }
    }

    /// Record a clinical act, pushing the expiry out. NOT called by `key_status` — see the
    /// module doc.
    pub fn touch(&mut self, now: Now) {
        self.last_activity = now;
    }

    /// A key with deterministic, runtime-derived material for the timing tests. Never a
    /// literal (house rule 6) — and never used outside `cfg(test)`.
    #[cfg(test)]
    pub fn for_test(now: Now) -> Self {
        let seed: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1));
        Self::new(
            cairn_event::SigningKey::from_bytes(&seed),
            "test-signer".into(),
            now,
        )
    }
}

/// The held key, plus a generation counter that closes the wrong-author race.
///
/// `signer_for_act` releases the session lock while the vault checks a passphrase. Without the
/// counter, a Lock pressed in that moment (or a second credentialed press) would be undone when
/// the slower release finished and stored its key: the next act with no credential would sign
/// as someone who had locked the terminal. Every wipe bumps the generation; a release stores its
/// key only if no wipe has happened since its own.
#[derive(Default)]
pub struct Held {
    pub key: Option<SessionKey>,
    generation: u64,
}

impl Held {
    /// Wipe the held key, and return the generation a later [`Held::store_if_current`] must
    /// still see.
    pub fn wipe(&mut self) -> u64 {
        self.key = None;
        self.generation += 1;
        self.generation
    }

    /// Pure: hold `key` only if nothing has wiped since `seen`. Returns whether it was held.
    pub fn store_if_current(&mut self, seen: u64, key: SessionKey) -> bool {
        if self.generation != seen {
            return false;
        }
        self.key = Some(key);
        true
    }

    /// Pure: what `signer_for_act` does with a key the vault just released for `staff_id`: keep
    /// it for the next act only on a terminal with a key hold, and only if no Lock or newer
    /// press wiped the slot since `seen` (the generation `signer_for_act`'s own wipe returned).
    /// Returns whether it was kept. The act it was released for signs either way.
    pub fn keep_for_next_act(
        &mut self,
        seen: u64,
        key_hold: Duration,
        key: cairn_event::SigningKey,
        staff_id: &str,
        now: Now,
    ) -> bool {
        if key_hold.is_zero() {
            return false;
        }
        self.store_if_current(seen, SessionKey::new(key, staff_id.to_string(), now))
    }
}

/// Everything a command needs. One database connection, because a single-patient window
/// has no concurrency worth pooling for, and the mutex makes the borrow rules explicit.
pub struct AppState {
    /// `None` in fixture mode: `--mock` runs with no database at all, which is what makes
    /// the operator accessibility pass and the timing runbook possible on a laptop.
    ///
    /// `Arc` because the funnel's live port reads and writes through the SAME connection
    /// (`cairn_gui_live::LiveData::sharing`), so the window describes one node. Reopened when
    /// it is lost (#807), so a Postgres restart no longer needs a window restart.
    pub db: Option<std::sync::Arc<cairn_gui_live::NodeConnection>>,
    /// The NODE's key — holds custody of every sealed body (ADR-0052) regardless of who
    /// signed the content. Distinct from the clinician's key in `session`. `None` in
    /// fixture mode.
    pub node_sk: Option<cairn_event::SigningKey>,
    pub node_origin: String,
    /// The chart this window is open on, if any — `None` shows the front door (the funnel).
    /// Chart commands resolve it only through `AppState::displayed_patient` (`funnel::window`),
    /// which also checks it against the chart the webview is showing — never on its own.
    pub chart: tokio::sync::Mutex<Option<crate::funnel::window::OpenChart>>,
    /// Where the funnel searches and registers — mock or live, fixed at launch.
    pub funnel_backend: crate::funnel::backend::FunnelBackend,
    /// Custody of the registration form's attested search and the name it ran on.
    pub funnel: tokio::sync::Mutex<cairn_gui_funnel::FunnelSession>,
    /// Every candidate a list on screen has shown since the chart last changed, by id — the
    /// only charts `open_chart` will open, and where their header text comes from.
    pub shown:
        tokio::sync::Mutex<std::collections::HashMap<uuid::Uuid, cairn_patient_search::Candidate>>,
    /// How long a released key is held for the next act (`--key-hold-seconds`, at most
    /// `IDLE_TIMEOUT`). Zero, the default, holds nothing: every act carries a credential.
    pub key_hold: Duration,
    pub session: tokio::sync::Mutex<Held>,
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

    /// Wipe the held key if it has gone idle, and hand back the guard.
    ///
    /// The ONE place a stale key can be reached from: both accessors below go through it,
    /// so there is no path that reads a held key without first asking whether it should
    /// still be held.
    async fn expire_if_idle(&self, now: Now) -> tokio::sync::MutexGuard<'_, Held> {
        let mut guard = self.session.lock().await;
        if guard
            .key
            .as_ref()
            .is_some_and(|k| k.is_expired(now, self.key_hold))
        {
            // Dropping the SessionKey drops its Zeroizing seed, which wipes it. No generation
            // bump: expiry cannot race a release, which brings a NEW key.
            guard.key = None;
        }
        guard
    }

    /// Is a key held, and whose staff ID — WITHOUT counting the question as activity.
    ///
    /// This is what the window's lock poll calls. It expires an idle key (so the poll is
    /// what makes the re-lock *visible* on screen) but never touches a live one, because
    /// nobody is at the keyboard when a timer fires. See the module doc.
    pub async fn key_status(&self, now: Now) -> Option<String> {
        self.expire_if_idle(now)
            .await
            .key
            .as_ref()
            .map(|k| k.staff_id.clone())
    }

    /// The key that signs this CLINICAL ACT, and its kid — or why there is none.
    ///
    /// The steps (`crate::signer` has the rules):
    /// 1. [`decide`] under the session lock. A held key that serves is
    ///    touched (using it is activity) and returned with no database call.
    /// 2. A typed passphrase first WIPES any held key, whatever the vault then says: someone
    ///    new is at the keyboard, and a mistyped passphrase must not leave the last person's
    ///    key signing the next act that carries none. Another person's staff ID with no
    ///    passphrase wipes it too ([`HeldDecision::Forget`]), without asking the vault.
    /// 3. The vault is asked with the session lock released (an Argon2 check takes a moment,
    ///    and the lock poll must not wait on it). A fault's detail goes to stderr, explained
    ///    through the connection guard so a cut-off says so (#893); the clinician gets
    ///    [`KeyRefusal::Fault`], which carries no database text.
    /// 4. The released key is held for the next act only on a terminal with a key hold, and
    ///    only if no Lock or newer press wiped the slot while the vault was checking
    ///    ([`Held::keep_for_next_act`]).
    ///
    /// An all-empty credential (both fields blank) counts as none, so it never wipes a hold.
    ///
    /// In fixture mode there is no vault: no key can be held, so this is `NeedCredential`
    /// without a credential and `Fault` with one. Every mock write is refused before it asks.
    pub async fn signer_for_act(
        &self,
        credential: Option<Credential>,
        now: Now,
    ) -> Result<(cairn_event::SigningKey, String), KeyRefusal> {
        let credential = credential.filter(|c| !c.is_blank());
        let (credential, seen) = {
            let mut held = self.expire_if_idle(now).await;
            match decide(held.key.as_ref(), credential.as_ref(), self.key_hold, now) {
                HeldDecision::UseHeld => {
                    let key = held.key.as_mut().ok_or(KeyRefusal::NeedCredential)?;
                    key.touch(now);
                    return Ok((key.signing_key(), key.kid.clone()));
                }
                HeldDecision::NeedCredential => return Err(KeyRefusal::NeedCredential),
                HeldDecision::Forget => {
                    held.wipe();
                    return Err(KeyRefusal::NeedCredential);
                }
                HeldDecision::Release => {
                    let seen = held.wipe();
                    (credential.ok_or(KeyRefusal::NeedCredential)?, seen)
                }
            }
        };
        let Some(db) = self.db.as_ref() else {
            return Err(KeyRefusal::Fault);
        };
        let released = match db.lock().await {
            Err(lost) => Err(anyhow::Error::from(lost)),
            Ok(db) => cairn_node::vault::release_for_act(
                &db,
                &credential.staff_id,
                &credential.passphrase,
                Purpose::Clinical,
            )
            .await
            .map_err(|e| db.explain(e)),
        };
        let (key, kid) = match released {
            Ok(release) => from_release(release)?,
            Err(e) => {
                // Operator diagnostics, as `record_timing` does. `release_for_act` never puts the
                // passphrase in an error (cairn_node::vault's house rules).
                eprintln!("the vault could not release a signing key: {e:#}");
                return Err(KeyRefusal::Fault);
            }
        };
        // A copy for the hold (a `SigningKey` wipes itself on drop); this act signs with `key`.
        self.session.lock().await.keep_for_next_act(
            seen,
            self.key_hold,
            key.clone(),
            &credential.staff_id,
            now,
        );
        Ok((key, kid))
    }

    /// Wipe the held key now — the window's explicit Lock.
    pub async fn forget_held_key(&self) {
        // A wipe, so a release still in flight does not re-store its key afterwards.
        self.session.lock().await.wipe();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state holding nothing but a session — enough to exercise the two accessors, which
    /// is where the lock's real behaviour lives.
    fn state_holding(session: Option<SessionKey>) -> AppState {
        let mut state = AppState::mock(None);
        // A personal workstation's longest hold: the timing tests below are about it.
        state.key_hold = IDLE_TIMEOUT;
        state.session.try_lock().expect("uncontended").key = session;
        state
    }

    /// A held key is what makes the whole-list gesture cost ONE act, so how long it is
    /// held is a clinical decision, not a timer detail. Pinned so changing it is visible
    /// in a diff.
    #[test]
    fn the_idle_timeout_is_fifteen_minutes() {
        assert_eq!(IDLE_TIMEOUT, Duration::from_secs(15 * 60));
    }

    #[test]
    fn a_key_is_live_until_the_timeout_elapses() {
        let start = Now::read();
        let key = SessionKey::for_test(start);
        assert!(!key.is_expired(start, IDLE_TIMEOUT));
        assert!(!key.is_expired(start.plus(Duration::from_secs(14 * 60 + 59)), IDLE_TIMEOUT));
    }

    #[test]
    fn a_key_expires_at_the_timeout() {
        let start = Now::read();
        let key = SessionKey::for_test(start);
        assert!(key.is_expired(start.plus(IDLE_TIMEOUT), IDLE_TIMEOUT));
        assert!(key.is_expired(start.plus(Duration::from_secs(60 * 60)), IDLE_TIMEOUT));
    }

    /// Idle means idle: activity resets the clock, so a clinician working continuously is
    /// never asked for their passphrase mid-list.
    #[test]
    fn activity_pushes_the_expiry_out() {
        let start = Now::read();
        let mut key = SessionKey::for_test(start);
        key.touch(start.plus(Duration::from_secs(10 * 60)));
        assert!(!key.is_expired(start.plus(Duration::from_secs(20 * 60)), IDLE_TIMEOUT));
        assert!(key.is_expired(start.plus(Duration::from_secs(25 * 60)), IDLE_TIMEOUT));
    }

    /// A closed laptop is the commonest unattended workstation there is, and the monotonic
    /// clock does not count the time it spent asleep. The wall clock must catch it.
    #[test]
    fn a_key_expires_across_a_machine_sleep_the_monotonic_clock_did_not_count() {
        let start = Now::read();
        let key = SessionKey::for_test(start);
        assert!(
            key.is_expired(start.slept(Duration::from_secs(8 * 60 * 60)), IDLE_TIMEOUT),
            "a machine asleep for eight hours must not reopen with a live signing key"
        );
    }

    /// An unknown idle time is not evidence of recent activity (principle 4 applied to a
    /// security control), so a backwards wall-clock step re-locks rather than extends.
    #[test]
    fn a_backwards_wall_clock_step_re_locks_rather_than_extending() {
        let start = Now::read();
        let key = SessionKey::for_test(start);
        assert!(key.is_expired(
            start.wall_stepped_back(Duration::from_secs(60 * 60)),
            IDLE_TIMEOUT
        ));
    }

    /// The kid must be the key's real public identity, not a label: it is what the node
    /// checks against the enrolled-human registry, and what the window shows the clinician
    /// as "who is signed in".
    #[test]
    fn the_kid_is_the_verifying_key_in_hex() {
        let key = SessionKey::for_test(Now::read());
        let expected = hex::encode(key.signing_key().verifying_key().to_bytes());
        assert_eq!(key.kid, expected);
    }

    /// The reconstructed key must be the SAME key every time, or a session's signatures
    /// would not verify against the kid the window reported.
    #[test]
    fn the_reconstructed_key_is_stable() {
        let key = SessionKey::for_test(Now::read());
        assert_eq!(
            key.signing_key().to_bytes(),
            key.signing_key().to_bytes(),
            "the held seed must rebuild one key, not a fresh one per call"
        );
    }

    /// THE REGRESSION TEST THIS SPLIT EXISTS FOR.
    ///
    /// `main.js` polls `lock_state` every 10 seconds so the lock is ambient rather than
    /// discovered when a signature is refused. When that poll went through the touching
    /// accessor it reset the idle clock every 10 seconds with nobody at the keyboard, so
    /// `is_expired` could never become true and the key stayed unlocked for the life of the
    /// window — an unattended workstation from which any passer-by could sign off a whole
    /// medication chart under the clinician's key. Every unit test above still passed,
    /// because they all exercise `SessionKey` in isolation and never the polled path.
    #[tokio::test]
    async fn polling_the_lock_state_never_keeps_the_key_alive() {
        let start = Now::read();
        let state = state_holding(Some(SessionKey::for_test(start)));

        // The window's own poll interval, run right across the timeout.
        let poll = Duration::from_secs(10);
        let mut now = start;
        for _ in 0..(IDLE_TIMEOUT.as_secs() / poll.as_secs()) {
            now = now.plus(poll);
            state.key_status(now).await;
        }

        assert!(
            state.key_status(now).await.is_none(),
            "a key polled every {}s must still re-lock after {}s of no CLINICAL activity",
            poll.as_secs(),
            IDLE_TIMEOUT.as_secs()
        );
    }

    /// The other half of the same rule: a clinician who IS working must never be asked for
    /// their passphrase mid-list, so a real act does push the expiry out.
    #[tokio::test]
    async fn signing_keeps_the_key_alive() {
        let start = Now::read();
        let state = state_holding(Some(SessionKey::for_test(start)));

        // A sign-off every 10 minutes for an hour — a plausible ward round.
        let mut now = start;
        for _ in 0..6 {
            now = now.plus(Duration::from_secs(10 * 60));
            assert!(
                state.signer_for_act(None, now).await.is_ok(),
                "a key in continuous clinical use must not re-lock"
            );
        }
    }

    /// And the boundary between them: the poll must still SHOW the re-lock, because the
    /// clinician learning about it from a refused signature is the failure the ambient
    /// lock display exists to prevent.
    #[tokio::test]
    async fn the_poll_reports_the_re_lock_it_did_not_cause() {
        let start = Now::read();
        let state = state_holding(Some(SessionKey::for_test(start)));

        assert!(state.key_status(start).await.is_some());
        assert!(state.key_status(start.plus(IDLE_TIMEOUT)).await.is_none());
        assert!(
            state.session.lock().await.key.is_none(),
            "the expired key must be WIPED by the check, not merely reported as expired"
        );
    }

    /// Fix round 1 (the wrong-author race): a release still in flight when Lock is pressed must
    /// NOT store its key. Otherwise the next act with no credential signs as her.
    #[test]
    fn a_lock_between_the_wipe_and_the_store_drops_the_released_key() {
        let now = Now::read();
        let mut held = Held::default();
        let seen = held.wipe();
        held.wipe(); // Lock (`forget_held_key`) while the vault is still checking.
        assert!(!held.store_if_current(seen, SessionKey::for_test(now)));
        assert!(held.key.is_none());
    }

    /// Two credentialed presses overlapping: the LAST TYPED wins, never the last to finish.
    #[test]
    fn a_newer_release_outranks_an_older_one_that_finishes_later() {
        let now = Now::read();
        let mut held = Held::default();
        let first = held.wipe();
        let second = held.wipe();
        let mut newer = SessionKey::for_test(now);
        newer.staff_id = "apark".into();
        assert!(held.store_if_current(second, newer));
        assert!(!held.store_if_current(first, SessionKey::for_test(now)));
        assert_eq!(
            held.key.as_ref().map(|k| k.staff_id.as_str()),
            Some("apark")
        );
    }

    /// The step `signer_for_act` takes after the vault answers (PR #897 review: the race rule was
    /// pinned only on its building block). A shared terminal (hold 0) never holds, even with
    /// nothing interleaved; a Lock while the vault was checking drops the key; otherwise it is held.
    #[test]
    fn a_released_key_is_kept_only_on_a_hold_terminal_and_only_if_nothing_wiped() {
        let now = Now::read();
        let key = || SessionKey::for_test(now).signing_key();
        let mut held = Held::default();
        let seen = held.wipe();
        assert!(!held.keep_for_next_act(seen, Duration::ZERO, key(), "jsmith", now));
        assert!(held.key.is_none(), "hold 0 holds nothing");

        let seen = held.wipe();
        held.wipe(); // Lock while the vault is checking.
        assert!(!held.keep_for_next_act(seen, IDLE_TIMEOUT, key(), "jsmith", now));
        assert!(held.key.is_none(), "a Lock in flight wins");

        let seen = held.wipe();
        assert!(held.keep_for_next_act(seen, IDLE_TIMEOUT, key(), " jsmith ", now));
        assert_eq!(
            held.key.as_ref().map(|k| k.staff_id.as_str()),
            Some("jsmith")
        );
    }

    /// ...and `signer_for_act` takes it with the generation its OWN wipe returned, not one read
    /// later (which would let a Lock in flight be undone).
    #[test]
    fn signer_for_act_keeps_the_key_with_the_generation_of_its_own_wipe() {
        let body: String = crate::source_guard::rust_fn(include_str!("state.rs"), "signer_for_act")
            .split_whitespace()
            .collect();
        let wipe = body.find("letseen=held.wipe();").expect(&body);
        let keep = body.find(".keep_for_next_act(seen,").expect(&body);
        assert!(wipe < keep, "{body}");
        assert_eq!(body.matches("store_if_current(").count(), 0, "{body}");
    }

    /// With nothing interleaved, the released key is held.
    #[test]
    fn an_uninterrupted_release_is_held() {
        let mut held = Held::default();
        let seen = held.wipe();
        assert!(held.store_if_current(seen, SessionKey::for_test(Now::read())));
        assert!(held.key.is_some());
    }

    /// `held_by` shows the staff ID without the spaces a keyboard left around it. Spaces only:
    /// case and Unicode normalisation live in SQL alone (`cairn_vault_staff_id`, R7).
    #[test]
    fn the_held_staff_id_is_trimmed_of_spaces_only() {
        let key = SessionKey::for_test(Now::read());
        let held = SessionKey::new(key.signing_key(), " JSmith ".into(), Now::read());
        assert_eq!(held.staff_id, "JSmith");
    }

    /// An all-empty credential is no credential: a webview sending empty fields never wipes a
    /// hold, and the held key still serves.
    #[tokio::test]
    async fn an_all_empty_credential_keeps_the_hold() {
        let now = Now::read();
        let state = state_holding(Some(SessionKey::for_test(now)));
        let blank = Credential {
            staff_id: "  ".into(),
            passphrase: Zeroizing::new(" ".into()),
        };
        assert!(state.signer_for_act(Some(blank), now).await.is_ok());
        assert!(state.session.lock().await.key.is_some());
    }

    /// Review focus 2, at the seam: a typed credential wipes the held key BEFORE the vault
    /// answers, so a mistyped passphrase (here, fixture mode's `Fault`) never leaves the last
    /// person's key to sign the next act that carries none.
    #[tokio::test]
    async fn a_typed_credential_wipes_the_held_key_whatever_the_answer() {
        let now = Now::read();
        let state = state_holding(Some(SessionKey::for_test(now)));
        let credential = Credential {
            staff_id: "apark".into(),
            passphrase: Zeroizing::new(["not", "hers"].join("-")),
        };
        assert_eq!(
            state.signer_for_act(Some(credential), now).await.err(),
            Some(KeyRefusal::Fault)
        );
        assert!(state.session.lock().await.key.is_none());
        assert_eq!(
            state.signer_for_act(None, now).await.err(),
            Some(KeyRefusal::NeedCredential)
        );
    }

    /// Final review item 1, at the seam: the holder's own staff ID with an empty passphrase signs
    /// with the held key; anyone else's wipes it and asks for a passphrase. Neither asks the vault
    /// (in fixture mode a vault call would be `Fault`), so an empty passphrase is never a counted
    /// wrong guess.
    #[tokio::test]
    async fn a_staff_id_alone_never_asks_the_vault() {
        let id_only = |id: &str| Credential {
            staff_id: id.into(),
            passphrase: Zeroizing::new(String::new()),
        };
        let now = Now::read();
        let state = state_holding(Some(SessionKey::for_test(now)));
        assert!(state
            .signer_for_act(Some(id_only(" test-signer ")), now)
            .await
            .is_ok());
        assert_eq!(
            state
                .signer_for_act(Some(id_only("apark")), now)
                .await
                .err(),
            Some(KeyRefusal::NeedCredential)
        );
        assert!(
            state.session.lock().await.key.is_none(),
            "another person's ID must forget the held key"
        );
        assert_eq!(
            state
                .signer_for_act(Some(id_only("apark")), now)
                .await
                .err(),
            Some(KeyRefusal::NeedCredential),
            "with nothing held, an ID alone is not a credential either"
        );
    }

    /// The explicit Lock wipes the held key.
    #[tokio::test]
    async fn forgetting_the_held_key_wipes_it() {
        let state = state_holding(Some(SessionKey::for_test(Now::read())));
        state.forget_held_key().await;
        assert!(state.key_status(Now::read()).await.is_none());
    }

    /// A locked window has no key to take, and asking must not create one.
    #[tokio::test]
    async fn an_empty_session_yields_nothing_from_either_accessor() {
        let state = state_holding(None);
        let now = Now::read();
        assert!(state.key_status(now).await.is_none());
        assert_eq!(
            state.signer_for_act(None, now).await.err(),
            Some(KeyRefusal::NeedCredential)
        );
    }
}
