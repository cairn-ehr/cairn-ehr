// The signer strip and the terminal lock (sign-in slice 1).
//
// Two different questions, kept apart:
//  - WHO signs the next act: the strip's staff ID + personal passphrase, sent with each write
//    (`credential: signerCredential()`). The node's vault checks it; this file never does.
//  - MAY ANYONE USE THIS TERMINAL: the shared terminal passphrase on the lock screen
//    (#terminal-lock), shown at launch in live mode. Screensaver-style, not a confirmation
//    dialog: it is an overlay on the page, never a modal.
// Classic script, loaded after main.js (uses its `el`, `setMessage`, `invoke`).

const RECENT_SIGNERS_MAX = 10;
// shortcut: forgotten on restart; persist per terminal if nurses ask
let recentSigners = [];
// The staff ID whose key the window holds for the next act (lock_state.held_by), or null.
let heldBy = null;
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
 * on screen for the next person. The staff ID stays only while the window holds that person's key.
 */
function clearSigner() {
  el("signer-passphrase").value = "";
  if (!heldBy) {
    el("staff-id").value = "";
    setMessage(el("signer-hint"), "");
  }
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
}

async function tryTerminalUnlock(event) {
  event.preventDefault();
  const field = el("terminal-passphrase");
  let verdict;
  try {
    verdict = await invoke("terminal_unlock", { passphrase: field.value });
  } catch (e) {
    verdict = "fault";
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
        : "The terminal could not be unlocked. Try again.",
  );
}

async function showSignerHint() {
  const staffId = el("staff-id").value;
  rememberSigner(staffId);
  let hint = null;
  if (staffId.trim() !== "") {
    try {
      hint = await invoke("vault_hint", { staffId: staffId });
    } catch (e) {
      hint = null; // a hint is a courtesy; the refusal on the act says what matters
    }
  }
  setMessage(el("signer-hint"), hint ? "Your hint: " + hint : "");
}

async function lockNow() {
  try {
    renderSigner(await invoke("forget_held_key"));
  } catch (e) {
    el("lock-state").textContent = "Could not lock the signing key: " + failureText(e);
    return;
  }
  el("signer-passphrase").value = "";
  el("staff-id").value = "";
  setMessage(el("signer-hint"), "");
}

// Enter in the strip must not submit the page (the strip has no submit of its own).
el("signer").addEventListener("submit", (event) => event.preventDefault());
el("staff-id").addEventListener("change", showSignerHint);
el("forget-key").addEventListener("click", lockNow);
el("terminal-form").addEventListener("submit", tryTerminalUnlock);
