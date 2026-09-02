//! Shared test fixtures for this crate's unit tests, used by `chunk`, `marker`, `container`,
//! `verify`, `record`, `segment` and `attest`'s `mod tests`. Centralised rather than copied
//! into each module: the `enroll()` fixture is what every attestation test's meaning rests
//! on, and `bytes()`/`record()`/`segment()` are what every custody and framing assertion in
//! `record`, `segment` and `attest` rests on — independent copies would be independent
//! places for one of them to quietly drift apart.

use crate::attest::{segment_commitment, tests_support};
use crate::container::{serialize_v3, MediumV3};
use crate::record::MediumRecord;
use crate::segment::{Plane, Segment};
use cairn_event::{event_address, sign, EventBody, Hlc, SigningKey};

/// Build a "clean" `MediumV3` fixture (no unknown segments, no torn tail) out of a
/// complete segment list, with an HONEST `complete_bytes`: these fixtures are hand-built,
/// not parsed, but `complete_bytes` is meant to be "the length of the intact prefix" — for
/// a fixture with no tear, that IS its full serialized length. Computing it this way (via
/// the crate's own `serialize_v3`) rather than a placeholder keeps the fixture from quietly
/// asserting something `parse_any` would never actually produce.
pub(crate) fn medium_v3(segments: Vec<Segment>) -> MediumV3 {
    let complete_bytes = serialize_v3(&segments)
        .expect("fixture segments fit the cap")
        .len();
    MediumV3 {
        segments,
        truncated_tail: false,
        complete_bytes,
    }
}

/// A `MediumV3` whose TORN-TAIL flag is set, for the health tests.
///
/// `medium_v3` hard-codes `truncated_tail: false`, which is precisely why no chain test had
/// ever seen a torn medium and why a torn tail could report as sound (#500 slice 2a review).
pub(crate) fn medium_v3_torn(segments: Vec<Segment>) -> MediumV3 {
    MediumV3 {
        truncated_tail: true,
        ..medium_v3(segments)
    }
}

pub(crate) fn sk() -> SigningKey {
    cairn_event::generate_key().unwrap().0
}

pub(crate) fn kid(sk: &SigningKey) -> String {
    hex::encode(sk.verifying_key().to_bytes())
}

pub(crate) fn node_id(ev: &[u8]) -> String {
    hex::encode(event_address(ev))
}

/// A real, validly-signed enroll for `sk` — its content-address IS the node-id.
pub(crate) fn enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: cairn_event::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid(sk),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "display_name": name, "address": "10.0.0.1:7843" }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Runtime-derived bytes for a fixture field. NEVER a literal: a byte-array literal in a
/// crypto context trips CodeQL's `rust/hard-coded-cryptographic-value` (house rule 6,
/// issue #146), and a wrapped DEK is exactly such a context.
pub(crate) fn bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
}

/// A `MediumRecord` fixture with the optional fields selected by `flags` (the same
/// three-bit layout `put_record`/`take_record` encode). Shared between `record`'s tests
/// (which exercise the record codec directly) and `segment`'s tests (which build `Segment`
/// fixtures out of these records) for the same reason as `bytes`: a second definition
/// would be a second place for the fixture's custody shape to drift from what the
/// record-layer tests assert against.
pub(crate) fn record(flags: u8) -> MediumRecord {
    MediumRecord {
        signed_bytes: bytes(1, 40),
        attestation: (flags & 0b001 != 0).then(|| bytes(2, 16)),
        attester_key: (flags & 0b010 != 0).then(|| bytes(3, 32)),
        dek_wrapped: (flags & 0b100 != 0).then(|| bytes(4, 48)),
        source_seq: 7,
    }
}

/// A `Segment` fixture with `n` records, an unsigned-shape self id, and an arbitrary
/// (not-actually-signed) attestation chunk. Shared between `segment`'s tests (which
/// exercise section framing) and `attest`'s tests (which need a plain `Segment` to mutate,
/// e.g. to check that an unsigned segment attests nothing) for the same reason as `bytes`
/// and `record`: one definition, so the two modules' fixtures cannot quietly drift apart.
pub(crate) fn segment(plane: Plane, index: u32, n: usize) -> Segment {
    Segment {
        plane,
        index,
        prev_commitment: if index == 0 {
            String::new()
        } else {
            "beef".into()
        },
        self_node_id_hex: "abcd".into(),
        attestation: Some(bytes(9, 64)),
        records: (0..n).map(|_| record(0b111)).collect(),
    }
}

/// `n` signed CLINICAL segments, correctly chained, with ascending `source_seq`.
/// Returns the medium and every seq it wrote, so a caller can assert on the watermark.
///
/// `lineage` is load-bearing, not decoration: it is threaded straight through to
/// `tests_support::distinct_record`, so two chains built with different lineages hold
/// genuinely different records. Verify's splice test relies on that — with one shared
/// lineage both chains would be byte-identical and "spliced from another medium" would
/// prove nothing. It is NOT called `salt`: see `distinct_record`'s doc comment for the
/// eighteen critical CodeQL alerts that name cost (#527).
pub(crate) fn chain_of(n: usize, lineage: u8) -> (MediumV3, Vec<i64>) {
    let sk = sk();
    let mut segments: Vec<Segment> = Vec::new();
    let mut seqs = Vec::new();
    let mut prev = String::new();
    for i in 0..n {
        let records = vec![tests_support::distinct_record(lineage, i as u8)];
        seqs.extend(records.iter().map(|r| r.source_seq));
        let seg = tests_support::signed(&sk, "abcd", Plane::Clinical, i as u32, &prev, records);
        prev = segment_commitment(&seg.records);
        segments.push(seg);
    }
    (medium_v3(segments), seqs)
}

/// A medium whose segment 0 is a NODE-plane segment carrying a real `node.enrolled`, so
/// `self_id_from_chain`'s genesis bind has something to bind to; segment 1 is clinical.
/// The attested self id is the genesis's own content address — that is what a node-id IS.
pub(crate) fn chain_with_genesis() -> (MediumV3, SigningKey) {
    let sk = sk();
    let genesis = enroll(&sk, "a");
    let self_id = hex::encode(cairn_event::event_address(&genesis));
    let node_records = vec![MediumRecord {
        signed_bytes: genesis,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 1,
    }];
    let s0 = tests_support::signed(&sk, &self_id, Plane::Node, 0, "", node_records);
    let prev = segment_commitment(&s0.records);
    let s1 = tests_support::signed(
        &sk,
        &self_id,
        Plane::Clinical,
        1,
        &prev,
        vec![tests_support::distinct_record(9, 0)],
    );
    (medium_v3(vec![s0, s1]), sk)
}

/// `n` signed CLINICAL segments whose records are GENUINELY SIGNED events, correctly
/// chained, with ascending `source_seq`.
///
/// Distinct from `chain_of`, whose records are `distinct_record`'s arbitrary bytes: those are
/// fine for chain/commitment assertions (a commitment is over a content address, which any
/// bytes have) but they FAIL signature verification, so a fixture built from them can never
/// be used to assert that a medium is SOUND. `health`'s composed verdict needs both halves to
/// pass, so it needs this.
pub(crate) fn verifiable_chain_of(n: usize) -> (MediumV3, Vec<i64>) {
    let sk = sk();
    let mut segments: Vec<Segment> = Vec::new();
    let mut seqs = Vec::new();
    let mut prev = String::new();
    for i in 0..n {
        let records = vec![MediumRecord {
            signed_bytes: enroll(&sk, &format!("node-{i}")),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: i as i64 + 1,
        }];
        seqs.extend(records.iter().map(|r| r.source_seq));
        let seg = tests_support::signed(&sk, "abcd", Plane::Clinical, i as u32, &prev, records);
        prev = segment_commitment(&seg.records);
        segments.push(seg);
    }
    (medium_v3(segments), seqs)
}

/// `n` clinical segments written with NO signing key available — correctly chained, and
/// carrying their self id, but not tamper-evident.
///
/// Its records hold GENUINELY SIGNED events (via `enroll`), unlike `distinct_record`'s
/// arbitrary bytes. `verify_records` checks Ed25519 signatures, so a fixture built from
/// those arbitrary bytes would fail verification before any test tampered with it — and the test
/// would then pass for the wrong reason, proving nothing about tampering.
pub(crate) fn unsigned_chain_of(n: usize) -> MediumV3 {
    let sk = sk();
    let mut segments = Vec::new();
    let mut prev = String::new();
    for i in 0..n {
        let records = vec![MediumRecord {
            signed_bytes: enroll(&sk, &format!("node-{i}")),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: i as i64 + 1,
        }];
        let seg = Segment {
            plane: Plane::Clinical,
            index: i as u32,
            prev_commitment: prev.clone(),
            self_node_id_hex: "abcd".into(),
            attestation: None,
            records,
        };
        prev = segment_commitment(&seg.records);
        segments.push(seg);
    }
    medium_v3(segments)
}
