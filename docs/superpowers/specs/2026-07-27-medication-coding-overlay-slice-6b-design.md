# Design — `clinical.medication` slice 6b: the coding-overlay event types

- **Date:** 2026-07-27
- **Implements:** [ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md)
  decision 3 — the second and final half of the code slice that ADR unblocks.
- **Builds on:** slice 6a (branch `feat/medication-coding-slice-6a-0059`, PR #297) — the inline
  `substance.coding` shape, the `medication_coding_system` registry, the two-tier floor
  `cairn_check_medication_coding`, and the `medication_coding` projection table.
- **Status:** design agreed; implementation plan to follow.
- **Scope:** working scaffolding, not canonical. The spec (`docs/spec/`) and the ADR log win on any
  disagreement.

## Why this slice exists

ADR-0059 decision 3 makes coding **a separable, separately-authored act**: it may ride inline on the
assertion (slice 6a) *or* arrive later as its own event, authored by whoever codes it — a pharmacist or
a professional coder, as a distinct contributor whose coding claim never overwrites the clinician's
clinical claim. Slice 6a shipped only the inline half. Today a medication asserted uncoded can never
become coded, and a miscoding can never be repaired.

This slice adds the two overlay event types, and the surface that routes uncoded medications to whoever
codes them.

## Scope

**In:**

1. `clinical.medication-coding.asserted` — code a thread that was not coded inline.
2. `clinical.medication-coding-correction.asserted` — replace a coding claim, or **strike** it back to
   honest not-yet-coded.
3. The floor for both (new `db/042_medication_coding_overlay.sql`), reusing 6a's
   `cairn_check_medication_coding` for the triple.
4. Both apply fns writing the existing `medication_coding` table under the existing winner rule, with
   the anchor columns made nullable and a `struck` flag added.
5. `patient_medication_uncoded` — the coder worklist ADR-0059 decision 3 names.
6. The CLI surface and the authorship path that makes a pharmacist's coding *theirs*.

**Out (with the reason each is out):**

| Deferred | Why |
|---|---|
| drugref lookup / type-ahead / DDI | §9 advisory tier; the cross-service connection model is its own design decision. Slice 6a's source guard keeps the trusted surface drugref-free, and this slice must not break it. |
| The §5.9 safety class | Owed by the safety-projection slice ([#294](https://github.com/cairn-ehr/cairn-ehr/issues/294), blocked on #232). |
| coded↔uncoded duplicate detection | Needs term→anchor resolution — the drug-matcher slice. ADR-0059 decision 5 is explicit that the key does not close it. |
| The coding UI (and its §1.2 time budget) | The med-list UI slice, [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) neighbourhood. |

## 1. The two event types

```
clinical.medication-coding.asserted
  { medication_id, coding: { system, code, display } }

clinical.medication-coding-correction.asserted
  { medication_id, corrects: <uuid>, coding: { … }  |  strike: true,  note? }
```

**`corrects` is required and its existence is deliberately unchecked.** It names the event whose coding
claim is being fixed — a prior coding overlay, or the assertion itself when the coding was inline.
Refusing an unknown target would break offline-first: the corrected event may replicate later, or never.
This mirrors `clinical.medication-dose-correction.asserted`, whose floor validates `corrects` as a uuid
and says so in as many words (`db/032`).

**Exactly one of `coding` / `strike` must be present.** The floor refuses both-present and
neither-present. Two shape choices worth recording:

- `strike: true` is a **boolean**, diverging from ADR-0050's `strike: ["dose","effective"]` array. That
  array exists because a dose correction patches three independent groups; a coding claim is one
  indivisible triple. A single-element array would be grammar-cosplay.
- The strike is **explicit**, never inferred from an absent `coding`. A caller who forgets the coding
  gets a refusal, not a silent un-coding of a medication.

**Why the strike exists at all** (the decision this slice turns on): a reviewer who establishes that a
medication is *not* metformin, but cannot say what it is, has exactly two alternatives without it —
leave a known-wrong anchor standing, or invent a substitute identity they cannot vouch for. The second
is the fabrication principle 4 forbids; the first keeps feeding a wrong anchor to the dup-key and the
group display. Append-only means the correction event is the only repair path, so it must be able to
express *"not that, and I don't know."*

**Twins** (mechanically derived, non-empty, deterministic):

```
coded as atorvastatin [drugref-moiety]
coding corrected to atorvastatin [drugref-moiety] — brand name was ambiguous
coding struck — no longer coded — not metformin; substance unidentified
```

## 2. The floor — new `db/042_medication_coding_overlay.sql`

Both types register in `event_type_class` as `('additive', FALSE)`. This follows
`clinical.medication-dose-correction.asserted`, which is registered exactly so: a correction **adds** a
claim rather than suppressing another author's, the original stays in the log, and the projection picks
a winner. The classification matters — `targets_other_author = TRUE` would route these through the
ADR-0043 suppression owner-gate, which would refuse a pharmacist correcting a coding authored by someone
else. That refusal would contradict ADR-0059 decision 3's whole premise.

A shared `cairn_check_medication_coding_overlay(p_type, b)` validates `medication_id` as a uuid (both
types), `corrects` as a uuid and the coding/strike exclusivity (correction only), then delegates the
triple to 6a's `cairn_check_medication_coding(p)` — so the structural-vs-registry two-tier split, the
canonical-uuid pin and the strict/lenient door behaviour are **inherited, not restated**. The correction
carries an optional free-text `note`.

Registered in `cairn_event_twin_check` with a hard twin requirement, like every other clinical type.
`SCHEMA_GENERATION` 41 → 42, with the entry added to cairn-node's `SCHEMA` list.

## 3. The projection — additive, exactly as 6a's shape promised

Both apply fns write the **existing** `medication_coding` table under the **existing**
`cairn_hlc_overlay_wins` rule. No view is re-routed and no view's column set changes (one view body
changes — see the predicate note below). This is the payoff of
6a's decision to give the coding its own table rather than columns on `medication_statement`: the
overlays add rows, and every downstream consumer — the widened read views, the `(system, code)` dup-key,
the prefer-coded group display, the anchor-conflict view — keeps working untouched.

The strike needs two schema changes, both additive:

```sql
ALTER TABLE medication_coding ALTER COLUMN coding_system  DROP NOT NULL;   -- ×3
ALTER TABLE medication_coding ADD COLUMN IF NOT EXISTS struck BOOLEAN NOT NULL DEFAULT FALSE;
```

Dropping `NOT NULL` is a widening (an existing row still satisfies the looser constraint), and the
`ADD COLUMN IF NOT EXISTS` is the paired-ALTER pattern #207 requires so an upgraded-in-place database
gets the column. A strike writes NULLs into the triple plus `struck = true`.

**That shape makes the degradation automatic.** The dup-key is
`coalesce('code:'||system||'|'||code, 'term:'||normalized-term)`, so a NULL anchor falls to the term
branch on its own; `medication_group_display`'s prefer-coded ordering keys on
`(mc.medication_id IS NOT NULL)`, which must therefore become a check on the *anchor* rather than the
row's existence — the one downstream edit this slice makes, and its test.

Both fns carry two things forward from 6a's ledger: the `jsonb_typeof(...) IS DISTINCT FROM 'null'`
guard (the floor treats an explicit JSON null as absent, so the projection must too), and a
`cairn_guard_medication_patient` call, so a coding event cannot silently re-home a thread onto another
chart (issue #192's hazard, which applies to every per-thread verb).

**Registry bookkeeping, in both places each** — the two-place pattern from #212, and the specific trap
recorded in this project's notes:

| Registry | Rust pin | SQL mirror |
|---|---|---|
| `cairn_event_twin_check` | 19 → 21 (`crates/cairn-node/tests/twin_registry.rs`) | `db/tests/034_twin_registry_test.sql` |
| `cairn_projection_apply` | 22 → 24 (`crates/cairn-node/tests/projection_registry.rs`) | 25 → 27 (`db/tests/039_projection_registry_test.sql`) |

Registry rows use the `ON CONFLICT … DO UPDATE … WHERE (…) IS DISTINCT FROM (…)` converging arm (#214),
including for `event_type_class` where the older sibling files still use `DO NOTHING` — this is the
direction [#254](https://github.com/cairn-ehr/cairn-ehr/issues/254) asks for, and a new file should not
add to that debt.

**One consequence to record:** `medication_coding` becomes a multi-type table, so `cairn_reproject` will
now refuse a narrow single-type prefix rebuild over it (db/039 already enforces that rebuild-scope must
not silently truncate another type's rows). Expected, and commented at the registration site.

## 4. The coder worklist

A new view `patient_medication_uncoded`, created only in `db/042` (so it never enters the multi-file
replay-coherence problem), with columns `patient_id, medication_id, term, previously_struck,
asserted_at`. It selects active (non-ceased) threads with no effective anchor — either no
`medication_coding` row at all, or a row whose anchor is NULL — with
`previously_struck = (a coding row exists AND struck)` distinguishing the two.

The distinction is clinically real, not bookkeeping: *"nobody has coded this yet"* invites a coder to
code it, whereas *"a reviewer established this is NOT what it was coded as"* is a warning against
re-coding from the same weak evidence that produced the error. Both must appear — a struck coding is
genuinely uncoded and must not vanish from the queue — but a coder needs to see which is which.

## 5. Node surface

Two CLI subcommands, both reusing 6a's `coding_from_parts` and the existing
`seal_sign_submit` / `--author-as` path so a pharmacist's coding is authored as **theirs**
(ADR-0053), not as the node's:

```
cairn-node medication-code <medication_id> --coding-system … --coding-code … --coding-display …
cairn-node medication-code-correct <medication_id> --corrects <event_id>
        [ --coding-system … --coding-code … --coding-display … | --strike ] [--note …]
```

The orchestrator refuses a correction that supplies neither a complete coding nor `--strike`, and one
that supplies both — the same all-or-nothing discipline `coding_from_parts` already applies, failing at
the source with a message naming the gap rather than at the DB floor.

## 6. Test obligations (RED first)

**Floor** — a valid coding overlay is accepted; a correction with both `coding` and `strike` is refused;
with neither is refused; a non-uuid `corrects` is refused; an unknown `corrects` is **accepted**
(offline-first); the inherited triple checks still fire (unregistered system refused at submit, admitted
at apply; non-canonical uuid refused at submit).

**Projection** — an overlay codes a previously-uncoded thread; a correction replaces the triple; a strike
NULLs the anchor and sets `struck`; a lower-HLC overlay arriving after a higher-HLC one does not win;
an overlay for a thread that does not exist locally still lands (arrival-order independence); a coding
event naming a different patient than the thread's standing chart is refused locally and flagged on
apply.

**Downstream, unchanged-by-design** — after a strike, the dup-key returns to the term branch, the
prefer-coded group display stops preferring that member, and the anchor-conflict view clears. These are
the tests that prove 6a's table-not-columns decision actually paid off.

**Worklist** — a never-coded active thread appears with `previously_struck = false`; a struck thread
appears with `true`; a coded thread does not appear; a ceased thread does not appear.

**Cross-node** — a coding overlay and a strike both converge on a second node.

**Structural** — the drugref source guard still passes (no drugref reference enters the tree), and both
registry row counts are updated in both places.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a pharmacist writing *"= atorvastatin"* beside *"Lipitor"* on a paper
  medication list — or striking that annotation through when it turns out to be wrong. **N = 1** human
  act in each direction.
- **Steps:** paper **N = 1** → architecture-forced **M = 1** (one event per act; coding and striking are
  each a single write) → UI bundling target **K = 1**. Coding remains optional at every layer, so no
  medication workflow gains a forced step: a clinician who never codes anything is unaffected.
- **Time + cognitive load:** no budget measured here — this slice exposes a CLI test/ops surface, not the
  clinician surface a budget would measure. Owed by the coding-UI slice (#288 neighbourhood), whose
  target is that accepting a suggested coding costs zero keystrokes over not coding at all.

## Risks

- **The `medication_group_display` prefer-coded predicate** currently tests row existence, not anchor
  presence. A struck coding leaves a row with a NULL anchor, so without the edit in §3 a struck member
  would still be preferred as the group's display — showing a group under a coding that was explicitly
  retracted. This is the one place a 6b change reaches back into 6a's SQL, and it needs its own test.
- **Registry counts drift silently** if either of the two places is missed; both are pinned by tests, and
  CI runs the SQL mirrors since PR #251.
- **Stacked branch:** 6b builds on 6a, which is unmerged (PR #297). The PR targets 6a's branch until that
  merges, then retargets to `main`.
