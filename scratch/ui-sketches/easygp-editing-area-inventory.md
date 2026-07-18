# easyGP data-entry mining → Cairn: the Editing Area grammar

> Scratch / conversation aid — NOT canonical. Sibling of
> [`easygp-consult-screen-inventory.md`](easygp-consult-screen-inventory.md) (which mined the
> consult screen's *information architecture*; this note mines the *data-entry grammar*) and of
> [`easygp-prefetch-notes.md`](easygp-prefetch-notes.md) (write-model mining → ADR-0020). If
> anything here graduates, it graduates into the
> [shell design spec](../../docs/superpowers/specs/2026-07-12-clinician-reference-gui-shell-design.md),
> a GP manifest, or spec prose — this file stays scaffolding.

- **Source (2026-07-18 batch):** 18 numbered screen snips from a resuscitated older easyGP
  build plus pp. 39–50 of the unfinished easyGP Developer's Guide (the "Structure of your new
  form" / "The Editing Area in detail" chapters), supplied by the easyGP co-author. The snips
  are grouped as *sequences* deliberately: they show how data is progressively/automatically
  entered before saving (allergy entry 01–03, pathology ordering 04–05, prescribing 06–07,
  notes review 08–11, drawing/webcam 12–13, referral 14–15, help 16, past-history/care-plan 17,
  toolbar/banner 18).
- **Confidentiality:** the source folder is **git-ignored** (`docs/untracked_for_brainstorming/`)
  and must stay uncommitted and unpublished — the patient identifiers are altered but the photo
  is real and note bodies may carry confidential content (a deceased patient who consented to
  educational use; **not for any publicly accessible space**). This note therefore records
  **mechanisms only** — no clinical content, no names, no images.
- **Why this batch matters:** the consult-screen note captured *what a GP needs visible*;
  this batch captures *how data gets in* — the part of EHR UX where paper-parity usually dies
  (per-form modal-dialog jungles). easyGP's answer, the **Editing Area**, was iterated in
  production from 1995 and is the co-author's explicitly-flagged philosophy chapter.

---

## 1. The headline: easyGP's six invariants ≅ Cairn's event envelope

The guide's core claim (paraphrased): conventional systems build *a different screen and modal
dialog stack per data type*; but allergies, pathology orders, letters, immunisations, and
excisions are **conceptually all the same**, because every clinical datum:

1. is always associated with a **date** — current, past, or future;
2. is always **entered by a person**;
3. has **1–n named input fields** (names differ, shape doesn't);
4. is at some point **judged "OK" by the user**;
5. is then **saved to the backend**;
6. when re-displayed, has **key elements which represent the totality** and can be shown as a
   list line — a human reading the list can imply the whole from the pieces.

That is, near line-for-line, the Cairn event envelope, derived independently from first
principles fifteen years later:

| easyGP invariant | Cairn primitive |
|---|---|
| 1. always dated (past/present/future) | bitemporal claim — asserted `t_effective` vs HLC `t_recorded` ([ADR-0003](../../docs/spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)) |
| 2. always entered by a person | contributor set + separable responsibility (principle 10, [ADR-0007](../../docs/spec/decisions/0007-authorship-and-accountability.md)/[ADR-0051](../../docs/spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)) |
| 3. 1–n typed fields, shape shared | typed event body, additive-only schema ([ADR-0012](../../docs/spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)) |
| 4. a "judged OK" moment | the sign-off/commitment moment — scratchpad → commit ([ADR-0020](../../docs/spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md), [ADR-0049](../../docs/spec/decisions/0049-commitment-based-sign-off-currency.md)) |
| 5. always saved to backend | append via the validated submit floor ([ADR-0022](../../docs/spec/decisions/0022-validated-submit-surface-the-write-path.md)) |
| 6. key elements imply the totality in a list | the **plaintext legibility twin** + projection list line (principle 11, ADR-0012/[0034](../../docs/spec/decisions/0034-demographic-legibility-twin.md)/[0039](../../docs/spec/decisions/0039-globalise-authored-legibility-twin.md)) |

**Standing consequence:** the event envelope is not just the right *wire* grammar — sixteen
years of clinician-driven UI evolution converged on it as the right *user-facing* grammar.
Invariant 6 is the strongest exhibit: "key elements from which a human can imply the totality"
is close to a definition of the legibility twin, discovered from the display side rather than
the durability side. The Cairn UI can therefore expose the envelope *directly* (one entry
grammar per event type) instead of inventing per-module UX.

## 2. The grammar in action (what the sequences show)

### 2.1 One editing area, zero modal dialogs

Every module is the same three-zone shape: an **edit area** (label:field rows), the module's
**saved-items list below** (invariant-6 key-element lines), and contextual side panels. Entering
an allergy, ordering a test, prescribing, and writing a referral are *the same gesture
sequence* in different vocabularies. Across all 18 snips there is not one confirmation dialog —
consistent with the consult-screen note's finding (its §4.3 exhibit 5), now shown for the
*write* path too. Cairn mapping: the shell's tabs + the ADR-0020 type-through flow are the same
stance; the manifest should enforce the shared grammar (a module contributes *fields*, never a
new interaction model).

### 2.2 Incremental search is the primary input mode

The guide's phrase: *"as much data as possible will be presented to the user by trolling the
backend in real time as the user types."* Every reference-data field is a type-ahead: drugs by
generic or brand fragment, pathology/radiology tests (one search across both, with panel
expansions), contacts by name/category/occupation, copy-recipients, providers. The active
search field is highlighted (green — a single-meaning colour, used *only* for
"this field is live-searching"), candidates render in a list directly under the field, arrow/
enter selects; the mouse is the fallback, not the path. A status line states the result count.
Cairn mapping: the type-through verbs already assume this; the *consistent visual state for
"searching"* and the count line are worth porting into the design language (and are a good
entry in the colour-vocabulary work item — one colour, one meaning, contrast the red overload).

### 2.3 Auto-fill to the fork in the decision tree

The guide names the principle: **"take as much work away from the user"** and auto-fill
everything derivable, halting for input only at a genuine **"fork in the decision tree"**
(its example: a drug with one tablet size auto-fills; several sizes → present the choice
list). Seen live: selecting a generic fills brand, strength/form, price, subsidy status,
default quantity/repeats; selecting a specialist fills organisation, branch, address, and
defaults the referral-type combo; selecting a lab fills branch + address + contact panel.
Defaults are *pre-selected but changeable* combos, never locked. Cairn mapping: this is the UI
counterpart of acknowledged uncertainty's "never force fabrication" — the system asserts what
it actually knows, asks only what it cannot know, and the fork-prompt is a *pick list of true
options*, not a blank form. Belongs in the shell spec as a named rule for every type-through
verb implementation.

### 2.4 Entry lifecycle: dirty → Accept → grayed + listed

The edit area signals unsaved state **ambiently** — a red border around the whole area (the
guide even documents the mechanism and insists the indicator "must be red"). Accept/Save
commits; the area then re-renders **grayed read-only** with the new item appended to the list
below — the *saved* state is as legible as the *dirty* state. New re-arms an empty area; Edit
loads an existing list item back into the same area. Cairn mapping: ADR-0020's durable
scratchpad + sign-off supersedes the save-button *semantics* (nothing is lost if Accept never
comes), but the **three visible states** (uncommitted / committing / committed-and-listed) map
cleanly onto scratchpad / commitment / event-appended and should survive into the shell: the
red-border-equivalent is the honest rendering of "this text is scratchpad, not yet record".

### 2.5 The vocabulary never blocks

Wherever a coded lookup exists there is an escape: a **New Term** checkbox on the order search
(free-text a test the vocabulary lacks), free-text detail fields beside every coded field, and
a "non-drug allergen" free path beside the drug search. Coding is *offered*, never *required*.
Cairn mapping: principle 4 + [ADR-0025](../../docs/spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)'s
overlay posture — the UI rule worth writing down is "every coded field has a first-class
uncoded sibling, and choosing it is not a workflow penalty".

## 3. Module-specific findings (mechanism level)

### 3.1 Allergy entry (sequence 01–03)

- **"Nil Known" checkbox** at the top: an explicit negative assertion, distinct from an empty
  list — pairs with the banner's asked-state (consult note 2.6). Recording "nil known" is one
  click, then *saving an actual allergy later implicitly retires it* (the banner updated after
  save in snip 18).
- **Date of Reaction vs Date Entered** — a bitemporal pair in production, 2009-era: the
  reaction date is the asserted historical claim, entry date the system fact (ADR-0003's
  `t_effective`/`t_recorded` split, lived).
- **Specificity radio: class / generic / brand** — the reaction is recorded at the granularity
  actually known (e.g. a whole antibiotic class, not the one product that triggered it). An
  imprecise near-truth beating a precise untruth, *as a prescribing-safety feature*: class-level
  records catch cross-reactivity that product-level records miss.
- **Confirmed: Yes/No** — a certainty grade on the assertion itself.
- Coded reaction + free-text details side by side (§2.5 again).
- **Case-mine (negative exhibit):** the demo itself contains an accidental four-digit-year typo
  backdating the reaction to the 11th century — accepted silently. Cairn keeps free backdating
  (principle 4) but the write surface should attach an **advisory plausibility flag** (before
  birth / before ~1900 / in the future beyond `t_recorded`) — flag, never block, per ADR-0003's
  clash-handling posture.

### 3.2 Prescribing (06–07)

- Generic-fragment search → candidate list showing strength, form, pack, subsidy status →
  selection auto-fills the whole script (brand, price, default quantity/repeats, script type);
  directions and **reason-for-use** are entered at prescribe time. Reason-for-use is the origin
  of the meds-table "For" column the consult note already flagged as quietly excellent
  (2.16) — the *indication travels with the prescription from the moment of writing*.
- **Interaction checking is ambient, not modal:** on selection, side panels populate with
  food–drug advice and drug–drug interaction narratives (per-drug tabs) *beside* the edit area,
  while the allergy banner stays in view. Nothing blocks; the prescriber reads what applies.
  This is the ADR-0030 advisory posture rendered as layout, and the strongest new paper-parity
  exhibit in the batch: the paper counterpart (MIMS on the desk + the allergy sticker in view)
  is *matched in glance cost*, where a blocking interaction popup would fail it.
- An equivalent-brands panel renders alongside (substitution transparency).
- "Add to List" accumulates multiple items into one script run — the session fold again.

### 3.3 Ordering (04–05)

- **One search across pathology and radiology**, with panel/battery expansions inline;
  requests accumulate into a list; per-request flags (urgent, fasting, phone/fax, copy to
  patient) are checkboxes on the side.
- The order form **auto-includes current medications and clinical notes** (toggleable) — the
  requester's context travels with the request without re-typing.
- Copy-recipient incremental search scoped by radio (organisation / person / **the patient**) —
  copying the patient themselves is a first-class recipient type.
- "Requests Ordered This Consultation" list = the encounter fold (prefetch-note machinery).

### 3.4 Referrals (14–15)

- Addressee found by occupation/category or name; everything postal auto-fills; referral-type
  and priority combos pre-defaulted; a **letter tag** field names the referral's topic.
- Structured **inclusions** (e.g. the care plan, selected documents) are checkboxes/lists, with
  reprint-suppression toggles for cc'd recipients.
- **Live preview tab renders the fully-assembled letter** (header, addressee, patient
  demographics block, typed body, signature) before anything is sent/printed; a send-as-HL7
  option sits beside print. Two folds below: referrals this consultation, documents
  sent/printed.
- Cairn mapping: a letter is a **projection** — structured events + a short authored body
  assembled into a human-legible document, previewed as what the recipient will actually see.
  Rhymes with the legibility-twin discipline and ADR-0019's export-as-document instinct. The
  paper counterpart (dictated letter, typed later) is *beaten* on latency at equal formality.

### 3.5 Past history / care plan (17 + guide p. 50)

- Onset accepts **Year OR Age** (whichever the patient actually remembers) **with an explicit
  "Uncertain" checkbox** — 2009-era production UI for graded time claims; maps directly onto
  the `(value, precision)` + uncertainty model (ADR-0003; consult note 2.19's mixed-precision
  observation, now shown at the *entry* surface).
- Laterality radio includes an explicit **"None"** (not-applicable as a first-class state, not
  a blank).
- Per-issue flags: operation, cause-of-death, risk-factor, confidential, significance tiers,
  active/inactive — the significance-split's authoring surface (consult note 2.19).
- Paired free-text panes: historical summary + management-plan summary; care-plan
  **contributions** structured as "GP will / patient will / other providers will" with a
  provider-contribution table — a team care plan as *attributed commitments*, which rhymes with
  contributor-role thinking (who does what, recorded per party).

### 3.6 The record-as-document (08–11)

The all-previous-notes view renders the whole record as **one continuous chronological
document**:

- a **margin provenance block** per entry — date, care setting, time, author + role — beside
  the narrative (the paper chart's date-column, kept);
- **structured events render as prose lines in the same flow** (a logged recall with its due
  date; an order with recipient and copies) — the timeline is *complete*, not just the typed
  text;
- **measurements render as hyperlinks** in the text (value shown, click → trend graph);
- **images and graphs embed inline** (wound photos within procedure notes; trend charts and
  device-download graphs pasted into the narrative) — notes are compound documents
  ([ADR-0013](../../docs/spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)
  references + renditions, displayed in place);
- a **whole-record Find bar** (find / next / highlight-all / match-case) — the record is
  searchable like a book, which beats paper honestly;
- **"Include Audit Trail"** checkbox: audit entries (e.g. a result viewed-and-filed with no
  action, with its source-row reference) interleave into the same timeline,
  **colour-highlighted** — off by default, one click away, never a separate app. The audit
  trail is part of the record's own narrative, in time order, beside the clinical entries it
  concerns.

Cairn mapping: this is the display side of principle 1 + [ADR-0041](../../docs/spec/decisions/0041-progress-note-narrative-format.md):
the projection fold rendered as a single legible document, with audit events as ordinary events
admitted to the same timeline under a display filter. The margin-provenance layout is a strong
candidate for the Note tab's read view; the audit-overlay toggle is a strong candidate for a
shell-level affordance (any timeline view can admit its audit stream).

**HH (2026-07-18) — audit-trail scope clarified:** the easyGP audit trail reflected **only what
actually got saved** — never-Accepted data left *no trace at all* — and it captured *"who left
a trace in the database, and when"* in **wall-clock time, relying on the computer clock
alone**. Two Cairn contrasts fall out:

1. **Time:** bare wall-clock audit timestamps are exactly the trust gap
   [ADR-0027](../../docs/spec/decisions/0027-trusted-time-anchoring.md) closes — `t_recorded`
   is an HLC with a graded clock-confidence interval, so "when was the trace left" carries its
   own honesty bounds instead of silently trusting the box's clock.
2. **The abandoned-entry blind spot closes by construction:** in easyGP, typed-but-never-
   Accepted work was invisible to both record and audit. Cairn's ADR-0020 model makes the
   scratchpad *durable*, and quarantine-commits it if sign-off never comes — so abandoned work
   is neither lost (the §4.1 loss mode) nor invisible (the audit blind spot). The blind spot
   and the loss mode were the same defect seen from two sides, and one mechanism retires both.

### 3.7 Drawing editor + webcam (12–13)

A draw panel with an **anatomical stencil library** (body outlines, face, eye, ear, hand,
foot…), simple annotation tools, and a **webcam tab** capturing photographs straight into the
note. Paper's sketch affordance — the wound diagram drawn on the continuation sheet — restored
rather than lost, and photography made cheaper than paper ever had it. Cairn mapping: ADR-0013
attachments with the stencil set as manifest content; a paper-parity item most modern EHRs
fail (describe-in-words-only is *slower and worse* than the paper sketch).

### 3.8 Adaptivity + self-documentation (16, 18, guide pp. 45–47)

- Every screen region sits in **user-draggable splitters whose layout persists per user**;
  a **global font-size preference** re-flows all forms (labels auto-resize). Production prior
  art for the shell §7 preference layer's *mechanism*, and directly relevant to the
  small-screen/Pi constraint.
- **Tooltips on every icon** and inline field-level hints ("what goes in this field") — plus an
  in-app manual whose chapter list is itself telling: *The Concept of 'Focus'* and *Keyboard
  Navigation* are first-class chapters, before any module documentation. The keyboard path was
  the documented path.
- The help text invites users to remove unused module buttons from their toolbar — the
  user-configurable action set (consult note 2.9) confirmed in the manual's own voice.

## 4. What NOT to port (superseded or negative exhibits)

1. **The Save/Accept-button lifecycle** — data typed but never Accepted is lost. ADR-0020's
   durable scratchpad + sign-off keeps the *commitment moment* (invariant 4) while removing the
   loss mode. Port the visible tri-state (§2.4), not the semantics.
2. **Per-module bespoke save plumbing** — the guide itself admits the save-key conventions
   "vary from module to module for historical reasons." The consult note's §1a lesson
   (containment discipline; one contract layer) already covers this.
3. **Silent acceptance of implausible backdates** (§3.1) — add the advisory plausibility flag.
4. **Rich-text formatting bar as the note's primary chrome** — already ruled (consult note
   2.11): keyboard-first markdown; the *drawing/photo* affordances survive, the font-fiddling
   does not.
5. Red doing multiple jobs, always-visible empty panels, truncating columns — already filed as
   consult-note tensions (§4.2); this batch adds the *green-means-searching* counter-example
   worth keeping (§2.2).

## 5. New principle-4 archaeology exhibits (continuing the consult note's §4.3 list)

6. **"Nil Known" as an explicit assertion** — the recorded negative, distinct from blank
   (allergies, §3.1).
7. **Allergy specificity class/generic/brand** — graded granularity of the allergen claim as a
   safety feature (§3.1).
8. **"Confirmed: Yes/No" on the allergy assertion** — certainty recorded beside the claim (§3.1).
9. **"Uncertain" checkbox on onset, Year-or-Age alternates** — graded, format-flexible time
   claims at the entry surface (§3.5).
10. **Laterality "None"** — not-applicable first-class, never a meaningful blank (§3.5).
11. **Date of Reaction vs Date Entered** — the bitemporal pair at a 2009 production write
    surface (§3.1).

## 6. Distilled GUI principles (candidates for graduation into the shell spec)

1. **One entry grammar.** All clinical data entry shares the six-invariant editing-area shape;
   modules contribute vocabularies (fields, verbs), never new interaction models. (§1, §2.1)
2. **Type-ahead against the record and reference data is the primary input mode**; keyboard
   path first; one visual state (one colour) means "searching". (§2.2)
3. **Auto-fill to the fork.** Assert everything derivable; prompt only at genuine forks, and
   prompt with true options, never a blank form. (§2.3)
4. **All entry state is ambient** — dirty, committed, searching, warned — and never modal;
   decision support renders *beside* the work, not *in front of* it. (§2.4, §3.2)
5. **The vocabulary never blocks**: every coded field has an uncoded sibling with no workflow
   penalty. (§2.5)
6. **Every module shows its session fold** ("…this consultation"). (§3.3)
7. **Documents are assembled projections with live preview** — the clinician sees what the
   recipient sees before committing. (§3.4)
8. **The record reads as one book**: chronological narrative with margin provenance, structured
   events as prose lines, inline attachments, whole-record find, and the audit stream one
   display-toggle away in the same timeline. (§3.6)
9. **Keep paper's drawing hand**: stencils + annotation + camera, inline in the note. (§3.7)
10. **Geometry, fonts, and action sets are per-user and persistent** (the §7 preference layer's
    production ancestor). (§3.8)

## 7. Open questions for HH / the co-author

- **Audit-overlay usage:** did the Include-Audit-Trail toggle see real clinical use (e.g.
  medico-legal review), or was it an admin tool? Shapes where Cairn surfaces it.
- **Draw editor usage:** did the stencil library earn its place in daily work, or was the
  webcam the survivor? (Same fossil-check discipline as the consult note.)
- **Invariant-4 failure rate:** how often was work lost to a never-clicked Accept (the §4.1
  loss mode ADR-0020 removes)? Anecdote is the *only* possible evidence — **HH (2026-07-18)
  confirmed the system itself recorded nothing for unaccepted data** (see the §3.6 audit-scope
  clarification), so the loss rate is unmeasurable by design; it calibrates how loudly the
  scratchpad model should advertise itself.
- **Did the one-grammar rule ever break?** Any module where the editing-area shape genuinely
  didn't fit (the guide hints excisions pushed it hardest: 0–9 images per record) — the
  counterexamples would stress-test principle 1 above.
- **Results-inbox screenshots still pending** — the consult note's 2.21 three-zone question
  rides on them; snip 04's right-pane results list is a partial preview but not the review
  workflow itself.
