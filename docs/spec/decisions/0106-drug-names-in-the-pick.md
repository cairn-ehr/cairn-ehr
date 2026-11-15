# ADR-0106 — Drug names in the pick

- **Status:** Accepted (builds charting slice 3: the two drug picks search every name drugref lists for a drug,
  and the medication and allergy lists show the generic first with the clinician's word after it)
- **Date:** 2026-11-15 (built), recorded 2026-11-15
- **Spec version at acceptance:** 0.108
- **Issues:** PR [#1121](https://github.com/cairn-ehr/cairn-ehr/pull/1121). Design:
  `docs/superpowers/specs/2026-11-10-charting-a-medication-design.md` §3.3, §5 item 3, §7 risk 3; plan
  `docs/superpowers/plans/2026-11-15-charting-slice-3-drug-names.md` (the maintainer's rulings R1–R3 of 2026-11-15,
  recorded below). Builds [#1065](https://github.com/cairn-ehr/cairn-ehr/issues/1065) and
  [#1073](https://github.com/cairn-ehr/cairn-ehr/issues/1073), on drugref's
  [drugref#231](https://github.com/cairn-ehr/drugref/issues/231). Leaves
  [#1081](https://github.com/cairn-ehr/cairn-ehr/issues/1081) (PBS forms) open.
- **Builds on:** [ADR-0098](0098-allergy-coding-and-the-medication-check.md) (db/076, the one file that reads
  drugref; availability read from the catalogue; a door's error is "no drug list") ·
  [ADR-0099](0099-charting-a-medication.md) (the drugs-only medication pick; the signed term, `term_to_sign`) ·
  [ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) (the coding is drugref's display name, signed as it was) ·
  [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (an in-place db/ edit replays by its content hash)

## Context

A nurse writes "adrenaline 1 mg IM" on the drug chart, and the next reader knows what it means. On the allergy
sheet the doctor writes "lignocaine — rash". Cairn's two picks searched only drugref's display name, and drugref
names those moieties *epinephrine* and *lidocaine*. So an Australian clinician typing the word they use found
nothing. They could retry under the other name or sign the entry uncoded; an uncoded allergy reads "not checked"
on every medication line (ADR-0098 D8). US names (*acetaminophen*) missed the same way. drugref#231 (closed
2026-11-13) added a view `drugref.moiety_name(moiety_uuid, name, name_kind)`. It lists each moiety's display
name, its UNII official names (`unii_official`, e.g. `ACETAMINOPHEN`; drugref db/070) and its MeSH synonyms
(`mesh_synonym`, the BAN/AAN names such as *Lignocaine*, *Frusemide*, *Adrenaline*; drugref db/071). A synonym
equal to the same moiety's display or official name is left out by the view. drugref's owner ruled the synonyms
search-only, and a name two moieties share is kept for both.

## Decision

1. **Names only (maintainer, R1).** This slice is the names. PBS forms and strengths are deferred, because no site
   has PBS data (`drugref.local_product` is empty in every database we hold); #1081 stays open. Brand → combination
   (*Augmentin* → amoxicillin + clavulanic acid) needs a drugref source that does not exist. It is filed as
   [drugref#244](https://github.com/cairn-ehr/drugref/issues/244), and until then a combination is still coded part by
   part from the line (ADR-0099).
2. **The view is optional and read by catalogue.** `cairn_drug_names_available()` (db/076 §1b) is true only when
   `cairn_drug_available()` is true AND `drugref.moiety_name` exists with the columns `moiety_uuid uuid`,
   `name text` and `name_kind text`, and the door's owner may SELECT it. It reads `pg_class`/`pg_attribute`, so it
   never raises. A drugref without the view, or with one unreadable or reshaped, is still a full drug list: the
   names are off, never the drugs. So the view is deliberately not one of `cairn_drug_available()`'s required
   objects.
3. **One shared match.** `cairn_drug_moiety_matches(pattern)` (db/076 §1c) is the one place a typed text becomes
   drugs, for both picks. It returns one row per moiety, `(moiety_uuid, display, matched)`. With the names, any name
   may match at a word start. When several of one drug's names match, the display name wins, then a name the typed
   text starts (so "adren" takes the racemate's "Adrenaline, Racemic Mixture", never the shorter "Racemic
   Adrenaline", which Sign could not complete to; final review I1), then the shorter name, then byte order, so a drug
   is listed once ("amox" lists amoxicillin, not also its synonym *Amoxycillin*). Without
   the names it is the display-name search the doors always did. It and the names check are revoked from PUBLIC
   and granted to no one: the window calls only the doors.
4. **`matched` says which name found it.** It is NULL when the display name matched or the matched name is the
   display in another case. A UNII official name is lower-cased, since UNII spells every name in capitals; a MeSH
   name keeps its own case, which can carry meaning ("5-FU", "L-Dopa"; final review M2), so the row reads
   "Adrenaline → epinephrine". Both
   doors gained it as a trailing column, `cairn_drug_search → (system, code, display, matched)` and
   `cairn_drug_medication_search → (code, display, uses, matched)`, each dropped first by the file's
   guarded-`DROP` idiom whenever its result shape is not the new one. Groups never carry one. Within a tier
   (allergy pick) or a use count (medication pick), a display-name match ranks first, then the shorter name that
   matched (`matched` when set, else `display`).
5. **A name two drugs share lists both.** "cefalexin" is a MeSH synonym of *cephalexin* and the UNII official name
   of *cephalexin anhydrous*: the pick shows both, each with its arrow, and never picks one (design §3.3, risk 3).
   A salt listed beside its base follows from drugref's ruling on #231 ("salt/base cross-matches stay").
6. **The pick row reads "Adrenaline → epinephrine" (maintainer, R2)** in all three drug picks: the allergy pick
   (`pickLabel`), the medication line (`medPickLabel`) and the de-label's tolerated drug (`delabel_pick.js`, which
   reads the medication door). In the allergy and de-label picks, choosing such a row writes that name into the
   field; the medication line keeps the words as typed, and Sign completes them (decision 8).
7. **`matched` is never signed.** The coding on the wire stays `{system, code, display}` with `display` the drug
   list's name (ADR-0059). In the window `matched` rides on `CodingArg` (`#[serde(default,
   skip_serializing_if)]`) only so Sign can complete the term, and with it in a draft line's saved parts. The
   signed coding is built field by field (`med_chart_write::substance`). The allergy pick holds only the three
   fields.
8. **The term completes to the name that matched.** `term_to_sign(typed, display, matched)` compares the typed
   text with `matched` when there is one, else with the display. A typed prefix of it completes to it ("adren"
   signs "Adrenaline"); the whole name typed in the clinician's own case stays theirs ("5-FU"); anything else stays
   as typed (a brand). Before this, "adren" picked as epinephrine would
   have signed the unfinished "adren". The allergy pick writes `matched || display` into the field on a choose.
9. **The list shows the generic first, the clinician's word after it (maintainer, R3; #1073).** The pure
   `cairn_medication_view::shown_name(display, term)` gives "epinephrine (Adrenaline)", and no bracket when the
   term is blank or the same word in another case. It is drawn in the medication list's name cell
   (`MedicationRow::shown_name`) and on every allergy line: the strip, the line, earlier versions,
   entered-in-error, and the de-labelled "Historically reported" line (`allergy::history_words::allergen_name`),
   for an entry coded to a drug (`drugref-moiety`) only. A group-coded entry keeps its words, since "Penicillins
   (allergy group) (penicillin)" adds nothing. Sorting stays on `display_name()`, which the bracket only follows,
   and the by-name allergy check still reads `term`. The CLI list is unchanged.
10. **The by-name allergy check also reads a drug-coded allergen's drug name** (final review I2). An allergy typed
    "adrenaline" and coded to epinephrine marks an uncoded line typed "epinephrine", as an allergy typed
    "epinephrine" always did. Before this slice the pick could not find "adren", so the clinician retried under
    drugref's name, and that name was the term; now the local name is the term, and the coded display carries
    drugref's (`check.rs` `pair`). Over-warning is the safe error. The bracketed text is never fed to the check.
11. **No wire change, no new db/ file.** db/076 is edited in place and replays by its content hash (ADR-0080).
    `SCHEMA_GENERATION` stays 82.

## Consequences

- A local or US name finds its drug in one pick, and the record says both words. Paper-parity: N = 1 (write the
  word) → M = 1 (pick + the existing Sign) → K = 1, down from a failed search plus a retry or an uncoded entry.
- A drugref older than db/070 behaves exactly as before. That is pinned by
  `drug_names_door.rs::without_names_the_doors_answer_as_before` and by
  `an_unreadable_or_reshaped_name_view_is_ignored`.
- Tests that moved ownership of the doors to a probe role must move the two helpers too (the pick calls the
  owner-only match). One owner loads the schema on a real node.

## Not guaranteed

- No brand → combination: *Augmentin* finds nothing until drugref has a source (drugref#244).
- No PBS forms or strengths (#1081).
- A synonym drugref lists for a salt shows the salt beside its base. The clinician chooses.
- Two spellings of one drug that the typed text both starts sign the shorter ("frus" signs "Frusemid", not
  "Frusemide"). The ordering across drugs can put a local name below drugs found by their display name, and in
  the medication pick a drug charted more often ranks above one whose display name was typed exactly (Enter takes
  the first row). "Version of which line?" still shows the bare name
  ([#1122](https://github.com/cairn-ehr/cairn-ehr/issues/1122)).
- The names are off without a trace: nothing logs that a drugref lacks or cannot read the view, and a fault that
  raises only while reading it (as opposed to a missing or reshaped view) empties the whole pick
  ([#1123](https://github.com/cairn-ehr/cairn-ehr/issues/1123)).
- Matching is `ILIKE` and a bare `lower()`: no accent folding and no named collation. Durable rule 17
  (`COLLATE pg_unicode_fast`) governs patient names, where a stored and a typed token must case-map alike; drugref's
  names are ASCII in practice. For a non-ASCII name, a cluster-dependent case map can change whether it matches and
  how `matched` is spelled; the coding is still the display of the row the clinician picked.
- RUNBOOK §37 (the human pass, with `drugref_synonyms` copied in) is owed.
