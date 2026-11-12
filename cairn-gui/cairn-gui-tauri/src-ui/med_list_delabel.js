// The same-drug de-label offer on a medication-list line (charting slice 1b). When an unsigned
// line is the drug of a labelled allergy, Rust sends `delabel_offers` with the line's view; each
// is drawn as ONE button under the line, with Rust's `words` as its text. Pressing it signs
// NOTHING: it opens the allergy list's own De-label row for that allergy, filled in (the drug,
// its coding, and a date only when the claim is true), and the clinician signs that row. Never a dialog (principle 3).
//
// Classic script, loaded after med_list.js (whose `drawLine` calls `drawDelabelOffers`); shares
// main.js's `el`, `cell`, `setMessage`; med_list.js's `medSigning`, `medList`, `medEditingLine`;
// allergies.js's `sayAllergy`, `allergyMayWrite`, `allergyBusy`; allergy_overlays.js's `openDelabel`.
"use strict";

const OFFER_GONE = "That allergy is no longer on the list; look at it again";
const ALLERGY_BUSY = "An allergy record is being signed; press again once it is answered";
const NO_ALLERGY_LIST = "The allergy list is not drawn for this chart; read it again";
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

/**
 * Say the offer's allergy is gone at the allergy pane (live region) AND beside the pressed line —
 * once: the line's note is not stored, so its next draw (a re-read) takes it down rather than
 * leaving it beside an offer drawn again.
 */
function sayOfferGone(line) {
  sayAllergy(OFFER_GONE);
  setMessage(line.row.note, OFFER_GONE);
}

/**
 * Draw `line`'s offers (its view's `delabel_offers`) in a box made once on its row. None while
 * the line is edited: like its marks, an offer is about the SAVED words the editor is replacing.
 */
function drawDelabelOffers(line) {
  const r = line.row;
  if (!r.offers) {
    r.offers = cell("p", "");
    r.li.append(r.offers);
  }
  const offers = line.view && medEditingLine !== line ? line.view.delabel_offers : [];
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
 * open its De-label row, prefilled. Signs nothing. The pane's state is read first: while it may
 * not write (a press in flight, no list drawn for this chart) say why and open nothing — never
 * "no longer on the list", which would be a claim about a list nobody read (principle 4). With a
 * list drawn but no such line in it (the list moved on), say so and open nothing.
 */
function pressDelabelOffer(line, offer) {
  if (!allergyMayWrite()) {
    sayAllergy(allergyBusy ? ALLERGY_BUSY : NO_ALLERGY_LIST);
    return;
  }
  const li = document.querySelector(LIVE_ALLERGY_LISTS.map((id) => "#" + id + ' > li[data-event="' + offer.target + '"]').join(", "));
  const delabel = li && li.querySelector('[data-act="delabel"]');
  if (!delabel) {
    sayOfferGone(line);
    return;
  }
  el("allergy-details").open = true;
  li.scrollIntoView({ block: "nearest" });
  openDelabel({ event_id: offer.target }, li, delabel, { tolerated: offer.tolerated, coding: offer.coding, date: prefillDate(line.content.charted_as) });
}
