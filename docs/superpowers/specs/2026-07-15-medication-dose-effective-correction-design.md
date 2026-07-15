# Design — Correct a dose event's effective-date / reason (slice 5 of the `clinical.medication` surface)

**Date:** 2026-07-15 · **Branch:** `feat/medication-dose-effective-correction` (proposed) · **Status:**
design, pending implementation plan.

**Scope of change:** additive Rust (`cairn-event::medication::dose` — extend `DoseCorrection`, its `*_body`
and twin; `cairn-node::medication::dose` — extend `CorrectDoseInput` + the `correct_dose` orchestrator;
CLI `MedicationCorrectDose` — new flags) + **one new DB migration** (`db/035_medication_dose_effective_correction.sql`:
`ALTER TABLE`-extends db/032's `medication_dose_correction`, re-defines the correction floor + apply trigger +
the two dose-timeline views) + **[ADR-0050](../../spec/decisions/)** recording the *dose-correction-as-per-field-patch*
precedent + a spec version bump (v0.50 → v0.51, `index.md` + a one-line §3.15/§3.16 note). Extends the dose
overlay opened by [slice 2](2026-07-12-medication-dose-overlay-design.md) on the surface from
[slice 1](2026-07-11-medication-recording-design.md). **No new event type, no new envelope field, no floor
bypass, no new founding principle, no SCHEMA-counter bump; db/031–034 files untouched (db/032's objects are
extended at load by the new db/035).** It graduates slice 2's
own honest gap ("the correction verb fixes the dose value only, not a mis-keyed effective date or reason") into
product code, reusing the settled dose-timeline projection, the append-only overlay primitives, and the
HLC-wins convergence helper.

---

## 1. What this is, and why now — the un-correctable date

Slice 2 gave the medication surface a **bitemporal dose timeline**: point 0 (seeded from the assert's
`started`) plus one point per `-dose-change`, and a `-dose-correction` verb that overlays a *targeted* point.
But the correction overlay (`medication_dose_correction`, db/032:133) carries only `amount`, `unit`, `reason`,
`info_source` — and both timeline views compute the effective date purely from the **original** event
(`de.effective_value`). So today:

- A **mis-keyed effective date on a `-dose-change` cannot be fixed.** "Increased to 80 mg, effective 2025-06"
  when it was actually 2024-01 is uncorrectable.
- This is not cosmetic. The **current dose is the latest-*effective* point** (db/032:224, the `DISTINCT ON`
  ordered by the effective sort-key). A wrong effective date can put the **wrong dose "current"** or scramble
  the chronological order of the titration trail. Correcting the date is the mechanism that repairs the
  *bitemporal* record, not just a display label.
- The correction's **clinical `reason` cannot be fixed** either (a `-dose-change` records *why* the dose
  changed, e.g. "titration"; a typo there is stuck).
- **Latent bug tidied along the way:** the correction table *stores* a `reason` column, but
  `patient_medication_dose_history` (db/032:239) reads `de.reason` (the original) — so the correction's reason
  is written and never surfaced. This slice repurposes and surfaces it (see §4).

**Principle 4 is the lens** ("an imprecise near-truth beats a precise untruth; uncertainty is first-class").
A clinician who fixes one field must not be forced to re-assert the others (which they may no longer vouch
for), and "I don't actually know the date — strike it" must stay first-class. That drives the two core
decisions below: **per-field patch** (§3) and an **explicit strike** for set-to-unknown.

---

## 2. Event / payload shape (all patch fields optional; reuse the existing verb)

No new event type. `clinical.medication-dose-correction.asserted` gains optional payload fields. The
`schema_version` bumps `clinical.medication-dose-correction/1 → /2` — cosmetic (nothing keys off it) but
**honest**, because the interpretation of an *omitted* field changes from "unknown" to "keep" (§3). No `/1`
correction event exists in production (pre-clinical posture).

```jsonc
{
  "medication_id": "<uuid>",             // required — the thread key (existing)
  "corrects":      "<uuid>",             // required — the target dose_event_id (existing)

  "dose":      { "amount": "20", "unit": "mg" },          // OPTIONAL — set the dose group
  "effective": { "value": "2024-01", "precision": "month" }, // OPTIONAL — set the effective group  (NEW)
  "reason":    "titration",              // OPTIONAL — set the point's clinical reason  (REPURPOSED, see §4)

  "strike":    ["dose"],                 // OPTIONAL — groups to set-unknown; subset of dose|effective|reason (NEW)
  "note":      "mis-keyed the date",     // OPTIONAL — why THIS correction was made (audit; additive)  (NEW)
  "info_source": "clinician-observed"    // OPTIONAL — provenance of the correction claim (existing)
}
```

All optional fields are **omitted when absent**, never serialized as `null` (the house idiom — an
added-later field never perturbs an existing event's content address, principle 11).

**Encoding choice (approaches considered).** (A) set-keys + a `strike` array; (B) a JSON-`null` sentinel per
field; (C) `{value, known:false}` objects. **Chosen: (A).** It is the only one that reads self-documentingly
(`strike:["effective"]` states intent), keeps the "never serialize null" convention, and renders cleanly into
the legibility twin. A group that appears **both** as a set-key and in `strike` is a contradiction the floor
rejects (§6).

---

## 3. Patch semantics — per-field, with an explicit strike

The correctable content of a dose point is three independent **patch groups**: **`dose`** (amount + unit
together — one clinical quantity), **`effective`** (value + precision), **`reason`** (the point's clinical
reason). For each group *G*:

| Payload state of *G*              | Effect on the point                     |
|-----------------------------------|-----------------------------------------|
| present as a **set-key**          | **touched** — value := the given value  |
| listed in **`strike`**            | **touched** — value := NULL (unknown)   |
| **neither**                       | **untouched** — original kept           |

The `dose` group is set as an **atomic object** (the same `{amount?, unit?}` shape as assert / `-dose-change`):
a set-key `{"amount":"20"}` with no `unit` restates the whole quantity as *20, unit-unknown* — it does **not**
keep the old unit. Per-field keep applies **between** groups (dose / effective / reason), not within the dose
quantity, which is one clinical value.

This is the per-field-patch model chosen in brainstorming. It makes the common case — *fix one field, leave
the rest* — safe: correcting a date typo never silently wipes the dose. It preserves the slice-2
"strike a false dose to unknown" capability, but makes it **explicit** (`strike:["dose"]`) instead of
implicit-by-omission. Consequences:

- A correction that **touches nothing** (no set-key, empty/absent `strike`) is a **no-op** and the floor
  rejects it. Under slice-2 semantics a bare `{corrects}` correction meant "strike dose to unknown"; that
  reshape was accepted at brainstorming and is the one observable behaviour change to the existing verb.
- The projection cannot infer "touched" from a nullable value column alone (NULL = struck *or* untouched?),
  so the overlay row records a **touched-flag per group** (§5).

---

## 4. `reason` vs `note` — repurpose the dead column

Slice 2's correction `reason` meant "*why I'm correcting*" (e.g. "mis-keyed") — stored but **never surfaced**
(dead data). This slice:

- **`reason` → the point's clinical reason** (the same field a `-dose-change` carries, e.g. "titration"),
  now patchable and surfaced in the dose history. This is what a clinician reads on the timeline.
- **`note` (new) → "why this correction was made"** — the audit rationale, always additive (not a patch
  group), rendered into the twin.

At the bedside `--reason` fixes the clinical reason a clinician sees; `--note` carries the correction's own
rationale. A guarded, idempotent backfill (§6) moves any pre-existing `reason` data into `note`
(pre-clinical, so cosmetic). This is cleaner than keeping `reason`="why" and adding a `--point-reason`, which
would be ambiguous at the point of care.

---

## 5. Projection changes (extend db/032's overlay + views; **no view widening**)

**`medication_dose_correction`** gains, via `ALTER TABLE … ADD COLUMN IF NOT EXISTS`:
`effective_value TEXT`, `effective_precision TEXT`, `note TEXT`, and three touched-flags
`dose_corrected` / `effective_corrected` / `reason_corrected`. The flags disambiguate "struck to NULL" from
"untouched" — a plain nullable value column cannot.

**`medication_current_dose`** and **`patient_medication_dose_history`** (both defined in db/032): each field
resolves as `CASE WHEN corr.<group>_corrected THEN corr.<x> ELSE de.<x> END`. Two things matter:

1. **The effective sort-key uses the *corrected* effective**:
   `cairn_dose_effective_sort_key(CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE
   de.effective_value END, de.hlc_wall)` in the `ORDER BY`. This is the load-bearing behaviour — winner
   selection (current dose) and the titration trail's chronology now reflect the corrected date.
2. **The corrected `reason` is finally surfaced** in `patient_medication_dose_history` (closes the §1 dead
   column).

**Column sets are unchanged** — only the *source expressions* of existing columns change. This respects the
replay rule (a later migration must not add columns to an earlier `CREATE OR REPLACE VIEW`, or db/032's
narrower replay aborts on reconnect). The existing `corrected` boolean stays as "some correction row exists
for this point." No new columns are exposed on these two views; a future "which fields were corrected" surface
would be a *separate* new view, never a widening of these.

**`patient_medication_current` / `_past`** need **no change** — they already source the dose from
`medication_current_dose` (db/032:266/277), so they inherit the corrected winner for free.

**Thread-scoping is preserved.** The correction→point join stays `AND corr.medication_id = de.medication_id`
(db/032:222/245): a mistargeted or hostile cross-thread correction remains a projection no-op (fail-safe),
while the signed event stays auditable in `event_log`.

---

## 6. Migration mechanics — `db/035_medication_dose_effective_correction.sql`

New migration, additive and replay-safe (db/031–034 untouched):

1. `ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS …` for the six new columns (§5). Booleans
   added **nullable** (not `NOT NULL DEFAULT`), so pre-existing rows are distinguishable from new ones.
2. **Guarded idempotent backfill** for pre-035 rows (`WHERE dose_corrected IS NULL`): those were whole-row
   dose corrections, so `dose_corrected := TRUE`, `effective_corrected := FALSE`, `reason_corrected := FALSE`,
   `note := reason`, `reason := NULL`. Runs on every connect but touches a row at most once.
3. `CREATE OR REPLACE FUNCTION cairn_check_medication_dose(text, jsonb)` — extend the correction branch with:
   `strike` must be a JSON array of strings drawn from the closed set `{dose, effective, reason}` (unknown
   token → legible reject, fail-closed; a future strikeable field is an additive floor change); a group must
   not be both a set-key and in `strike` (conflict → reject); and the no-op reject (§3). `medication_id` /
   `corrects` uuid checks are unchanged. The #173 registry mapping is untouched (same fn name).
4. `CREATE OR REPLACE FUNCTION medication_dose_correction_apply()` — write the new columns + touched-flags,
   parsing set-key presence and `strike` membership. Keeps the existing HLC-wins `ON CONFLICT DO UPDATE`
   (`cairn_hlc_overlay_wins`) so a twice-corrected point converges deterministically.
5. `CREATE OR REPLACE VIEW medication_current_dose` and `patient_medication_dose_history` per §5 (same column
   sets).

The correction trigger itself (db/032:314) is unchanged — it already points at the replaced function by name.

---

## 7. Rust + CLI

- **`cairn-event::medication::dose`** — extend `DoseCorrection` with `effective`, `effective_precision`,
  `note`, `strike: &[&str]`; `reason` semantics → point-reason. `dose_correction_body` emits each optional
  field honestly (omit-when-absent) and the `strike` array when non-empty. `render_dose_correction_twin`
  renders set values, struck groups ("effective date struck (unknown)"), the clinical reason, and the note;
  always non-empty.
- **`cairn-node::medication::dose`** — extend `CorrectDoseInput` (same new fields) and thread them through the
  pure `build_dose_correction_body`; the `correct_dose` orchestrator's submit/attest plumbing is unchanged.
  `resolve_correction_target` unchanged.
- **CLI `MedicationCorrectDose`** — add `--effective`, `--effective-precision`, `--note`, and a repeatable
  `--strike <dose|effective|reason>`; re-document `--reason` (the clinical reason) and remove the
  "omit → unknown" help (now `--strike`). Existing `--dose-amount` / `--dose-unit` / `--info-source` / attest
  flags unchanged.

---

## 8. ADR + spec

- **[ADR-0050](../../spec/decisions/)** — *dose correction is a per-field patch* — records: patch (not
  whole-point restatement) so a one-field fix never wipes the rest (principle 4); an explicit `strike` sentinel
  keeps set-to-unknown first-class; the **corrected effective date drives current-dose winner selection**
  (bitemporal repair, not display); and the correction-note is separate from the point's clinical reason.
  Matches the ADR pattern of the earlier medication slices (0047 reconciliation, 0049 attestation).
- **Spec** — v0.50 → v0.51 (`index.md`) + a one-line §3.15/§3.16 note that a dose correction patches the
  targeted point's dose / effective / reason and that a corrected effective participates in winner selection.

---

## 9. Testing (TDD)

Failing-test-first, in dependency order:

- **Pure builder** (`cairn-event`): patch-set of each group; `strike` emission; `note`; omit-when-absent;
  twin non-empty for set, for strike-only, and for note-only.
- **Floor** (DB-gated): `strike` must be an array of known tokens (unknown token rejected legibly);
  set-key + same group in `strike` rejected; no-op correction rejected; valid single-field patch accepted;
  `corrects`/`medication_id` uuid checks still hold.
- **Projection** (DB-gated) — the load-bearing set:
  - **corrected effective flips the current-dose winner** (the headline test: a later-effective point becomes
    current once an earlier point's date is corrected forward, or vice-versa);
  - corrected effective reorders `patient_medication_dose_history`;
  - **corrected reason is surfaced** (closes the dead column);
  - `strike:["dose"]` shows unknown while `effective`/`reason` are untouched (per-field isolation);
  - a patch of one group leaves the others at their original values;
  - twice-corrected point → HLC-wins convergence;
  - cross-thread mistarget (correction naming thread X, `corrects` a point of thread Y) stays a projection
    no-op;
  - backfill idempotency (a pre-035-shaped row reads correctly and is not re-clobbered on replay).

All tests must pass (`cargo test --workspace`, fmt, clippy `-D warnings`, and the touched SQL mirror) before
commit.

---

## 10. Out of scope (documented boundaries)

- **Correcting the medication's top-level `started` date** shown in `patient_medication_current.started_value`
  — that is the *assert's* field (db/031 statement projection), a separate concern. A correction targeting
  point 0 (the initial point) fixes the point's effective in the **dose timeline** only; the statement-level
  `started` is untouched. A future "correct the statement's started/term/sig" slice handles that.
- **Structured effective-date validation** — the effective `value` stays an uncertainty-capable free string
  (matching `-dose-change`); the floor checks structure/type only, never parses the date (locale-neutral,
  §5.1).
- **Field-merge across multiple corrections of the *same* point.** The overlay keeps the existing db/032 shape:
  one row per corrected point, highest-HLC-wins **wholesale** (`cairn_hlc_overlay_wins`), so set-union sync stays
  convergent. Per-field patch therefore applies *within* one correction (against the original point), but a later
  correction of the same point **supersedes** an earlier one rather than field-merging with it — to keep an
  earlier field-fix, restate it. True per-field merge would need per-field HLC tracking (multiple rows or per-field
  overlay guards) to remain convergent; that is a deliberate future refinement, noted in ADR-0050.
- No change to attestation, reconciliation, cessation, or the assert verb.
