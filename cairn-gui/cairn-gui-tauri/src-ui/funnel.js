// The front door: the §5.3/§5.8 search-before-create funnel (slice 2c).
//
// Like main.js, this file renders and decides nothing clinical. Every sentence it shows comes
// from Rust (`funnel::view`, under `cargo test`), and every rule — when the machine searches
// unasked, how many candidates the prompt may show, which search a registration attests — is
// one layer down in `cairn-gui-funnel`. What this file DOES own is bookkeeping about time:
//
// REVISIONS. Every edit of the register form increments `registerRevision`, tells the backend
// (`form_edited`, which discards the held search), and forgets the held token AT ONCE — so a
// click on Register after an edit can never attest the search for the form as it was. Every
// search carries the revision it was run for, and an answer for an older revision is dropped:
// searches run in the background as the clerk types, and two in flight can finish in either
// order. The backend enforces the same rule (`FunnelSession`); this is the display half.
//
// Loaded after main.js as a classic script, so `el`, `refresh`, `say` and `invoke` are shared
// globals.
"use strict";

/** How long typing must pause before a search runs (ms). */
const SEARCH_DEBOUNCE_MS = 250;

/**
 * How long a freshly arrived step-3 prompt must have been on screen before a click on Register
 * may register off it (ms). The background search can land just before the clerk's click and
 * flip the button's meaning from "search first" to "none of these" under the pointer; without
 * this, a registration would swear to rows nobody had time to read (final review #2). A click
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
 * a failed search reads as "nobody matched" (final review #5).
 */
function failureText(failure) {
  return (failure && failure.text) || "The window could not reach its backend: " + String(failure);
}

/**
 * A clickable candidate row. The VISIBLE text names the act as well as the patient, and is the
 * accessible name too: a sighted clerk and a screen-reader user are told the same thing, and
 * nobody has to guess that clicking a name opens that chart.
 */
function candidateItem(cand, verb) {
  const li = document.createElement("li");
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = verb + ": " + cand.name + " — " + cand.age + " — identity " + cand.trust;
  button.addEventListener("click", () => openChart(cand.patient_id));
  li.append(button);
  return li;
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
      ...browseView.candidates.map((cand) => candidateItem(cand, "Open chart")),
    );
    el("browse-status").textContent = browseView.candidates.length
      ? browseView.candidates.length + " existing chart(s) found."
      : "No existing chart matched.";
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

/** The form changed: whatever search was on screen no longer describes it. */
function onRegisterEdited() {
  registerRevision += 1;
  heldToken = null;
  el("prompt").hidden = true;
  el("register-outcome").textContent = "";
  setRegisterButton("Register new patient", true);
  void invoke("form_edited", { revision: registerRevision });
  debouncedPrompt();
}

function renderPrompt(prompt) {
  // The waiting sentence, or the prompt's own announcement (from Rust, `prompt_summary`), in
  // the live status region — so a screen-reader clerk HEARS that matches appeared and what
  // Register now means (final review #6).
  el("prompt-status").textContent = prompt.waiting || prompt.summary || "";
  heldToken = prompt.token;
  if (heldToken === null) {
    el("prompt").hidden = true;
    return;
  }
  promptShownAt = Date.now();
  // Partiality BEFORE the rows it qualifies.
  setMessage(el("prompt-incomplete"), prompt.incomplete_reason);
  el("prompt-list").replaceChildren(
    ...prompt.candidates.map((cand) => candidateItem(cand, "This is them — open chart")),
  );
  const any = prompt.candidates.length > 0;
  el("prompt").hidden = !any;
  setRegisterButton(any ? "None of these — register a new patient" : "Register new patient", true);
}

async function runPrompt(force) {
  const form = registerForm();
  try {
    const prompt = await invoke("prompt_search", { form, force });
    if (prompt.stale || prompt.revision !== registerRevision) {
      // A search the clerk ASKED for must never end in silence (final review #3).
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
    el("prompt").hidden = true;
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
    // The prompt arrived under the pointer: this click is "show me", not "register".
    el("register-outcome").textContent =
      "The list above has just changed. Read it, then press Register again.";
    return;
  }
  const sent = heldToken;
  registering = true;
  setRegisterButton("Saving…", false);
  try {
    const header = await invoke("register", { token: sent });
    heldToken = null;
    enterChart(header);
  } catch (failure) {
    el("register-outcome").textContent = failureText(failure);
    if (!(failure && failure.retry === "now") && heldToken === sent) {
      // A verdict, a dropped search, or an operator's job: this token cannot help. Forget it
      // — but only if no newer search replaced it during the save — and leave Register
      // USABLE: the next click searches again and shows the answer, and the one after that
      // registers. Never a disabled button with advice the clerk cannot follow (final review #4).
      heldToken = null;
    }
    if (failure && failure.retry === "after_operator") setMessage(el("provisioning"), failure.text);
    setRegisterButton("Register new patient", true);
  } finally {
    registering = false;
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
  // chart, and nothing of the previous one is on screen (final review, Critical #1).
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

async function openChart(patientId) {
  try {
    enterChart(await invoke("open_chart", { patientId }));
  } catch (refusal) {
    el("browse-status").textContent = String(refusal);
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
  await invoke("close_chart");
  el("chart-view").hidden = true;
  el("front-door").hidden = false;
  resetFrontDoor();
  el("browse-name").focus();
}

// ---- Start -------------------------------------------------------------------------------

async function boot() {
  const status = await invoke("funnel_status");
  // Resume above the backend's revision floor: after a reload this counter would otherwise
  // restart at 0 and every search would be dropped as stale (final review #3).
  registerRevision = status.revision;
  const mockNote = status.mock
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
