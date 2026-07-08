# ADR-0042 — The concrete attachment-reference shape (Attachment / Rendition / SealRef)

- **Status:** Accepted
- **Date:** 2026-07-08
- **Refines:** [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) (attachment reference shape)
- **Reconciles with:** [ADR-0041](0041-progress-note-narrative-format.md) (progress-note media manifest)

## Context

[ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) / [§3.14](../data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)
settled the day-one *requirements* for the attachment-reference shape — five reserves: a
self-describing content digest, a seal indicator / DEK-wrap reference, clear-text descriptor
metadata, a rendition set, and the inline-vs-reference distinction — but never froze the
*concrete encoding*. In the meantime [ADR-0041](0041-progress-note-narrative-format.md) (the
progress-note narrative format) already **referenced** this shape for its `payload.media`
manifest, describing it in prose as carrying "self-describing content digest, media type, byte
length, rendition set, seal indicator / DEK-wrap reference, inline-vs-reference, and the
clear-text descriptor metadata … plus a note-local `id`" — but the code had not caught up: the
walking-skeleton `AttachmentRef` in `cairn-event` was still a flat single-reference stub
satisfying only two of the five reserves (digest + descriptor metadata), with no nesting, no
seal field, and no inline distinction.

The first real consumer — the §5.4 John-Doe photo-evidence attachment — forces finalizing the
shape now: the reference rides the signed, immutable event, so once the first attachment-bearing
event is signed, its field order is frozen under that signature forever. **No production event
carries an attachment yet** (every event today sets `attachments: []`), which is the last free
moment to get this right without a migration.

## Decision

Attachments are a **three-type shape**, implemented in `crates/cairn-event/src/attachment.rs`
and re-exported as `EventBody.attachments: Vec<Attachment>`. The field order below **is the
canonical CBOR encoding** (structural move 1 — one writer, one serialization) and must never be
reordered once the first attachment-bearing event is signed:

```rust
pub struct Attachment {
    pub descriptor: String,
    pub renditions: Vec<Rendition>,
}

pub struct Rendition {
    pub role: String,
    pub alg: String,
    pub digest_hex: String,
    pub media_type: String,
    pub byte_len: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<serde_bytes::ByteBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<SealRef>,
}

pub struct SealRef {
    pub alg: String,
    pub dek_wrap: String,
}
```

1. **`Attachment` is one logical binary clinical artifact:** a clear-text `descriptor` (feeds the
   event twin and the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
   safety projection even when bytes are sealed or absent) plus a `renditions` set — N
   content-addressed views of the *same* logical artifact (original, preview, extracted text, …),
   each with its own sync priority. A photo ships one `original` rendition today; the set is
   reserved for previews/extracts added later without a wire change.

2. **`Rendition` carries the per-view identity and the two reserved options.** `alg` +
   `digest_hex` are the self-describing multihash content address: **`digest_hex` names the
   plaintext bytes when `seal` is `None`, and the ciphertext bytes when `seal` is `Some`** —
   there is no convergent encryption for sealed blobs (deriving the key from the plaintext would
   leak *"someone holds this exact file,"* a confirmation attack; confidentiality outranks dedup).
   `media_type` and `byte_len` are the clear-text descriptor metadata that lets a sealed, pending,
   or unparseable blob still render down the legibility ladder. `inline` (`Some(bytes)` = tiny
   blob embedded on the eager plane; `None` = by-reference on the lazy [sync §6.6](../sync.md#66-attachments-the-lazy-byte-tier)
   tier) and `seal` (`Some` = sealed under a per-blob DEK, crypto-shreddable via the
   [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) ladder; `None` = plaintext) are
   both `Option` with `skip_serializing_if = "Option::is_none"`, so the common unsealed
   by-reference rendition omits both keys from the wire entirely rather than encoding an explicit
   null.

3. **What is *structurally* can't-retrofit versus *additive-but-reserved-now*.** `renditions:
   Vec<Rendition>` is the one genuinely can't-retrofit field: a flat single reference today could
   never become a set later without changing the bytes — and therefore the content-address — of
   every attachment already signed. It must be nested from day one, and is. `seal` and `inline`
   are, in the strict technical sense, additive later (a new `Option` field defaults to absent
   under [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)'s
   additive-only rule) — they are reserved *now* rather than deferred because (a) §3.14 already
   names them as day-one requirements, and (b) shipping them now lets the shape be validated
   against a real consumer (the photo) in one pass rather than bolted on to a shape already
   carrying production data.

4. **Byte-identical migration from the walking-skeleton stub.** Every event minted before this
   ADR carries `attachments: []`; an empty `Vec` serializes to an empty CBOR array regardless of
   element type, so replacing `AttachmentRef` with `Attachment` changes the content-address of
   **zero** existing events.

5. **`EventBody.attachments` versus the ADR-0041 note manifest — one primitive, two carriers.**
   `EventBody.attachments: Vec<Attachment>` carries attachments on **non-narrative** events (the
   photo-evidence event; marks/belongings/EMS-context later) — no inline anchoring is needed, so
   no note-local id is needed either. The [ADR-0041](0041-progress-note-narrative-format.md)
   `payload.media` manifest is the **same** `Attachment`/`Rendition`/`SealRef` primitive, defined
   once here and never duplicated, **plus** a note-local `id` (so a markdown anchor
   `![descriptor](cairn:att/<id>)` can address a manifest entry) and ADR-0041's own tightening of
   `descriptor` to mandatory-non-empty at the note floor. ADR-0041 described this shape before the
   code existed; this ADR supplies and freezes the implementation ADR-0041 was already pointing
   at — no reconciliation gap remains between the two ADRs.

6. **The floor learns a lazy blob reference per *rendition*, not per attachment.** The submit
   door and the mirrored remote-apply door already walk `attachments[*]` and learn a lazy blob
   reference by reading `digest_hex`/`media_type`/`byte_len` off each element (`db/005`, `db/020`)
   — that walk moves one level deeper, to `attachments[*].renditions[*]`, skipping any rendition
   carrying `inline` (its bytes ride the event, not the lazy tier; no blob reference to learn).
   This is safety-critical in-DB SQL and is built as `cairn_learn_attachment_refs(b)` in
   `db/027`, called from both doors in place of their current inline loop — one shared
   implementation, no drift between the two enforcement points, the same single-source
   discipline the twin hook already used ([ADR-0039](0039-globalise-authored-legibility-twin.md)).

## Consequences

**Easier / now guaranteed:**
- The attachment-reference shape is validated by a real consumer (photo evidence) before any
  production data exists, rather than shipped speculatively and discovered wrong later.
- The rendition set, seal reserve, and inline reserve are all expressible from the first
  attachment-bearing event — no future envelope change is needed to add previews, sealing, or
  small inline blobs.
- ADR-0041's forward reference to this shape is now backed by code; the two ADRs describe the
  identical primitive with no drift.
- The by-reference, unsealed common case stays minimal on the wire (`inline`/`seal` both absent).

**Harder / accepted costs, honest limits:**
- **Plaintext only.** `seal` ships reserved-and-`None`; per-blob DEK sealing and byte-tier key
  custody (ADR-0005 applied to blobs) is deferred to a later slice. No attachment can yet be
  crypto-shredded.
- **Single rendition, no preview.** The first consumer ships one `original` rendition; preview /
  thumbnail generation needs an image-processing dependency (AGPL-compatible, still to be
  selected) and is deferred. The rendition *set* is structurally ready; only one member exists.
- **Bytes stay local.** The reference replicates eagerly, as designed, but cross-node byte fetch
  (the [sync §6.6](../sync.md#66-attachments-the-lazy-byte-tier) lazy swarm/resumable tier) is
  existing POC machinery not yet wired into `cairn-node`; a receiving peer correctly renders
  *"referenced here — not yet retrieved,"* but remote retrieval of this shape's bytes is
  unexercised.
- **The frozen POC harness diverges.** `poc/walking-skeleton/harness/{spike_0002,agent_standin}.py`
  author `advisory.added` events with the flat pre-ADR-0042 `AttachmentRef` shape. `poc/` is
  frozen historical spikes, outside the workspace build and CI, so nothing breaks — but after
  this ADR their flat attachments no longer register a blob reference through
  `cairn_learn_attachment_refs`. Recorded as a known, accepted divergence, not fixed.

**How we would know the bet is failing:** a real attachment workflow needs a `Rendition` or
`Attachment` field this ADR did not reserve (forcing a second, harder envelope change against
live data); or the rendition-per-element floor walk (point 6) proves too costly on Pi-class
hardware for attachments with large rendition sets (mitigation: the walk is O(renditions), which
stays small — previews and extracts, not raw gigabytes, ride as separate renditions only in
metadata).

**Not a new founding principle.** This is [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)'s
day-one envelope reserve given a concrete, frozen encoding — principle 1 (content-addressing)
and principle 11 (additive-only, self-describing artifacts) applied to the exact byte layout,
plus the §9 one-writer-one-serialization discipline [ADR-0040](0040-signing-context-domain-separation.md)
already established for signatures.
