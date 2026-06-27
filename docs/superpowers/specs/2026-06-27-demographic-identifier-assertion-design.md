# Design — Demographic identifier assertion (slice 1 of the demographics subsystem)

**Date:** 2026-06-27 · **Spec home:** demographics §4.1/§4.4/§4.5 · **ADRs:** [0033](../../spec/decisions/0033-patient-identifier-representation.md) (representation), [0034](../../spec/decisions/0034-demographic-legibility-twin.md) (twin), [0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md) (profile bundle) · **Substrate:** `cairn-node`

## Purpose

This is the **first production clinical surface** on the Cairn node. The architecture spec is complete
(v0.36) and demographics is fully specified, but no demographic functionality exists in code — the only
demographic shape today is the deliberately-naive `patient_chart(name/dob/sex TEXT)` projection built to
*measure* projection cost (Bet B), plus a placeholder twin in `submit_event`.

This slice builds the **§4.4 patient-identifier assertion** end-to-end — author → in-DB floor →
set-union projection — as the spine the other demographic fields (address §4.3, names/DOB/sex §4.2) reuse.
Identifiers are chosen first because they are the safety-critical carrier (the §4.2 hard veto), have the
crispest floor invariants, and have the simplest projection (pure set-union, no LWW/recency/provenance
precedence).

### Scope boundaries (what this slice is NOT)

- **Matching is out of scope.** The §4.4 hard veto lives in the identity §5.2 matching pipeline — a
  separate, later subsystem. This slice *carries* the `system`/`normalized`/`profile` facets in the event
  and enforces the *structural floor invariants*, but does **not** implement veto logic. It proves
  author→floor→projection, not author→match.
- **No CLI verb.** Authoring is exercised by the test harness (and the future product CLI). The CLI is the
  least safety-critical part and would enlarge the slice without proving anything new about the shape.
- **Authored-twin fix is demographics-only.** `submit_event` carries + floor-checks the §4.5 authored twin
  only for the new demographic event type; legacy spike event types keep deriving their skeleton twin.
  Globalising the authored twin (principle 11 / §3.13, every event) is a noted follow-up, not this slice.
- **No provenance precedence in the projection.** Identifiers are set-union, never LWW (§4.2). First-seen
  row wins; provenance/profile "best-of" merge is a future refinement (YAGNI now).

## Why graduate into `cairn-node` (substrate)

`crates/cairn-node/src/db.rs` already loads the full clinical event tier (`db/001–006`: signed
append-only `event_log`, the validated `submit_event` floor, the projection mechanism) *plus* the
node-federation tier. So "graduate into cairn-node" is not a port — the proven envelope + write-door +
projection machinery is already running there. Demographics is a **new migration layered onto a schema
cairn-node already loads**, not a parallel build on the throwaway walking-skeleton rig.

## The event shape

**Event type:** `demographic.identifier.asserted` — registered **additive** (`targets_other_author =
FALSE`) in `event_type_class` (the additive-only registry; unknown types fail closed).

**Payload** (the §4.1 assertion, identifier specialization §4.4) — lives in `EventBody.payload`:

```jsonc
{
  "field":       "identifier",
  "provenance":  "document-verified",   // §4.1 ladder; required-present, value-open ("unknown" is honest)
  "value":       "943 476 5919",        // §4.4 mandatory — as-entered, never rewritten
  "system":      "nhs-number",          // §4.4 mandatory — stable content-addressed namespace (or literal "unknown")
  "normalized":  "9434765919",          // §4.4 optional — materialised when a profile is present
  "profile":     "nhs-number@b3-…",     // §4.4 optional — namespace@hash validator bundle reference
  "use":         "national-id"          // §4.4 optional — recommended-but-open vocabulary
}
```

**Additive change to `EventBody`** (`crates/cairn-event/src/lib.rs`): add
`plaintext_twin: Option<String>` with `#[serde(default)]`, **appended last** so it is additive-only —
old events (without it) decode as `None` and their content-addresses are unchanged. This carries the §4.5
**authored** twin in the signed body (today `submit_event` re-derives a skeleton twin; §4.5 requires it
materialised at authoring). The first red test pins this additive canonical-CBOR property before any other
work.

## Rust pure functions

New module `crates/cairn-event/src/demographics.rs` (keeps the already-773-line `lib.rs` from growing —
house rule 4; no unrelated refactor of the existing module):

- `identifier_assertion_body(...) -> serde_json::Value` — builds the payload above from explicit typed
  inputs. Pure, unit-tested.
- `render_identifier_twin(...) -> String` — the materialised §4.5 plaintext twin, profile-independent,
  e.g. `"NHS number, document-verified: 943 476 5919"`. Pure, unit-tested.

`lib.rs` gains only `mod demographics;` and the one additive `EventBody` field.

## Data flow

1. **Author** (test harness now; product CLI later): `identifier_assertion_body(...)` → payload;
   `render_identifier_twin(...)` → twin; assemble `EventBody { event_type:
   "demographic.identifier.asserted", payload, plaintext_twin: Some(twin), … }`; `sign()` → `SignedEvent`.
2. **Submit**: `submit_event(signed_bytes)` → verify signature (`cairn_verify`) → parse body
   (`cairn_body`) → resolve actor (enrolled, non-revoked) → classify (additive) → **demographic branch**
   (floor checks + carry authored twin) → append to `event_log` → trigger → set-union into
   `patient_identifier`.
3. **Read**: query the `patient_identifier` projection.

No pgrx change expected: `cairn_body` converts CBOR→JSONB generically, so the new body field passes
through. A test confirms this.

## The in-DB floor — `db/010_demographics.sql`

Added to `cairn-node/src/db.rs` `SCHEMA` after `009`. Three parts:

### (a) Floor helper — `cairn_check_identifier_assertion(b jsonb)`

Enforces only the §4.4 culture-neutral **structural** invariants, each a distinct legible `RAISE`:

- `value` present, text, non-empty (trimmed)
- `system` present, text, non-empty (may be the literal `"unknown"`)
- `provenance` present, non-empty text (value-open)
- `normalized` — if present, must be text
- **`normalized` present ⇒ `profile` named** (the materialised-key invariant — §4.4)

It **never holds a profile, never runs a checksum, never validates a format, never rejects on validation**
(§4.4 / principle 12). Cross-facet verification (`normalized == normalizer(value)`) is advisory and belongs
to profile-holding nodes, never the floor.

### (b) `submit_event` re-declared (`CREATE OR REPLACE`)

Supersedes the `db/005` definition (standard additive-migration delta; the loaded function is `010`'s).
One demographic branch wired in after classification:

```sql
IF v_type = 'demographic.identifier.asserted' THEN
    PERFORM cairn_check_identifier_assertion(b);
    v_twin := b ->> 'plaintext_twin';                 -- carry the AUTHORED twin (§4.5)
    IF v_twin IS NULL OR length(trim(v_twin)) = 0 THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    END IF;
ELSE
    v_twin := format('[%s] %s for patient %s', …);     -- legacy spike types unchanged
END IF;
```

### (c) Projection — `patient_identifier` + trigger

```sql
patient_identifier(
    patient_id, system, match_key, value, normalized, profile, use, provenance,
    asserted_hlc_wall, asserted_hlc_count, asserted_origin, first_seen)
PRIMARY KEY (patient_id, system, match_key)      -- match_key = coalesce(normalized, value)
```

`patient_identifier_apply()` — `AFTER INSERT ON event_log WHERE event_type =
'demographic.identifier.asserted'` — does **set-union** (§4.2 "set union, never LWW"):
`INSERT … ON CONFLICT (patient_id, system, match_key) DO NOTHING`. Same system+normalized dedups; same
system+different normalized = two rows (the veto *signal* preserved as data; the veto itself is out of
scope). First-seen row wins.

## File layout & sizes (all <500 lines)

| File | Change |
|---|---|
| `crates/cairn-event/src/lib.rs` | +1 additive field on `EventBody`; `mod demographics;` |
| `crates/cairn-event/src/demographics.rs` | **new** — two pure fns + unit tests |
| `db/010_demographics.sql` | **new** — floor helper, `submit_event` re-decl, projection + trigger |
| `crates/cairn-node/src/db.rs` | +1 line in `SCHEMA` |
| `crates/cairn-node/tests/demographics.rs` | **new** — integration tests |

## Acceptance tests (TDD, red-first)

1. **Additive CBOR** — an old `EventBody` (no twin) decodes; the content-address of a pre-existing event
   is unchanged when the field is added.
2. **`cairn_body` passthrough** — the new twin field reaches JSONB.
3. **Happy path** — a well-formed assertion → accepted → exactly one `patient_identifier` row with the
   expected facets.
4. **Floor rejections** — each invariant violation yields a distinct legible rejection with nothing
   written to `event_log` or the projection: value-empty · system-missing · provenance-missing ·
   normalized-non-text · **normalized-without-profile** · empty-authored-twin.
5. **Set-union** — same system+normalized → one row; same system+different normalized → two rows.
6. **Honest degradation** — `normalized` absent + `profile` absent → accepted; `match_key = value`.
7. **Regression** — `patient.created` still works via the derived-twin path.

## Risks / things to verify first

- **Canonical-CBOR additivity** (test 1) is load-bearing: if appending the `Option` field changed an
  existing event's bytes/address it would violate append-only. Verify before building on it.
- **`cairn_body` passthrough** (test 2): assumed generic CBOR→JSONB; confirm no typed deserialize drops
  the field. If it does, a pgrx change enters scope (not expected).
- **`submit_event` re-declaration** duplicates the function body across `005` and `010`. Accepted as the
  additive-migration pattern; the demographic logic is isolated in the `cairn_check_identifier_assertion`
  helper to keep the re-declared function legible.

## Follow-ups (explicitly deferred)

- Globalise the authored twin to every event type (principle 11 / §3.13); retire the skeleton-twin TODO.
- Address (§4.3), names/DOB/sex (§4.2) assertions — reuse this spine.
- The §5.2 matching pipeline + the §4.4 hard veto (the advisory matcher, Python/fit-for-purpose).
- Provenance/profile "best-of" projection refinement for identifiers.
- Product CLI verb for authoring demographic assertions.
