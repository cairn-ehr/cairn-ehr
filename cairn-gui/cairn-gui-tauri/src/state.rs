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
use crate::signer::{decide, from_release_noting, Credential, HeldDecision, KeyRefusal};
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
    /// The staff ID it was released for, trimmed of spaces (`new`): the window shows it as "who
    /// signs next".
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

    /// Pure: wipe the held key if it is still `kid`. A stale key found by the re-check
    /// (`AppState::held_if_still_current`) goes; a newer key a press stored meanwhile stays.
    pub fn forget_if_holding(&mut self, kid: &str) {
        if self.key.as_ref().is_some_and(|k| k.kid == kid) {
            self.wipe();
        }
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
    /// This terminal's stable id (`crate::terminal_id`): what a note draft is held by, so a draft
    /// is edited only at the terminal it was started on (design §3.2). A `OnceLock` because only
    /// `main`'s setup hook can name the config directory it lives in, and the state is managed
    /// before that hook runs; commands read it through `notes::terminal_of`.
    pub terminal_id: std::sync::OnceLock<String>,
    /// Who the terminal says is at the keyboard, their recent people and bookmarks, and the idle
    /// re-lock interval — in memory only (`crate::whos_here`).
    pub whos_here: tokio::sync::Mutex<crate::whos_here::Here>,
    /// The drafts whose LAST quarantine commit in this window failed (`notes::commit`), so the
    /// pane says the commit failed rather than promising another try. Cleared by a try that did
    /// not fail. shortcut: a failed draft later signed or discarded stays listed until the window
    /// closes; harmless (nothing reads an id no draft has), bounded by the drafts seen.
    pub commit_failed: tokio::sync::Mutex<std::collections::HashSet<uuid::Uuid>>,
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
    /// 1. [`decide`] under the session lock. A held key that serves is touched (using it is
    ///    activity), then re-checked with the lock released: one that was re-keyed, repudiated or
    ///    expired elsewhere is wiped and the act asks for a passphrase (#1138,
    ///    `held_if_still_current`).
    /// 2. A typed passphrase first WIPES any held key, whatever the vault then says: someone
    ///    new is at the keyboard, and a mistyped passphrase must not leave the last person's
    ///    key signing the next act that carries none. Another person's staff ID with no
    ///    passphrase wipes it too ([`HeldDecision::Forget`]), without asking the vault.
    /// 3. The vault is asked with the session lock released (an Argon2 check takes a moment,
    ///    and the lock poll must not wait on it). A fault's detail goes to stderr, explained
    ///    through the connection guard so a cut-off says so (#893); the clinician gets
    ///    [`KeyRefusal::Fault`], which carries no database text.
    /// 4. The released key is held for the next act only on a terminal with a key hold, only if
    ///    who's here names nobody or names the key's own person, under whose claimed ID it is held
    ///    (#1128, `hold_name_for_claim`), and only if no Lock or newer press wiped the slot while
    ///    the vault was checking ([`Held::keep_for_next_act`]).
    ///
    /// An all-empty credential (both fields blank) counts as none, so it never wipes a hold.
    ///
    /// In fixture mode there is no vault: no key can be held, so this is `NeedCredential`
    /// without a passphrase and `Fault` with one. Every mock write is refused before it asks.
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
                    let kept = (key.signing_key(), key.kid.clone());
                    drop(held); // the check below must not hold the session lock over a database call
                    return self.held_if_still_current(kept).await;
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
        // The vault's answer as a key or a refusal. `from_release_noting` runs while the
        // connection is in hand: a replaced passphrase looks up its notice once.
        let released = match db.lock().await {
            Err(lost) => Err(anyhow::Error::from(lost)),
            Ok(db) => {
                match cairn_node::vault::release_for_act(
                    &db,
                    &credential.staff_id,
                    &credential.passphrase,
                    Purpose::Clinical,
                )
                .await
                {
                    Ok(release) => {
                        // Operator diagnostics: the screen says "ask IT"; IT needs the reason.
                        // `why` is a fixed sentence from `cairn_node::conn_posture`.
                        if let cairn_node::vault::Release::OverPowered { why } = &release {
                            eprintln!("the window's connection is over-powered: {why}");
                        }
                        Ok(from_release_noting(&db, &credential.staff_id, release).await)
                    }
                    Err(e) => Err(db.explain(e)),
                }
            }
        };
        let (key, kid) = match released {
            Ok(answer) => answer?,
            Err(e) => {
                // Operator diagnostics, as `record_timing` does. `release_for_act` never puts the
                // passphrase in an error (cairn_node::vault's house rules).
                eprintln!("the vault could not release a signing key: {e:#}");
                return Err(KeyRefusal::Fault);
            }
        };
        // Never hold another person's key under the name who's here shows (#1128): this act signs
        // with the key just released either way, since its owner typed the passphrase.
        let Some(holder) = self.hold_name_for_claim(&kid, &credential.staff_id).await else {
            return Ok((key, kid));
        };
        // A copy for the hold (a `SigningKey` wipes itself on drop); this act signs with `key`.
        self.session
            .lock()
            .await
            .keep_for_next_act(seen, self.key_hold, key.clone(), &holder, now);
        Ok((key, kid))
    }

    /// A held key signs only while the vault would still release it (#1138 item 1): one that was
    /// re-keyed, repudiated or expired at another terminal is wiped here, and the act asks for the
    /// passphrase (a re-keyed person's new one releases the new key). A check that cannot be made
    /// refuses the act as a fault ("ask IT"), as the passphrase path does, and keeps the key (a
    /// blip must not cost a passphrase). Never "sign anyway": the floor refuses a re-keyed or
    /// repudiated key, but never reads the site's expiry, so this check is that rule's only door.
    async fn held_if_still_current(
        &self,
        (key, kid): (cairn_event::SigningKey, String),
    ) -> Result<(cairn_event::SigningKey, String), KeyRefusal> {
        let Some(db) = self.db.as_ref() else {
            return Ok((key, kid)); // fixture mode never holds a key; nothing to ask
        };
        let may_sign = match db.lock().await {
            Err(lost) => Err(anyhow::Error::from(lost)),
            Ok(db) => cairn_node::vault::key_may_sign(&db, &kid)
                .await
                .map_err(|e| db.explain(e)),
        };
        match may_sign {
            Ok(true) => Ok((key, kid)),
            Ok(false) => {
                self.session.lock().await.forget_if_holding(&kid);
                Err(KeyRefusal::NeedCredential)
            }
            Err(e) => {
                eprintln!(
                    "the held key's currency could not be checked, so it did not sign: {e:#}"
                );
                Err(KeyRefusal::Fault)
            }
        }
    }

    /// The staff ID key `kid` is held under for the next act, or `None` to hold nothing
    /// ([`crate::whos_here::Claim::hold_under`]; `typed` is the act's typed ID). A named
    /// person's current key is read from the vault; a read that fails holds nothing (the next act
    /// asks for a passphrase, never a wrong signer), and says why on stderr for IT. No hold on
    /// this terminal means nothing to decide, and no database call.
    async fn hold_name_for_claim(&self, kid: &str, typed: &str) -> Option<String> {
        if self.key_hold.is_zero() {
            return None;
        }
        let claim = self.whos_here.lock().await.claim().clone();
        let claimed_key = match (&claim, self.db.as_ref()) {
            (crate::whos_here::Claim::Named { staff_id, .. }, Some(db)) => {
                let read = match db.lock().await {
                    Err(lost) => Err(anyhow::Error::from(lost)),
                    Ok(db) => cairn_node::medication::draft::writer_key(&*db, staff_id)
                        .await
                        .map_err(|e| db.explain(e)),
                };
                read.unwrap_or_else(|e| {
                    eprintln!("who's here: {staff_id}'s current key could not be read, so no key is held: {e:#}");
                    None
                })
            }
            _ => None,
        };
        claim
            .hold_under(claimed_key.as_deref(), kid, typed)
            .map(str::to_string)
    }

    /// Wipe the held key now — the window's explicit Lock.
    pub async fn forget_held_key(&self) {
        // A wipe, so a release still in flight does not re-store its key afterwards.
        self.session.lock().await.wipe();
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
