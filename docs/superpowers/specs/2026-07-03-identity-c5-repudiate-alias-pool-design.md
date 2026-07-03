# C5 — `repudiate` + the known-alias pool (the first *suppressing* identity event)

**Date:** 2026-07-03 · **Spec home:** §5.5(a) (fabricated persona → repudiation + alias pool) / §5.7
(identity event algebra) · **Principle:** 1 (append-only — the false assertion is never erased, only
struck from display), 2 (never merge/erase, always overlay), 4 (acknowledged uncertainty — a known-false
name is a *precise untruth*; suppressing it and showing nothing beats displaying a lie) ·
**Blast radius:** safety-critical (in-DB / Rust) — a defect could either leak a known-false name back into
the display projection (a lie in the chart header) or, worse, silently drop a *true* name from display.

## Why this slice

C1–C4 built the §5.7 identity core and its trust-state contract (confirmed / unconfirmed / under-review).
Every one of those events was **additive or annotative**: link/unlink adds a graph edge, dispute/identify
annotate the trust state. None removes anything from a projection. `repudiate` is different — it is the
**first suppressing identity event**, and building it is how the *suppressing* discipline (already carried
by `salience.downgrade` / `visibility.suppress` in db/005) first meets the demographic display surface.

The §5.5(a) case: a patient presents under a **deliberately fabricated persona** (a false name). Later —
confession, forensic match, a document surfacing — the fabrication is established. Paper handling is a
**strike-through**: the false name stays on the record (the *fact of presentation* under it is
medico-legally required — it is how the chart was labelled during that care), but it is struck from the
active header, and the registry keeps it as a **known alias** so that if the same persona returns the
staff recognise it. Cairn's mechanism is identical, expressed in overlay events:

> `repudiate` | Known-false assertions → alias pool | **Human** *(§5.7 algebra)*

> confession → link assertion to real chart + **repudiation events** marking false assertions. Repudiated
> values leave the displayed projection but enter a **known-alias pool** retained by the matcher (aliases
> are reused). The fact of presentation under a false name is preserved (medico-legally required). *(§5.5(a))*

## What this slice delivers

- **One additive-DDL, *suppressing*-mode event type** `identity.repudiate.asserted` through the reused
  `submit_event` door (db/005). Its payload names the chart (`subject`), the exact known-false name
  `value`, and a required-non-empty `reason` (why it is known false — §4.1 value-open).
- A **structural floor** `cairn_check_repudiation_assertion` (culture-neutral: valid subject uuid, a
  non-empty `value`, a non-empty `reason`) + a **HARD-required legibility twin** (identity events are
  legible-critical, like link/dispute/identify).
- A **`name_repudiation` standing overlay** keyed by `(subject, value)`, HLC-latest-wins (idempotent
  re-assert; a future reversal composes in by HLC without a rewrite).
- **`patient_name_current` reworked to anti-join the overlay** — a repudiated name leaves the display
  winner. `patient_name` (db/012's retained set) is **left physically untouched**: the struck name stays
  in the set as evidence and for a future chart-history view (§5.5 "visible in its chart-history view").
- **`patient_alias_pool`** — a new VIEW exposing the repudiated `(patient_id, value)` names to the §5.2
  matcher as reusable known aliases.
- Pure `cairn-event` builder (`RepudiationAssertion` + `repudiation_assertion_body` + `render_repudiate_twin`).

## The load-bearing design calls

### 1. Suppressing mode is how §5.7's "Human" becomes a *floor*, not a policy request

`repudiate` is registered `mode='suppressing'`. The db/005 attestation gate (step 4) already forces a
**valid attestation token from an enrolled human** on any suppressing event — so *every* repudiation
structurally requires a responsibility-bearing human to vouch, with **no floor special-case**. This is the
deliberate contrast with C1/C3/C4: those were `additive`, so their "human vouches" requirement only bit
when a responsibility-bearing contributor was named (workflow-tier policy). Repudiation *removes clinical
display content*; §5.7 marks it **Human**; suppressing-mode makes that unbypassable in the database
(principle 12 — the floor holds even against a client talking raw SQL). No new gate code — it reuses the
one that already guards `salience.downgrade`.

### 2. Suppression lives in the projection, never in the event or the retained set (digital strike-through)

The name assertion event stays in `event_log` forever (principle 1). The retained-set row in `patient_name`
(db/012) is **not deleted and not flagged in place** — this slice does not touch db/012's table or trigger.
Instead `name_repudiation` is a *separate composed overlay*, and `patient_name_current` is
`CREATE OR REPLACE`d to **anti-join** it. This mirrors C3's review-driven choice (compose
`person_chart_trust` on top of `person_chart` rather than mutate it): the earlier migration stays
droppable-free and the retained set stays intact for chart-history. The struck name is *excluded from the
winner*, never erased.

### 3. Value-grained, not use-grained and not event-grained — deliberately

The overlay keys on `(subject, value)` — the raw name string — **not** `(subject, use_key, value)` and
**not** a `target_event_id`:

- **Why not use-grained.** A fabricated name is false *however it was labelled*; "John Smith" recorded as
  `legal` and again as `alias` is one false value, not two independent falsehoods. Value-grained also
  **eliminates a real drift hazard**: matching a stored `use_key` would force this slice to replicate
  db/012's exact `use` fold (`lower(… COLLATE "C")`, blank→`unspecified`) and stay bug-for-bug identical
  forever, or silently fail to suppress. Keying on the raw opaque `value` (which db/012 stores verbatim as
  `p ->> 'value'`, and this slice stores verbatim too) makes the anti-join a plain exact-string equality
  with **no fold on either side** — nothing to drift.
- **Why not event-grained.** §5.5(a) says "repudiated **values**"; the matcher alias pool is a set of name
  *strings*; and a value re-asserted by several events is one member in the retained set. A `target_event_id`
  would be the wrong grain (it would strike one assertion of a value while others survive).
- **Accepted degenerate case.** Because the strike is per `(subject, value)`, if one chart ever held the
  *identical byte string* as both a legitimate name and a separately-fabricated one, repudiating it strikes
  both from display. This is a contradiction in the data (the same exact string cannot be both this person's
  true name and their fabrication), so treating it as one false value is the honest reading — and the strike
  is auditable and reversible-by-overlay, never a data loss.
- **Honest limit (documented, not a bug).** The match is **exact string equality** on an opaque value —
  culture-neutral and deterministic (the only convergent choice for arbitrary scripts). A repudiation must
  therefore name the *exact* value it strikes; in practice the repudiation UI pre-fills it from the chart's
  name list. Fuzzy/normalised recognition of a returning alias is the **advisory matcher's** job (it reads
  `patient_alias_pool`), never the suppression floor's — the floor must be precise or it risks striking the
  wrong (possibly true) name.

### 4. Striking a chart's only name yields *no* display name — and that is correct

db/012's winner has a paper-parity fallback: with no legal name, the newest name of any use wins, so "the
header always shows something." If a chart's **only** recorded name is the repudiated one, the anti-join
leaves `patient_name_current` with **no row** for that chart. That is the honest outcome: the name is now
genuinely unknown, and showing the known-false one would be a *precise untruth* (principle 4). "Show
something" is then satisfied one layer up by the §5.4 callsign / *unconfirmed* rendering (C4) — not by
lying in the header. This is out of this slice's scope but is exactly why C4's unconfirmed state exists;
the slices compose.

## Deliverable boundary (what this slice does NOT do)

- No **reversal / de-repudiation** event (a repudiation made in error). The overlay is HLC-versioned so a
  future reversal composes in by HLC with no rewrite; not built here (the append-only correction path is a
  separate §5.5 decision). Recorded as deferred.
- No **chart-history view** rendering struck names (a UI read surface; the retained set + overlay already
  carry the data).
- No **reattribution** (§5.5 event-granular strike-through of *clinical documentation*) — that needs a
  clinical-note surface that does not yet exist (only demographics do); premature.
- No **matcher wiring** that consumes `patient_alias_pool` (the view is the seam; the §5.2 matcher reading
  it is a later matcher slice).
- No **SCHEMA / ADR / spec bump** — this implements settled §5.5/§5.7; db/010–024 are left untouched, and
  this migration `CREATE OR REPLACE`s only the shared `cairn_event_twin` hook and `patient_name_current`.

## Test plan (TDD)

**Pure (`cairn-event`):** body carries subject/value/reason and *only* those; twin renders subject + value
+ reason and is non-empty.

**DB-gated integration (`crates/cairn-node/tests/identity_repudiate.rs`):**
1. A human-attested repudiation is accepted; the struck name leaves `patient_name_current`; a surviving
   name becomes the new winner.
2. The struck name enters `patient_alias_pool` (presence only — the view is reason-free); the `reason` is
   retained in the base overlay, and `patient_alias_pool` is asserted to carry **no** `reason` column
   (the ADR-0006 confidentiality split — added in review).
3. The retained set `patient_name` still contains the struck name (evidence preserved).
4. Striking a chart's *only* name → `patient_name_current` has no row for it (honest, no lie).
5. Idempotent re-assert = one overlay row; HLC-latest-wins on the `reason`.
6. **HLC-blind anti-join pinned:** a strictly-newer re-assertion of the struck value does NOT un-strike it
   (reversal-only recourse — added in review).
7. **The §5.7 "Human" floor, both branches:** an un-attested repudiation is refused; an *agent*-attested
   repudiation is refused ("not an enrolled human actor" — added in review).
8. Floor rejections, each a distinct legible exception: empty `value`, empty `reason`, bad/missing
   `subject`, missing authored twin — and (added in review) each asserts it rejects at the *floor*, not the
   attestation gate.

*(All review additions came from a 3-agent adversarial review; the SQL-correctness agent found 0 hard bugs.
The main functional change was reason-confinement — dropping `reason` from the matcher-facing view and
withholding the base-table grant from `cairn_agent` — plus `reason NOT NULL`.)*
