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
**at least two name tokens and a date of birth** — a search runs in the background over that
completed data. If it finds likely matches, it asks: *could this be one of these existing patients?*
The clerk either recognises one (and that chart opens instead) or confirms none fit and registration
proceeds.

> **Revised 2026-09-22 (slice 2a).** This read "a given name, a surname and a date of birth" until
> the plan for 2a met the code. That phrasing implies separate given/surname fields, which is *one
> culture's name model* — the cultural capture ADR-0014 forbids — and it fails outright for a
> mononymous patient, a patronymic, or Han name order. It also forced the raw name to be
> *reassembled* from two boxes, and `cairn_node::patient::register::register_patient`'s own doc warns
> that its `name` argument **must be the same typed string `SearchQuery` was built from**, with
> nothing in the types to enforce it.
>
> So the form keeps **one free name field**, exactly as the CLI's `--name` does, and the trigger
> counts whitespace-separated tokens. Same information content, no name model, and the drift
> `register.rs` warns about becomes structurally impossible: one typed string feeds
> `SearchQuery::new` and `register_patient` alike, never reassembled.

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
is the opposite — it runs automatically over the completed registration data, and its results are
shown in a bounded prompt. (It *re-runs* as the clerk edits; what matters is that only **one** run —
the one preceding the commit — is ever attested. Slice 2a found that the difference between "runs
once" and "one run is attested" is exactly where the custody bugs live.)

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

> **Split 2026-09-22 (slice 2a), and only half of this is built.** The decision has two limbs, and
> the code can carry only one of them today.
>
> - The **negative** limb — sex never enters `SearchQuery`, so no candidate can be made invisible by
>   it — is the safety content, and slice 2a honours it *by construction*: there is no sex field in
>   the browse form at all, so there is nothing to exclude with. A test pins it.
> - The **positive** limb — ordering and annotating — **cannot be built**, because
>   `cairn_patient_search::Candidate` carries no sex. Its seven fields are `patient_id`,
>   `display_name`, `age`, `trust`, `last_activity`, `locale`, `photo_ref`, and a round-trip test
>   pins that count at seven precisely so that adding one is a deliberate act on a read path
>   budgeted at *no spinner*. Deferred to
>   [#645](https://github.com/cairn-ehr/cairn-ehr/issues/645), which states what an additive
>   `Candidate.sex` would cost.

### 5. The wrong-chart affordance is possession, not a gate

Opening a chart — by picking in step 1, recognising in step 3, or registering — lands on the same
surface with a persistent identity header carrying name, DOB and identifier. A mis-pick stays
visible at every later step rather than being caught at one moment. This restores the paper
affordance the existing note names instead of adding a confirmation dialog, which §1.2 rejects.

## Slicing (added 2026-09-22)

The design is built in two slices, on the DR 2a/2b precedent, because the whole of it spans a new
crate, two ports, a mock, an optional `--patient`, a new command module, the frontend, a drift-guard
extension, a DB-gated attestation test and the §1.2 measurement.

- **2a — the pure core.** Every rule in *Testing strategy → Pure*, as a new `cairn-gui-funnel`
  crate, plus the two ports in `cairn-gui-data` and their mock implementation. No Tauri, no
  database, no frontend. Mergeable on its own.
- **2b — the surface.** The commands, the shell state, the frontend, the drift-guard extension, the
  DB-gated attestation test, and the end-to-end §1.2 measurement this design owes.

The seam is deliberate: 2a is where every *decision* on this page becomes an executable rule, and
2b is wiring. A rule that cannot be stated without Tauri belongs in 2b; anything else belongs in 2a,
where it is testable with no window and no database.

> **Split again 2026-09-22, with the maintainer: 2b became 2b + 2c.** The list above put the
> ports, the commands, the shell state, the frontend, the drift guard, the DB-gated attestation
> test and the §1.2 measurement in one slice. The seam that actually exists is **testability**:
> the ports are provable against a real floor today, and the measurement cannot be taken until a
> runnable surface exists. So, on the same DR 2a/2b/2c/2d precedent:
>
> - **2b — the data path.** A new `cairn-gui-live` crate implementing both ports over a real
>   node connection, `DataError::Refused` (#648), and the DB-gated proof that a registration
>   attests what the **prompt bounded** rather than the node's raw answer. Nothing under
>   `crates/`, nothing on screen.
> - **2c — the window.** The commands, the shell state, `--patient` becoming optional, the
>   frontend, the JS/Rust drift-guard extension, **the end-to-end §1.2 measurement this design
>   owes** (in `--mock` *and* against a database), and narrowing `main.rs`'s *"Writes are
>   refused in this mode"* to **clinical** writes.
>
> The measurement does not move: this page assigned it to the slice that first exposes a
> runnable surface, and that is now 2c.

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

> **Sharpened 2026-09-22 (slice 2a review).** The shape above is right; three things about it turned
> out to be load-bearing in ways the page did not say, so 2b should read these rather than the
> sentences above alone.
>
> - **`register` takes the attested pair BY VALUE, not a token.** *"`register` takes only that
>   token"* closes the frontend path, but a Rust caller holding the pair could still register twice.
>   `register(attested: AttestedSearch, name) -> Result<Uuid, (DataError, AttestedSearch)>` consumes
>   it and hands it back inside the error, so the flow is linear —
>   `record → take → register → Ok: commit / Err: restore` — and the failing branch is the only way
>   to get the value needed for a retry.
> - **The attested list is its own type.** The list a registration swears it displayed must be the
>   bounded one, and a bounded list is shape-identical to a raw node list, so the cap lived on the
>   honour system. `bound_for_prompt` now returns a `PromptList` with a private field and
>   `TokenStore::record` accepts nothing else.
> - **"Editing the form discards the token" is necessary but not sufficient.** Discarding must also
>   *prevent a later restore*: a registration failing while the clerk edits used to put the
>   pre-edit search back, so a chart could be born attesting a search for a different spelling of
>   the name. The store counts invalidations rather than inferring them from an empty slot, and
>   refuses a second `take` while one registration is still in flight.

**Commands.** A new module, not `commands.rs`: that file is already 456 lines and would cross the
project's 500-line guideline. (`cairn-gui-tab-medications/src/view.rs` is already 645 — noted, out of
scope.)

> **Added 2026-09-23 (PR #661): how the window gets an actor, and what 2c owes.** #654 asked
> whether the reference window should provision a `device` actor the way the CLI silently did.
> **Answer: neither surface does.** `cairn-node init` enrols, `cairn-node enroll-device-actor` is
> the named remedy for a node that never ran `init`, and all fifteen CLI write subcommands now
> refuse instead of provisioning — the asymmetry where a node's behaviour depended on which
> surface touched it first is gone, and provisioning-as-a-write-path-side-effect (trap 2,
> ADR-0066 decision 6) is now a rule rather than a preference.
>
> **What 2c owes:** `cairn_node::actor_enrolment::device_actor_standing` — a FOUR-state enum, not
> a boolean, because a `true/false` launch probe re-creates the dead end it fixed — so
> `build_live_state` can **probe at launch** and say so in the chrome — the same discipline it
> already follows by loading the node key up front rather than discovering at sign-off that it can
> never seal anything. That is #654's option 2. `LiveData` itself stays unchanged: its refusal is
> already correctly classified, and making it *actionable* is rendering.

> **Built 2026-09-23 (slice 2c), and where it departs from the page above.**
>
> - **The registration form is one name field + date of birth only** (maintainer decision). No
>   identifier entry: it needs a system picker this page puts out of scope
>   ([#672](https://github.com/cairn-ehr/cairn-ehr/issues/672)).
> - **The raw typed name travels WITH its token** (`cairn_gui_funnel::FunnelSession`). `register`
>   takes only the token; the name it registers is the one the attested search ran on, so the
>   Rust side cannot pass a second name. Searches carry the webview's edit revision, and one for
>   an older revision is dropped instead of replacing a newer search.
> - **The provisioning check is the window's, not the port's.** `LiveData::require_provisioned`
>   runs before a registration takes its attestation (#665). `PatientRegistration::register` is
>   unchanged, so the port suites still reach db/005's refusal inside the transaction.
> - **The identity header shows AGE, not date of birth, for a picked chart**, because `Candidate`
>   carries no DOB ([#673](https://github.com/cairn-ehr/cairn-ehr/issues/673)); a registered chart
>   shows the typed DOB, and a `--patient` launch says its name was not read.
> - **Only a candidate that some list on screen showed can be opened**, so the webview cannot open
>   an arbitrary id.
> - **The prompt read guard is SOFT POLICY** (maintainer decision, 2026-09-23,
>   [#677](https://github.com/cairn-ehr/cairn-ehr/issues/677)). For 800 ms after a step-3 result
>   lands, a click on Register counts as "show me", not "register", so a registration does not swear
>   to rows that appeared under the pointer. It lives in `funnel.js` only: it is ergonomics in the
>   ADR-0021 sense, and another front-end may choose a different interval or none. It is not part of
>   what the attestation asserts, so the floor and `FunnelSession` do not enforce it.

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

> **Revised 2026-09-22 (slice 2b).** Three things the live ports settled, none of which this
> section could have known before there was a floor to fail against.
>
> - **A failure is now three facts, not two.** `DataError` gained `Refused` (#648): the in-DB
>   floor deciding against a call is not an outage, and offering a retry on a verdict is a
>   precise untruth on a wrong-chart-prevention surface (principle 4). The discriminator is the
>   SQLSTATE — a bare `RAISE EXCEPTION` is `P0001`, which `db/001_envelope.sql` states is a
>   contract rather than an accident.
> - **The refusal does NOT change what the caller does with the attestation**, though `port.rs`
>   predicted it would. `restore` and `commit` are the two mandatory ends of every `take`;
>   `commit` after a refusal would be a lie, and the clerk's next act — editing the form —
>   `discard`s the doomed search on a new generation anyway. **Both arms restore; only the
>   sentence on screen differs.** 2c writes those two sentences.
> - ***"`register_patient` validates the DOB shape before ticking any HLC"* is true and has a
>   consequence this page did not draw.** That validation happens in **Rust**, so its refusal
>   carries no SQLSTATE at all and is today indistinguishable from a dropped connection —
>   deterministic, verdict-shaped, and reported as an outage.
>   [#651](https://github.com/cairn-ehr/cairn-ehr/issues/651) has the argument; a test pins the
>   wrong behaviour so it is visible in every run rather than only in the issue.

> **Resolved 2026-09-23 (slice 2c prerequisites, PR #661).** A Rust-side pre-flight refusal is now
> a `cairn_node::db_diagnosis::DeliberateRefusal` and reaches the clerk as `Refused` (#651);
> `data_error_from` asks two complementary questions — the SQLSTATE *and* the marker — and the test
> that pinned the wrong behaviour expects the right one. **What is still wrong is the other half
> of the rule (#655):** a constraint violation, a privilege refusal (`42501`) and a never-loaded
> schema (`42P01`) are floor *decisions* carrying their own SQLSTATE, and they still land in
> `Unavailable`. Adding a second discriminator did not make the first one right.
>
> Two more things the same PR settled, which this section owes 2c:
>
> - **`--mock` can fail now** (`MockData::fail_next`, #660), one shot at a time, so the two
>   sentences 2c writes are testable in the mode the accessibility and timing passes run in — not
>   only in a DB-gated suite that renders nothing.
> - **`TokenStore::settle` is how a handler should end a `take`** (#659). Writing
>   `.map_err(|(e, _)| e)?` instead drops the attestation and latches the store shut, and
>   `discard` — the clerk's own recovery gesture — does not clear it.

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

- **The step-3 trigger is a heuristic.** Requiring two name tokens + DOB is a guess at "enough to
  identify a person". A mononymous patient, or one whose DOB is genuinely unknown, never trips it —
  and registration must still be possible for them (principle 4: *unknown* is a first-class value).
  The trigger therefore cannot be the only path to the prompt; registering without a completed
  trigger must run the search anyway, on whatever is there. The token-minting search is the
  **commit-time** one for exactly this reason: the trigger only decides whether it also runs
  *early*, never whether it runs at all.
- **The bounded prompt assumes few candidates.** True for full name plus DOB, but not guaranteed.
  If it is routinely incomplete, the cap is wrong and the design needs revisiting rather than
  quietly signing partial lists.

> **Measured 2026-09-23 (slice 2c): it is routinely incomplete, and the assumption was false for a
> structural reason.** `db/046` is a DISJUNCTION (any name token OR the exact DOB), so a
> full-name-plus-DOB search returned a median of 103.5 candidates over 50,000 real names, and
> `search_patients` ordered them by chart age: the prompt showed the five OLDEST charts, and an
> existing duplicate was among them in only 20% of registrations. Two responses:
>
> - **Built in 2c (maintainer decision):** `search_patients` ranks by passes matched, then chart
>   age. An EXACTLY-typed duplicate was first in 500 of 500 searches. Order only; the set, the
>   wire and the attestation shape are unchanged. **A duplicate typed with a wrong date of birth
>   is still shown only 20% of the time, ranked or not**: it matches only the name pass, which
>   counts once however many name tokens matched.
> - **Not built, filed as [#671](https://github.com/cairn-ehr/cairn-ehr/issues/671):** the prompt
>   still truncates on 92% of registrations, so the signed `incomplete` flag carries almost no
>   signal. That is this bullet's revisit condition, and it needs an ADR-level decision because
>   it touches a signed body. No search had more than five candidates matching two or more
>   passes, but withholding single-pass candidates would withhold exactly the wrong-DOB
>   duplicate. Evidence:
>   `cairn-gui/cairn-gui-tauri/results/2026-09-23-funnel-prompt-truncation.md`.

> **Decided 2026-09-26 ([ADR-0075](../../spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md),
> #671): this bullet's revisit condition is RETIRED.** The prompt is a best-effort nudge and
> truncation is its normal state; duplicates are expected and repaired by `link`. `incomplete`
> means only that the search was partial (ADR-0061's meaning); truncation is shown on screen, not
> signed. Ranking gains name-tokens-matched and a DOB near-miss key. Design:
> `2026-09-26-step3-prompt-is-a-nudge-671-design.md`.
