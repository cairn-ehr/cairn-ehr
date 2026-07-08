# Day-One Attachment-Reference Shape + First Photo Evidence — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finalize the §3.14 can't-retrofit attachment-reference shape in `cairn-event`, teach the in-DB floor to learn a blob reference per rendition, then wire the first photo evidence attachment onto a John-Doe chart through a new `identity.evidence.asserted` event.

**Architecture:** A three-type wire shape (`Attachment` → `Rendition` → `SealRef`) replaces the walking-skeleton `AttachmentRef` stub; it is the shared attachment primitive the (future) ADR-0041 note manifest will also build on. The submit + remote-apply floors learn references by walking `attachments[*].renditions[*]`. The photo rides the top-level `EventBody.attachments`; its bytes are stored as a present, self-verified local blob through the db/026 verify trigger; the event twin renders from descriptor fields, never pixels.

**Tech Stack:** Rust (`cairn-event` wire core, `cairn-node` glue), PostgreSQL ≥ 18 + `cairn_pgx` (pgrx) floor, `serde`/`ciborium` (CBOR), `tokio-postgres`, `clap` CLI.

## Global Constraints

- **License:** AGPL-3.0; every new dependency must be AGPL-3.0-compatible, checked *before* adding. (New dep this plan: `serde_bytes` — MIT OR Apache-2.0, compatible.)
- **TDD:** failing test first, then code. Load-bearing on the safety-critical surface (the shape, the floor SQL).
- **Inline docs for a junior contributor:** every non-trivial fn/module explains *why* it exists and *how* it fits, not just *what*.
- **File size:** aim < 500 LOC/file; split by responsibility.
- **Language by defect blast radius (§9):** the reference shape + floor SQL are safety-critical → Rust / in-DB, reviewer-legible. CLI/glue is fit-for-purpose.
- **Immutable wire commitment:** `Attachment`/`Rendition`/`SealRef` field order IS the canonical CBOR encoding — frozen by ADR-0042. `EventBody` field order is unchanged.
- **Spec version:** currently **0.42**; this slice bumps to **0.43** and adds **ADR-0042** (0041 is taken — progress-note format).
- **Test rigs:** pure Rust tests: `cargo test -p cairn-event`. DB-gated node tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx` ≥ 0.3.0); they self-serialize via a Postgres advisory lock, so `cargo test --workspace` is reliable.
- **Commit style:** conventional commits; end body with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

---

## File Structure

- `crates/cairn-event/src/attachment.rs` (new) — `Attachment`/`Rendition`/`SealRef` + pure constructors + `render_attachment_twin`. The wire shape.
- `crates/cairn-event/src/lib.rs` (modify) — `pub mod attachment;` + re-exports; `EventBody.attachments: Vec<Attachment>`; delete `AttachmentRef`; update two `LegacyBody` tests.
- `crates/cairn-event/Cargo.toml` (modify) — add `serde_bytes`.
- `crates/cairn-event/src/identity_evidence.rs` (new) — `photo_evidence_body` + `render_identity_evidence_twin` + type/schema/kind constants.
- `db/027_attachment_rendition_references.sql` (new) — `cairn_learn_attachment_refs(b)`.
- `db/005_submit.sql`, `db/020_apply_remote_event.sql` (modify) — inline learn-loop → helper call.
- `crates/cairn-node/src/db.rs` (modify) — register `027` in the `SCHEMA` array.
- `crates/cairn-node/src/photo_evidence.rs` (new) — `prepare_local_blob` (pure) + `build_photo_evidence_body` (pure) + `assert_photo_evidence` (orchestrator).
- `db/028_identity_evidence.sql` (new) — registers `identity.evidence.asserted` in the fail-closed `event_type_class` table (additive).
- `crates/cairn-node/src/lib.rs` (modify) — `pub mod photo_evidence;`.
- `crates/cairn-node/src/main.rs` (modify) — `AssertPhotoEvidence` CLI subcommand + handler.
- `crates/cairn-node/tests/attachment_refs.rs` (new) — DB-gated floor test.
- `crates/cairn-node/tests/photo_evidence.rs` (new) — DB-gated e2e.
- `docs/spec/decisions/0042-concrete-attachment-reference-shape.md` (new) — the ADR.
- `docs/spec/data-model.md` §3.14 (modify) + `docs/spec/index.md` (modify) — concrete-shape note + version bump.

---

## Phase 1 — the attachment-reference shape (cairn-event)

### Task 1: The `Attachment`/`Rendition`/`SealRef` shape module

**Files:**
- Create: `crates/cairn-event/src/attachment.rs`
- Modify: `crates/cairn-event/Cargo.toml` (add `serde_bytes`)
- Modify: `crates/cairn-event/src/lib.rs` (add `pub mod attachment;` + re-export)
- Test: inline `#[cfg(test)]` in `attachment.rs`

**Interfaces:**
- Consumes: `crate::blob_address(&[u8]) -> Vec<u8>` (multihash BLAKE3), `hex::encode`.
- Produces:
  - `pub struct Attachment { pub descriptor: String, pub renditions: Vec<Rendition> }`
  - `pub struct Rendition { pub role: String, pub alg: String, pub digest_hex: String, pub media_type: String, pub byte_len: i64, pub inline: Option<serde_bytes::ByteBuf>, pub seal: Option<SealRef> }`
  - `pub struct SealRef { pub alg: String, pub dek_wrap: String }`
  - `pub const RENDITION_ROLE_ORIGINAL: &str = "original";`
  - `pub const BLOB_ALG_BLAKE3: &str = "blake3";`
  - `Rendition::reference(role: &str, bytes: &[u8], media_type: &str) -> Rendition`
  - `Attachment::single(descriptor: &str, rendition: Rendition) -> Attachment`
  - `pub fn render_attachment_twin(a: &Attachment) -> String`

- [ ] **Step 1: Add the dependency (verify license first)**

Confirm `serde_bytes` is MIT OR Apache-2.0 (AGPL-compatible), then add to `crates/cairn-event/Cargo.toml` under `[dependencies]`, after the `hex = "0.4"` line:

```toml
serde_bytes = "0.11"       # CBOR byte-string encoding for reserved inline blob bytes
```

- [ ] **Step 2: Write the failing tests**

Create `crates/cairn-event/src/attachment.rs` with only the test module referencing not-yet-written items (it will fail to compile — that is the red state):

```rust
//! §3.14 attachment-reference shape (ADR-0013, concrete encoding frozen by ADR-0042).
//!
//! One logical binary clinical artifact is referenced by the signed event (EAGER) while
//! its bytes live on the §6.6 lazy byte tier. This shape is the ONE can't-retrofit piece
//! of ADR-0013 — every field is a day-one envelope reserve. The field ORDER here is the
//! canonical CBOR encoding (structural move 1: one writer, one serialization), so it must
//! never be reordered once the first attachment-bearing event is signed.
//!
//! Shared primitive: the (future) ADR-0041 progress-note `payload.media` manifest builds on
//! `Attachment`/`Rendition` too (adding a note-local `id` + inline anchor binding). Defining
//! the shape once here keeps the wire commitment single-source.

use serde::{Deserialize, Serialize};

/// The role of a rendition within its attachment's rendition set.
pub const RENDITION_ROLE_ORIGINAL: &str = "original";
/// The multihash digest algorithm for blob content addresses (§4.4).
pub const BLOB_ALG_BLAKE3: &str = "blake3";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob_address;

    #[test]
    fn reference_rendition_carries_the_blake3_multihash_of_the_bytes() {
        let r = Rendition::reference(RENDITION_ROLE_ORIGINAL, b"hello", "text/plain");
        assert_eq!(r.role, "original");
        assert_eq!(r.alg, "blake3");
        assert_eq!(r.digest_hex, hex::encode(blob_address(b"hello")));
        assert_eq!(r.media_type, "text/plain");
        assert_eq!(r.byte_len, 5);
        assert!(r.inline.is_none(), "a by-reference rendition inlines no bytes");
        assert!(r.seal.is_none(), "a plaintext rendition carries no seal");
    }

    #[test]
    fn single_wraps_exactly_one_rendition_under_a_descriptor() {
        let r = Rendition::reference(RENDITION_ROLE_ORIGINAL, b"x", "image/jpeg");
        let a = Attachment::single("frontal face photograph", r);
        assert_eq!(a.descriptor, "frontal face photograph");
        assert_eq!(a.renditions.len(), 1);
        assert_eq!(a.renditions[0].media_type, "image/jpeg");
    }

    #[test]
    fn twin_renders_from_descriptor_and_rendition_summary_never_bytes() {
        let r = Rendition::reference(RENDITION_ROLE_ORIGINAL, b"PIXELDATA", "image/jpeg");
        let a = Attachment::single("wound on left forearm", r);
        let twin = render_attachment_twin(&a);
        assert!(twin.contains("wound on left forearm"), "descriptor must appear: {twin}");
        assert!(twin.contains("image/jpeg"), "media type must appear: {twin}");
        assert!(!twin.contains("PIXELDATA"), "twin must NOT contain pixel bytes: {twin}");
        assert!(!twin.trim().is_empty());
    }

    #[test]
    fn shape_expresses_all_five_reserves_and_round_trips() {
        // A fully-populated rendition exercising inline + seal, proving the shape can carry
        // every §3.14 day-one reserve and survives a CBOR round-trip unchanged.
        let sealed = Rendition {
            role: "original".into(),
            alg: "blake3".into(),
            digest_hex: hex::encode(blob_address(b"ciphertext")), // sealed → ciphertext hash
            media_type: "image/jpeg".into(),
            byte_len: 10,
            inline: Some(serde_bytes::ByteBuf::from(b"inlinebytes".to_vec())),
            seal: Some(SealRef { alg: "xchacha20poly1305".into(), dek_wrap: "dek-ref-1".into() }),
        };
        let a = Attachment { descriptor: "d".into(), renditions: vec![sealed] };
        let mut buf = Vec::new();
        ciborium::into_writer(&a, &mut buf).unwrap();
        let back: Attachment = ciborium::from_reader(&buf[..]).unwrap();
        assert_eq!(a, back);
        assert!(back.renditions[0].seal.is_some());
        assert!(back.renditions[0].inline.is_some());
    }

    #[test]
    fn by_reference_rendition_omits_inline_and_seal_keys_on_the_wire() {
        // skip_serializing_if keeps the common (unsealed, by-reference) case minimal: the
        // CBOR map must NOT contain "inline" or "seal" keys.
        let r = Rendition::reference(RENDITION_ROLE_ORIGINAL, b"x", "image/jpeg");
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("inline").is_none(), "None inline must be omitted, not null");
        assert!(v.get("seal").is_none(), "None seal must be omitted, not null");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail (do not compile)**

Run: `cargo test -p cairn-event attachment`
Expected: FAIL — `cannot find type Rendition` / `Attachment` / `SealRef` / `render_attachment_twin`.

- [ ] **Step 4: Write the implementation**

Add, above the `#[cfg(test)]` block in `attachment.rs`:

```rust
/// Seal indicator / DEK-wrap reference — the day-one reserve that makes a blob
/// crypto-shreddable later (ADR-0005 key custody). Absent (`None`) means plaintext.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealRef {
    /// Seal algorithm (e.g. "xchacha20poly1305", mirroring the keystore's `seal.rs`).
    pub alg: String,
    /// Reference to the wrapped per-blob DEK: where to obtain the key to decrypt, and
    /// what is crypto-shredded to erase the bytes.
    pub dek_wrap: String,
}

/// One content-addressed rendition within an attachment's rendition set: the original
/// bytes, a preview, extracted report text, … Each has its own address + sync priority.
/// FIELD ORDER IS FROZEN (ADR-0042) — it is the canonical CBOR encoding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rendition {
    /// Role in the set: "original" | "preview" | "extracted-text" | … (open string).
    pub role: String,
    /// Multihash digest algorithm ("blake3") — self-describing, so a future algorithm is
    /// an additive migration while this digest stays fixed.
    pub alg: String,
    /// Hex of the multihash content address. Names the PLAINTEXT bytes when `seal` is
    /// None; the CIPHERTEXT bytes when sealed (no convergent encryption — §3.14).
    pub digest_hex: String,
    pub media_type: String,
    pub byte_len: i64,
    /// Inline-vs-reference distinction. `Some(bytes)` = inlined on the eager plane (tiny
    /// blobs below a node threshold); `None` = by-reference on the lazy byte tier. Omitted
    /// from the wire when None, so a by-reference rendition encodes minimally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<serde_bytes::ByteBuf>,
    /// Seal indicator / DEK-wrap reference (above). Omitted from the wire when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<SealRef>,
}

/// One logical binary clinical artifact, referenced by the signed event. The reference is
/// EAGER (rides the signed body); the bytes are LAZY (the §6.6 byte tier).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    /// Clear-text, always-legible descriptor (media-independent): what this is, in words.
    /// Feeds the event twin + the §5.9 safety projection even when bytes are sealed/absent.
    pub descriptor: String,
    /// The rendition set: N content-addressed views of the SAME logical artifact. A photo
    /// ships a single `original` today; the set is reserved for previews/extracts later.
    pub renditions: Vec<Rendition>,
}

impl Rendition {
    /// Build a by-reference, plaintext "original"-style rendition for `bytes`: computes the
    /// BLAKE3 multihash content address and byte length. Pure — no I/O, no DB. `inline` and
    /// `seal` are None (bytes ride the lazy tier; plaintext). Used for the common case; a
    /// sealed or inlined rendition is constructed field-by-field.
    pub fn reference(role: &str, bytes: &[u8], media_type: &str) -> Rendition {
        Rendition {
            role: role.to_string(),
            alg: BLOB_ALG_BLAKE3.to_string(),
            digest_hex: hex::encode(crate::blob_address(bytes)),
            media_type: media_type.to_string(),
            byte_len: bytes.len() as i64,
            inline: None,
            seal: None,
        }
    }
}

impl Attachment {
    /// The one-rendition attachment (a photo's `original`). Pure.
    pub fn single(descriptor: &str, rendition: Rendition) -> Attachment {
        Attachment { descriptor: descriptor.to_string(), renditions: vec![rendition] }
    }
}

/// Render an attachment's legibility fragment from its DESCRIPTOR FIELDS ONLY, never its
/// bytes (§3.14: the twin is not derived from pixels). One line per rendition summarising
/// role + media type + size. This is what a text-only node, a screen reader, and the RAG
/// substrate see for the attachment.
pub fn render_attachment_twin(a: &Attachment) -> String {
    let mut out = a.descriptor.clone();
    for r in &a.renditions {
        out.push_str(&format!(" ({}: {}, {} bytes)", r.role, r.media_type, r.byte_len));
    }
    out
}
```

Then wire the module in `crates/cairn-event/src/lib.rs`. Add near the other `pub mod` lines (after `pub mod john_doe;`):

```rust
pub mod attachment;
```

and add a re-export near `pub use ed25519_dalek::…` (line ~30):

```rust
pub use attachment::{Attachment, Rendition, SealRef};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cairn-event attachment`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/attachment.rs crates/cairn-event/src/lib.rs crates/cairn-event/Cargo.toml crates/cairn-event/Cargo.lock
git commit -m "$(printf 'feat(event): the day-one attachment-reference shape (Attachment/Rendition/SealRef)\n\nSatisfies all five §3.14 day-one reserves: content digest, descriptor metadata,\nrendition set (nested), seal indicator, inline-vs-reference. Pure constructors +\ntwin fragment (from descriptor, never pixels). Field order frozen by ADR-0042.\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

### Task 2: Swap `EventBody.attachments` to `Vec<Attachment>` and prove byte-stability

**Files:**
- Modify: `crates/cairn-event/src/lib.rs` (delete `AttachmentRef`; change field type; update two `LegacyBody` tests; add a stability test)
- Test: inline `#[cfg(test)]` in `lib.rs`

**Interfaces:**
- Consumes: `Attachment` (Task 1).
- Produces: `EventBody.attachments: Vec<Attachment>` (all downstream `attachments: vec![]` sites keep compiling — the element type is inferred).

- [ ] **Step 1: Write the failing stability test**

Add to the `#[cfg(test)] mod tests` in `crates/cairn-event/src/lib.rs`:

```rust
#[test]
fn event_with_one_attachment_round_trips_through_canonical_cbor() {
    let r = crate::attachment::Rendition::reference("original", b"jpegbytes", "image/jpeg");
    let att = crate::attachment::Attachment::single("id photo", r);
    let body = EventBody {
        event_id: "e".into(), patient_id: "p".into(),
        event_type: "identity.evidence.asserted".into(),
        schema_version: "identity.evidence.asserted/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None, signer_key_id: "k".into(),
        contributors: serde_json::json!([]), payload: serde_json::json!({"kind":"photo"}),
        attachments: vec![att.clone()], plaintext_twin: Some("t".into()),
    };
    let bytes = canonical_cbor(&body).unwrap();
    let back: EventBody = ciborium::from_reader(&bytes[..]).unwrap();
    assert_eq!(back.attachments, vec![att]);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cairn-event event_with_one_attachment`
Expected: FAIL to compile — `attachments: vec![att]` is a `Vec<Attachment>` but the field is still `Vec<AttachmentRef>`.

- [ ] **Step 3: Delete `AttachmentRef` and change the field type**

In `crates/cairn-event/src/lib.rs`:

1. Delete the entire `AttachmentRef` struct definition (the `/// A §3.14 attachment reference…` doc comment + `#[derive(...)] pub struct AttachmentRef { … }`, lines ~220-229).
2. Change the `EventBody` field (line ~246) from:

```rust
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
```

to:

```rust
    #[serde(default)]
    pub attachments: Vec<Attachment>,
```

3. In the two `LegacyBody` test structs (the `twin_none_encodes_byte_identically` and `legacy_bytes_decode_with_twin_none` tests, ~lines 930 and 966), change both occurrences of:

```rust
            payload: &'a serde_json::Value, attachments: &'a Vec<AttachmentRef>,
```
to
```rust
            payload: &'a serde_json::Value, attachments: &'a Vec<Attachment>,
```

and both occurrences of:

```rust
        let attachments: Vec<AttachmentRef> = vec![];
```
to
```rust
        let attachments: Vec<Attachment> = vec![];
```

(These two existing tests already assert an empty-attachments body encodes byte-identically to the pre-twin-field struct — updating the element type keeps them proving that the shape swap changed no existing bytes.)

- [ ] **Step 4: Run the full cairn-event suite to verify it passes**

Run: `cargo test -p cairn-event`
Expected: PASS (including `twin_none_encodes_byte_identically`, `legacy_bytes_decode_with_twin_none`, and the new round-trip test).

- [ ] **Step 5: Verify the whole workspace still compiles (all `vec![]` sites)**

Run: `cargo build --workspace`
Expected: SUCCESS — every `attachments: vec![]` site in `cairn-node`/`cairn-sync` infers `Vec<Attachment>` unchanged.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/lib.rs
git commit -m "$(printf 'feat(event)!: EventBody.attachments is now Vec<Attachment>; remove AttachmentRef stub\n\nEmpty-vec encodes identically regardless of element type, so every existing\nzero-attachment event keeps its content-address (proven by the byte-identity\ntest). All attachments: vec![] sites infer the new type unchanged.\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

### Task 3: ADR-0042 + spec §3.14 concrete-shape note + version bump

**Files:**
- Create: `docs/spec/decisions/0042-concrete-attachment-reference-shape.md`
- Modify: `docs/spec/data-model.md` (§3.14 — add a concrete-shape paragraph)
- Modify: `docs/spec/index.md` (version 0.42 → 0.43)

**Interfaces:** none (docs).

- [ ] **Step 1: Write ADR-0042**

Create `docs/spec/decisions/0042-concrete-attachment-reference-shape.md`. Follow the house ADR format (see `0040-signing-context-domain-separation.md` for style). Required content:

- Header: `# ADR-0042 — The concrete attachment-reference shape (Attachment / Rendition / SealRef)`; Status: Accepted; Date: 2026-07-08; **Refines:** ADR-0013 (attachment reference shape); **Reconciles with:** ADR-0041 (progress-note media manifest).
- **Context:** ADR-0013/§3.14 settled the day-one *requirements* (five reserves) but not the concrete encoding; ADR-0041 already referenced this shape for the note manifest while the code was still the walking-skeleton `AttachmentRef` stub; the first photo evidence forces finalizing it. No production event carries an attachment yet — the free moment.
- **Decision:** the three-type shape (quote the Rust struct field order verbatim — it IS the canonical CBOR encoding). State which fields are *structurally* can't-retrofit (`renditions: Vec`) vs additive-but-reserved-now (`seal`, `inline` as `Option` + skip_serializing_if). Digest names plaintext-hash unsealed / ciphertext-hash sealed. `EventBody.attachments` carries attachments on non-narrative events; the ADR-0041 note `payload.media` manifest is the same primitive plus a note-local `id` + anchor binding.
- **Floor consequence:** the submit + remote-apply doors learn a lazy blob reference per *rendition* (`cairn_learn_attachment_refs`, db/027), skipping inline renditions.
- **Consequences / honest limits:** ships plaintext (seal reserved), single-rendition (no preview), bytes local (no cross-node fetch); the frozen POC harness diverges.

- [ ] **Step 2: Add the §3.14 concrete-shape note**

In `docs/spec/data-model.md`, at the end of the §3.14 section (after "The day-one envelope reserve." paragraph), add one paragraph:

```markdown
> **Concrete shape ([ADR-0042](decisions/0042-concrete-attachment-reference-shape.md)).** The reference is `Attachment { descriptor, renditions: [Rendition{ role, alg, digest_hex, media_type, byte_len, inline?, seal? }] }`, with `seal` a `SealRef { alg, dek_wrap }`. The rendition set is nested from day one (structurally can't-retrofit); `seal` and `inline` are reserved (omitted from the wire when absent). Attachments on non-narrative events ride the signed `EventBody.attachments`; the [ADR-0041](decisions/0041-progress-note-narrative-format.md) note `payload.media` manifest is the same primitive plus a note-local `id`.
```

- [ ] **Step 3: Bump the spec version**

In `docs/spec/index.md`, change `**Spec version:** 0.42` to `**Spec version:** 0.43`.

- [ ] **Step 4: Verify the docs build**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: builds without warnings about the new ADR link. (If `uv`/mkdocs is unavailable in the environment, at minimum confirm the internal links resolve by grepping the new filenames.)

- [ ] **Step 5: Commit**

```bash
git add docs/spec/decisions/0042-concrete-attachment-reference-shape.md docs/spec/data-model.md docs/spec/index.md
git commit -m "$(printf 'docs(spec): ADR-0042 — concrete attachment-reference shape (spec 0.42 -> 0.43)\n\nFreezes the Attachment/Rendition/SealRef field order as the canonical encoding;\nrefines ADR-0013, reconciles with ADR-0041 (shared primitive, note adds id).\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Phase 1b — the floor learns the rendition set (db/)

### Task 4: `cairn_learn_attachment_refs` + wire it into both doors

**Files:**
- Create: `db/027_attachment_rendition_references.sql`
- Modify: `db/005_submit.sql` (replace inline learn-loop)
- Modify: `db/020_apply_remote_event.sql` (replace inline learn-loop)
- Modify: `crates/cairn-node/src/db.rs` (register `027` in `SCHEMA`)
- Test: `crates/cairn-node/tests/attachment_refs.rs` (new, DB-gated)

**Interfaces:**
- Consumes: `blob_note_reference(addr BYTEA, mt TEXT, len BIGINT)` (db/003).
- Produces: `cairn_learn_attachment_refs(b jsonb) RETURNS void`.

- [ ] **Step 1: Write the failing DB-gated test**

Create `crates/cairn-node/tests/attachment_refs.rs`:

```rust
//! §3.14/ADR-0042 floor coverage: the submit door learns a lazy blob reference for every
//! BY-REFERENCE rendition of an event's attachments (reference-eager, byte-lazy), skipping
//! inline renditions. Real Postgres, gated on $CAIRN_TEST_PG.

use cairn_event::attachment::{Attachment, Rendition};
use cairn_event::{blob_address, generate_key, sign, EventBody, Hlc};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, blob_store, blob_chunk CASCADE").await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    (sk, kid)
}

/// Author a note.added-style event carrying `attachments`, through the real submit door.
async fn submit_with_attachments(
    db_: &mut Client, sk: &cairn_event::SigningKey, kid: &str, node: &str, atts: Vec<Attachment>,
) {
    let h = db::next_hlc(db_, node).await.unwrap();
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "note.added".into(),          // registered fail-closed type, allows attachments
        schema_version: "advisory/1".into(),
        hlc: h, t_effective: None, signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "see attachment"}),
        attachments: atts,
        plaintext_twin: Some("note with attachment".into()),
    };
    let signed = sign(&body, sk).unwrap();
    db_.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap();
}

#[tokio::test]
async fn by_reference_rendition_registers_a_blob_reference_row() {
    let Some(base) = cs() else { eprintln!("skip: no CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();  // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let r = Rendition::reference("original", b"jpegbytes", "image/jpeg");
    let att = Attachment::single("id photo", r);
    submit_with_attachments(&mut c, &sk, &kid, "n", vec![att]).await;

    let addr = blob_address(b"jpegbytes");
    let row = c.query_one(
        "SELECT media_type, byte_len, present FROM blob_store WHERE blob_address = $1", &[&addr])
        .await.unwrap();
    let mt: String = row.get(0);
    let len: i64 = row.get(1);
    let present: bool = row.get(2);
    assert_eq!(mt, "image/jpeg");
    assert_eq!(len, 9);
    assert!(!present, "reference-eager, byte-lazy: the row is a reference only");
}

#[tokio::test]
async fn inline_rendition_registers_no_blob_reference() {
    let Some(base) = cs() else { eprintln!("skip: no CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();  // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let mut r = Rendition::reference("original", b"tiny", "image/png");
    r.inline = Some(serde_bytes::ByteBuf::from(b"tiny".to_vec())); // bytes ride the event
    let att = Attachment::single("tiny inline sketch", r);
    submit_with_attachments(&mut c, &sk, &kid, "n", vec![att]).await;

    let n: i64 = c.query_one("SELECT count(*) FROM blob_store", &[]).await.unwrap().get(0);
    assert_eq!(n, 0, "an inline rendition's bytes are in the event; no lazy blob reference");
}
```

Note: `cairn-node`'s dev-deps must include `serde_bytes` for the inline test. Add `serde_bytes = "0.11"` under `[dev-dependencies]` in `crates/cairn-node/Cargo.toml` (same license check; already vetted in Task 1).

- [ ] **Step 2: Run it to verify it fails**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test attachment_refs`
Expected: FAIL — with the current flat-shape loop, `by_reference_…` finds no `blob_store` row (the loop read `digest_hex` off the attachment, which is now absent), or the submit errors. Confirms the floor needs the rendition walk.

- [ ] **Step 3: Create the helper migration**

Create `db/027_attachment_rendition_references.sql`:

```sql
-- Cairn — the attachment floor learns the RENDITION SET (ADR-0042, refines ADR-0013).
--
-- Before ADR-0042 the attachment reference was flat: digest_hex/media_type/byte_len sat
-- on each attachment, and the submit/apply doors learned one blob reference per attachment.
-- ADR-0042 nests those under a rendition set (one logical attachment = N content-addressed
-- renditions), so the doors must learn a reference per BY-REFERENCE rendition. Extracted
-- into one shared helper so the two doors (db/005 submit, db/020 remote-apply) never drift
-- (the single-source discipline db/015 used for the twin hook).

BEGIN;

-- Learn a lazy blob reference (reference-eager, byte-lazy) for every by-reference rendition
-- of every attachment in a signed body `b`. Skips INLINE renditions: their bytes ride the
-- event itself, so there is no lazy blob to fetch (noting one would create a phantom
-- present=FALSE row that never resolves). Idempotent via blob_note_reference's ON CONFLICT.
CREATE OR REPLACE FUNCTION cairn_learn_attachment_refs(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    a jsonb;
    r jsonb;
BEGIN
    FOR a IN SELECT jsonb_array_elements(COALESCE(b -> 'attachments', '[]'::jsonb)) LOOP
        FOR r IN SELECT jsonb_array_elements(COALESCE(a -> 'renditions', '[]'::jsonb)) LOOP
            -- Inline renditions carry their bytes in the event; no lazy blob reference.
            CONTINUE WHEN r ? 'inline';
            PERFORM blob_note_reference(
                decode(r ->> 'digest_hex', 'hex'),
                r ->> 'media_type',
                (r ->> 'byte_len')::bigint);
        END LOOP;
    END LOOP;
END;
$$;

COMMIT;
```

- [ ] **Step 4: Replace the inline loop in both doors**

In `db/005_submit.sql`, replace the block (lines ~216-220):

```sql
    -- Learn any attachment references (reference-eager, byte-lazy).
    FOR c IN SELECT * FROM jsonb_array_elements(COALESCE(b -> 'attachments','[]'::jsonb)) LOOP
        PERFORM blob_note_reference(decode(c ->> 'digest_hex','hex'), c ->> 'media_type',
                                    (c ->> 'byte_len')::bigint);
    END LOOP;
```

with:

```sql
    -- Learn any attachment references, per rendition (reference-eager, byte-lazy).
    -- Shared with the remote-apply door via cairn_learn_attachment_refs (db/027) so the
    -- two doors never drift.
    PERFORM cairn_learn_attachment_refs(b);
```

If the surrounding `DECLARE` block declared `c jsonb` (or similar) only for this loop, remove that now-unused declaration to avoid a warning. (Grep the function's DECLARE for `c ` and check no other use before removing.)

Apply the identical replacement in `db/020_apply_remote_event.sql` at its mirror loop (~line 231-235).

- [ ] **Step 5: Register db/027 in the schema loader**

In `crates/cairn-node/src/db.rs`, in the `SCHEMA` array, after the `026` entry, add:

```rust
    ("027_attachment_rendition_references", include_str!("../../../db/027_attachment_rendition_references.sql")),
```

(PL/pgSQL late-binding: db/005 and db/020 reference `cairn_learn_attachment_refs` before db/027 defines it; all migrations load before any submit, so runtime resolution succeeds.)

- [ ] **Step 6: Run the floor tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test attachment_refs`
Expected: PASS (both tests).

- [ ] **Step 7: Run the broader node suite to confirm no regression**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node`
Expected: PASS (existing attachment-free events are unaffected; the advisory length-check is untouched).

- [ ] **Step 8: Commit**

```bash
git add db/027_attachment_rendition_references.sql db/005_submit.sql db/020_apply_remote_event.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/attachment_refs.rs crates/cairn-node/Cargo.toml
git commit -m "$(printf 'feat(db)!: attachment floor learns the rendition set (ADR-0042)\n\nboth submit + remote-apply doors now walk attachments[*].renditions[*] via the\nshared cairn_learn_attachment_refs helper (db/027), skipping inline renditions.\nWithout it a rendition-shaped attachment would call blob_note_reference(NULL).\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Phase 2 — the photo author path

### Task 5: `identity.evidence.asserted` body + twin (cairn-event)

**Files:**
- Create: `crates/cairn-event/src/identity_evidence.rs`
- Modify: `crates/cairn-event/src/lib.rs` (`pub mod identity_evidence;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::attachment::Attachment`, `crate::attachment::render_attachment_twin`, `crate::evidence::CLINICIAN_OBSERVED_PROVENANCE`.
- Produces:
  - `pub const IDENTITY_EVIDENCE_EVENT_TYPE: &str = "identity.evidence.asserted";`
  - `pub const IDENTITY_EVIDENCE_SCHEMA_VERSION: &str = "identity.evidence.asserted/1";`
  - `pub const PHOTO_EVIDENCE_KIND: &str = "photo";`
  - `pub fn photo_evidence_body(basis: Option<&str>) -> serde_json::Value`
  - `pub fn render_identity_evidence_twin(kind: &str, basis: Option<&str>, attachment: &Attachment) -> String`

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-event/src/identity_evidence.rs`:

```rust
//! §5.4 identity evidence — clinician-observed evidence about an unidentified patient that
//! is NOT a demographic field: a photograph today; distinguishing marks / belongings / EMS
//! pickup context as future `kind` values. A distinct, non-demographic event type
//! (`identity.evidence.asserted`) whose attachment (if any) rides the top-level
//! `EventBody.attachments`. The twin renders from the descriptor, never the bytes (§3.14).

use crate::attachment::{render_attachment_twin, Attachment};
use serde_json::{json, Value};

/// The event type for clinician-observed identity evidence. Non-demographic: the db/015
/// twin floor carries its authored twin verbatim (no floor branch needed).
pub const IDENTITY_EVIDENCE_EVENT_TYPE: &str = "identity.evidence.asserted";
/// schema_version for identity-evidence assertions.
pub const IDENTITY_EVIDENCE_SCHEMA_VERSION: &str = "identity.evidence.asserted/1";
/// The `kind` discriminator for a photograph (future kinds: mark, belongings, ems-context).
pub const PHOTO_EVIDENCE_KIND: &str = "photo";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::Rendition;

    #[test]
    fn photo_body_is_a_clinician_observed_photo_kind_with_optional_basis() {
        let with = photo_evidence_body(Some("photographed on arrival for identification"));
        assert_eq!(with["kind"], "photo");
        assert_eq!(with["provenance"], "clinician-observed");
        assert_eq!(with["basis"], "photographed on arrival for identification");

        let without = photo_evidence_body(None);
        assert_eq!(without["kind"], "photo");
        assert!(without.get("basis").is_none(), "absent basis is omitted, never null");
    }

    #[test]
    fn twin_is_legible_from_descriptor_and_names_the_kind_never_bytes() {
        let r = Rendition::reference("original", b"PIXELS", "image/jpeg");
        let att = Attachment::single("frontal face photograph", r);
        let twin = render_identity_evidence_twin("photo", Some("on arrival"), &att);
        assert!(twin.contains("photo"), "twin names the kind: {twin}");
        assert!(twin.contains("frontal face photograph"), "descriptor: {twin}");
        assert!(twin.contains("image/jpeg"));
        assert!(twin.contains("on arrival"), "basis when present: {twin}");
        assert!(!twin.contains("PIXELS"));
        assert!(!twin.trim().is_empty());
    }

    #[test]
    fn twin_omits_the_basis_clause_when_absent() {
        let r = Rendition::reference("original", b"x", "image/jpeg");
        let att = Attachment::single("photo", r);
        let twin = render_identity_evidence_twin("photo", None, &att);
        assert!(!twin.contains(" — "), "no trailing basis separator when basis is None: {twin}");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cairn-event identity_evidence`
Expected: FAIL — `photo_evidence_body` / `render_identity_evidence_twin` undefined.

- [ ] **Step 3: Write the implementation**

Add above the test module in `identity_evidence.rs`:

```rust
/// Build the payload for a photo identity-evidence event: `{ kind, provenance, basis? }`.
/// The photograph itself is NOT in the payload — it rides the top-level
/// `EventBody.attachments` as an `Attachment` (ADR-0042); the payload is the clinical
/// framing. `basis` (how/why observed) is optional and omitted entirely when None
/// (principle 4: never manufacture a basis).
pub fn photo_evidence_body(basis: Option<&str>) -> Value {
    let mut body = json!({
        "kind": PHOTO_EVIDENCE_KIND,
        "provenance": crate::evidence::CLINICIAN_OBSERVED_PROVENANCE,
    });
    if let Some(b) = basis {
        body["basis"] = json!(b);
    }
    body
}

/// Render the authored §3.13/§4.5 twin for an identity-evidence event: names the kind,
/// then the attachment's own descriptor-derived twin (never the bytes), then the basis if
/// stated. This is a pure mechanical derivation (ADR-0039 pattern) the db/015 floor carries
/// verbatim for this non-demographic type.
pub fn render_identity_evidence_twin(kind: &str, basis: Option<&str>, attachment: &Attachment) -> String {
    let mut out = format!("identity evidence ({kind}): {}", render_attachment_twin(attachment));
    if let Some(b) = basis {
        out.push_str(&format!(" — {b}"));
    }
    out
}
```

Wire the module in `crates/cairn-event/src/lib.rs` (after `pub mod identity;`):

```rust
pub mod identity_evidence;
```

Confirm `crate::evidence::CLINICIAN_OBSERVED_PROVENANCE` is `pub` (it is — `evidence.rs:28`).

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p cairn-event identity_evidence`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/identity_evidence.rs crates/cairn-event/src/lib.rs
git commit -m "$(printf 'feat(event): identity.evidence.asserted body + twin (§5.4 photo evidence)\n\nNon-demographic evidence event; photo rides EventBody.attachments, payload is\nkind+provenance+basis; twin from descriptor, never pixels. kind future-homes\nmarks/belongings/ems-context.\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

### Task 6: `prepare_local_blob` + `build_photo_evidence_body` + `assert_photo_evidence` (cairn-node)

**Files:**
- Create: `crates/cairn-node/src/photo_evidence.rs`
- Create: `db/028_identity_evidence.sql` (register the new type — the floor is fail-closed)
- Modify: `crates/cairn-node/src/db.rs` (register `028` in `SCHEMA`)
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod photo_evidence;`)
- Test: inline `#[cfg(test)]` for the pure parts; the DB e2e is Task 8.

**Interfaces:**
- Consumes: `cairn_event::attachment::{Attachment, Rendition}`, `cairn_event::identity_evidence::*`, `cairn_event::{blob_address, blob_outboard, sign, EventBody, Hlc, SigningKey}`, `crate::db::next_hlc`.

- [ ] **Step 0: Register the event type (the floor is fail-closed on unknown types)**

Create `db/028_identity_evidence.sql`:

```sql
-- Cairn — register the §5.4 identity-evidence event type (ADR-0042 photo evidence slice).
--
-- submit_event fails closed on an unregistered event_type (db/005), so a new type must add
-- its classification row before any node can author it. identity.evidence.asserted is
-- ADDITIVE (clinician-observed evidence never suppresses another author's content) and does
-- NOT target another author. No structural twin-floor branch is needed: it is non-demographic,
-- so db/015's cairn_event_twin carries its authored twin verbatim (ADR-0039).

BEGIN;

INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.evidence.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

COMMIT;
```

Register it in `crates/cairn-node/src/db.rs`'s `SCHEMA` array, after the `027` entry:

```rust
    ("028_identity_evidence", include_str!("../../../db/028_identity_evidence.sql")),
```
- Produces:
  - `pub struct LocalBlob { pub addr: Vec<u8>, pub outboard: Vec<u8>, pub rendition: Rendition }`
  - `pub fn prepare_local_blob(bytes: &[u8], media_type: &str) -> LocalBlob`
  - `pub fn build_photo_evidence_body(event_id: Uuid, patient_id: Uuid, kid: &str, hlc: Hlc, basis: Option<&str>, attachment: Attachment) -> EventBody`
  - `pub async fn assert_photo_evidence(client: &mut Client, sk: &SigningKey, kid: &str, node_origin: &str, patient_id: Uuid, bytes: &[u8], media_type: &str, descriptor: &str, basis: Option<&str>) -> anyhow::Result<Uuid>`

- [ ] **Step 1: Write the failing pure tests**

Create `crates/cairn-node/src/photo_evidence.rs`:

```rust
//! §5.4 photo identity-evidence author path. Splits into pure helpers (unit-tested here) and
//! an async orchestrator (`assert_photo_evidence`, e2e-tested in tests/photo_evidence.rs):
//!   * `prepare_local_blob` — pure: content address + bao outboard + the "original" Rendition.
//!   * `build_photo_evidence_body` — pure: assemble the signed EventBody (payload + the
//!     top-level attachment + the authored twin).
//!   * `assert_photo_evidence` — store the bytes present (through the db/026 verify floor) and
//!     author the event in ONE transaction, so bytes + reference land atomically.

use cairn_event::attachment::{Attachment, Rendition, RENDITION_ROLE_ORIGINAL};
use cairn_event::identity_evidence::{
    photo_evidence_body, render_identity_evidence_twin, IDENTITY_EVIDENCE_EVENT_TYPE,
    IDENTITY_EVIDENCE_SCHEMA_VERSION, PHOTO_EVIDENCE_KIND,
};
use cairn_event::{blob_address, blob_outboard, sign, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// A locally-prepared blob ready to INSERT present=TRUE plus the Rendition that references it.
pub struct LocalBlob {
    pub addr: Vec<u8>,      // multihash BLAKE3 content address (blob_store PK)
    pub outboard: Vec<u8>,  // bao verified-streaming tree (serves slices; stored with content)
    pub rendition: Rendition,
}

/// PURE: compute the content address, bao outboard, and the by-reference "original"
/// Rendition for `bytes`. No DB, so it is unit-testable; the orchestrator does the INSERT.
pub fn prepare_local_blob(bytes: &[u8], media_type: &str) -> LocalBlob {
    LocalBlob {
        addr: blob_address(bytes),
        outboard: blob_outboard(bytes),
        rendition: Rendition::reference(RENDITION_ROLE_ORIGINAL, bytes, media_type),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc { Hlc { wall: 7, counter: 0, node_origin: "n".into() } }

    #[test]
    fn prepare_local_blob_addresses_and_renders_the_original_rendition() {
        let lb = prepare_local_blob(b"jpegbytes", "image/jpeg");
        assert_eq!(lb.addr, blob_address(b"jpegbytes"));
        assert_eq!(lb.rendition.role, "original");
        assert_eq!(lb.rendition.digest_hex, hex::encode(blob_address(b"jpegbytes")));
        assert_eq!(lb.rendition.byte_len, 9);
        assert!(!lb.outboard.is_empty());
    }

    #[test]
    fn body_is_the_identity_evidence_event_with_the_photo_in_top_level_attachments() {
        let pid = Uuid::now_v7();
        let eid = Uuid::now_v7();
        let att = Attachment::single(
            "frontal face photograph",
            prepare_local_blob(b"x", "image/jpeg").rendition,
        );
        let body = build_photo_evidence_body(eid, pid, "kid", hlc(), Some("on arrival"), att.clone());

        assert_eq!(body.event_type, IDENTITY_EVIDENCE_EVENT_TYPE);
        assert_eq!(body.schema_version, IDENTITY_EVIDENCE_SCHEMA_VERSION);
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["kind"], PHOTO_EVIDENCE_KIND);
        assert_eq!(body.payload["basis"], "on arrival");
        assert_eq!(body.attachments, vec![att.clone()], "photo rides top-level attachments");
        // additive event → recorded role, no attestation demanded
        assert_eq!(body.contributors[0]["role"], "recorded");
        // authored twin, legible, no pixel bytes
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(twin.contains("frontal face photograph"));
        assert_eq!(twin, &render_identity_evidence_twin("photo", Some("on arrival"), &att));
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cairn-node --lib photo_evidence`
Expected: FAIL — `build_photo_evidence_body` / `assert_photo_evidence` undefined (compile error).

- [ ] **Step 3: Write `build_photo_evidence_body` + `assert_photo_evidence`**

Add above the test module in `photo_evidence.rs`:

```rust
/// PURE: assemble the signed `EventBody` for a photo identity-evidence event. The photo
/// (`attachment`) rides the top-level `EventBody.attachments` (ADR-0042); the payload carries
/// the clinical framing (kind/provenance/basis); the twin is authored from the descriptor.
/// The sole contributor is the recording actor with role `recorded` — additive, no attestation.
pub fn build_photo_evidence_body(
    event_id: Uuid,
    patient_id: Uuid,
    kid: &str,
    hlc: Hlc,
    basis: Option<&str>,
    attachment: Attachment,
) -> EventBody {
    let twin = render_identity_evidence_twin(PHOTO_EVIDENCE_KIND, basis, &attachment);
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: IDENTITY_EVIDENCE_EVENT_TYPE.into(),
        schema_version: IDENTITY_EVIDENCE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: photo_evidence_body(basis),
        attachments: vec![attachment],
        plaintext_twin: Some(twin),
    }
}

/// Store `bytes` as a present, self-verified local blob and author the identity-evidence
/// event referencing it, in ONE transaction (bytes + reference land atomically — a chart
/// never references a blob whose bytes failed to store). The blob INSERT passes present=TRUE
/// through the db/026 verify trigger (the first non-hostile writer through that floor); the
/// floor's own `blob_note_reference` for our rendition is a harmless ON CONFLICT no-op. The
/// caller (CLI edge) owns file I/O; this takes bytes so it stays testable. Returns the event id.
pub async fn assert_photo_evidence(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient_id: Uuid,
    bytes: &[u8],
    media_type: &str,
    descriptor: &str,
    basis: Option<&str>,
) -> anyhow::Result<Uuid> {
    let lb = prepare_local_blob(bytes, media_type);
    let attachment = Attachment::single(descriptor, lb.rendition.clone());

    // Tick the HLC once (self-committing, like john_doe.rs) before the transaction.
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_photo_evidence_body(event_id, patient_id, kid, hlc, basis, attachment);
    let signed = sign(&body, sk)?;
    let byte_len = bytes.len() as i64;

    let tx = client.transaction().await?;
    // Store the bytes present=TRUE — verified in-DB by the db/026 trigger before it commits.
    // ON CONFLICT DO UPDATE (not DO NOTHING): a reference-only placeholder row (present=FALSE,
    // content NULL) may already sit at this content address — e.g. a remote-synced event
    // referenced the same photo before we held its bytes, or blob_note_reference created it.
    // DO NOTHING would leave that placeholder unfilled while the event still commits, so the
    // chart would reference a blob whose bytes were silently discarded (the exact invariant
    // this atomic txn exists to hold). DO UPDATE flips it present with our verified bytes;
    // content-addressing guarantees identical bytes for a matching address, so this is safe
    // even if the row was already present=TRUE. Mirrors cairn-sync's cmd_put_blob.
    tx.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard, present, fetched_at)
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
             SET content = EXCLUDED.content, outboard = EXCLUDED.outboard, present = TRUE,
                 media_type = EXCLUDED.media_type, byte_len = EXCLUDED.byte_len,
                 fetched_at = EXCLUDED.fetched_at",
        &[&lb.addr, &media_type, &byte_len, &bytes, &lb.outboard],
    ).await?;
    // Author the event (its floor learns the reference — ON CONFLICT no-op against the row above).
    tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;
    tx.commit().await?;

    Ok(event_id)
}
```

Wire the module in `crates/cairn-node/src/lib.rs`:

```rust
pub mod photo_evidence;
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p cairn-node --lib photo_evidence`
Expected: PASS (2 pure tests). `cargo build --workspace` also SUCCESS.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/photo_evidence.rs crates/cairn-node/src/lib.rs db/028_identity_evidence.sql crates/cairn-node/src/db.rs
git commit -m "$(printf 'feat(node): photo identity-evidence author path (prepare_local_blob + assert)\n\nPure prepare_local_blob + build_photo_evidence_body; assert_photo_evidence stores\nthe blob present (through the db/026 floor) and authors the event atomically in\none txn. Photo rides EventBody.attachments. db/028 registers the new type\n(fail-closed floor).\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

### Task 7: `assert-photo-evidence` CLI subcommand

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add the `Cmd::AssertPhotoEvidence` variant + handler)

**Interfaces:**
- Consumes: `cairn_node::photo_evidence::assert_photo_evidence`, `cairn_node::identity::load_local`, `load_signing_key`, `ensure_registration_actor`.

- [ ] **Step 1: Add the subcommand variant**

In `crates/cairn-node/src/main.rs`, in the `enum Cmd`, after the `AssertObservedEvidence { … }` variant, add:

```rust
    /// Attach a clinician-observed photograph as §5.4 identity evidence to an existing chart.
    /// The photo becomes a content-addressed blob stored locally (present + self-verified) and
    /// referenced by an `identity.evidence.asserted` event. OWNER ceremony: enrolls the node key
    /// as a registration actor on first use (a real UI attaches the operating clerk's actor).
    AssertPhotoEvidence {
        /// The patient UUID to attach the photo to.
        patient: Uuid,
        /// Path to the image file on disk.
        #[arg(long)]
        file: std::path::PathBuf,
        /// The MIME media type of the file (e.g. image/jpeg). Caller-supplied — no sniffing.
        #[arg(long = "media-type")]
        media_type: String,
        /// Honest human description of the photo (required, non-empty — principle 4).
        #[arg(long)]
        descriptor: String,
        /// How/why the photo was taken (optional).
        #[arg(long)]
        basis: Option<String>,
    },
```

- [ ] **Step 2: Add the handler**

In the `match cli.cmd { … }` block, after the `Cmd::AssertObservedEvidence { … } => { … }` arm, add:

```rust
        Cmd::AssertPhotoEvidence { patient, file, media_type, descriptor, basis } => {
            if descriptor.trim().is_empty() {
                anyhow::bail!("--descriptor must be non-empty (§5.4/principle 4: say what the photo shows)");
            }
            let bytes = std::fs::read(&file)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", file.display()))?;
            let sk = load_signing_key(&cli.key, true)?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &kid).await?;

            let event_id = cairn_node::photo_evidence::assert_photo_evidence(
                &mut db, &sk, &kid, &id.node_id_hex, patient, &bytes, &media_type,
                &descriptor, basis.as_deref()).await?;
            println!("attached photo evidence {event_id} to {patient}");
        }
```

- [ ] **Step 3: Verify it compiles and the CLI help renders**

Run: `cargo build -p cairn-node && cargo run -p cairn-node -- assert-photo-evidence --help`
Expected: SUCCESS; help lists `--file`, `--media-type`, `--descriptor`, `--basis`.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "$(printf 'feat(node): assert-photo-evidence CLI subcommand (§5.4)\n\nReads the image file at the edge, attaches it as identity evidence. Rejects an\nempty descriptor (principle 4).\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

### Task 8: DB-gated e2e — store, reference, verify, legibility

**Files:**
- Create: `crates/cairn-node/tests/photo_evidence.rs`

**Interfaces:**
- Consumes: `cairn_node::photo_evidence::assert_photo_evidence`, `cairn_node::john_doe::register_john_doe`, `cairn_node::db`.

- [ ] **Step 1: Write the failing e2e test**

Create `crates/cairn-node/tests/photo_evidence.rs`:

```rust
//! §5.4 photo evidence e2e (ADR-0042 attachment tier, first clinical-surface use). Registers
//! a John Doe, attaches a photo, and proves: the blob is present + re-verifies against the
//! db/026 floor, the event references it, and the authored twin is legible without pixels.
//! Real Postgres, gated on $CAIRN_TEST_PG.

use cairn_event::blob_address;
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, blob_store, blob_chunk CASCADE").await.unwrap();
    db::reset_node_federation_tables(c).await.unwrap();
    let (sk, kid) = cairn_event::generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    (sk, kid)
}

#[tokio::test]
async fn photo_evidence_stores_a_verified_blob_and_references_it() {
    let Some(base) = cs() else { eprintln!("skip: no CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();  // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();

    // A realistic §5.4 flow: register the unidentified patient, then attach a photo.
    let (patient, _callsign) = cairn_node::john_doe::register_john_doe(
        &mut c, &sk, &kid, &id.node_id_hex, "ED", "site1", "2026-07-08",
        "unconscious ED arrival, no ID").await.unwrap();

    let photo = b"\xff\xd8\xff\xe0JFIF-pretend-jpeg-bytes";
    let event_id = cairn_node::photo_evidence::assert_photo_evidence(
        &mut c, &sk, &kid, &id.node_id_hex, patient, photo, "image/jpeg",
        "frontal face photograph of unidentified patient", Some("on arrival")).await.unwrap();

    // 1. The blob is present, and its bytes re-verify against the address (db/026 floor fn).
    let addr = blob_address(photo);
    let row = c.query_one(
        "SELECT present, cairn_blob_verify(blob_address, content) FROM blob_store WHERE blob_address = $1",
        &[&addr]).await.unwrap();
    let present: bool = row.get(0);
    let verifies: bool = row.get(1);
    assert!(present, "the locally-authored blob is present");
    assert!(verifies, "content re-hashes to the address (self-verifying)");

    // 2. The event references the blob by digest in its stored attachments, and NOT the bytes.
    let (atts_text, twin): (String, String) = {
        let r = c.query_one(
            "SELECT attachments::text, plaintext_twin FROM event_log WHERE event_id = $1",
            &[&event_id]).await.unwrap();
        (r.get(0), r.get(1))
    };
    assert!(atts_text.contains(&hex::encode(&addr)), "attachment names the blob by digest");
    assert!(!atts_text.contains("JFIF-pretend"), "pixel bytes are NOT inlined in the reference");

    // 3. The authored twin is legible and pixel-free.
    assert!(twin.contains("frontal face photograph of unidentified patient"), "twin: {twin}");
    assert!(twin.contains("image/jpeg"));
    assert!(!twin.contains("JFIF-pretend"), "twin is descriptor-derived, never pixels");
}

#[tokio::test]
async fn photo_evidence_fills_a_preexisting_reference_only_placeholder() {
    // Regression for the ON CONFLICT DO NOTHING bug: a present=FALSE placeholder row may
    // already sit at the photo's content address (e.g. a remote-synced event referenced the
    // same photo before this node held its bytes). assert_photo_evidence must FLIP it to
    // present=TRUE with the real bytes (DO UPDATE), never silently discard them and commit
    // an event referencing an empty blob.
    let Some(base) = cs() else { eprintln!("skip: no CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();  // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();
    let (patient, _callsign) = cairn_node::john_doe::register_john_doe(
        &mut c, &sk, &kid, &id.node_id_hex, "ED", "site1", "2026-07-08",
        "unconscious ED arrival, no ID").await.unwrap();

    // Pre-seat a reference-only placeholder (present=FALSE, content NULL) at the address.
    let photo = b"\xff\xd8\xff\xe0JFIF-second-photo-bytes";
    let addr = blob_address(photo);
    c.execute("SELECT blob_note_reference($1, 'image/jpeg', $2)",
              &[&addr, &(photo.len() as i64)]).await.unwrap();
    let before: bool = c.query_one(
        "SELECT present FROM blob_store WHERE blob_address = $1", &[&addr]).await.unwrap().get(0);
    assert!(!before, "placeholder starts present=FALSE");

    // Now author the photo evidence with the real bytes.
    cairn_node::photo_evidence::assert_photo_evidence(
        &mut c, &sk, &kid, &id.node_id_hex, patient, photo, "image/jpeg",
        "second identification photograph", None).await.unwrap();

    let row = c.query_one(
        "SELECT present, cairn_blob_verify(blob_address, content) FROM blob_store WHERE blob_address = $1",
        &[&addr]).await.unwrap();
    let present: bool = row.get(0);
    let verifies: bool = row.get(1);
    assert!(present, "the placeholder must be flipped present with the real bytes, not left empty");
    assert!(verifies, "the filled bytes re-hash to the address");
}
```

- [ ] **Step 2: Run it to verify it fails then passes**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test photo_evidence`
Expected: if Tasks 4-6 are complete it should PASS; if run before them it FAILs to compile. Confirm PASS here.

- [ ] **Step 3: Run the full workspace suite**

Run: `CAIRN_TEST_PG=… cargo test --workspace && cargo clippy --workspace --all-targets`
Expected: PASS, clippy clean.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/tests/photo_evidence.rs
git commit -m "$(printf 'test(node): e2e photo evidence — store, reference, verify, legibility (§5.4)\n\nregister John Doe -> attach photo -> blob present + re-verifies (db/026), event\nreferences it by digest (bytes not inlined), authored twin legible sans pixels.\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Final: HANDOVER + ROADMAP + PR

### Task 9: Update HANDOVER/ROADMAP and open the PR

**Files:**
- Modify: `docs/HANDOVER.md` (fix stale `v0.41` → this slice's `v0.43`; add the slice summary; prune)
- Modify: `docs/ROADMAP.md` (record the attachment tier + photo evidence slice)

- [ ] **Step 1:** Update `docs/HANDOVER.md`: correct the header `Spec/ADRs` line (it read `v0.41`, now `v0.43` after this slice and the pre-existing v0.42 progress-note ADR); add a concise "This session" block (ADR-0042 shape + db/027 floor + photo evidence); update the §5.4 status (photo evidence BUILT; marks/belongings/EMS = future kinds; cross-node byte fetch + preview + sealing deferred). Keep under 500 lines (prune the oldest condensed blocks if needed).
- [ ] **Step 2:** Update `docs/ROADMAP.md` with the new slice entry.
- [ ] **Step 3:** Commit: `docs: record the attachment-shape + photo-evidence slice in HANDOVER + ROADMAP`.
- [ ] **Step 4:** Push the branch and open a PR to `main` with a description covering: the can't-retrofit shape finalization (ADR-0042), the ADR-0041 reconciliation, the floor change (db/027), the photo author path, and the honest limits (plaintext, single rendition, bytes local, POC divergence). Link no specific issue unless one is opened for §5.4 photo.

---

## Self-Review (completed)

**Spec coverage:** Section 1 (shape) → Tasks 1-2; ADR + spec bump → Task 3; the floor change (Section 4 "The floor change") → Task 4; Section 2 event home + author path → Tasks 5-7; Section 4 e2e → Task 8; honest limits carried into ADR (Task 3) + HANDOVER (Task 9). Covered.

**Placeholder scan:** no TBD/TODO; every code step shows the code; every command has an expected result.

**Type consistency:** `Attachment`/`Rendition`/`SealRef`/`LocalBlob`, `prepare_local_blob`, `build_photo_evidence_body`, `assert_photo_evidence`, `photo_evidence_body`, `render_identity_evidence_twin`, `cairn_learn_attachment_refs`, and the `IDENTITY_EVIDENCE_*`/`PHOTO_EVIDENCE_KIND`/`RENDITION_ROLE_ORIGINAL`/`BLOB_ALG_BLAKE3` constants are used identically across producing and consuming tasks.
