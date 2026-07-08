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
        Attachment {
            descriptor: descriptor.to_string(),
            renditions: vec![rendition],
        }
    }
}

/// Render an attachment's legibility fragment from its DESCRIPTOR FIELDS ONLY, never its
/// bytes (§3.14: the twin is not derived from pixels). One line per rendition summarising
/// role + media type + size. This is what a text-only node, a screen reader, and the RAG
/// substrate see for the attachment.
pub fn render_attachment_twin(a: &Attachment) -> String {
    let mut out = a.descriptor.clone();
    for r in &a.renditions {
        out.push_str(&format!(
            " ({}: {}, {} bytes)",
            r.role, r.media_type, r.byte_len
        ));
    }
    out
}

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
        assert!(
            r.inline.is_none(),
            "a by-reference rendition inlines no bytes"
        );
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
        assert!(
            twin.contains("wound on left forearm"),
            "descriptor must appear: {twin}"
        );
        assert!(
            twin.contains("image/jpeg"),
            "media type must appear: {twin}"
        );
        assert!(
            !twin.contains("PIXELDATA"),
            "twin must NOT contain pixel bytes: {twin}"
        );
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
            seal: Some(SealRef {
                alg: "xchacha20poly1305".into(),
                dek_wrap: "dek-ref-1".into(),
            }),
        };
        let a = Attachment {
            descriptor: "d".into(),
            renditions: vec![sealed],
        };
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
        assert!(
            v.get("inline").is_none(),
            "None inline must be omitted, not null"
        );
        assert!(
            v.get("seal").is_none(),
            "None seal must be omitted, not null"
        );
    }
}
