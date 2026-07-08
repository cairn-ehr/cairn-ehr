# §5.4 marks / belongings / EMS-context identity evidence — design

**Date:** 2026-07-08 · **Slice:** §5.4 identity-evidence, text kinds · **Spec home:** §5.4 (identity)

## Summary

Add three **text-shaped** `kind` values — `mark`, `belongings`, `ems-context` — to the existing
`identity.evidence.asserted` event type. These record clinician-observed corroboration about an
unidentified ("John Doe") patient that is **not** a demographic field and has **no attachment**:
a distinguishing bodily mark, personal belongings found on the patient, or the EMS pickup context.

The photo slice (ADR-0042) established this event type and named these three as the planned future
kinds. This slice fills them in. Because the type is already registered (`db/028`), additive, and
non-demographic (the `db/015` twin floor carries the authored twin verbatim), there is:

**No new migration · no floor change · no SCHEMA bump · no ADR · no spec-prose change.**

It is the "same event, zero wire change" continuation of the photo work — pure `cairn-event`
builders + a text author path in `cairn-node` + one CLI subcommand + an e2e read-back test.

## Why these are text, not attachments

The photo kind carries its bytes on the top-level `EventBody.attachments` and derives its twin from
the attachment *descriptor* (never the pixels). Marks, belongings, and EMS-context are prose the
clinician types at the bedside ("scar on left forearm, ~5 cm, healed"; "blue wallet with €40, a
cracked phone, house keys"; "found unconscious at the central bus stop, brought by ambulance ~14:30").
The observed content **is** the text, so it lives in the payload and `attachments` stays empty
(`vec![]`) — which preserves the content-address byte-identity proven for zero-attachment events.

## Payload shape

```json
{
  "kind": "mark" | "belongings" | "ems-context",
  "provenance": "clinician-observed",
  "description": "<required, non-empty>",
  "basis": "<optional; omitted entirely when absent>"
}
```

- `description` — the observed content. **Required, non-empty** (principle 4 honest-content floor:
  an evidence assertion that says nothing is meaningless). This is the text analogue of the photo
  slice's mandatory descriptor.
- `basis` — how/why it was observed, optional and **omitted, never null** when absent (principle 4:
  never manufacture a basis). For `ems-context`, the relayed/hearsay distinction lives here
  (e.g. `"reported by attending paramedic"`), because provenance is fixed (below).
- `provenance` — fixed `"clinician-observed"` for all three kinds (the clinician is the recorder;
  the vocabulary stays tight; these events do not feed provenance-precedence the way demographic
  fields do — they are human-facing corroboration in the event log).

The `kind` set is **closed and validated**: an unknown kind is rejected before any DB work. The
event type itself remains open per ADR-0012 additive evolution; the closed set is a typo-drift guard
at the author edge, not a wire constraint.

## Components

### 1. `cairn-event/src/identity_evidence.rs` (extend, ~+55 lines)

Pure, no DB, no clock. Reuses `crate::evidence::CLINICIAN_OBSERVED_PROVENANCE`.

- `MARK_EVIDENCE_KIND` / `BELONGINGS_EVIDENCE_KIND` / `EMS_CONTEXT_EVIDENCE_KIND` constants.
- `parse_text_evidence_kind(&str) -> anyhow::Result<&'static str>` — closed-set validator that maps
  an input string to its canonical constant, erroring on anything else. A test cross-checks it
  against the three constants so a new kind cannot be added to one without the other.
- `text_evidence_body(kind: &str, description: &str, basis: Option<&str>) -> Value` — builds the
  payload above; omits `basis` when `None`.
- `render_text_evidence_twin(kind: &str, description: &str, basis: Option<&str>) -> String` —
  the authored §3.13/§4.5 twin, e.g. `identity evidence (mark): scar on left forearm ~5cm — visible
  on primary survey`. The trailing ` — <basis>` clause is present only when `basis` is `Some`.
  Mirrors `render_identity_evidence_twin` but renders the description directly (no attachment).

### 2. `cairn-node/src/identity_evidence.rs` (new module, ~+80 lines)

- `validate_description(&str) -> anyhow::Result<()>` — refuses empty/whitespace-only. Lives in the
  **library** (like `photo_evidence::validate_photo_descriptor`) so every caller — a future UI
  backend included — inherits the honest-content floor, not just the CLI.
- `build_text_evidence_body(event_id, patient_id, kid, hlc, kind, description, basis) -> EventBody`
  — pure assembly: payload via `text_evidence_body`, authored twin via `render_text_evidence_twin`,
  contributor `[{actor_id: kid, role: "recorded"}]` (additive → no attestation), `attachments:
  vec![]`, `t_effective: None`.
- `assert_text_evidence(client, sk, kid, node_origin, patient_id, kind, description, basis) -> Uuid`
  — the async orchestrator: `validate_description` → `parse_text_evidence_kind` → tick HLC
  (`db::next_hlc`) → mint `event_id` (`Uuid::now_v7`) → `build_text_evidence_body` → `sign` → **one
  transaction: `SELECT submit_event($1)` only** (no blob store, no `db/026` path). Returns the id.

### 3. `cairn-node/src/main.rs` (CLI, ~+20 lines)

New subcommand:

```
AssertIdentityEvidence {
    patient: Uuid,
    #[arg(long)] kind: String,          // parsed against the closed set, fast-fail before DB
    #[arg(long)] description: String,   // required, non-empty (library re-checks)
    #[arg(long)] basis: Option<String>,
}
```

Handler mirrors `AssertPhotoEvidence`: `parse_text_evidence_kind(&kind)?` and
`validate_description(&description)?` as fast pre-DB checks (single source of truth in the library),
load signing key, connect, `load_local`, `ensure_registration_actor` (the same OWNER ceremony —
a real UI attaches the operating clerk's human actor), then `assert_text_evidence`.

### 4. `cairn-node/tests/identity_evidence_text.rs` (e2e, DB-gated)

- Provision a chart (reuse the John-Doe registration or a plain patient), assert a `mark`, read it
  back from `event_log`: `event_type == identity.evidence.asserted`, payload `kind`/`description`
  correct, provenance `clinician-observed`.
- Assert the twin is legible in the twin projection and contains the description (never a null).
- Assert the event's `attachments` is empty (content-address byte-identity path exercised).
- Reject cases: an unknown `--kind` errors; an empty `--description` errors — both before commit.

## Data flow

```
CLI (assert-identity-evidence)
  → parse_text_evidence_kind + validate_description   (fast-fail, no DB)
  → load signer + ensure_registration_actor            (OWNER ceremony)
  → assert_text_evidence
      → next_hlc → build_text_evidence_body → sign
      → txn { submit_event }                           (db/005 floor: registered type ✓,
                                                         authored twin carried verbatim ✓)
  → event in event_log, legible via its twin
```

No projection table and no worklist — evidence is retrievable from the event log and legible via its
twin, identical to the photo slice. No matcher signal (these are human-facing corroboration, not
demographic fields).

## Error handling

| Condition | Where caught | Result |
|---|---|---|
| Unknown `--kind` | `parse_text_evidence_kind` (CLI, pre-DB) | error, no event authored |
| Empty/whitespace `--description` | `validate_description` (library, pre-DB) | error, no event authored |
| Unregistered type / floor violation | `submit_event` (db/005) | unchanged floor error surfaced |

## Testing (TDD order)

1. Pure `cairn-event` tests first: payload shape, `basis` omission, twin legibility (with/without
   basis), `parse_text_evidence_kind` accept/reject, kind-constant cross-check.
2. Pure `cairn-node` tests: `build_text_evidence_body` shape + empty `attachments`,
   `validate_description` accept/reject.
3. e2e DB-gated read-back + reject cases (above).

All tests written before the code they drive. Suite must be green before commit.

## Honest limits (deliberate, recorded)

- **Free-text `description` only** — no structured belongings item list (YAGNI; a clinician writes
  prose; a structured schema can be an additive future kind or facet without a wire break).
- **Provenance fixed to `clinician-observed`** — the relayed/hearsay distinction for `ems-context`
  lives in `basis` prose, not a distinct provenance term (keeps the vocabulary tight; revisit only
  if a reader/matcher ever needs to machine-distinguish witnessed vs. relayed).
- **No projection / worklist / matcher signal** — evidence is log-retrievable + twin-legible, same
  as the photo slice.
- **Out of scope (separate deferred §5.4 slices):** the "prior history now available" push-alert on
  link (§5.12, no notification tier yet) and the search-before-create registration funnel
  (§5.3/§5.8, UI/API tier).

## Size

`cairn-event` ~+55 lines, new `cairn-node` module ~+80, CLI ~+20, tests. Every touched file stays
well under the 500-line guideline.
