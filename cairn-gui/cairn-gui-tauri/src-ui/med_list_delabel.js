// The same-drug de-label offer on a medication-list line (charting slice 1b). When an unsigned
// line is the drug of a labelled allergy, Rust sends `delabel_offers` with the line's view; each
// is drawn as ONE button under the line, with Rust's `words` as its text. Pressing it signs
// NOTHING: it opens the allergy list's own De-label row for that allergy, filled in (the drug,
// its coding, and a date only when the claim is true), and the clinician signs that row. Never a dialog (principle 3).
//
// Classic script, loaded after med_list.js (whose `drawLine` calls `drawDelabelOffers`); shares
// main.js's `el`, `cell`, allergies.js's `sayAllergy`, allergy_overlays.js's `openDelabel`.
"use strict";

const OFFER_GONE = "That allergy is no longer on the list; look at it again";
const LIVE_ALLERGY_LISTS = ["allergy-drug", "allergy-other", "allergy-adverse"];

/** Today as dd/mm/yyyy, local time (the form the row's date field reads). */
function todayDdMmYyyy() {
  const now = new Date();
  const two = (n) => String(n).padStart(2, "0");
  return two(now.getDate()) + "/" + two(now.getMonth() + 1) + "/" + now.getFullYear();
}

/**
 * The row's date prefill: "tolerated today" is true of an Already-taking line (`pre-existing`),
 * not of a Start line (`started-here`: not had in this episode), whose date stays empty = not known.
 */
function prefillDate(chartedAs) {
  return chartedAs === "pre-existing" ? todayDdMmYyyy() : "";
}

/** Say the offer's allergy is gone at the allergy pane (live region) AND beside the pressed line. */
function sayOfferGone(line) {
  sayAllergy(OFFER_GONE);
  medLineNotes.set(line.id, OFFER_GONE);
  setMessage(line.row.note, OFFER_GONE);
}

/** Draw `line`'s offers (its view's `delabel_offers`) in a box made once on its row. */
function drawDelabelOffers(line) {
  const r = line.row;
  if (!r.offers) {
    r.offers = cell("p", "");
    r.li.append(r.offers);
  }
  const offers = line.view ? line.view.delabel_offers : [];
  r.offers.replaceChildren(...offers.map((offer) => {
    const button = cell("button", offer.words, { type: "button", class: "med-list-delabel" });
    // Down exactly when the line's other controls are (another terminal holds the list; Sign in flight).
    button.disabled = medSigning || medList === null || medList.elsewhere !== null;
    button.addEventListener("click", () => pressDelabelOffer(line, offer));
    return button;
  }));
  r.offers.hidden = offers.length === 0;
}

/**
 * A press on an offer: bring the allergy pane's drawn LIVE line for `offer.target` into view and
 * open its De-label row, prefilled. Signs nothing. With no such line drawn (the list moved on),
 * say so and open nothing; while the allergy pane may not write (busy, no list), do nothing.
 */
function pressDelabelOffer(line, offer) {
  const li = document.querySelector(LIVE_ALLERGY_LISTS.map((id) => "#" + id + ' > li[data-event="' + offer.target + '"]').join(", "));
  const delabel = li && li.querySelector('[data-act="delabel"]');
  if (!delabel) {
    sayOfferGone(line);
    return;
  }
  if (!allergyMayWrite()) return;
  el("allergy-details").open = true;
  li.scrollIntoView({ block: "nearest" });
  openDelabel({ event_id: offer.target }, li, delabel, { tolerated: offer.tolerated, coding: offer.coding, date: prefillDate(line.content.charted_as) });
}
