# ADR-0081 — A medication withhold is a time-bounded hold

- **Status:** Accepted
- **Date:** 2026-10-21
- **Spec version at acceptance:** 0.83
- **Issues:** [#803](https://github.com/cairn-ehr/cairn-ehr/issues/803) (feeds [#804](https://github.com/cairn-ehr/cairn-ehr/issues/804)) — a maintainer decision of 2026-10-21; design: `docs/superpowers/specs/2026-10-21-medication-withhold-803-design.md`
- **Derives from:** [principle 3](../index.md#founding-principles-the-lens-for-every-decision) (paper-parity, §1.2: the paper "W" on a drug chart) and principle 4 (acknowledged uncertainty: "until reviewed" is an explicit value)
- **Applies:** [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (additive-only evolution, the legibility twin) ·
  [ADR-0043](0043-suppression-self-only-disagreement-is-additive.md) (cross-human suppression is refused; the cessation twin precedent) ·
  [ADR-0047](0047-medication-reconciliation-resolution.md) (lines are reconciled groups) ·
  [ADR-0049](0049-commitment-based-sign-off-currency.md) (sign-off is a commitment) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) (born-sealed bodies) ·
  [ADR-0053](0053-per-write-human-authorship.md) (per-write human authorship)

## Context

On a paper inpatient drug chart a drug in doubt is not deleted: someone writes **W** (withheld) with a reason, and the
drug is not given while the W stands. A doctor may write it in advance, a nurse at the bedside after a pre-dose check,
or a doctor *pro tem* until an indication has been discussed. Afterwards the drug is simply given again, with no one
acting; only the *pro tem* kind ends with an explicit act. Cairn had only cessation, which is terminal, so a clinician
who wanted to pause a drug had to cease it, or leave it and hope.

## Decision

### 1. Two new event types

- `clinical.medication-withhold.asserted` (schema `clinical.medication-withhold/1`). Payload: `medication_id` (uuid
  string, required); `reason` (required, non-empty after trim); `from` (optional instant); `until` (required: an
  instant, or the literal string `"reviewed"`). Optional fields are omitted, never `null`.
- `clinical.medication-withhold-release.asserted` (schema `clinical.medication-withhold-release/1`). Payload:
  `medication_id` (required); `releases` (a non-empty array of withhold **event ids**, uuid strings); `reason`
  (optional).

Both are `additive` with `targets_other_author` FALSE: a W is the withholder's own act on the thread and does not edit
or suppress anyone else's prescription (ADR-0043). No existing event type changes (ADR-0012). The legibility twins
carry no drug name, following the cessation precedent: the name lives on the thread's assertion and a copy could
disagree with a later correction of it. Withhold: `Withheld medication from <from|"now"> until <until|"reviewed"> —
<reason>`. Release: `Released withheld medication (<n> hold(s))`, plus ` — <reason>` when given.

A release names the withholds it ends by **event id**, not content address. It still ends only holds its author saw,
and event ids are what the CLI prints and `cairn-node event <id>` reads.

### 2. A hold is a time span; `until` is required; exact instants only

`until` is required and is either an instant or `"reviewed"`, the explicit open end. A clinician who does not know when
a hold should end says so with `"reviewed"`; they are never asked to invent an end, and a missing field never stands in
for it (principle 4). Only an explicit `"reviewed"` may outlive time.

`from` and `until` are **exact instants**: ISO-8601 with an explicit UTC offset (the `cairn_t_effective` pin), and an
offset-less value is refused. This departs from the `{value, precision}` times elsewhere in the medication stream, on
purpose: a hold is compared against *now*, and a day-precision `until` has no defined instant of expiry. A future
`from` (a doctor's advance W for tonight) must go in the payload, because `t_effective <= t_recorded` is enforced at the
submit door and the envelope cannot carry it.

### 3. Expiry is a read-time fold: no event, default resume

A hold is in force at `now` when `from <= now` (absent `from` means the event's effective time), `until` is
`"reviewed"` or `until > now` (strictly: at `until == now` the hold has ended), and no release naming it has an
effective time `<= now`. Instants compare as instants, whatever their offsets. Nothing stores "withheld" as a fact;
the status is computed against the node's clock. When a span ends the line reads as it did before the W, with no act by
anyone: the default is **resume**, never "stays held". Several holds can be in force at once; all are shown, newest
first. A hold not yet started is shown as "W from <time>" on an active line.

### 4. A release names withholds by event id

A release ends the named holds from its effective time onward. It never ends a hold it does not name, so a W that
reached the node after the releaser decided cannot be ended unseen. A named hold whose span already ended is a
**no-op**, not an error. A named hold that has **not started yet** is **cancelled** and never comes into force: a
prescriber who writes "W from 20:00" and releases it at 14:00 means the dose to be given. A release that arrives by
sync before the withhold it names is admitted, and ends that hold once it arrives.

### 5. Cessation wins over any hold

A standing cessation makes the line ceased, whatever holds exist (cessation is terminal; the existing rule is
unchanged).

### 6. The hold sits beside the active/ceased status

`MedicationStatus` stays `{Active, Ceased}`; a row carries a list of holds (`holds: Vec<MedicationHold>`) beside it.
There is no `Withheld` variant. Existing readers treat `status == Active` as "on the current list, signable, ceasable";
a third variant would silently drop withheld drugs out of sign-off, which is the very failure this design avoids. A
withheld line therefore stays current and signable. A line is withheld if a hold is in force on any member thread of its
reconciled group (ADR-0047), a group with a ceased member and an active member reads active, and a withhold is written
once, on the line's representative thread.

### 7. Sign-off

Both event types join the sign-off commitment (ADR-0049), so a W or a release written after a sign-off makes that
line's sign-off stale. Threads with no withhold events keep their commitment, so there is no mass staleness. A hold
*expiring* changes no event and so does not stale a sign-off: the signer took responsibility for "W until 20:00",
which includes what happens at 20:00. Signing a line upholds every W in force on it.

### 8. Refusals live at the local door only

The local door refuses, with a legible refusal: a withhold on a ceased line, an empty reason, a malformed time, `until`
not after `from`, a missing `until`; a release with an empty `releases` or naming an event that is not a withhold on the
same thread. Sync apply never refuses on a clinical-state veto (a node-local veto would fork the event set); the
fold is defined for every arrival order. The floor's structural checks still run at both doors.

A refusal that reads only the payload is **structural** and runs at both doors (db/059's
`cairn_check_medication_withhold`): an empty reason, a missing or malformed `until`, a malformed `from`, `until` not
after `from` when both are given, a bad `medication_id`, an empty or malformed `releases`. A refusal that reads what
this node holds is **clinical state** and runs at the local door only (db/059's apply functions): a ceased line, an
`until` not after the hold's start once an absent `from` is resolved to the event's time, and a release naming
something that is not a withhold on the same thread.

### 9. Wording

"Withheld" is the clinical word and means the hold only. Clinician-facing text where it meant *hidden* is reworded to
"not shown", "left out of sign-off" or "hidden". Internal identifiers are not renamed.

## Consequences

- A node on schema generation 58 admits the new events uninterpreted and shows a withheld drug as **active**.
  Acceptable pre-clinically (no node runs clinical care yet).
- A node with a wrong clock expires holds early or late. The list prints `until`, so a human can judge.
- There is **no administration record (MAR)** and no dose slots; a time span stands in for "the slots the W was
  written into". The ward-level list of new withholds across patients, and any forced acknowledgement of a nurse's W,
  are later work or not wanted.
- Anyone may withhold; restricting it is local policy above the floor.
