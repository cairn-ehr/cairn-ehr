# Design — the registration/search funnel as the reference UI's front door (slice 2)

- **Date:** 2026-09-20 (workflow revised 2026-09-21)
- **Depends on:** slice 1, `2026-09-21-patient-search-fragment-matching-design.md` — this UI is not
  viable on exact-token matching, see *Why slice 1 comes first*
- **Spec sections:** §5.3 / §5.8 (search-before-create), §9.5 (UI pluralism), §5.11 (read budgets)
- **ADRs:** [ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md) ·
  [ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) ·
  [ADR-0021](../../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) ·
  [ADR-0060](../../spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)
  decision 2 (partial completion is reported, never implied)
- **Issues:** [#636](https://github.com/cairn-ehr/cairn-ehr/issues/636) (fragment lookup — now a
  prerequisite, addressed by slice 1)

## Context

The node side of the funnel is built (`search_patients`, `register_patient`). The UI side is
deliberately unbuilt; `cairn-gui-tauri/src/main.rs` says why:

> There is no patient picker: the §5.3/§5.8 search-before-create funnel is unbuilt, and inventing a
> throwaway one here would put an untested wrong-chart hazard in front of a clinician (principle 3 —
> the paper affordance for "am I on the right chart?" is possession, not a dropdown).

Reused unchanged: `SessionKey` (the clinician's signing key held for the session, ADR-0053), the
`cease` / `sign_off` write shape, and `--mock`.

## Why slice 1 comes first

A clerk will not type `Fyodorowksi-Eschenbacher` or a full Latino compound surname to find a chart.
They type a fragment and pick from a list. Exact-token matching cannot serve that, so building this
UI on it would ship a front door people work around. Slice 1 makes fragment lookup real; this slice
assumes it.

## The workflow

**Step 1 — browse.** One search field set: part of a name, and/or date of birth. Results are a
**scrollable** candidate list. The clerk picks one and the chart opens. Done.

**Step 2 — register, only if nothing fits.** The clerk moves to the data-entry screen. Fields
prefill from whatever was typed in step 1, so nothing is retyped.

**Step 3 — the machine searches, automatically.** Once the form holds enough to identify a person —
**a given name, a surname and a date of birth** — a search runs in the background over that
completed data. If it finds likely matches, it asks: *could this be one of these existing patients?*
The clerk either recognises one (and that chart opens instead) or confirms none fit and registration
proceeds.

This minimises keystrokes: the cheap fragment lookup catches the common case, and the expensive
exact check happens once, automatically, on data the clerk has already typed for another reason.

## Decisions

### 1. The whole funnel, not the search half

Search *and* registration. A search surface alone leaves ADR-0061's candidate-naming attestation
untested.

### 2. The attested search is the COMMIT-TIME one, not the browse search

This is the load-bearing decision, and it resolves what would otherwise be a direct conflict.

`SearchAttestation { query, displayed }` is meaningful only as a matched pair from one
`search_patients` call. The browse search cannot supply it: the clerk types fragments, edits freely,
and browses a long list, so its query and its displayed set are a moving target. The step-3 search
is the opposite — it runs once, automatically, over the finished registration data, and its results
are shown in a bounded prompt.

Two consequences follow, and both are why this design works:

- **The browse list is free to scroll**, because it carries no signed claim. The non-scrolling
  constraint applies only to the *attested* list.
- **Query/displayed drift becomes structurally impossible.** An earlier draft of this design worried
  about a clerk correcting `Jon` → `John` between searching and registering, leaving the attestation
  describing a search that never ran for the name actually created. In this workflow the attested
  search runs *on whatever the clerk finally typed*, so the two cannot disagree.

### 3. The step-3 prompt is bounded and does not scroll

It is the attested list, so *"the candidate ids that were on the screen"* must be literally true.
Signing that 40 were displayed when 3 were visible is a precise untruth (principle 4), and it is
exactly the claim someone would later use to argue the clerk should have seen the duplicate. The
prompt shows at most what fits without scrolling; beyond that it is marked `incomplete` with its
reason.

This is cheap here in a way it would not have been for the browse list: a search over a full name
plus date of birth returns few candidates by construction.

Rejected: viewport tracking (a signed clinical record should not assert "this row was on screen",
and no test can pin it) and attesting everything rendered (signs what the clerk may never have
seen).

### 4. Gender displays and ranks; it never excludes

A candidate whose sex is unrecorded — or recorded wrongly, a common entry error — must not become
invisible because the clerk typed a sex. That is the matcher's *no-data-is-never-disagreement* rule
(principle 4) applied to search.

So gender is **client-side only**: it orders and annotates the candidates the node returned, and is
never sent to `cairn_search_candidates`. A useful consequence — it never enters `SearchQuery`, so
the signed attestation stays truthful about what actually narrowed the search, and no additive
change to a signed body is needed.

### 5. The wrong-chart affordance is possession, not a gate

Opening a chart — by picking in step 1, recognising in step 3, or registering — lands on the same
surface with a persistent identity header carrying name, DOB and identifier. A mis-pick stays
visible at every later step rather than being caught at one moment. This restores the paper
affordance the existing note names instead of adding a confirmation dialog, which §1.2 rejects.

## Architecture

**Ports (`cairn-gui-data`).** `ClinicalData` stays read-only. Two narrow traits beside it:
`PatientSearch::search(&SearchQuery, today) -> CandidateList` and
`PatientRegistration::register(token, name) -> Uuid`. Split so the write surface is one method wide
and `--mock` exercises browsing with no signing at all.

**The token.** The step-3 search returns an opaque token naming the `(SearchQuery, CandidateList)`
pair it produced, held in `state.rs`. `register` takes only that token — never a query and a list as
separate arguments — so the frontend cannot mint an attestation, and a UI bug fails to register
rather than signing a false one. Editing the form after step 3 discards the token and re-runs the
search.

**Commands.** A new module, not `commands.rs`: that file is already 456 lines and would cross the
project's 500-line guideline. (`cairn-gui-tab-medications/src/view.rs` is already 645 — noted, out of
scope.)

**Shell and frontend.** The front door is a shell state, not a tab — a tab presupposes a patient.
`--patient <uuid>` keeps working, so the timing runbook and the `--mock` accessibility pass do not
move. Plain JS in `src-ui/`, per the no-npm rule.

## Error handling

- **Browse search fails.** The failure renders as itself. A failure must never read as "nothing
  found" — the distinction principle 4 cares about.
- **Step-3 search fails.** No token, so registration cannot proceed. Registering without its
  due-diligence search is exactly what ADR-0061 forbids.
- **Register fails.** The form keeps its values. `register_patient` validates the DOB shape before
  ticking any HLC, so a malformed date refuses the whole call with no partial chart.
- **Session locked.** Routed through the existing `unlock`.

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the registration desk and the alphabetical patient index drawer.

**Steps:**

| | Register a new patient | Find an existing chart |
|---|---|---|
| Paper acts (N) | 5 — ask details, flip drawer, take blank card, write it, file it | 3 — ask details, flip drawer, pull card |
| Architecture-forced (M) | 4 — type fragment, read list, complete the form, answer the prompt | 2 — type fragment, pick |
| UI bundling target (K) | 4 | 2 |

`M ≤ N` for both, so there is no architecture defect to file. Registration's M is 4 rather than 3
because ADR-0061's due-diligence search produces a prompt the clerk must answer; that answer *is*
the act, so it is not a step to bundle away. Finding an existing chart beats paper at 2 versus 3.

**Time + cognitive load.** Browse results within §5.11's "no spinner"; the 5 s ceiling for *find an
existing chart* is inherited from db/046 and re-measured by slice 1. This slice is the funnel's
first runnable surface, so it owes the end-to-end measurement, taken in `--mock` and against a
database with the existing timing instrumentation. Cognitive load adds the step-3 prompt (which
fires only when it has something to say) and removes the need to hold the patient's identity in
your head, since the header is always visible.

## Testing strategy

TDD throughout.

**Pure.** The step-3 trigger rule (given name + surname + DOB present); the bounded prompt's cap and
`incomplete` derivation; gender ranking never dropping a candidate; token pairing, and an edit
discarding the token.

**Mock port.** The whole funnel with no database: browse, scroll, pick; and browse, no fit, register,
prompt, commit.

**DB-gated.** `register` writes an attestation whose `displayed` is exactly the ids the step-3 prompt
rendered, in display order, and whose `query` is the one that produced them.

**Negative.** `register` with a discarded or absent token refuses — the invariant in the direction
that matters, since the UI must not be able to sign a false attestation.

## Out of scope

No dedupe or merge UI. No John-Doe path. No identifier-type picker beyond what `SearchQuery` carries.
No editing a candidate before opening it. No change to the advisory matcher.

## Risks

- **The step-3 trigger is a heuristic.** Requiring given name + surname + DOB is a guess at "enough
  to identify a person". A mononymous patient, or one whose DOB is genuinely unknown, never trips it
  — and registration must still be possible for them (principle 4: *unknown* is a first-class value).
  The trigger therefore cannot be the only path to the prompt; registering without a completed
  trigger must run the search anyway, on whatever is there.
- **The bounded prompt assumes few candidates.** True for full name plus DOB, but not guaranteed.
  If it is routinely incomplete, the cap is wrong and the design needs revisiting rather than
  quietly signing partial lists.
