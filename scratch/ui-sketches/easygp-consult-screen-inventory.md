# easyGP consult screen → Cairn GP-facing UI: panel-by-panel inventory

> Scratch / conversation aid — NOT canonical. Sibling of
> [`easygp-prefetch-notes.md`](easygp-prefetch-notes.md) (write-model/prefetch mining, since
> promoted to ADR-0020) and of the private schema-mining companion note kept outside this repo.
> If anything here graduates, it graduates into the
> [shell design spec](../../docs/superpowers/specs/2026-07-12-clinician-reference-gui-shell-design.md),
> a GP manifest, or spec prose — this file stays scaffolding.

- **Source:** a screenshot of easyGP v0.532.2450 (2019) supplied by an easyGP co-author,
  showing the main consult screen with a fictional patient. The image is **deliberately not
  committed** (it carries a real clinician's name in the title bar and a person's photo); this
  note describes every panel textually so it stands alone. **More screenshots are expected**
  from the co-author; this note will grow as they arrive.
- **Updated 2026-07-16** with HH's answers to the first round of §4.4 questions — folded in
  inline as **HH:** annotations. Resolved: billing placement (2.13), the toolbar verdict (2.9),
  Decision Support / Research module contents (2.20), the results/inbox nutshell (2.21).
- **Why this artifact matters:** it is a compressed record of what an Australian GP needed
  visible, unprompted, at the moment of consult, refined over ~16 years of daily production use.
  The pixels are 2019; the **information architecture** is the durable payload.
- **Method:** for each panel — what it shows; its **paper counterpart** (principle 3's required
  question); an estimated **glance frequency** per consult; the **cost if it moves one click
  away**; its proposed **home in the Cairn shell**
  ([shell design](../../docs/superpowers/specs/2026-07-12-clinician-reference-gui-shell-design.md):
  titlebar / pinned safety zone / left rail / two-pane tabbed workspace — now Tauri 2 per the
  framework pivot); the **backing Cairn primitive**; and a verdict.
- **Verdicts:** **ZERO-CLICK** (earns permanent screen space), **ONE-CLICK** (a tab), or
  **FOSSIL-CHECK** (suspected accretion — HH to confirm or veto).

> [!IMPORTANT]
> **Glance-frequency estimates are Claude's clinical-reasoning guesses, not field data.**
> Only HH (and colleagues) can correct them. Same caveat as the prefetch note's "hit/miss was
> never measured": treat every frequency below as a hypothesis to be annotated, and remember the
> **survivorship-bias caveat** — this screen is the end state of years of accretion; some panels
> earned their place, some are fossils nobody dared remove.
>
> **HH (2026-07-16):** glance frequency **depends on clinician, patient
> situation/context/history, and familiarity with the patient — difficult to generalize.**
> Standing consequence for the design: fixed per-panel frequencies are the wrong model; the
> layout must be **per-user (and per-context) configurable** within what the role manifest
> offers — exactly the shell design's site/role ⊕ user preference layering (§7). The 2.9
> answer below is production prior art for the same conclusion.

---

## 1. The one-line thesis

The screen is a **single-screen consult cockpit**: identity + risk across the top, narrative
centre-left, current therapy + trajectory on the right, admin/billing along the bottom — nothing
requires navigation to *see*. That is paper-parity in its purest form (a paper file open on a
desk shows the summary sheet and the progress notes simultaneously), and it is the standing
challenge to Cairn's shell: **the ZERO-CLICK set below must fit the pinned safety zone plus the
two default panes, or the GP manifest needs a denser layout.** The legitimate criticism of the
original is density *without hierarchy* — everything renders at the same visual weight — not
density itself. Modern minimalist EHR layouts fail the same clinicians from the opposite
direction: every fact costs two clicks and a context switch.

## 1a. Why easyGP ultimately failed — and what that validates (HH, 2026-07-16)

The main failure cause was **not the UX** — it was the **choice of programming language**
(Gambas, adopted over HH's heavy objection): the obscure substrate made it impossible to
attract more developers, and poor separation of UI from program logic produced spaghetti code
and a poorly structured source tree. **Under the hood it was a mess; the user experience was
fabulous regardless.**

Three Cairn validations fall straight out:

1. **The §9 substrate-selection rule** (choose by defect blast radius, optimize the critical
   surface for reviewer-legibility, keep everything AGPL-mainstream) is precisely the
   anti-Gambas rule — contributor-attraction is a survival property, not a taste preference.
2. **The shell design's containment discipline** (framework-free contract/data/manifest layers;
   render bodies swappable) is the structural answer to "poor separation of UI and logic." It
   has already paid out once: the iced→Tauri pivot survived because the contract layers were
   framework-free.
3. **The mining method of this note is the right one:** harvest the UX and the information
   architecture; port none of the code. The fabulous-UX/mess-underneath split means the
   screenshot layer is exactly the valuable layer.

## 2. Panel inventory

### 2.1 Title bar — user, practice role, version, database host

- **Shows:** logged-in clinician + role ("Practice Principal"), product version, DB host.
- **Paper counterpart:** knowing whose desk and whose drawer you're at.
- **Glance:** rare, but load-bearing when it matters (wrong login, wrong node).
- **Shell home:** titlebar (already specified: node identity, online/offline, current user).
- **Verdict:** ZERO-CLICK (already in the shell design).

### 2.2 "TEST DATABASE" banner (loud red, top-left)

- **Shows:** unmissable environment flag on a non-production database.
- **Paper counterpart:** writing on a photocopy vs. the original chart.
- **Cairn mapping:** wrong-*node*/wrong-*environment* prevention is the same affordance family
  as wrong-chart prevention (possession semantics,
  [ADR-0008](../../docs/spec/decisions/0008-point-of-care-identity-possession-and-salvage.md)).
  A physical, ambient signal — not a confirmation dialog.
- **Verdict:** ZERO-CLICK — carry the idea into the titlebar spec: a demo/test/training node
  states itself ambiently and unmissably.

### 2.3 Left navigation rail (Clinical, Inbox, Clerical, Research, Admin, Patients, Contacts, Room Setup, Preferences, Library, Help)

- **Shows:** module-level navigation; "Clinical" is the consult cockpit described here.
- **Paper counterpart:** the different physical places in a practice (consulting room, front
  desk, mail tray, library shelf).
- **Glance:** a few times per session, not per consult.
- **Shell home:** the **left rail**, populated from the manifest — a direct match. Note the
  scope difference: easyGP put *whole-practice* modules (Clerical, Admin) in one binary; the
  shell spec deliberately scopes the reference UI to clinician work and leaves front-desk to a
  future product. The rail entries are therefore a *subset* of easyGP's list.
- **Verdict:** ZERO-CLICK (already in the shell design).

### 2.4 Patient banner — photo, name/DOB/address, phones, age, occupation, record number, insurer

- **Shows:** patient photo; a one-line identity string (name, birth year, street) that doubles
  as the patient switcher; home/mobile numbers; DOB + computed age; occupation ("Labourer
  (Retired)"); local record number; health-fund banner; smoking status in red ("EX-SMOKER
  CEASED AGE 23").
- **Paper counterpart:** the chart cover + summary sheet header; the photo is the face of the
  patient in front of you.
- **Glance:** consult start, every consult; the photo is a continuous ambient check.
- **Cost one click away:** unacceptable — this is the wrong-chart defence.
- **Cairn mapping:** the **pinned identity safety card** (already in the shell design's safety
  zone). The photo is a physical affordance against wrong-chart error — same instinct as
  ADR-0008 possession semantics. The record number is a node-local surrogate; the wire identity
  is the immortal UUID + assertion stream (demographics §4,
  [ADR-0031](../../docs/spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)
  dual-identifier discipline). Occupation and smoking status are clinical/demographic assertions
  with the usual principle-4 uncertainty states.
- **Verdict:** ZERO-CLICK. Concrete content list for the identity card: photo, name(s), DOB +
  age, sex/gender per §4, local record no., key contact, and *a small number* of
  clinician-promoted flags (see 2.5).

### 2.5 Inline risk flags in the banner (smoking status; insurer)

- **Shows:** "EX-SMOKER CEASED AGE 23" rendered in red at banner level; NIB health-fund strip.
- **Paper counterpart:** the sticker on the chart cover.
- **Cairn mapping:** *clinician-promoted* banner flags = a visibility/salience **overlay** on
  ordinary events (principle 2 applied to display, exactly the delete-vs-erase display-layer
  logic in the prefetch note). Which flags belong at banner level is **soft policy** — per-site
  or per-clinician manifest/preference, never schema. The insurer strip is AU-manifest content,
  not core.
- **Verdict:** ZERO-CLICK for the *mechanism* (promote-to-banner overlay); the *contents* are
  policy. Fossil-check the red colour coding: red is doing four unrelated jobs on this screen
  (test-DB, smoking, allergy header, billing warning) — Cairn's design language should assign
  colour a small consistent vocabulary.

### 2.6 Allergies/Sensitivities — "Asked - No Known Allergies"

- **Shows:** allergy status with the **elicitation state made explicit**: *asked-and-negative*
  is distinguished from *never-asked*.
- **Paper counterpart:** the allergy box on the summary sheet — which on paper is blank both
  when there are no allergies and when nobody asked. **easyGP improved on paper here**, seven
  years before Cairn wrote it down.
- **Glance:** every prescribing moment; ambient the rest of the time.
- **Cost one click away:** unacceptable (prescribing safety).
- **Cairn mapping:** principle 4 verbatim — *unknown ≠ not-yet-asked ≠ refused* as first-class
  values ([ADR-0003](../../docs/spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)).
  This is **principle-4 archaeology**: field prior art worth citing in spec prose. Lives on the
  pinned **meds+allergies safety card** (already in the shell design).
- **Verdict:** ZERO-CLICK.

### 2.7 Warnings panel — "NOT HAD FLUVAX FOR 2026", "BONE DENSITY ??", "Click to add warnings"

- **Shows:** a short stack of clinician-authored or rule-generated warnings. Note "BONE DENSITY
  ??" — a warning carrying an **explicit uncertainty marker** in its own text.
- **Paper counterpart:** the sticky note on the chart cover.
- **Glance:** consult start; ambient after.
- **Cairn mapping:** the pinned **urgent-actions card** (in the shell design). Generated
  warnings are advisory-actor output through the
  [ADR-0030](../../docs/spec/decisions/0030-advisory-actor-integration-contract.md) contract;
  human-authored warnings are ordinary events promoted by the 2.5 overlay mechanism. Crucially
  these are **ambient, never modal** — the whole screen contains zero confirmation dialogs,
  which is principle 3's ruling arrived at independently.
- **Verdict:** ZERO-CLICK.

### 2.8 Preventive-care recall list — "Overdue:106M Diabetes Annual Cycle … Overdue:75M tetanus booster"

- **Shows:** every overdue recall with **how overdue it is in months**, sorted, always visible.
  Nine items for this (deliberately extreme) test patient.
- **Paper counterpart:** none as-good — paper recalls lived in a card-file at the front desk.
  This is a place the screen *beats* paper, legitimately (paper-parity is a floor, not a
  ceiling).
- **Glance:** roughly once per consult — AU general practice runs opportunistic prevention
  (and its funding model rewards it), so "while you're here…" is a core GP move.
- **Cost one click away:** high — opportunistic prevention dies when the prompt isn't ambient.
- **Cairn mapping:** recalls are the canonical
  [ADR-0009](../../docs/spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)
  citizens: salience-routed, acknowledgment-floored, **never modal**. easyGP's raw
  everything-always list is the pre-ADR-0009 version; Cairn should keep the ambience but rank
  and de-noise (the case-study 0001 reader comments are blunt about prompt fatigue). The recall
  *generator* is an advisory actor; a recall-spawned context is the prefetch note's non-human
  context author (ADR-0007).
- **Verdict:** ZERO-CLICK as a compact ranked card in the safety zone; the *full* worklist is a
  ONE-CLICK tab (also the practice-level recall/prevention view).

### 2.9 Icon toolbar (~17 small glyphs) and the secondary notes toolbar

- **Shows:** two rows of small icon buttons (mostly unlabelled) for jumping to
  sections/actions; below them All Notes / Today's Notes / View GPMP / Care Planning / Export
  and New / Edit / Save / Print / Preview / Refresh.
- **Paper counterpart:** none — this is pure UI chrome.
- **Glance:** power-user muscle memory for a handful of them; the rest are onboarding debt
  ("mystery meat").
- **Cairn mapping:** partly superseded by the shell's own mechanisms: the rail + manifest
  replace section-jumping; the ADR-0020 type-through verbs (`rx!`, `tx!`, …) replace
  action-buttons *without leaving the keyboard*; explicit Save is replaced by the durable
  scratchpad + sign-off model (ADR-0020 active-write). A small set of labelled, semantic actions
  survives per tab.
- **HH (2026-07-16) — verdict corrected, not a fossil:** the icons had **tooltips** (and most
  GPs could derive their function from the glyph alone), and — the important part — the toolbar
  was **user-configurable in settings**: a few always-needed core icons, the rest picked and
  chosen per user. Rightly so: GP workflows and styles differ widely (urban subspecialised vs.
  rural true generalist; different or all age groups).
- **Verdict (revised):** a **user-configurable action set** — production prior art for the
  shell design's §7 **user preference layer** (the user arranges/chooses among what the role
  manifest offers). Port the *configurability*, not the specific toolbar. The "always-needed
  core + user-chosen rest" split maps exactly onto site/role layer vs. user layer.

### 2.10 Consult date + care-setting selector — "Consult Date 16/07/2026 16:12", "At consulting rooms"

- **Shows:** an **editable** consult date/time and a care-setting dropdown (consulting rooms /
  home visit / …).
- **Paper counterpart:** writing the date at the head of the note — including writing
  yesterday's date on a note you're catching up on.
- **Glance:** consult start; edited occasionally (catch-up notes, home visits).
- **Cairn mapping:** a clean split the original conflates: the editable date is **asserted
  `t_effective`** (freely backdatable claim), while **`t_recorded`** (HLC) is the untouchable
  ceiling — clashes flagged, never auto-resolved (ADR-0003, spec §3.6/§3.7). The care setting is
  a **possibly-absent descriptor on the thin context** (prefetch note: never force a formality
  the encounter didn't have). The visible "current consult" header *is* the armed write-context
  (ADR-0008).
- **Verdict:** ZERO-CLICK (it's the write-context header of the Note tab, not a separate
  panel). Honest backdating beats paper here: paper backdating is silent; Cairn's is recorded.

### 2.11 Progress-note editor (All Previous Notes / New General Notes; formatting bar; Templates; Fee Schedule)

- **Shows:** the main free-text editor with a rich-text formatting bar, template picker, and —
  telling — a Fee Schedule button *inside the note area*. Tab to all previous notes.
- **Paper counterpart:** the continuation sheet. Free-flowing prose is the GP's native format.
- **Glance:** continuous — this is where the consult lives; ~60% of screen real estate.
- **Cairn mapping:** the **Note tab, left-pane default** (already in the shell design). The
  Cairn version keeps the free-text *feel* while the ADR-0020 type-through model co-produces
  structured events + readable note lines in one flow (the legibility twin born at authoring
  time — see the prefetch note, which carries the full write-model). Markdown-source + preview
  per the shell spec; a WYSIWYG formatting bar is explicitly *not* the priority — 16 years of
  keystroke elimination argue the keyboard-only path is the one that matters.
- **Verdict:** ZERO-CLICK (exists). Templates = ONE-CLICK affordance inside the tab. The Fee
  Schedule button placement is a symptom of 2.13, not a feature to copy.

### 2.12 Todo / BMI strip (Height, Weight, BMI)

- **Shows:** a slim strip for the consult's measurements/todo under the editor.
- **Paper counterpart:** jotting obs in the note margin.
- **Glance:** when taking measurements; otherwise dead space.
- **Cairn mapping:** measurements are ordinary typed events entered through the type-through
  flow (`bp!`, `wt!`-style verbs) rather than a permanently visible strip; trends then render in
  2.17.
- **Verdict:** FOSSIL-CHECK — suspect a permanent strip is accretion; a type-through verb +
  the trend panel covers it. HH to confirm whether the strip earned its place.

### 2.13 Billing/appointment band — timer, "No Appointment Found → auto-billing unavailable", Quick Item Select, Patient Billing Level "Bulk Bill", BB toggle, Post

- **Shows:** a consult **timer** (2:24, pausable — Medicare time-tiered items), appointment
  linkage state with an *explanatory* degradation message ("As no appointment found,
  auto-billing unavailable" — green box, reason stated, nothing blocked), billing level,
  item-number quick pick, and Post.
- **Paper counterpart:** the billing slip filled at the end of the consult — by the *GP*, at
  the desk, in AU general practice. Billing here is a clinician workflow, not (only) a
  front-desk one.
- **Glance:** end of every consult; timer is ambient.
- **Cost one click away:** moderate — but forgetting it costs the practice real money, which is
  why easyGP kept it on-screen.
- **HH (2026-07-16) — RESOLVED, essentially option (b), and the tension was overstated:**
  consult-time billing is a **non-issue**. Billing proper lives in a **companion app for
  front-desk staff**; the GP only enters **item numbers or a short comment to billing staff at
  the end of the consult**, in an **unobtrusive widget** (e.g. overlaying the timer when
  stopping it, or a small tab). And the **timer is not an honest timekeeper**: both easyGP
  principals routinely ran **multiple patients in multiple rooms near-simultaneously**
  (alternating rooms while a practice nurse attended in between) — which is typical. The timer
  is a *visual cue for the GP* and an *input to billing suggestions*, nothing more.
- **Cairn mapping:** the clinician UI emits a tiny end-of-consult billing event (item numbers /
  free-text note to billing staff) consumed by the front-desk product; principle 4 applies to
  the **timer itself** — elapsed wall-clock is *uncertain evidence* of consult duration
  (multi-room interleaving), so any duration-derived billing suggestion is **advisory, never
  auto-asserted**. The honest auto-billing degradation message remains principle-4 archaeology:
  state *why* the automation can't run, never fake it, never block.
- **Verdict:** the *end-of-consult widget* is ZERO-CLICK-adjacent GP-manifest chrome (one
  gesture: stop timer → enter items/comment); everything else is the companion app's problem.

### 2.14 Coded reasons + favourite coded terms (ICPC-2 PLUS-style list, e.g. "Check up;blood pressure (K31001)")

- **Shows:** a search box for coding the consult's reasons + a personal favourites list of
  ~12 coded terms (certificates, care plans, BP checks, driver's licence exams, mental-health
  care plans, referrals…) — one click to code the consult.
- **Paper counterpart:** none (paper notes are uncoded); the driver is accreditation/PIP/data
  quality, plus care-plan billing items (GPMP — see the View GPMP / Care Planning buttons).
- **Glance:** end of most consults.
- **Cairn mapping:** [ADR-0025](../../docs/spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)
  structurally: ICPC-2(-PLUS) is the GP-native working vocabulary = the **local-terminology
  overlay** on the canonical interlingua; the favourites list is per-user code weighting (the
  schema-mining note found `usr_codes_weighting` — same instinct). In Cairn this should melt
  into the type-through flow (code as you write the reason) rather than remain a separate
  bottom-left panel.
- **Verdict:** ONE-CLICK panel today → **zero-extra-keystroke** aspiration inside the Note tab.

### 2.15 "Medications Prescribed Today" + "Sticky Notes From Office Staff (14)"

- **Shows:** (a) a session-scoped list of what you've prescribed *this consult*; (b) an
  async message stream from front-desk to clinician, timestamped, patient-linked ("Find out
  cost of tests for Sarah for Fragile X — see her notes").
- **Paper counterpart:** (a) glancing back up the page you're writing; (b) the actual sticky
  note on the chart — a beloved, real workflow.
- **Glance:** (a) during prescribing consults; (b) consult start.
- **Cairn mapping:** (a) is a **free fold**: all events sharing the current thin-context key —
  no feature needed, just a rendering of the encounter fold (prefetch note §"order provenance").
  (b) is an ADR-0009 salience-routed message stream whose display is patient-contextual; the
  sticky note's paper affordances (visible on chart-open, dismissible, never blocking) are the
  benchmark.
- **Verdict:** (a) ZERO-CLICK within the Note tab (it's the tail of the current fold);
  (b) ZERO-CLICK as a compact card when unread items exist, ONE-CLICK archive.

### 2.16 Current medications list (Active/Inactive; Drug/Dose/For/Qty/Rpt; authority markers; Webster-pack flag; Brands/Generics toggle)

- **Shows:** the full active-meds table — drug, dose, indication ("For": *prevention of gout*,
  *thin the blood*…), quantity/repeats, authority-script markers ("A"), Uses-Webster-Pack flag,
  brands/generics display toggle. Columns truncate and scroll horizontally (the screen's worst
  ergonomic sin).
- **Paper counterpart:** the medication card in the chart.
- **Glance:** most consults; continuously during prescribing.
- **Cost one click away:** high — meds + allergies are the safety pair.
- **Cairn mapping:** the **Meds tab** — the natural right-pane default for a GP manifest
  (the shell's first slice used Demographics on the right; for GP daily work, meds wins). The
  "For" column is quietly excellent: indication travels *with* the prescription — keep it.
  Authority/PBS script types, Webster-pack, brands-vs-generics are AU-manifest concerns riding
  the ADR-0020 prescribing model (script-type state machine is already on the easyGP-session
  build-prep list). A compact meds+allergies *summary* also lives on the pinned safety card;
  the tab is the full working view.
- **Verdict:** ZERO-CLICK (summary on safety card) + the full tab as right-pane **default** in
  the GP manifest. Fix the truncation: if a column matters enough to show, it matters enough to
  read.

### 2.17 Trend chart + metric switcher (BP 2010–2018; BP/Ht/Wt/eGFR/Hb/HbA1c/CEA) + "Cockcroft Gault Cr Clear unreliable at BMI of 40.46" + BP averages

- **Shows:** an always-visible longitudinal chart with one-click metric switching across
  vitals/labs, 12-month BP averages, and a decision-support line that **declares the limits of
  its own validity** (Cockcroft-Gault unreliable at this BMI).
- **Paper counterpart:** flipping to the obs chart / growth-chart page.
- **Glance:** chronic-disease consults — roughly half of AU GP work (estimate; HH to correct).
- **Cairn mapping:** trajectory views are folds over append-only events
  (latest-truth-per-timepoint — already on the prefetch note's verify list). The CG line is
  **principle-4 archaeology, second exhibit**: an advisory actor honestly bounding itself =
  exactly the ADR-0030 posture. The metric switcher is the right interaction (one chart, many
  series) — see `chart-two-zone.svg` in this directory for the current Cairn sketch.
- **Verdict:** ONE-CLICK — a **Trends tab** (right pane, one tab-switch from Meds), *not*
  pinned: permanent chart real estate is what squeezed the meds table into truncation. The
  two-pane shell makes "note left, trend right" a first-class arrangement anyway.

### 2.18 Tasks Needing Attention

- **Shows:** a patient-linked task list (empty for this patient) with New/Save.
- **Paper counterpart:** the follow-up note in the chart / the practice diary.
- **Glance:** when populated — which is the point: it earns attention only when non-empty.
- **Cairn mapping:** ADR-0009 again — tasks are salience-routed notifications with an
  acknowledgment floor. Display rule worth copying: **an empty panel should cost ~zero pixels**;
  easyGP kept the empty box visible (fossil behaviour).
- **Verdict:** ZERO-CLICK when non-empty (collapses into the urgent-actions card), ONE-CLICK
  full view.

### 2.19 Problem list — "Most Significant" vs "Less Significant But Active", with onset years; Active (≈20) / Inactive (21) tabs

- **Shows:** the health-issues list **split by clinical significance**, not merely
  active/inactive — sleep apnoea, pacemaker, metastatic bowel carcinoma up top; hypertension,
  obesity, 1971-onset items below; onset year on every line; inactive list one tab away.
- **Paper counterpart:** the summary-sheet problem list — which on paper is one flat list. The
  significance split is another spot easyGP *beat* paper.
- **Glance:** most consults (orientation: "who is this person clinically?").
- **Cost one click away:** high for the significant set.
- **Cairn mapping:** significance is a **clinician-authored ranking overlay** on problem
  events — display-layer, per-patient, editable, never schema (the 2.5 mechanism again;
  principles 2 + 9: mechanism ships, ranking policy is the clinician's). Onset year is
  `t_effective` with honest imprecision ("1997", "02/2007" — mixed precision on one screen is
  *correct* and Cairn's DOB-style `(value, precision)` model represents it natively,
  principle 4).
- **Verdict:** ZERO-CLICK for the Most-Significant set (candidate: the fourth safety-zone
  card, or the top block of a right-pane Summary tab); ONE-CLICK for the full/inactive lists.
  **HH (2026-07-16): confirmed as the right approach** (card-vs-Summary-tab placement still
  open).

### 2.20 Clinical Lists / Decision Support tabs + Plans / Forms

- **Shows:** the right panel is itself tabbed; Decision Support's content is not visible in
  the screenshot. Plans/Forms link to care-planning documents (GPMP etc.).
- **HH (2026-07-16) — contents supplied.** Two modules, and HH (who as a locum has used most
  Australian GP systems) knows **no other AU software with comparable features**:
  - **Decision Support** = a **condition-centric trajectory dashboard**: e.g. for a diabetic,
    HbA1c + cardiac risk factors + renal profile over time — *all the data needed to design a
    treatment plan visible in a single glance* — plus links to relevant literature (e.g.
    Therapeutic Guidelines) and risk calculators.
  - **Research** (the left-rail module) = the **same idea at practice-population scope**: list
    all patients with e.g. deteriorating HbA1c or renal function, **ordered by
    urgency/priority**; percentage of chronic-disease patients achieving management goals —
    informing practice policy and resource-allocation decisions.
- **Cairn mapping:** both are **folds — the same machinery at two scopes**, which is fractal
  topology showing up in the UI:
  - the condition dashboard is a **condition-scoped multi-series fold** (2.17's trend machinery,
    grouped by a condition template instead of one metric); guideline links and risk
    calculators are advisory actors through the
    [ADR-0030](../../docs/spec/decisions/0030-advisory-actor-integration-contract.md) contract
    (a calculator that declares its validity bounds — see the 2.17 Cockcroft-Gault line — is
    the house style).
  - the Research module is the **identical fold run at practice-node scope over the population**
    — a node's role is configuration, so a practice node ranking its own chronic-disease
    population is the same projection code, wider scope. The ranked deteriorating-patients
    worklist is ADR-0009-salience applied to a population (and rhymes with ADR-0014's
    background-sweep-whose-worklist-doubles-as-a-metric pattern).
- **Verdict:** Decision Support = a high-priority **Condition-dashboard tab** (ONE-CLICK,
  right pane, likely *the* chronic-disease-consult view); Research = a practice-scope module
  with its own rail entry in the GP manifest. Both are strong candidates for early slices
  precisely because they were easyGP's unmatched differentiator.

### 2.21 Results/inbox (not visible in this screenshot — HH description, 2026-07-16)

A topic in its own right (own session when its screenshots arrive); the nutshell as told:

- **On opening a patient**, new/unchecked results are **highlighted in the results tab**;
  **critical un-actioned results make the tab visually flash and demand attention — but no
  dialogs open.** (Principle 3 lived, again: ambient, insistent, never blocking.)
- **Before and after consulting sessions**, most GPs review incoming results and reports.
  Grouping is **per-clinician choice**: by urgency, by date/time, or by patient — more
  evidence for the user-preference layer (2.9, glance-frequency note).
- **The differentiator** (again unmatched in AU software HH has used): **real-time context for
  a result.** Click an incoming result in a sidebar → a **vertical splitter** shows the actual
  result/report on one side and **the most relevant consult note** on the other; if the result
  has a trajectory over time, it is **graphed in a bottom horizontal splitter**.
- **Cairn mapping:** this is almost a description of machinery Cairn already has on paper:
  - "most relevant consult note" = the **order-provenance fold** from the prefetch note
    (`result → order → order.encounter → fold that encounter`), with the honest labelled
    fallback ("most recent · ordering consult unknown") for externally-sourced results;
  - result-beside-note = the shell's **two-pane workspace + `OpenInOtherPane` intent**;
  - the flashing critical tab = **ADR-0009's acknowledgment floor** rendered as chrome;
  - the trajectory strip = the 2.17 fold.
  - **One real design question:** the three-zone review layout (result | note | trajectory
    below) exceeds the shell's strict "two panes, tabs own no internal tiling" rule. Either the
    results-review tab gets a sanctioned internal bottom strip, or the shell grows an optional
    horizontal third zone. Flag for the results-inbox session — don't improvise it.
  - Existing sketch: [`results-inbox.svg`](results-inbox.svg) in this directory.
- **Verdict:** deferred to its own session, but the *contract implications* (order-provenance
  fold, acknowledgment floor, three-zone question) are already actionable.

## 3. Summary table

| # | Panel | Verdict | Shell home |
|---|-------|---------|-----------|
| 2.1 | Title bar | zero-click | titlebar (exists) |
| 2.2 | Test-DB banner | zero-click | titlebar: ambient environment flag |
| 2.3 | Module nav | zero-click | left rail (exists) |
| 2.4 | Patient banner + photo | zero-click | pinned identity card (exists) |
| 2.5 | Banner risk flags | zero-click (mechanism) | promote-to-banner overlay; contents = policy |
| 2.6 | Allergies + asked-status | zero-click | pinned meds+allergies card (exists) |
| 2.7 | Warnings | zero-click | pinned urgent-actions card (exists) |
| 2.8 | Recalls (overdue, aged) | zero-click compact / one-click full | safety-zone card + Prevention tab |
| 2.9 | Icon toolbars | user-configurable action set (HH) | shell §7 user preference layer |
| 2.10 | Consult date + setting | zero-click | Note tab write-context header |
| 2.11 | Note editor | zero-click | Note tab, left-pane default (exists) |
| 2.12 | Todo/BMI strip | **fossil-check** | type-through verbs + Trends tab |
| 2.13 | Billing band + timer | resolved (HH): companion app | end-of-consult widget in GP-manifest chrome |
| 2.14 | Coded reasons + favourites | one-click → in-flow | inside Note tab (ADR-0025 overlay) |
| 2.15a | Prescribed today | zero-click | tail of the encounter fold, Note tab |
| 2.15b | Staff sticky notes | zero-click when unread | ADR-0009 card + one-click archive |
| 2.16 | Medications table | zero-click summary + tab | Meds tab, right-pane **default** (GP manifest) |
| 2.17 | Trend chart + CG caveat | one-click | Trends tab (not pinned) |
| 2.18 | Tasks | zero-click when non-empty | urgent-actions card + tab |
| 2.19 | Problem list (significance-split) | zero-click (significant set) — HH confirmed | 4th safety card or Summary tab top |
| 2.20 | Decision Support / Research | one-click condition dashboard; practice-scope module | Condition-dashboard tab + Research rail entry |
| 2.21 | Results/inbox (own session) | deferred | two panes + acknowledgment floor + fold; three-zone question |

## 4. Outputs

### 4.1 GP-manifest seed (for the shell's manifest layer, §7 of the shell design)

- **Pinned safety zone (4 cards):** identity (photo, names, DOB/age, record no., promoted
  flags) · meds+allergies summary (with elicitation state) · urgent-actions (warnings ∪
  non-empty tasks ∪ unread staff notes) · **compact ranked recalls** (the GP-specific card —
  ward/ED manifests would swap this out).
- **Left pane default:** Note tab (armed write-context header; type-through verbs; coded
  reasons in-flow; prescribed-today fold at the tail).
- **Right pane default:** **Meds tab** (not Demographics — that was a slice-1 convenience).
- **Tab priority for future slices:** 1 Meds → 2 Problems/Summary → 3 Trends → 4
  **Condition dashboard** (2.20 — the unmatched differentiator) → 5 Prevention/recalls
  worklist → 6 Tasks/messages → 7 Results/inbox (own design session first, 2.21) →
  Demographics and Note already exist. **Research** (practice-population scope) gets its own
  rail entry rather than a tab slot.
- **GP-manifest chrome:** consult timer (ambient, pausable, *advisory only* — see 2.13) with
  the end-of-consult item-numbers/comment widget attached to its stop action.

### 4.2 Tensions surfaced

1. ~~**Consult-time billing vs the shell's front-desk exclusion**~~ — **RESOLVED (HH,
   2026-07-16):** non-issue; companion front-desk app + an unobtrusive end-of-consult
   item-numbers/comment widget in the clinician UI (2.13).
2. **Recall ambience vs prompt fatigue** — easyGP shows everything always; case-study 0001's
   reader comments show GPs revolt at noise. ADR-0009 ranking + the acknowledgment floor is the
   synthesis; the GP card must stay *compact*. **HH: confirmed as a real work item.**
3. **Zero-click budget.** The ZERO-CLICK rows above must fit the safety zone + two panes.
   If they don't, the GP manifest needs a denser safety zone than the four-card default —
   test on real screens early (the Pi/small-screen constraint cuts the other way).
   **HH: confirmed as a real work item.**
4. **Colour vocabulary** — one screen, red meaning four things. Assign colour a small,
   consistent semantic set in the design language. **HH: confirmed as a real work item.**
5. **Empty panels must cost nothing** (2.18) — a layout rule for every card/tab.
6. **The three-zone results-review layout** (2.21) vs the shell's two-pane/no-internal-tiling
   rule — decide in the results-inbox session, not ad hoc.

### 4.3 Principle-4 archaeology (spec-citable prior art, 2019, production)

1. **"Asked - No Known Allergies"** — elicitation state first-class (principle 4 / ADR-0003).
2. **"Cockcroft Gault Cr Clear unreliable at BMI of 40.46"** — an advisory tool declaring its
   own validity bounds (ADR-0030 posture).
3. **"As no appointment found, auto-billing unavailable"** — automation degrading honestly,
   with its reason, without blocking (principle 4 + the ADR-0014 degrade-to-human pattern).
4. **"BONE DENSITY ??"** — recorded uncertainty in a warning's own text.
5. **Zero confirmation dialogs on the entire screen** — principle 3's ruling, lived.

### 4.4 Open questions for HH (updated 2026-07-16 — answered items struck)

- ~~Glance-frequency guesses~~ → answered structurally: context-dependent, don't generalize;
  per-user configurability is the design consequence (see the callout in the header).
- ~~Toolbar icons~~ → answered: tooltips + user-configurable action set (2.9).
- ~~Decision Support contents~~ → answered (2.20); more screenshots to come.
- ~~Billing placement~~ → resolved: companion app + end-of-consult widget (2.13).
- **Still open:**
  - Did the **Todo/BMI strip** earn its place (2.12), or does a type-through verb cover it?
  - Which panels did you (or colleagues) **never look at** — the pure fossils?
  - The 2.19 sub-choice: Most-Significant problems as the **fourth safety-zone card** vs.
    **top block of a Summary tab** (approach itself confirmed).
  - What else is **missing from this screenshot** that a GP needs at consult time
    (immunisation view? correspondence?) — likely answered as further screenshots arrive.
  - The **Research module**'s ranking logic (how "urgency/priority" was computed for the
    deteriorating-patients list) — worth capturing when its screenshot comes; it's the seed of
    the practice-scope salience policy.
