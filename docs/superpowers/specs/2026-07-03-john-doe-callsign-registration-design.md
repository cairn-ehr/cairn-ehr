# §5.4 — John Doe registration (slice A): callsign minting + matcher placeholder exclusion

**Date:** 2026-07-03 · **Spec home:** §5.4 (unidentified registration — John Doe) / §5.3 (registration
classes) / §5.2 (matching pipeline — placeholder exclusion) · **Principle:** 2 (identity is a claim — a
callsign is an honest "we don't know who this is", never a plausible fake name), 3 (paper-parity — an
unconscious arrival gets a chart *now*, exactly like a paper ED folder labelled "Unknown male, bay 3"),
4 (acknowledged uncertainty — the chart renders *unconfirmed*, "no history available") · **Blast radius:**
mixed — the callsign generator + registration compose are **fit-for-purpose** (a bad callsign is cosmetic;
the UUID is the real identity), but the **matcher placeholder exclusion is safety-relevant** (a callsign
that leaks into the matcher feature space could false-merge two different John Does — the dangerous
direction, §5.2's "false merge ≫ worse than false split"). The exclusion lives in the advisory matcher,
which is the correct layer (§5.2/§5.13), and is defended by the fact that blocking excludes the callsign
token so the bad pair is never even generated.

## Why this slice

C4 (db/024) built the *unconfirmed* trust state and the `identity.pending.asserted` / `identity.identify.
asserted` events — the projection-side "John Doe" contract. But C4 explicitly **did not** deliver the §5.4
**registration front door**: the thing a clinician actually invokes when an unconscious, unknown patient
arrives. §5.4 names four moving parts:

> - UUID minted immediately; care proceeds without delay.
> - **System-generated callsign** (e.g. `Unknown-ED-<site>-<date>-A`), never plausible fake names;
>   matcher excludes placeholder names from its feature space.
> - Identity evidence captured as **clinician-observed assertions** (estimated age, observed sex, photo,
>   marks, belongings, EMS context).
> - **Identity-pending is an active workflow state**; matcher re-runs on every new evidence assertion.

This slice delivers the **first two** (UUID + callsign, and the matcher exclusion that makes the callsign
safe), composing them onto C4's already-built pending marker. The evidence-assertion subsystem and the
"prior history now available" push-alert are deferred (see below) — they are larger and this is the
foundational piece they all sit on.

## What this slice delivers

- **Pure callsign generator** in `cairn-event` (`john_doe::callsign`): `Unknown-<class>-<site>-<date>-<suffix>`,
  a culture-neutral, deterministic, obviously-not-a-real-name string. Pure (no clock, no randomness):
  the caller supplies `class`, `site`, `date`, and a `suffix` so it is fully testable. Sanitizes each part
  to a safe token set so the callsign is a single legible field with no delimiter collisions.
- **`register-john-doe` compose** in `cairn-node` (`john_doe.rs` + a CLI subcommand): mints a fresh
  patient UUID, then authors **two events** through the existing 1-arg `submit_event` door —
  1. a **callsign name assertion** (`demographic.field.asserted`, `field="name"`, `facets.use="callsign"`),
     reusing `cairn-event::name_assertion_body` + `render_name_twin`; and
  2. the **C4 `identity.pending.asserted`** marking the chart *unconfirmed*, reusing
     `cairn-event::pending_assertion_body` + `render_pending_twin`.
  No new event types, **no new `db/` migration, no floor change, no SCHEMA/ADR/spec bump** — this composes
  settled, already-built primitives.
- **Matcher placeholder exclusion** (advisory, `matcher/pipeline/db.py`): both the blocking `name_tokens`
  CTE and the `load_candidate` scoring name query exclude names whose `use_key` is in a small reserved
  **placeholder set** (`{'callsign'}` today). A callsign therefore contributes **zero** matcher features —
  it never blocks, never scores — so two John Does registered at the same site on the same day never
  false-match on their shared callsign tokens (`unknown`, the site, the date).

## The load-bearing design calls

### 1. The callsign is a real name in `patient_name`, excluded only from *matching* — not withheld

The callsign is the chart header for an unidentified patient. db/012's `patient_name_current` already has
the fallback: *"When no legal name exists, the newest name of ANY use wins (the unidentified-patient
fallback)."* So the callsign **must** live in `patient_name` as an ordinary name — that is how the chart
renders "Unknown-ED-…-A" in the banner. It is not withheld; it is stored and displayed.

The §5.4 requirement — "matcher excludes placeholder names from its feature space" — is therefore a
**query-time exclusion in the advisory matcher**, not a floor rule and not a decision to not store the name.
This is the correct layering (principle 12): the floor stores every asserted name verbatim; the matcher
(fit-for-purpose Python + read-only SQL, §5.2/§5.13) owns what enters its feature space. The hard-veto
floor (db/016) still applies to any *real* demographic clash on a John Doe chart; only the callsign name is
feature-excluded.

### 2. Why `facets.use = "callsign"` (a reserved, system-set use token)

A name's placeholder-ness has to travel with the name so any node's matcher excludes it identically. The
natural carrier is the name's existing `use` facet (db/012 folds it to `use_key`). `callsign` is a
**system-generated, culture-neutral** reserved token — it is not a human name-use vocabulary term (unlike
`legal`/`maiden`/`nickname`), so it introduces **no cultural capture**: the system sets it, every node
recognises it, and it never competes with a real name's use.

The matcher exclusion keys on a **reserved placeholder set** (currently `{'callsign'}`), not the single
literal, so a future placeholder kind (e.g. a human-tagged `placeholder`) joins by adding one member — the
same "additive, never rewrite" discipline the rest of the identity code follows.

### 3. The suffix is partition-safe, therefore UUID-derived — *not* a coordinated A/B/C counter

§5.4's example callsign ends in `-A`, suggesting a per-site-per-day sequence (A, then B, then C…). A
readable sequence is tempting, but computing it requires asking "how many John Does have been registered at
this site today?" — a **global-coordination read that is not partition-safe**. §5.4 is explicit that John
Doe registration is *"local, partition-safe by construction."* Two offline nodes at the same site would
both compute `-A` and mint a colliding callsign.

Callsigns are excluded from *matching* and the UUID is the real identity, so a duplicate callsign string is
never a false-merge issue — but it is **not merely cosmetic**: two live unidentified charts with an
IDENTICAL worklist header is a wrong-chart hazard (paper-parity, principle 3 — paper "Unknown male bay 3"
folders are physically distinct), so the collision must be made **negligible**, not tolerated. The suffix is
derived from the minted UUID (the last **8 hex characters** of its simple form = 32 bits of entropy;
`SUFFIX_HEX_LEN` in `cairn-node::john_doe`): globally unique with **zero coordination**, partition-safe by
construction, and colliding only ~1 in 4.3 billion per same-site/same-day pair. (An earlier draft used 4 hex
/ 16 bits — ~1 in 65 536, enough to flake the coexistence test and, worse, to occasionally print two
identical bedside headers; widened during review.) The readable sequential-suffix option is recorded as
deferred (it needs a per-day count query and a partition story).

### 4. Registration class is C4's pending marker — no new "unidentified" flag

§5.3 lists an **Unidentified** registration class. That class *is* the identity-pending state: a chart in
`chart_identity_state.state = 'pending'` renders *unconfirmed* and sits on the "still-John-Doe" worklist
(db/024 already indexes exactly that partial set). So registering a John Doe = emitting C4's
`identity.pending.asserted`; there is no separate registration-class column to add. This keeps the slice a
pure compose of built primitives.

### 5. Advisory exclusion is enough because it also removes the pair from *blocking*

Defense-in-depth check: could a callsign reach the C2b auto-link path and cause a false merge before the
exclusion bites? No. The exclusion is applied in **both** the blocking `name_tokens` CTE (candidate-pair
generation) **and** `load_candidate` (scoring). With callsign tokens removed from blocking, the John-Doe ×
John-Doe pair on a shared callsign token is **never generated**, so it is never scored and never banded
`auto_candidate`. If the two charts later share a *real* feature (an estimated DOB, an identifier), they
block/score on that — which is correct, they might genuinely be the same person. The exclusion is precise:
it removes only the placeholder signal, never a real one.

## Explicitly deferred (recorded, not lost)

- **Clinician-observed evidence assertions** (§5.4): estimated age with basis, observed sex, photo,
  distinguishing marks, belongings, EMS pickup context. Age and observed sex map onto existing demographic
  fields (dob-with-precision/basis, sex) and could be a fast follow; photo/marks/belongings/EMS context
  have **no field home yet** and need their own design (a generic clinician-observation field, or per-kind
  fields). Larger; separate slice.
- **The "prior history now available — N allergies, M active medications" push alert on link** (§5.4/§5.12)
  — a notification-surface concern; there is no notification tier to hang it on yet.
- **Registration-class partitioning of the search-before-create funnel** (§5.3/§5.8) — the enforced
  "new patient unreachable until local matching ran" funnel is a UI/API-tier workflow, above this slice.
- **`identify` wiring into a resolution flow** — C4 built the `identify` event; wiring "John Doe → identify
  → optional link to a prior chart" into one operator flow is a later compose (this slice registers; it
  does not resolve).
- **Readable sequential callsign suffix** (`-A`/`-B`/…) — needs a partition-safe per-site-per-day count;
  deferred behind the UUID-derived suffix (see design call 3).
- **A human-tagged `placeholder` name use** beyond the system callsign — the matcher exclusion is written
  as a reserved *set* so this joins by adding one member, but no UI/authoring path is built for it here.
- **A mechanical guard coupling the two placeholder-use constants** — `cairn-event::john_doe::CALLSIGN_USE`
  (Rust, the emitter) and `matcher…pipeline.db.PLACEHOLDER_NAME_USES` (Python, the excluder) are a
  hand-maintained mirror. Note the drift direction is **not recall-safe**: if a use Rust emits as a
  placeholder is *missing* from the Python set, those callsign names UNDER-exclude, re-enter the feature
  space, and two same-site/same-day John Does can block+score+auto-band into a **false merge** (the
  dangerous direction, §5.2). So any addition on the Rust side MUST be mirrored in Python; a cross-language
  test asserting set-equality is the intended guard (deferred, documented on both sides).

## Files

| Concern | File | Change |
|---|---|---|
| Pure callsign generator | `crates/cairn-event/src/john_doe.rs` (new) + `lib.rs` (mod) | `callsign(class, site, date, suffix)` + part sanitizer; unit tests |
| Registration compose | `crates/cairn-node/src/john_doe.rs` (new) + `lib.rs` + `main.rs` | build+sign+submit callsign name + pending; `register-john-doe` CLI; DB-gated integration tests |
| Matcher exclusion | `matcher/src/cairn_matcher/pipeline/db.py` | reserved placeholder-use set; exclude in `_GROUPS_SQL` `name_tokens` CTE + `load_candidate` name query; DB-gated tests |

No `db/` migration, no SCHEMA bump, no ADR, no spec edit — implements settled §5.4/§5.3/§5.2.

## Test plan (TDD, red-first)

**cairn-event (pure):** callsign has the `Unknown-` prefix; carries class/site/date/suffix in order;
sanitizes whitespace/delimiters in each part; is deterministic; distinct suffixes → distinct callsigns.

**cairn-node (DB-gated, `crates/cairn-node/tests/john_doe.rs`):** `register-john-doe` (a) creates a chart
that renders *unconfirmed* on `chart_trust`; (b) puts the callsign into `patient_name` / `patient_name_current`
with `use_key='callsign'`; (c) the callsign twin and pending twin are both stored (floor accepts); (d) is
idempotent-safe to re-run with a fresh UUID (two John Does coexist).

**matcher (DB-gated, `matcher/tests/test_john_doe_exclusion.py`):** (a) two charts sharing only a callsign
token generate **no** candidate pair (blocking exclusion); (b) `load_candidate` on a John-Doe chart returns
**no** name feature from the callsign; (c) a *real* name on the same chart is still blocked/scored normally
(the exclusion is placeholder-only, not name-wide).
