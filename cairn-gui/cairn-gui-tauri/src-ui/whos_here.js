// Who's here (sign-in slice 3b-1, design §3.1): the terminal's CLAIM of who is at the keyboard —
// the bar at the top, the picker, the hot key and the bookmark offer.
//
// A claim, never authentication: naming someone signs nothing and releases no key, and reading a
// chart never needs a name. It is asked only when writing begins (New note, "Continue here"), or
// when someone presses "Who's here?" / Ctrl+Shift+H (Cmd+Shift+H on macOS). "Don't know / just
// reading" is an allowed answer. The window's own Rust side (`whos_here.rs`, in `AppState` — not
// cairn-node, which never learns who is here) holds the claim and words its sentences, all but
// the picker's few lines below; this file draws it, and decides two orders, each pinned in
// whos_here_webview_tests.rs:
//  - a switch takes the chart off screen and AWAITS its saves before a name can be picked
//    (Review Focus 1), so Nurse A's last words are saved under Nurse A;
//  - the claim drawn here changes only at a pick or the re-lock: the 10-second lock poll seeds
//    it at launch and never again, so an older poll answer cannot undo a newer pick.
//
// Recent people and bookmarks live in the window's Rust memory only, never browser storage.
// Classic script, loaded after notes.js, sharing main.js's globals (`el`, `cell`, `setMessage`,
// `invoke`, `pollLock`), signer.js's `onStaffIdChange`, funnel.js's `closeChart`/`openChart` and
// funnel.js's `failureText` and person_menu.js's `hideMenu`/`refreshMenuCount` — all read only at
// a press, long after every script has loaded.
"use strict";

/** Said in the picker for a Sign that settled while a switch waited (no patient identifier:
 * the next person may read it). */
const SIGNED_LINE = "Your note was signed.";
// The refusal's reason was said in the note pane the switch cleared, so this line promises
// only what reopening the chart gives: the chance to sign again.
const NOT_SIGNED_LINE = "Your note was NOT signed. Open the chart and sign it again.";
const SIGN_UNKNOWN_LINE = "Whether your note was signed is not known — check the chart.";
/** … and for a Discard (slice 3b-2b), in its own words: a discard is never a signature. A refused
 * discard's reason was in the cleared pane too, and its text was saved again, so the line sends
 * the clinician to the chart, where the draft stands. */
const DISCARDED_LINE = "Your draft was discarded.";
const NOT_DISCARDED_LINE = "Your draft was NOT discarded — open the chart to see where it stands.";
const DISCARD_UNKNOWN_LINE = "Whether your draft was discarded is not known — check the chart.";
/** … and for "Sign off this note" (slice 3c-2). A sign-off that went through wrote a vouch for
 * every part the presser had not signed; one that did not wrote nothing (a refusal, the key's
 * words, or a node failure, all said in the cleared pane); one whose commit lost its answer may
 * have done either. */
const SIGNED_OFF_LINE = "Your sign-off went through: every part of the note now carries your signature.";
const NOT_SIGNED_OFF_LINE = "Your sign-off signed nothing — open the chart to see where the note stands.";
const SIGN_OFF_UNKNOWN_LINE = "Whether your sign-off went through is not known — check the chart.";
/** … and for "Co-sign as reviewed" (slice 3c-3a), in its own words. A review skips some of the
 * presser's supervisees' parts (one owing a mandatory co-sign, one another supervisor reviewed),
 * so its line names no parts: Sign off's "every part", or "the parts your supervisees signed",
 * would be false. */
const REVIEWED_LINE = "Your review co-signature went through — open the chart to see which parts it covers.";
const NOT_REVIEWED_LINE = "Your co-signature signed nothing — open the chart to see where the note stands.";
const REVIEW_UNKNOWN_LINE = "Whether your co-signature went through is not known — check the chart.";
/** The bar's words when a refused pick's claim could not be read back: nobody is named. */
const CLAIM_UNREAD = "Nobody is named at this terminal — say who's here";

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
  // A different person (or nobody) is named: the bar's menu is theirs, so it is read afresh.
  const changed = claimedStaffId() !== (claim.staff_id || "");
  currentClaim = claim;
  if (changed) {
    hideMenu(); // person_menu.js: the last person's list leaves before the next person's read
    void refreshMenuCount();
    medListClaimChanged(); // med_list.js: the last person's medication lines leave, typed into by no one else
  }
  const bar = el("whos-here-name");
  if (bar.textContent !== claim.words) bar.textContent = claim.words;
}

/** The lock poll's claim (main.js's `renderLockState`): taken only at launch, never over a pick. */
function seedWhosHere(claim) {
  if (currentClaim === null) showClaim(claim);
}

/** The claim's staff id for the signer strip, or "" when nobody is named (signer.js). */
function claimedStaffId() {
  return (currentClaim && currentClaim.staff_id) || "";
}

/**
 * Take a claim the window's Rust side now holds (a pick, a refused pick, the re-lock). The signer strip's
 * staff ID becomes the claim's — the directory's own id, verbatim — or empty when nobody is
 * named. The passphrase is never touched: a claim never signs anything.
 */
function takeClaim(claim) {
  showClaim(claim);
  el("staff-id").value = claim.staff_id || "";
  onStaffIdChange(); // its hint and its expiry warning, the last person's gone (signer.js)
}

/**
 * "Who's here?" and the hot key. A Sign, Discard or sign-off in flight settles first, so its
 * outcome is said on the chart it acted on (the pick also forgets the held key). Then the chart leaves the screen, its
 * queued saves awaited (`closeChart`), so nothing typed is saved under the next person; then
 * the picker opens.
 */
async function switchWhosHere() {
  if (!el("whos-here-picker").hidden) return;
  // Captured BEFORE closeChart, which clears the note pane where the Sign said how it went.
  const signing = signInFlight();
  const signed = await signsSettled();
  await closeChart();
  openPicker(null, signing ? signed : null);
}

/**
 * The picker's line for a Sign's, a Discard's, a sign-off's or a review's outcome (notes.js's `noteSigning`);
 * "" for none.
 * Pure.
 */
function signOutcomeLine(outcome) {
  if (outcome && outcome.words) return outcome.words; // the medication list's Sign, in Rust's words
  if (outcome === "signed") return SIGNED_LINE;
  if (outcome === "not-signed") return NOT_SIGNED_LINE;
  if (outcome === "unknown") return SIGN_UNKNOWN_LINE;
  if (outcome === "discarded") return DISCARDED_LINE;
  if (outcome === "not-discarded") return NOT_DISCARDED_LINE;
  if (outcome === "discard-unknown") return DISCARD_UNKNOWN_LINE;
  if (outcome === "signed-off") return SIGNED_OFF_LINE;
  if (outcome === "not-signed-off") return NOT_SIGNED_OFF_LINE;
  if (outcome === "sign-off-unknown") return SIGN_OFF_UNKNOWN_LINE;
  if (outcome === "reviewed") return REVIEWED_LINE;
  if (outcome === "not-reviewed") return NOT_REVIEWED_LINE;
  if (outcome === "review-unknown") return REVIEW_UNKNOWN_LINE;
  return "";
}

/** Ask who's here, then run `then` once a claim is made (New note, Continue here). */
function askWhosHere(then) {
  if (!el("whos-here-picker").hidden) return;
  openPicker(then);
}

/**
 * Show the picker over the page, the rest of it out of Tab's reach, the search focused. `signed`
 * is the outcome of a Sign, Discard or sign-off the switch waited for, said above the heading, or
 * null.
 */
async function openPicker(then, signed) {
  afterPick = then;
  setMessage(el("whos-here-signed"), signOutcomeLine(signed));
  // Someone new may be at the keyboard: a passphrase typed and not used never stays beside the
  // next person's ID (it is only ever cleared here, never set).
  el("signer-passphrase").value = "";
  dismissOffer();
  el("whos-here-search").value = "";
  setMessage(el("whos-here-status"), "");
  el("whos-here-list").replaceChildren();
  resetPickerCounts(); // person_menu.js
  el("whos-here-picker").hidden = false;
  document.querySelector("main").inert = true;
  el("whos-here-search").focus();
  let listed = true;
  try {
    pickerEntries = await invoke("whos_here_directory");
  } catch (failure) {
    pickerEntries = [];
    listed = false;
    setMessage(el("whos-here-status"), "Could not read the staff list: " + failureText(failure));
  }
  drawPicker();
  // person_menu.js: the suffixes join the list once they land. Not after a failed list: the
  // counts' own failure line would overwrite the staff-list one, and nobody is listed to count.
  if (listed) void loadPickerCounts();
}

/**
 * Close the picker (Esc or Back: the claim stays as it was; or after a pick). Focus goes back to
 * the page — "Who's here?" over a chart, the search at the front door — never left on a hidden
 * element. A pick's next step (the offer, New note) may move it on.
 */
function closePicker() {
  afterPick = null;
  setMessage(el("whos-here-signed"), "");
  el("whos-here-picker").hidden = true;
  document.querySelector("main").inert = !el("terminal-lock").hidden;
  el(displayedPatient === null ? "browse-name" : "whos-here-switch").focus();
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
        entry.display_name +
          " — " +
          entry.role +
          " (" +
          entry.staff_id +
          ")" +
          (entry.recent ? " · recent here" : "") +
          (pickerSuffix[entry.staff_id] || ""), // person_menu.js: Rust's " · N waiting"
        { type: "button" },
      );
      b.addEventListener("click", () => void pick(entry.staff_id));
      li.append(b);
      return li;
    }),
  );
}

/**
 * Say who is here: `staffId` from the directory, verbatim, or null for "Don't know". The window's
 * Rust side forgets any held key and the old claim FIRST, so a refused pick leaves nobody named: the
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
      // The window's Rust side forgot the old claim before refusing; never go on showing it,
      // or the next New note would capture the last person as its writer (PR #966 review).
      console.error("the claim could not be read back", e);
      takeClaim({ kind: "not-asked", staff_id: null, display_name: null, words: CLAIM_UNREAD });
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
  // A blank search names nobody: the list's head is the person who just left.
  if (el("whos-here-search").value.trim() === "") return;
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
