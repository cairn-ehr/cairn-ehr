# Design — the day-one attachment-reference shape + the first photo evidence attachment

**Date:** 2026-07-08 · **Slice:** §5.4 John-Doe evidence (photo) + ADR-0013 attachment envelope
· **Layer:** wire core (safety-critical, Rust in `cairn-event`) + node author path (fit-for-purpose)

## Why this slice exists

The §5.4 spec captures John-Doe identity evidence as clinician-observed assertions:
*"estimated age with basis, observed sex, **photo**, distinguishing marks, belongings, EMS pickup
context."* Slice B built estimated-age + observed-sex on existing demographic fields. The **photo** is
the next evidence type — and it is the first clinical-surface use of the content-addressed **blob /
attachment tier** (ADR-0013, §3.14 / §6.6).

Wiring a photo forces a decision that cannot be deferred: **the attachment-reference shape is the one
can't-retrofit piece of ADR-0013**, because the reference rides the signed, immutable event. Today's
`AttachmentRef` in `cairn-event/src/lib.rs` is a walking-skeleton stub that satisfies only two of the
five §3.14 day-one reserves. **No production event carries an attachment yet** (every event sets
`attachments: []`), so we are at the free moment: finalize the shape now, validate it with a real
consumer (the photo), and never break the wire again.

### Reconciliation with ADR-0041 (progress-note format)

ADR-0041 (progress-note narrative format, 2026-07-04, spec v0.42) already *refines ADR-0013's
attachment-reference shape*: its Decision point 2 says a note body's `media` manifest entries each carry
*"the ADR-0013 day-one attachment-reference shape — self-describing content digest, media type, byte
length, rendition set, seal indicator / DEK-wrap reference, inline-vs-reference, and the clear-text
descriptor metadata … plus a note-local `id`."* ADR-0041 **described** that shape as if it existed; the
code was never updated past the stub. This slice supplies the missing implementation — ADR-0041's own
Consequences section names it: the shape *"unblocks … the §5.4 photo/marks/belongings evidence (waits on
the attachment tier)."*

**The two carriers, one primitive.** ADR-0041 puts a note's media in the **payload** (`payload.media`,
each entry with a note-local `id` so `![descriptor](cairn:att/<id>)` anchors can reference it inline in
narrative). This slice puts the photo in the **top-level `EventBody.attachments`** field (already
committed as `[]` on every signed event). The convention, recorded in ADR-0042:

- `EventBody.attachments: Vec<Attachment>` — attachments on **non-narrative** events (photo-evidence
  here; marks/belongings/EMS later). No inline anchoring, no note-local id needed.
- A note's `payload.media` (future ADR-0041 build) — the **note-specific** manifest that needs inline
  anchors + note-local ids.

Both carriers reuse the **same `Attachment`/`Rendition` primitives** defined in Phase 1, so the
attachment-reference shape is defined once and never duplicated. The note manifest entry is that
primitive **plus** a note-local `id` (a note-format concern, added additively when that surface is built)
and the ADR-0041 tightening of `descriptor` to mandatory-non-empty at the note floor.

### The §3.14 day-one reserve, current vs. required

| §3.14 required field | current `AttachmentRef` | this slice |
|---|---|---|
| self-describing content digest (alg + value, multihash) | ✅ `alg` + `digest_hex` | keep |
| clear-text descriptor metadata (media type, byte length, modality/descriptor) | ✅ `media_type`/`byte_len`/`descriptor` | keep |
| **seal indicator / DEK-wrap reference** (crypto-shreddable later) | ❌ missing | **add** (reserved, None) |
| **the rendition set** (one attachment = N content-addressed renditions) | ❌ flat single ref | **nest** |
| **inline-vs-reference distinction** | ❌ missing | **add** (reserved, None) |

Project posture (2026-07-08): there is **no real clinical data and no legacy users** — the EHR is far
from clinical use. POC nodes/GUIs run periodically to assess shape and unmask flaws. So we can afford to
build the infrastructure properly, one step at a time; the reason to get the *shape* right now is not
migrating existing data but never breaking the *wire* once the first attachment is signed.

## Governing principles in play

- **Principle 1 (append-only + causal ordering).** A blob is named by its content digest — the digest is
  to a blob what the signature is to an event body. Same bytes → same address → idempotent set-union.
- **Principle 4 (acknowledged uncertainty).** A not-yet-retrieved blob renders honestly as *"referenced
  here — not yet retrieved,"* never as absent.
- **Principle 11 (legibility across time) / §3.14.** The plaintext twin is **not** derived from the
  pixels; it is derived from the event's clear-text descriptor fields. The lightweight rendition is the
  blob's own twin.
- **Principle 12 (uniform core).** The blob self-verification floor (db/026, `cairn_blob_verify`) binds
  every writer — including this new author path — unbypassably in the database.
- **§9 defect-blast-radius rule.** The reference shape can silently corrupt signed events across nodes →
  **Rust in `cairn-event`**, optimized for reviewer-legibility. The photo author path / CLI is
  fit-for-purpose node glue.

## Section 1 — the concrete attachment-reference shape (immutable reserve)

Replace the flat `AttachmentRef` with a three-type shape in a new `cairn-event/src/attachment.rs`
(re-exported from `lib.rs`; `EventBody.attachments` becomes `Vec<Attachment>`).

```rust
/// One logical binary clinical artifact, referenced by the signed event. The reference is
/// EAGER (rides the signed body); the bytes are LAZY (§6.6 byte tier). This shape is the ONE
/// can't-retrofit piece of ADR-0013 — every field is a day-one envelope reserve (ADR-0042).
pub struct Attachment {
    /// Clear-text, always-legible descriptor (media-independent): what this is, in words.
    /// Feeds the event twin + the §5.9 safety projection even when bytes are sealed/absent.
    pub descriptor: String,
    /// The rendition set: N content-addressed views of the SAME logical artifact
    /// (original + preview + extracted-text …), each with its own address + sync priority.
    /// A photo ships as a single `original` today; the set is reserved for later views.
    pub renditions: Vec<Rendition>,
}

/// One content-addressed rendition within an Attachment's rendition set.
pub struct Rendition {
    /// Role in the set: "original" | "preview" | "extracted-text" | …
    pub role: String,
    /// Multihash digest algorithm ("blake3") — self-describing, so a future algorithm is an
    /// additive migration while this digest stays fixed.
    pub alg: String,
    /// Hex of the multihash content address. Names the PLAINTEXT bytes when `seal` is None;
    /// the CIPHERTEXT bytes when sealed (no convergent encryption — §3.14).
    pub digest_hex: String,
    pub media_type: String,
    pub byte_len: i64,
    /// Inline-vs-reference distinction. `Some(bytes)` = inlined on the eager plane (tiny blobs
    /// below a node threshold); `None` = by-reference on the lazy byte tier. Omitted from the
    /// wire when None (skip_serializing_if), so a by-reference rendition encodes minimally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<serde_bytes::ByteBuf>,
    /// Seal indicator / DEK-wrap reference. `None` = plaintext; `Some` = sealed under a per-blob
    /// DEK, crypto-shreddable via the ADR-0005 key-custody ladder. Omitted when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<SealRef>,
}

/// Seal indicator / DEK-wrap reference — makes the blob crypto-shreddable later (ADR-0005).
pub struct SealRef {
    /// Seal algorithm (e.g. "xchacha20poly1305", mirroring keystore `seal.rs`).
    pub alg: String,
    /// Reference to the wrapped per-blob DEK: where to obtain the key to decrypt, and what is
    /// crypto-shredded to erase.
    pub dek_wrap: String,
}
```

**What is genuinely can't-retrofit vs. additive-but-reserved-now:**

- `renditions: Vec<Rendition>` is **structurally** can't-retrofit — a flat single-ref now could not
  become a set later without changing the bytes of every attachment written today. This is the field we
  *must* nest from day one.
- `seal` and `inline` are `Option` + `skip_serializing_if` — technically additive later, but reserved
  now because (a) §3.14 names them day-one, and (b) the shape is coherent and validated by a real
  consumer this session rather than bolted on. A `None` on both is omitted from the wire, so an unsealed
  by-reference photo encodes minimally.

**Field order is frozen as the canonical CBOR encoding** (structural move 1 — one writer, one
serialization). ADR-0042 records the order.

**Byte-safe migration.** Every existing event has `attachments: []`. Serde encodes an empty `Vec` as an
empty CBOR array regardless of element type, so swapping `AttachmentRef` → `Attachment` changes **zero**
existing content-addresses. (The `attachments` field has `#[serde(default)]`, no `skip_serializing_if`,
so `[]` is always present — confirmed against `lib.rs`.)

**Pure constructors + twin fragment (TDD, no I/O):**

- `Rendition::reference(role, bytes, media_type) -> Rendition` — computes `blob_address`/`byte_len` from
  the plaintext bytes; `inline = None`, `seal = None`. (`alg = "blake3"`, `digest_hex` = hex of the
  multihash.)
- `Attachment::single(descriptor, rendition) -> Attachment` — the one-rendition case.
- `render_attachment_twin(&Attachment) -> String` — legible from **descriptor fields only, never
  pixels**: e.g. `"photograph — <descriptor> (image/jpeg, 42 KB)"`. Used by the evidence twin.

## Section 2 — the photo-evidence event home + local author path

**Event home (new, non-demographic):**

```
event_type:      "identity.evidence.asserted"
schema_version:  "identity.evidence.asserted/1"
payload:         { "kind": "photo", "descriptor": "<clear-text>", "basis": "<how/why observed>" }
attachments:     [ Attachment { descriptor, renditions: [ Rendition{ role:"original", … } ] } ]
contributors:    [ { actor_id: kid, role: "recorded" } ]   // additive → no attestation demanded
plaintext_twin:  authored, rendered from descriptor + attachment twin (NOT pixels)
```

- Non-demographic type → the db/015 twin floor **prefers the authored twin, no floor change**; a novel
  type with an authored twin is carried verbatim.
- Provenance `clinician-observed` (rank 30), reusing slice B's ladder term.
- `kind` future-homes **marks / belongings / EMS-context** as further values — zero wire change.

**Pure body builder (`cairn-event`, extend `evidence.rs` or a sibling `identity_evidence.rs`):**

- `photo_evidence_body(descriptor, basis, attachment) -> serde_json::Value` — the payload.
- `render_identity_evidence_twin(kind, descriptor, basis, &Attachment) -> String` — the authored twin.

**Local blob author path (`cairn-node`, new `photo_evidence.rs`):**

```rust
/// PURE: compute the content address, bao outboard, and the "original" Rendition for bytes.
/// No DB — so it is unit-testable; the orchestrator does the INSERT inside the txn.
pub struct LocalBlob { pub addr: Vec<u8>, pub outboard: Vec<u8>, pub rendition: Rendition }
fn prepare_local_blob(bytes: &[u8], media_type: &str) -> LocalBlob

/// Take photo bytes → prepare_local_blob → INSERT present=TRUE (through the db/026 verify
/// trigger — the first non-hostile writer) → author the identity.evidence event carrying the
/// Attachment → sign → submit_event, ALL in one transaction (bytes + reference land atomically).
async fn assert_photo_evidence(
    client, sk, kid, node_origin, patient_id, bytes: &[u8], media_type, descriptor, basis,
) -> anyhow::Result<Uuid>   // the evidence event_id
```

- Pure/impure split (house rule 1): `prepare_local_blob` is a pure function (address + outboard +
  rendition), unit-tested; the orchestrator owns the DB. The CLI edge reads the file → passes bytes.
- One transaction: the `blob_store` INSERT (present) and the `submit_event` land atomically — a chart
  never references a blob whose bytes failed to store, and vice versa. (The floor's own
  `blob_note_reference` for our rendition is a harmless `ON CONFLICT DO NOTHING` no-op — the present row
  already exists.)
- HLC ticked once for the single evidence event (same tick-then-submit shape as `john_doe.rs`).

**CLI subcommand:**

```
cairn-node assert-photo-evidence \
    --patient <uuid> --file <path> --media-type <mt> --descriptor <text> [--basis <text>]
```

The CLI edge owns file I/O and the clock; the media type is caller-supplied (no sniffing dependency).

## Section 3 — honest limits & deferred

- **Bytes stay local.** The reference syncs (eager); cross-node **byte fetch** (the lazy §6.6 swarm/
  resumable tier) is existing POC machinery, **not** wired into cairn-node here. A receiving peer sees
  `"referenced here — not yet retrieved"` (correct honest-assembly), but remote retrieval is unexercised.
- **Single `original` rendition; no preview/thumbnail** — preview generation needs an image library (new
  AGPL-compatible dep), deferred. The rendition set ships with one member.
- **Plaintext, unsealed** — seal field reserved-and-None. Per-blob DEK sealing + byte-tier key custody
  (ADR-0005 applied to blobs) is its own later slice.
- **Matcher unchanged** — a photo is not a comparator feature and `identity.evidence.asserted` is not a
  demographic field, so the matcher pipeline never selects it. One-line note, no test.
- **Not in scope:** marks/belongings/EMS-context kinds; `--observed-year`; readable callsign suffix;
  identify→link resolution flow — all remain separate John-Doe slices.

## Section 4 — testing & phasing (TDD throughout)

**Phase 1 — the shape + ADR (`cairn-event`; safety-critical, pure).**
- Replace `AttachmentRef` → `Attachment`/`Rendition`/`SealRef`; add pure constructors + twin fragment.
- Tests (red first): five-reserve presence in the shape; plaintext-vs-sealed digest semantics
  (unsealed digest = plaintext hash; sealed = ciphertext hash, seal indicator set); inline-vs-reference;
  **empty-vec byte-identical stability** (an event with `attachments: []` encodes to the exact bytes it
  did before the type change — the content-address is unchanged); round-trip; twin renders from
  descriptor, contains no byte content.
- Then **ADR-0042** (refines ADR-0013, and notes the ADR-0041 reconciliation; freezes field order + the
  can't-retrofit-vs-additive reasoning), a §3.14 concrete-shape note in `docs/spec/data-model.md`, and the
  spec-version bump in `index.md` (**0.42 → 0.43**).

**Phase 1b — the floor learns the rendition set (`db/`; safety-critical SQL).**
- `db/027_attachment_rendition_references.sql`: `cairn_learn_attachment_refs(b)` (walks
  `attachments[*].renditions[*]`, skips inline); register it in `db.rs`'s `SCHEMA` array.
- Edit `db/005` + `db/020` learn-loops → `PERFORM cairn_learn_attachment_refs(b);`.
- DB-gated test (`tests/attachment_refs.rs`): submitting an event whose attachment carries a by-reference
  rendition registers a `blob_store` reference row (`present = FALSE`, correct address/media_type/byte_len);
  an inline rendition registers none; a two-rendition attachment registers two.

**Phase 2 — the photo author path (`cairn-event` body + `cairn-node` + CLI).**
- Pure `photo_evidence_body` + `render_identity_evidence_twin` unit tests.
- DB-gated e2e (`CAIRN_TEST_PG`, PG18 + cairn_pgx ≥ 0.3.0): `assert_photo_evidence` on a provisioned
  node → the blob is `present` and re-verifies (`cairn_blob_verify`), the reference appears in
  `event_log.attachments`, the event twin is legible and contains no pixel bytes, and a deliberately
  **wrong-hash** reference is rejected by the db/026 floor.

**File sizes** stay well under 500 LOC: new `cairn-event/src/attachment.rs` (shape), extend
`evidence.rs` (or new `identity_evidence.rs`), new `cairn-node/src/photo_evidence.rs`.

## What changes, at a glance

| Area | Change | Kind |
|---|---|---|
| `cairn-event/src/attachment.rs` (new) | `Attachment`/`Rendition`/`SealRef` + constructors + twin | wire core |
| `cairn-event/src/lib.rs` | `attachments: Vec<Attachment>`; re-export | wire core |
| `cairn-event` evidence body | `photo_evidence_body` + evidence twin | wire core |
| `cairn-node/src/photo_evidence.rs` (new) | `prepare_local_blob` (pure) + `assert_photo_evidence` (orchestrator) | node glue |
| `cairn-node` CLI | `assert-photo-evidence` subcommand | node glue |
| `db/027_attachment_rendition_references.sql` (new) | `cairn_learn_attachment_refs(b)` — walks the rendition set | **floor** |
| `db/005_submit.sql` + `db/020_apply_remote_event.sql` | inline learn-loop → `cairn_learn_attachment_refs(b)` call | **floor** |
| `docs/spec/decisions/0042-*.md` (new) | concrete attachment-reference shape (refines 0013; reconciles 0041) | ADR |
| `docs/spec/data-model.md` §3.14 + `index.md` | concrete-shape note + version bump (0.42 → 0.43) | spec |

### The floor change (revised — this is NOT reserve-only)

The submit floor **already** walks `attachments[*]` and learns a lazy blob reference per element by
reading `digest_hex`/`media_type`/`byte_len` **off each attachment** (`db/005:217`, mirrored at the
remote-apply door `db/020:232`). The new shape moves those fields onto the **rendition**, so the loop
must walk `attachments[*].renditions[*]` instead — otherwise `c ->> 'digest_hex'` goes NULL and
`blob_note_reference(NULL,…)` violates the `blob_address` PK. This is **safety-critical in-DB SQL** and
gets its own TDD task:

```sql
-- db/027: learn a lazy blob reference for every BY-REFERENCE rendition of every attachment
-- (reference-eager, byte-lazy). Skips inline renditions (bytes ride the event — no lazy blob).
CREATE OR REPLACE FUNCTION cairn_learn_attachment_refs(b jsonb) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE a jsonb; r jsonb;
BEGIN
  FOR a IN SELECT jsonb_array_elements(COALESCE(b -> 'attachments','[]'::jsonb)) LOOP
    FOR r IN SELECT jsonb_array_elements(COALESCE(a -> 'renditions','[]'::jsonb)) LOOP
      IF r ? 'inline' THEN CONTINUE; END IF;   -- inlined bytes ride the event; no lazy blob
      PERFORM blob_note_reference(decode(r ->> 'digest_hex','hex'),
                                  r ->> 'media_type', (r ->> 'byte_len')::bigint);
    END LOOP;
  END LOOP;
END;
$$;
```

`db/005` and `db/020` each replace their inline learn-loop with `PERFORM cairn_learn_attachment_refs(b);`
(one shared implementation, no drift across the two doors — the same single-source discipline db/015 used
for the twin hook). PL/pgSQL late-binding lets the doors reference the helper before db/027 loads; all
migrations load before any submit. Editing the two doors in place is safe: migrations are re-applied
idempotently on every `connect_and_load_schema` (`CREATE OR REPLACE`), and there are **no deployed nodes
with data** (pre-production). The `advisory.added` non-empty-attachments check stays inline in db/005 —
shape-agnostic, unaffected.

**Honest limit — the frozen POC harness diverges.** `poc/walking-skeleton/harness/{spike_0002,agent_standin}.py`
author `advisory.added` with the *flat* pre-ADR-0042 attachment shape. `poc/` is frozen historical spikes
(not in the workspace build/CI), so nothing breaks; but after this slice their flat attachments no longer
register a blob reference through the new loop. Recorded, not fixed (frozen spikes).

**No matcher change, no SCHEMA bump, no cairn_pgx bump.** The only new event type is additive; the only
genuinely immutable commitment is the frozen reference-shape field order (ADR-0042).
