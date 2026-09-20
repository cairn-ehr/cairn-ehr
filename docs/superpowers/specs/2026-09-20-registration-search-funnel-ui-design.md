# Design — the registration/search funnel as the reference UI's front door

- **Date:** 2026-09-20
- **Spec sections:** §5.3 / §5.8 (search-before-create), §9.5 (UI pluralism), §5.11 (read budgets)
- **ADRs:** [ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)
  (registration is an act that carries its search) · [ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md)
  (per-write human authorship) · [ADR-0021](../../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)
  (layering) · [ADR-0060](../../spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)
  (decision 2: partial completion is reported, never implied)
- **Issues:** [#636](https://github.com/cairn-ehr/cairn-ehr/issues/636) (filed from this design —
  exact-token search cannot do fragment lookup)

## Context

The node side of the funnel is **built**: `cairn_node::patient::search_patients` maps this node's
projections to a `CandidateList`, and `cairn_node::patient::register_patient` authors the
registration act plus the name / DOB / identifier assertions, attesting
`SearchAttestation { query, displayed, incomplete }` — the candidate ids that were on the screen, in
display order.

The UI side is **unbuilt, deliberately**. `cairn-gui-tauri/src/main.rs` states why:

> There is no patient picker: the §5.3/§5.8 search-before-create funnel is unbuilt, and inventing a
> throwaway one here would put an untested wrong-chart hazard in front of a clinician (principle 3 —
> the paper affordance for "am I on the right chart?" is possession, not a dropdown).

This design retires that note by building the funnel properly.

### What already exists and is reused

- **A held human identity.** `state.rs`'s `SessionKey` holds the clinician's unsealed signing key
  and its `kid` for the session, distinct from the node key, with an `unlock` command. Built for
  ADR-0053; reused unchanged.
- **An established write shape.** `cease` and `sign_off` already sign with the session key and
  submit through a `cairn-node` function. Registration is a third command in that shape, not a new
  pattern.
- **A mock mode.** `--mock` runs the window against fixtures with no database. It is a shipped mode
  (the accessibility pass and timing runbook use it), so the funnel must work in it.

## The finding that shaped this design

The name pass in `db/046_patient_search.sql` matches **exact tokens**, not prefixes or substrings:

```sql
CROSS JOIN LATERAL regexp_split_to_table(lower(normalize(pn.value, NFC)), '\s+') AS tok
JOIN unnest(COALESCE(p_name_tokens, ARRAY[]::text[])) t
  ON tok = lower(normalize(t, NFC))
```

The file states the trade-off: *"Exact equality, NOT `LIKE '%token%'`: a leading-wildcard match
cannot use an index at all, and the §1.2 paper-parity budget is 5 s to find an existing chart."*

So typing `michaelow` does **not** find *Samantha Michaelowski*. It returns zero whether or not she
exists, which means **a zero from a fragment is not evidence of absence**. A clerk who then
completes the name while registering moves the true match count from 0 to 1 — exactly the duplicate
ADR-0061 exists to prevent.

Two things follow, and both are decisions below: the UI must never let a zero read as absence, and
the background re-search is **load-bearing safety**, not a convenience. Fixing the root cause
(prefix matching) is out of scope and filed as #636.

## Decisions

### 1. The whole funnel, not the search half

Search *and* registration. A search surface alone cannot demonstrate ADR-0061's point, and leaves
the candidate-naming attestation untested.

### 2. The search fields ARE the registration form — one screen, not two

`register_patient(name, query, displayed)` takes **the query itself** as the source of DOB and
identifiers. The search terms are the registration content, so a separate prefilled form would
duplicate them and let the two drift.

The drift is not cosmetic. `SearchAttestation { query, displayed }` is meaningful only as a matched
pair from **one** `search_patients` call. With a separate form, a clerk who corrects `Jon` → `John`
before registering forces a choice between attesting the original query (honest about what was
searched, but no search ever ran for the name actually created) and attesting the edited query with
the old list (a straightforward lie — those candidates were not that query's results).

One screen dissolves it: the list beside the form always belongs to the values in the form.

### 3. Editing re-searches in the background; a banner announces new candidates

Non-blocking. Registration is always one action away, which is the workflow the maintainer asked
for. When an edit turns up candidates where there were none, a banner says so **before** the
registration commits. Given the exact-token finding, this is the only thing standing between a
fragment search and a duplicate chart.

### 4. "Displayed" means displayed: a bounded, non-scrolling list

The attestation signs *"the candidate ids that were on the screen."* A scrolling list breaks that:
signing that 40 were displayed when 3 were visible is a precise untruth (principle 4), and it is
exactly the claim someone would later use to argue the clerk should have seen the duplicate.

So the list shows at most what fits without scrolling at the window's minimum supported size. Beyond
that the list is marked `incomplete` with its reason, and the clerk is prompted to add identifying
detail — which matches paper, where you ask for a date of birth rather than fanning out forty cards.

Rejected: viewport tracking (a signed clinical record should not assert "this row was on screen",
and no test can pin it) and attesting everything rendered (signs what the clerk may never have
seen).

### 5. The wrong-chart affordance is possession, not a gate

Selecting a candidate opens the chart, and a persistent identity header carries name, DOB and
identifier so a mis-pick stays visible at every later step. This restores the paper affordance the
existing note names, rather than adding a confirmation dialog — which §1.2 rejects explicitly.

Registration ends the same way: the new chart opens with the same header. One continuous surface,
so the chart you are holding is the one you opened.

## Architecture

### Ports (`cairn-gui-data`)

`ClinicalData` stays read-only and untouched. Two new narrow traits beside it:

```rust
pub trait PatientSearch {
    fn search(&self, query: &SearchQuery, today: &str) -> Result<CandidateList, DataError>;
}

pub trait PatientRegistration {
    fn register(&self, token: SearchToken, name: Option<&str>) -> Result<Uuid, DataError>;
}
```

Split so the write surface is one method wide and `--mock` can exercise search with no signing at
all.

### The search token — the ADR-0061 invariant, structurally

`search` returns `{ candidates, incomplete, incomplete_reason, token }`. The token is an opaque id
for the last **completed** search, held in `state.rs` alongside the `(SearchQuery, CandidateList)`
it names. `register` takes the token, never a query and a list as separate arguments.

The frontend cannot mint a token, and editing any field discards the one it holds. A UI bug
therefore **fails to register** rather than signing a false attestation. This is the single most
important structural choice in the design: it moves ADR-0061's guarantee from discipline to types.

Race: if an edit is in flight when the clerk registers, `register` waits for the in-flight search
rather than using a stale token. The §5.11 budget makes that wait imperceptible in the common case.

### Commands (`cairn-gui-tauri`)

A **new module**, not `commands.rs` — that file is already 456 lines and two more commands would
push it past the project's 500-line guideline.

- `search(query) -> SearchView` — debounced from the frontend.
- `register(token, name) -> Uuid` — unlocks first if the session is locked.

### Shell and frontend

The front door is a **shell state**, not a tab: a tab presupposes a patient context, and this
surface exists before there is one. `--patient <uuid>` keeps working unchanged, so the timing
runbook and the `--mock` accessibility pass do not move.

Frontend is plain JS in `src-ui/`, per the project's no-npm rule — no bundler, no `package.json`.

## Data flow

1. Window starts with no `--patient` → front door.
2. Clerk types → debounced `search` → `cairn_node::patient::search_patients`.
3. Render at most the non-scrolling cap. If the node reports `incomplete`, **or** the result exceeds
   the cap, show the incomplete marker and its reason.
4. Any edit → background re-search. If candidates appear where there were none, raise the banner.
5. Select a candidate → chart opens with the identity header.
6. No suitable candidate → **Register** → `register(token, name)` → `register_patient` signs with the
   session key → new uuid → the chart opens with the same header.

## Error handling

- **Search fails.** No token is issued, so `register` is unavailable. A failure can never be
  mistaken for "nothing found" — the distinction principle 4 cares about.
- **Register fails.** The form keeps its values and the error is legible.
  `register_patient` already validates the DOB shape *before* ticking any HLC, so a malformed date
  refuses the whole call with no partial chart.
- **Session locked.** Routed through the existing `unlock` command.
- **Node unreachable.** `DataError::Unavailable` renders as itself; the funnel does not pretend to
  have searched.

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the registration desk and the alphabetical patient index drawer.

| | Register a new patient | Find an existing chart |
|---|---|---|
| **Paper acts (N)** | 5 — ask details, flip drawer, take blank card, write it, file it | 3 — ask details, flip drawer, pull card |
| **Architecture-forced (M)** | 3 — enter details, observe candidates, register | 3 — enter details, observe candidates, select |
| **UI bundling target (K)** | 3 | 3 |

`M ≤ N` for both, so there is no architecture defect to file. M cannot go below 3: observing the
candidates **is** ADR-0061's act, so it is not a step to bundle away.

**Time + cognitive load.** Search results within §5.11's "no spinner" budget; the 5 s ceiling for
*find an existing chart* is inherited from db/046. This slice is the first runnable surface for the
funnel, so **it owes the measurement** — taken in `--mock` and against a database, using the
existing timing instrumentation. Cognitive load adds one element (the banner, which fires only when
it has something to say) and removes one (the identity header is always visible, where a paper card
is only in your hand while you hold it).

## Testing strategy

TDD throughout; the failing test comes first.

**Pure (no database, no window).**
- The non-scrolling cap and the `incomplete` derivation: a list longer than the cap is marked
  incomplete with a reason, and the rendered set is exactly the attested set.
- The banner rule: fires when a re-search yields candidates where the previous result had none.
- Token pairing: a token names exactly one `(query, list)` pair; an edit discards it.

**Mock port.** The whole funnel with no database — search, edit, banner, register, chart opens.

**DB-gated.** `register` writes an attestation whose `displayed` is exactly the ids the view
rendered, in display order, and whose `query` is the one that produced them. This is the test that
would have caught the separate-form drift.

**Negative.** `register` with a discarded or absent token refuses. This pins decision 2's invariant
in the direction that matters — the UI cannot sign a false attestation.

## Out of scope

No dedupe or merge UI. No John-Doe path. No identifier-type picker beyond what `SearchQuery`
already carries. No editing a candidate before opening it. No prefix/fragment search (#636). No
change to `db/046` or to the advisory matcher's blocking keys.

## Risks

- **#636 is mitigated, not solved.** The banner makes a fragment's false zero loud rather than
  silent, but a clerk still cannot type `Mich` and find `Michaelowski`. If the measurement in the
  §1.2 budget shows *find an existing chart* failing on realistic names, #636 stops being a
  follow-on and becomes a prerequisite.
- **The non-scrolling cap is a layout-derived number.** It must be computed and asserted by a test,
  not chosen by hand, or "displayed" drifts back to being true only by discipline.
