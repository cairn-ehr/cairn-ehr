// The front door: the §5.3/§5.8 search-before-create funnel (slice 2c).
//
// Like main.js, this file decides nothing clinical. Every sentence about what a SEARCH found or
// what a failure means comes from Rust (`funnel::view`, under `cargo test`) — including both
// "nobody matched" announcements, the one wording that licenses a new chart — and every rule
// (when the machine searches unasked, how many candidates the prompt may show, which search a
// registration attests) is one layer down in `cairn-gui-funnel`. The few sentences written
// HERE are about this file's own bookkeeping — an answer dropped as stale, the read guard, an
// edit made during a save, an IPC failure with no Rust text — and the fixture-mode note.
//
// What this file DOES own is bookkeeping about time:
//
// REVISIONS. Every edit of the register form increments `registerRevision`, tells the backend
// (`form_edited`, which discards the held search), and forgets the held token AT ONCE — so a
// click on Register after an edit can never attest the search for the form as it was. Every
// search carries the revision it was run for, and an answer for an older revision is dropped:
// searches run in the background as the clerk types, and two in flight can finish in either
// order. The backend enforces the same rule (`FunnelSession`); this is the display half.
//
// SOFT POLICY. The read guard (PROMPT_READ_GUARD_MS) and disabling the candidate rows while a
// registration saves are UI policy in the ADR-0021 sense: another front-end may choose
// differently. The backend's own backstop for the second is `register_impl` refusing to write,
// or to switch charts, once a chart is open. Whether the read guard should move below the UI is
// #677.
//
// Loaded after main.js as a classic script, so these are shared globals: `el`, `setMessage`,
// `refresh`, `clearChart`, `say` and `invoke` (read), and `displayedPatient` (WRITTEN here —
// it is how every chart command names the chart on screen, `AppState::displayed_patient`).
"use strict";

/** How long typing must pause before a search runs (ms). */
const SEARCH_DEBOUNCE_MS = 250;

/**
 * How long a freshly arrived step-3 prompt must have been on screen before a click on Register
 * may register off it (ms). The background search can land just before the clerk's click and
 * flip the button's meaning from "search first" to "none of these" under the pointer; without
 * this, a registration would swear to rows nobody had time to read (PR #674 review #2). A click
 * inside the window is treated as "show me", never as "register".
 */
const PROMPT_READ_GUARD_MS = 800;

let browseRevision = 0;
let registerRevision = 0;
/** The token of the step-3 search on screen, or null when Register may not use one. */
let heldToken = null;
/** True while a registration is being saved, so a second click cannot start another. */
let registering = false;
/** When the held token's prompt was rendered (ms since epoch), for PROMPT_READ_GUARD_MS. */
let promptShownAt = 0;
/** Whether the prompt on screen listed anyone — what the Register button's label must say. */
let promptHadRows = false;
/** Set when the form is edited while a registration is being saved: that edit is NOT saved. */
let editedDuringSave = false;
/** The fixture-mode note, kept so the provisioning line can be reset to it after a success. */
let mockNote = "";

/** A debounced `fn`, with `.cancel()` so a forced search can pre-empt a pending one. */
function debounce(fn) {
  let timer = null;
  const run = () => {
    clearTimeout(timer);
    timer = setTimeout(fn, SEARCH_DEBOUNCE_MS);
  };
  run.cancel = () => clearTimeout(timer);
  return run;
}

/**
 * The text of a failure. Rust sends an `ErrorView`; a failure raised by the IPC layer itself
 * arrives as a bare string with no `.text`, and must still SAY something — a blank status after
 * a failed search reads as "nobody matched" (PR #674 review #5).
 */
function failureText(failure) {
  return (failure && failure.text) || "The window could not reach its backend: " + String(failure);
}

/**
 * A clickable candidate row. The VISIBLE text names the act as well as the patient, and is the
 * accessible name too: a sighted clerk and a screen-reader user are told the same thing, and
 * nobody has to guess that clicking a name opens that chart.
 */
function candidateItem(cand, verb, statusFor) {
  const li = document.createElement("li");
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = verb + ": " + cand.name + " — " + cand.age + " — identity " + cand.trust;
  button.dataset.candidate = "true";
  // Not clickable while a registration saves: "This is them" after "none of these" is already
  // on its way would race two contradictory acts (see setCandidatesEnabled).
  button.disabled = registering;
  button.addEventListener("click", () => openChart(cand.patient_id, statusFor));
  li.append(button);
  return li;
}

/**
 * Enable or disable every candidate row on the front door. Disabled while a registration saves:
 * the clerk has just said "none of these", and a click on one of them before the save lands
 * would leave the window between a chart they recognised and a new chart they created. The
 * backend refuses to write (or to switch charts) behind an open chart regardless; this keeps
 * the contradiction from being clickable at all.
 */
function setCandidatesEnabled(enabled) {
  for (const button of document.querySelectorAll("button[data-candidate]")) {
    button.disabled = !enabled;
  }
}

// ---- Step 1: browse ----------------------------------------------------------------------

async function runBrowse() {
  const revision = ++browseRevision;
  const form = {
    revision,
    raw_name: el("browse-name").value,
    birth_date: el("browse-dob").value,
  };
  const list = el("browse-list");
  if (!form.raw_name.trim() && !form.birth_date.trim()) {
    list.replaceChildren();
    setMessage(el("browse-incomplete"), "");
    el("browse-status").textContent = "";
    return;
  }
  try {
    const browseView = await invoke("browse", { form });
    if (browseView.revision !== browseRevision) return; // a newer browse is on its way
    setMessage(el("browse-incomplete"), browseView.incomplete_reason);
    list.replaceChildren(
      ...browseView.candidates.map((cand) => candidateItem(cand, "Open chart", "browse-status")),
    );
    // From Rust (`view::browse_summary`): "nobody matched" and "the search did not finish"
    // lead to opposite acts, so neither is worded here.
    el("browse-status").textContent = browseView.summary;
  } catch (failure) {
    if (revision !== browseRevision) return;
    // A failure is shown AS a failure and the old list is cleared: an empty-looking list
    // after a failed search reads as "nobody matched" (principle 4).
    list.replaceChildren();
    setMessage(el("browse-incomplete"), "");
    el("browse-status").textContent = failureText(failure);
  }
}

// ---- Steps 2 and 3: register, and the prompt ---------------------------------------------

function registerForm() {
  return {
    revision: registerRevision,
    raw_name: el("reg-name").value,
    birth_date: el("reg-dob").value,
  };
}

function setRegisterButton(label, enabled) {
  const button = el("register");
  button.textContent = label;
  button.disabled = !enabled;
}

/** Hide the prompt and everything that qualifies it. */
function hidePrompt() {
  el("prompt").hidden = true;
  setMessage(el("prompt-incomplete"), "");
  promptHadRows = false;
}

/** The form changed: whatever search was on screen no longer describes it. */
function onRegisterEdited() {
  registerRevision += 1;
  heldToken = null;
  hidePrompt();
  // The announcement said what Register MEANT for the old search ("none of these"); left in
  // place it is a false sentence in the live region until the next search lands.
  el("prompt-status").textContent = "";
  el("register-outcome").textContent = "";
  if (registering) {
    // The save in flight is for the form as it was; this edit will not be in it. Keep the
    // button disabled (a click would do nothing) and say so when the save lands.
    editedDuringSave = true;
  } else {
    setRegisterButton("Register new patient", true);
  }
  invoke("form_edited", { revision: registerRevision }).catch((failure) => {
    // Safe to continue — the token above is already forgotten, and the backend's revision floor
    // drops a search for the old form — but never silent.
    el("prompt-status").textContent = failureText(failure);
  });
  debouncedPrompt();
}

function renderPrompt(prompt) {
  // The waiting sentence, or the prompt's own announcement (from Rust, `prompt_summary`), in
  // the live status region — so a screen-reader clerk HEARS that matches appeared and what
  // Register now means (PR #674 review #6).
  el("prompt-status").textContent = prompt.waiting || prompt.summary || "";
  heldToken = prompt.token;
  if (heldToken === null) {
    hidePrompt();
    return;
  }
  promptShownAt = Date.now();
  // Partiality BEFORE the rows it qualifies — and OUTSIDE the prompt section, so it is shown
  // even when the search showed nobody and the section is hidden: that is exactly the case in
  // which "partial" and "nobody matched" must not be confused.
  setMessage(el("prompt-incomplete"), prompt.incomplete_reason);
  el("prompt-list").replaceChildren(
    ...prompt.candidates.map((cand) =>
      candidateItem(cand, "This is them — open chart", "prompt-status"),
    ),
  );
  promptHadRows = prompt.candidates.length > 0;
  el("prompt").hidden = !promptHadRows;
  setRegisterButton(registerLabel(), true);
}

/** What the Register button means while a search is held: "none of these", or plain register. */
function registerLabel() {
  return promptHadRows ? "None of these — register a new patient" : "Register new patient";
}

async function runPrompt(force) {
  const form = registerForm();
  try {
    const prompt = await invoke("prompt_search", { form, force });
    if (prompt.stale || prompt.revision !== registerRevision) {
      // A search the clerk ASKED for must never end in silence (PR #674 review #3).
      if (force && form.revision === registerRevision) {
        el("prompt-status").textContent =
          "The form changed while searching. Press Register again to search what is typed now.";
      }
      return;
    }
    renderPrompt(prompt);
  } catch (failure) {
    if (form.revision !== registerRevision) return;
    // No token after a failed search: registering without its search is what ADR-0061
    // forbids, so Register runs the search again rather than proceeding.
    heldToken = null;
    hidePrompt();
    el("prompt-status").textContent = failureText(failure);
  }
}

const debouncedPrompt = debounce(() => runPrompt(false));
const debouncedBrowse = debounce(runBrowse);

async function onRegister(event) {
  event.preventDefault();
  if (registering) return;
  if (heldToken === null) {
    // Nothing searched yet for this form (the trigger is advisory: a mononymous patient or an
    // unknown date of birth never trips it). Search now and SHOW the answer; the clerk's next
    // click is the act. Never register on the strength of a search nobody saw. A pending
    // background search is cancelled first, so two searches at one revision cannot race.
    debouncedPrompt.cancel();
    await runPrompt(true);
    return;
  }
  if (Date.now() - promptShownAt < PROMPT_READ_GUARD_MS) {
    // The search answer arrived under the pointer: this click is "show me", not "register".
    // Worded for both shapes of answer — a list of rows, or a sentence that nobody matched.
    el("register-outcome").textContent =
      "The search result above has just changed. Read it, then press Register again.";
    return;
  }
  const sent = heldToken;
  registering = true;
  editedDuringSave = false;
  setRegisterButton("Saving…", false);
  setCandidatesEnabled(false);
  try {
    const header = await invoke("register", { token: sent });
    heldToken = null;
    // The form this registration consumed is gone: a prompt answer still in flight for it must
    // be dropped as stale, not set a token on the hidden front door (#675 item 4).
    registerRevision += 1;
    // A registration succeeded, so this node may write: an operator warning from launch (or
    // from an earlier refusal) is no longer true, and a stale warning teaches clerks to
    // ignore the line.
    setMessage(el("provisioning"), mockNote);
    const lostEdit = editedDuringSave;
    enterChart(header);
    if (lostEdit) {
      say(
        "You edited the form after pressing Register; that edit was NOT saved. This chart " +
          "was registered as the form read when you pressed Register — check the name and " +
          "date of birth above.",
      );
    }
  } catch (failure) {
    const text = failureText(failure);
    el("register-outcome").textContent = text;
    // A refusal that arrives while a chart is open (the clerk opened one mid-save) must be
    // read ON that chart: the front door and its outcome line are hidden.
    if (displayedPatient !== null) say(text);
    if (!(failure && failure.retry === "now") && heldToken === sent) {
      // A verdict, a dropped search, or an operator's job: this token cannot help. Forget it
      // — but only if no newer search replaced it during the save — and leave Register
      // USABLE: the next click searches again and shows the answer, and the one after that
      // registers. Never a disabled button with advice the clerk cannot follow.
      heldToken = null;
      promptHadRows = false;
    }
    if (failure && failure.retry === "after_operator") setMessage(el("provisioning"), failure.text);
    // A kept search still means what its prompt said ("none of these"); say so again.
    setRegisterButton(heldToken === null ? "Register new patient" : registerLabel(), true);
  } finally {
    registering = false;
    editedDuringSave = false;
    setCandidatesEnabled(true);
  }
}

// ---- Opening and closing a chart ---------------------------------------------------------

function showIdentity(header) {
  el("patient-heading").textContent = header.name;
  el("identity-born").textContent = header.born;
  el("identity-trust").textContent = "Identity: " + header.trust;
  el("identity-id").textContent = "Chart " + header.patient_id;
}

function enterChart(header) {
  // Bind first, clear second, read third: from this line on, every chart command names THIS
  // chart, and nothing of the previous one is on screen (PR #674 review, Critical #1).
  displayedPatient = header.patient_id;
  clearChart();
  el("front-door").hidden = true;
  el("chart-view").hidden = false;
  showIdentity(header);
  say("");
  void refresh();
  // Move focus to the name, so a screen reader announces whose chart just opened.
  const heading = el("patient-heading");
  heading.tabIndex = -1;
  heading.focus();
}

/** Open a listed chart; a refusal is shown next to the list it was clicked in (`statusId`). */
async function openChart(patientId, statusId) {
  if (registering) return; // the rows are disabled; this is the belt to those braces
  try {
    enterChart(await invoke("open_chart", { patientId }));
  } catch (refusal) {
    el(statusId).textContent = String(refusal);
  }
}

function resetFrontDoor() {
  for (const id of ["browse-name", "browse-dob", "reg-name", "reg-dob"]) el(id).value = "";
  browseRevision += 1;
  el("browse-list").replaceChildren();
  el("browse-status").textContent = "";
  setMessage(el("browse-incomplete"), "");
  el("prompt-status").textContent = "";
  el("register-outcome").textContent = "";
  onRegisterEdited();
}

async function closeChart() {
  displayedPatient = null;
  clearChart();
  say("");
  try {
    await invoke("close_chart");
  } catch (failure) {
    // Stay on the (now empty) chart view and SAY so, rather than leaving a blank view with a
    // "Loading…" button and no word. Every chart command is already refused: nothing is displayed.
    say("Could not return to the front door: " + failureText(failure));
    return;
  }
  el("chart-view").hidden = true;
  el("front-door").hidden = false;
  resetFrontDoor();
  el("browse-name").focus();
}

// ---- Start -------------------------------------------------------------------------------

async function boot() {
  let status;
  try {
    status = await invoke("funnel_status");
  } catch (failure) {
    // The front door starts hidden; without this the clerk would see an empty window.
    setMessage(el("provisioning"), "This window could not start: " + failureText(failure));
    return;
  }
  // Resume FROM the backend's revision floor (an equal revision is accepted): after a reload this
  // counter would otherwise restart at 0 and every search would be dropped as stale. `max`,
  // because the clerk may already have typed — and announced higher revisions — before this
  // answer arrived; lowering the counter then would make every search look stale.
  registerRevision = Math.max(registerRevision, status.revision);
  mockNote = status.mock
    ? "Fixture data — patients registered here live only as long as this window."
    : "";
  setMessage(el("provisioning"), [status.provisioning, mockNote].filter(Boolean).join(" "));
  if (status.chart) {
    enterChart(status.chart);
  } else {
    el("front-door").hidden = false;
    el("browse-name").focus();
  }
}

el("browse-form").addEventListener("input", debouncedBrowse);
el("browse-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void runBrowse();
});
el("register-form").addEventListener("input", onRegisterEdited);
el("register-form").addEventListener("submit", onRegister);
el("close-chart").addEventListener("click", closeChart);

void boot();
