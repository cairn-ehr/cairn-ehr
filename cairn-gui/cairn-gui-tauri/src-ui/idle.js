// The idle re-lock (sign-in slice 3b-1, design §3.3): a terminal nobody has touched for
// `--idle-lock-minutes` takes the chart off screen, forgets the held key and who's here, and
// asks for the terminal passphrase again — the folder closed and put back in the rack.
//
// Only a person at the keyboard resets the clock: a key, a pointer press, typed input or a
// scroll wheel, listened for in the capture phase so a keystroke in any field (the passphrase
// field included) counts. Never a poll, an interval or an IPC answer — main.js's lock-poll note
// tells how a poll that counted as activity kept a key from ever re-locking. So the clock is
// first armed by the first touch, and the poll only hands over the interval (`setIdleLock`).
//
// Classic script, loaded after whos_here.js, sharing main.js's `el` and `invoke`, notes.js's
// `signsSettled`, funnel.js's `closeChart`, signer.js's `relockTerminal` and whos_here.js's
// `takeClaim`/`closePicker`/`dismissOffer` — all read only when the timer fires.
"use strict";

/** This terminal's idle re-lock in ms (lock_state.idle_lock_secs); 0 until the poll says. */
let idleLockMs = 0;
/** The pending re-lock, or null: none armed yet, or it has fired. */
let idleTimer = null;

/** The poll's interval (main.js's `renderLockState`). Never arms or pushes back the timer. */
function setIdleLock(secs) {
  idleLockMs = secs * 1000;
}

/** A person touched the terminal: the re-lock starts counting again from now. */
function noteActivity() {
  clearTimeout(idleTimer);
  idleTimer = idleLockMs > 0 ? setTimeout(idleRelock, idleLockMs) : null;
}

/**
 * The timer fired. A Sign in flight is waited for first (Review Focus 4): cutting it would leave
 * its outcome unsaid. Someone who touched the terminal meanwhile is there, so nothing locks.
 * Then the chart closes (its queued saves awaited), the window's Rust side forgets the key and
 * the claim, and the terminal lock shows again.
 */
async function idleRelock() {
  idleTimer = null;
  if (!el("terminal-lock").hidden) return; // already locked: nothing on screen to put away
  await signsSettled();
  if (idleTimer !== null) return;
  if (!el("whos-here-picker").hidden) closePicker();
  dismissOffer();
  await closeChart();
  let lock = null;
  try {
    lock = await invoke("relock");
    takeClaim(lock.whos_here);
  } catch (failure) {
    console.error("the re-lock could not reach the node", failure);
  }
  relockTerminal(lock);
}

for (const kind of ["keydown", "pointerdown", "input", "wheel"]) {
  document.addEventListener(kind, noteActivity, { capture: true, passive: true });
}
