// The drugs-only pick on a De-label row's "Tolerated" field (charting slice 1b): typing three or
// more characters offers drugs from the node's drug list (`med_drug_search`, the same search the
// medication line uses), so the tolerated drug can be signed CODED (recorded only: nothing reads
// the code yet, #1085). Choosing is optional: a name left as typed is signed as typed.
//
// The one safety rule, as drug_pick.js's: a coding vouches for THE NAME IT WAS PICKED WITH. ANY
// edit of the field drops it (the first thing the `input` handler does), and while one is held
// the row says so in words ("Coded as: ...", drug_pick.js's `codedAsWords`). Unlike drug_pick.js
// (one global instance bound to the allergy form) each row holds its own coding, on the row.
//
// Classic script, loaded BEFORE allergy_overlays.js; shares main.js's `el`, `cell`, `invoke`,
// drug_pick.js's `PICK_DELAY_MS`, `PICK_MIN_CHARS`, `NO_DRUG_LIST_HINT`, `codedAsWords` (read only
// when a row is open, long after every script has loaded) and allergy_overlays.js's `rowInput`.
"use strict";

/** The coding the row holds (`{system, code, display}`), or null: nothing picked, or the name edited. */
function delabelCoding(row) {
  return row.heldCoding || null;
}

/** The ONLY writer of a row's coding: holds it and draws or hides its "Coded as" line with it. */
function holdDelabelCoding(row, coding) {
  row.heldCoding = coding;
  const line = row.querySelector('[data-note="coded-as"]');
  line.textContent = coding ? codedAsWords(coding) : "";
  line.hidden = !coding;
}

/** Wire the row's Tolerated field to the pick. `row` is the freshly cloned De-label row. */
function wireDelabelPick(row) {
  const input = rowInput(row, "tolerated");
  const list = row.querySelector('[data-field="pick"]');
  const hint = row.querySelector('[data-note="pick-hint"]');
  let timer = null;
  let revision = 0; // an older answer is dropped
  const close = () => {
    clearTimeout(timer);
    revision += 1;
    list.replaceChildren();
    list.hidden = true;
    hint.hidden = true;
  };
  const choose = (option) => {
    input.value = option.display;
    holdDelabelCoding(row, { system: option.system, code: option.code, display: option.display });
    close();
  };
  const ask = async (text) => {
    const mine = ++revision;
    try {
      const answer = await invoke("med_drug_search", { text: text });
      if (mine !== revision) return;
      close();
      if (!answer.available) {
        hint.textContent = NO_DRUG_LIST_HINT;
        hint.hidden = false;
        return;
      }
      list.replaceChildren(...answer.options.map((option) => {
        const li = cell("li", option.display, { role: "option", tabindex: "0" });
        li.addEventListener("click", () => choose(option));
        li.addEventListener("keydown", (event) => {
          if (event.key === "Enter" || event.key === " ") { event.preventDefault(); choose(option); }
        });
        return li;
      }));
      list.hidden = answer.options.length === 0;
    } catch (failure) {
      // The pick is optional: the name is signed as typed. Logged, so a broken search is not "no matches".
      console.error("med_drug_search failed:", failure);
      if (mine === revision) close();
    }
  };
  input.addEventListener("input", () => {
    holdDelabelCoding(row, null); // any edit of the name drops the coding
    close();
    if (input.value.trim().length >= PICK_MIN_CHARS) timer = setTimeout(() => void ask(input.value), PICK_DELAY_MS);
  });
}
