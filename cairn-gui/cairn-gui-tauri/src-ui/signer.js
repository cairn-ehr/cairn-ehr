// The signer strip and the terminal lock (sign-in slice 1).
//
// Two different questions, kept apart:
//  - WHO signs the next act: the strip's staff ID + personal passphrase, sent with each write
//    (`credential: signerCredential()`). The node's vault checks it; this file never does.
//  - MAY ANYONE USE THIS TERMINAL: the shared terminal passphrase on the lock screen
//    (#terminal-lock), shown once at launch in live mode (it does not re-lock when idle). Not
//    a confirmation dialog: it is an overlay on the page, never a modal.
// Classic script, loaded after main.js (uses its `el`, `setMessage`, `invoke`).

const RECENT_SIGNERS_MAX = 10;
// shortcut: forgotten on restart; persist per terminal if nurses ask
let recentSigners = [];
// The staff ID whose key the window holds for the next act (lock_state.held_by), or null.
let heldBy = null;
// This terminal's key hold in seconds (lock_state.key_hold_secs): 0 on a shared terminal.
let keyHoldSecs = 0;
let terminalUnlocked = false;

const NOT_SET_SENTENCE =
  "No terminal passphrase is set. Run cairn-node vault-set-terminal-passphrase on the node.";

/** The strip as `{staffId, passphrase}`, or null when both fields are empty (never empty strings). */
function signerCredential() {
  const staffId = el("staff-id").value;
  const passphrase = el("signer-passphrase").value;
  if (staffId.trim() === "" && passphrase.trim() === "") return null;
  return { staffId: staffId, passphrase: passphrase };
}

/**
 * Called after EVERY write comes back, success and refusal alike: a typed passphrase never stays
 * on screen for the next person. The staff ID stays on a terminal with a key hold, so the holder
 * carries on with no passphrase; it is not `heldBy` that decides, because `heldBy` is only
 * refreshed by the poll below, after this runs. Keeping it is safe: the node signs an ID with no
 * passphrase only with that same person's live held key, never by asking the vault.
 * `signed` is true on the success path: the ID then joins the type-ahead list.
 */
function clearSigner(signed) {
  const typed = el("staff-id").value.trim();
  // Only a SIGNED act enters the recent list, so a typo never does (shared terminal).
  if (signed) rememberSigner(el("staff-id").value);
  el("signer-passphrase").value = "";
  if (keyHoldSecs === 0) {
    el("staff-id").value = "";
    setMessage(el("signer-hint"), "");
    // The expiry warning belongs to the ID just cleared: never the next person's.
    setMessage(el("signer-notice"), "");
  }
  // A typed ID was warned at its `change` (onStaffIdChange). A held-key act with NO ID typed
  // never was, so it asks once now, for the holder (design §5.5).
  if (signed && typed === "" && heldBy) void showExpiryNotice(heldBy);
  // Refresh the lock state at once, so "Signed in for the next act" and `heldBy` are current.
  void pollLock();
}

/** Remember an ID for the type-ahead list (most recent first, at most 10, this session only). */
function rememberSigner(staffId) {
  const id = staffId.trim();
  if (id === "") return;
  recentSigners = [id, ...recentSigners.filter((x) => x !== id)].slice(0, RECENT_SIGNERS_MAX);
  el("recent-signers").replaceChildren(
    ...recentSigners.map((x) => {
      const option = document.createElement("option");
      option.value = x;
      return option;
    }),
  );
}

/** Draw the strip and the lock screen from a `lock_state` answer (main.js polls it). */
function renderSigner(lock) {
  heldBy = lock.held_by || null;
  keyHoldSecs = lock.key_hold_secs || 0;
  el("forget-key").hidden = !heldBy;
  if (lock.mock) {
    // terminal_unlock always answers "wrong" in fixture mode: a lock here would trap the window.
    setTerminalLocked(false);
    el("lock-state").textContent =
      "Fixture data. This window is showing a sample chart and cannot write to a record.";
    return;
  }
  setTerminalLocked(!terminalUnlocked);
  el("lock-state").textContent = heldBy ? "Signed in for the next act: " + heldBy + "." : "";
}

/** Show or hide the lock screen; while it shows, the page behind it cannot be reached by Tab. */
function setTerminalLocked(locked) {
  el("terminal-lock").hidden = !locked;
  document.querySelector("main").inert = locked;
  if (locked) el("terminal-passphrase").focus();
}

async function tryTerminalUnlock(event) {
  event.preventDefault();
  const field = el("terminal-passphrase");
  let verdict;
  try {
    verdict = await invoke("terminal_unlock", { passphrase: field.value });
  } catch (e) {
    verdict = "fault"; // the call itself failed: said as a fault, never as a wrong passphrase
  }
  field.value = "";
  if (verdict === "unlocked") {
    terminalUnlocked = true;
    setMessage(el("terminal-status"), "");
    setTerminalLocked(false);
    el("staff-id").focus();
    return;
  }
  setMessage(
    el("terminal-status"),
    verdict === "not-set"
      ? NOT_SET_SENTENCE
      : verdict === "wrong"
        ? "That passphrase is not right."
        : "The terminal could not be unlocked. Try again.", // "fault", or the call itself failed
  );
}

async function showSignerHint() {
  const staffId = el("staff-id").value;
  let said = "";
  if (staffId.trim() !== "") {
    try {
      const hint = await invoke("vault_hint", { staffId: staffId });
      said = hint ? "Your hint: " + hint : "";
    } catch (e) {
      // Said, not hidden: an empty line here would read as "you set no hint". Rust sends a
      // plain sentence; only an IPC-level failure is something else.
      said = typeof e === "string" ? e : failureText(e);
    }
  }
  setMessage(el("signer-hint"), said);
}

/**
 * The days-ahead warning for an unconfirmed key (§5.5), BEFORE the act when the ID is typed:
 * never a surprise at the moment of signing. Nothing when none applies. Shown only if `staffId`
 * is still the one in the strip (or the holder, with none typed) when the answer arrives, so a
 * slow answer never labels the next person.
 */
async function showExpiryNotice(staffId) {
  let said = "";
  try {
    said = await invoke("expiry_notice", { staffId: staffId });
  } catch (e) {
    // Only an IPC-level failure lands here (Rust answers None for a failed read); an expired
    // key's refusal still says why at the press.
  }
  if (staffId === (el("staff-id").value.trim() || heldBy)) setMessage(el("signer-notice"), said);
}

/** A new staff ID in the strip: its own hint and its own expiry warning, the last one's gone. */
function onStaffIdChange() {
  setMessage(el("signer-notice"), "");
  void showSignerHint();
  const staffId = el("staff-id").value.trim();
  if (staffId !== "") void showExpiryNotice(staffId);
}

async function lockNow() {
  // Emptied FIRST: a Lock whose call fails must still leave no typed credential behind for the
  // next person's press to sign with.
  el("signer-passphrase").value = "";
  el("staff-id").value = "";
  setMessage(el("signer-hint"), "");
  setMessage(el("signer-notice"), "");
  try {
    renderSigner(await invoke("forget_held_key"));
  } catch (e) {
    el("lock-state").textContent = "Could not lock the signing key: " + failureText(e);
  }
}

// Enter in the strip must not submit the page (the strip has no submit of its own).
el("signer").addEventListener("submit", (event) => event.preventDefault());
el("staff-id").addEventListener("change", onStaffIdChange);
el("forget-key").addEventListener("click", lockNow);
el("terminal-form").addEventListener("submit", tryTerminalUnlock);
