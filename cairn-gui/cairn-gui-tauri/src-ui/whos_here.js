// Who's here (sign-in slice 3b-1, design §3.1): the terminal's CLAIM of who is at the keyboard —
// the bar at the top, the picker, the hot key and the bookmark offer.
//
// A claim, never authentication: naming someone signs nothing and releases no key, and reading a
// chart never needs a name. It is asked only when writing begins (New note, "Continue here"), or
// when someone presses "Who's here?" / Ctrl+Shift+H (Cmd+Shift+H on macOS). "Don't know / just
// reading" is an allowed answer. The node (`whos_here.rs`) holds the claim and words every
// sentence; this file only draws it, and decides two orders, each pinned in
// whos_here_webview_tests.rs:
//  - a switch takes the chart off screen and AWAITS its saves before a name can be picked
//    (Review Focus 1), so Nurse A's last words are saved under Nurse A;
//  - the claim drawn here changes only at a pick or the re-lock: the 10-second lock poll seeds
//    it at launch and never again, so an older poll answer cannot undo a newer pick.
//
// Recent people and bookmarks live in the node's window memory only, never browser storage.
// Classic script, loaded after notes.js, sharing main.js's globals (`el`, `cell`, `setMessage`,
// `invoke`, `pollLock`), signer.js's `onStaffIdChange`, funnel.js's `closeChart`/`openChart` and
// funnel.js's `failureText` — all read only at a press, long after every script has loaded.
"use strict";

/** The claim on screen (Rust's `ClaimView`), or null before the first lock poll answers. */
let currentClaim = null;
/** The directory the picker shows (Rust's `StaffEntryView`s), read each time it opens. */
let pickerEntries = [];
/** What to do once a claim is made from the picker (New note, Continue here), or null. */
let afterPick = null;

/** The claim on screen, for notes.js to capture as a new part's writer. */
function whosHere() {
  return currentClaim;
}

/** Has anybody said who is here (a name, or "Don't know")? Nobody asked owns no part. */
function claimAsked() {
  return currentClaim !== null && currentClaim.kind !== "not-asked";
}

/**
 * Draw a claim in the bar. The words are written only when they change: the bar is a polite
 * live region, and rewriting the same words every poll would re-announce them.
 */
function showClaim(claim) {
  currentClaim = claim;
  const bar = el("whos-here-name");
  if (bar.textContent !== claim.words) bar.textContent = claim.words;
}

/** The lock poll's claim (main.js's `renderLockState`): taken only at launch, never over a pick. */
function seedWhosHere(claim) {
  if (currentClaim === null) showClaim(claim);
}

/**
 * Take a claim the node now holds (a pick, a refused pick, the re-lock). The signer strip's
 * staff ID becomes the claim's — the directory's own id, verbatim — or empty when nobody is
 * named. The passphrase is never touched: a claim never signs anything.
 */
function takeClaim(claim) {
  showClaim(claim);
  el("staff-id").value = claim.staff_id || "";
  onStaffIdChange(); // its hint and its expiry warning, the last person's gone (signer.js)
}

/**
 * "Who's here?" and the hot key: the chart leaves the screen first, its queued saves awaited
 * (`closeChart`), so nothing typed is saved under the next person; then the picker opens.
 */
async function switchWhosHere() {
  if (!el("whos-here-picker").hidden) return;
  await closeChart();
  openPicker(null);
}

/** Ask who's here, then run `then` once a claim is made (New note, Continue here). */
function askWhosHere(then) {
  if (!el("whos-here-picker").hidden) return;
  openPicker(then);
}

/** Show the picker over the page, the rest of it out of Tab's reach, the search focused. */
async function openPicker(then) {
  afterPick = then;
  dismissOffer();
  el("whos-here-search").value = "";
  setMessage(el("whos-here-status"), "");
  el("whos-here-list").replaceChildren();
  el("whos-here-picker").hidden = false;
  document.querySelector("main").inert = true;
  el("whos-here-search").focus();
  try {
    pickerEntries = await invoke("whos_here_directory");
  } catch (failure) {
    pickerEntries = [];
    setMessage(el("whos-here-status"), "Could not read the staff list: " + failureText(failure));
  }
  drawPicker();
}

/** Close the picker without a pick (Esc): the claim stays as it was. */
function closePicker() {
  afterPick = null;
  el("whos-here-picker").hidden = true;
  document.querySelector("main").inert = !el("terminal-lock").hidden;
}

/** The people matching the search, by name or staff ID, recent people first (the node's order). */
function matchingEntries() {
  const typed = el("whos-here-search").value.trim().toLowerCase();
  return pickerEntries.filter(
    (entry) =>
      entry.display_name.toLowerCase().includes(typed) || entry.staff_id.toLowerCase().includes(typed),
  );
}

/** Draw the picker's list: one button per person, named for a screen reader as it reads. */
function drawPicker() {
  el("whos-here-list").replaceChildren(
    ...matchingEntries().map((entry) => {
      const li = document.createElement("li");
      const b = cell(
        "button",
        entry.display_name + " — " + entry.role + " (" + entry.staff_id + ")" + (entry.recent ? " · recent here" : ""),
        { type: "button" },
      );
      b.addEventListener("click", () => void pick(entry.staff_id));
      li.append(b);
      return li;
    }),
  );
}

/**
 * Say who is here: `staffId` from the directory, verbatim, or null for "Don't know". The node
 * forgets any held key and the old claim FIRST, so a refused pick leaves nobody named: the
 * claim is then read back, never assumed. A switch offers the person's bookmark; a pick asked
 * for by New note or Continue here goes on with that instead.
 */
async function pick(staffId) {
  let picked;
  try {
    picked = await invoke("whos_here_pick", { staffId });
  } catch (failure) {
    setMessage(el("whos-here-status"), failureText(failure));
    try {
      takeClaim((await invoke("lock_state")).whos_here);
    } catch (e) {
      console.error("the claim could not be read back", e);
    }
    return;
  }
  const then = afterPick;
  closePicker();
  takeClaim(picked.claim);
  void pollLock(); // the held key was forgotten: the strip says so now, not at the next poll
  if (then) then();
  else if (picked.offer) showOffer(picked.offer);
}

/**
 * The bookmark: the chart this person last opened here, OFFERED as one pre-focused button,
 * never auto-opened (maintainer decision 3). Esc or a search dismisses it.
 */
function showOffer(offer) {
  const b = el("whos-here-return");
  b.textContent = "Return to " + offer.name + " — " + offer.born;
  b.dataset.patient = offer.patient_id;
  el("whos-here-offer").hidden = false;
  b.focus();
}

function dismissOffer() {
  el("whos-here-offer").hidden = true;
}

/** Ctrl+Shift+H (Cmd+Shift+H on macOS) is "Who's here?"; Esc closes the picker or the offer. */
function onHotKey(event) {
  if (!el("terminal-lock").hidden) return; // nobody can switch at a locked terminal
  if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === "h") {
    event.preventDefault();
    void switchWhosHere();
  } else if (event.key === "Escape") {
    if (!el("whos-here-picker").hidden) closePicker();
    else dismissOffer();
  }
}

document.addEventListener("keydown", onHotKey);
el("whos-here-switch").addEventListener("click", () => void switchWhosHere());
el("whos-here-search").addEventListener("input", drawPicker);
// Enter in the search picks the first match, so a typed name or ID is one keystroke away.
el("whos-here-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const first = matchingEntries()[0];
  if (first) void pick(first.staff_id);
});
el("whos-here-unknown").addEventListener("click", () => void pick(null));
el("whos-here-cancel").addEventListener("click", closePicker);
el("whos-here-return").addEventListener("click", () => {
  dismissOffer();
  void openChart(el("whos-here-return").dataset.patient, "browse-status");
});
el("browse-form").addEventListener("input", dismissOffer);
