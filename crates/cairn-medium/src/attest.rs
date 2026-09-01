//! The segment attestation: the signed object that makes a CAIRNB3 segment self-naming
//! and tamper-evident, without needing a whole-medium head marker.
//!
//! WHY A SEPARATE MODULE FROM `segment`: `segment.rs` already carries the section framing
//! (encode/decode, torn-tail vs. corruption, the plane/index/chain shape) at ~350 lines.
//! The attestation is a further ~200 — folding it in would push that file past the crate's
//! 500-line cap, the same pressure that forced the chunk/record/segment split one task ago.
//! So the attestation is its own concern here: it signs and verifies a [`crate::segment::Segment`]
//! the way [`crate::marker`] signs and verifies a whole CAIRNB2 medium, but bound to one
//! append increment instead of the whole set (see `crate::segment`'s module docs for why a
//! whole-set commitment cannot survive an append).
//!
//! WHAT IT BINDS, and why each bind matters:
//!   - the segment's own records ([`segment_commitment`]) — so altering, adding, or
//!     dropping one record invalidates the attestation;
//!   - the segment's `plane` and `index` — so a genuine segment cannot be replayed at a
//!     different position or relabelled to a different plane;
//!   - the segment's `prev_commitment` — so a genuine segment cannot be spliced elsewhere
//!     in the chain, even one whose records are identical.
//!
//! Together, contents + plane + position + predecessor are what let an append cost ONE
//! signature instead of a whole-file rewrite: each segment proves its own place in the
//! chain without needing anything written after it to also change.
//!
//! FAIL CLOSED, the same asymmetry [`crate::marker`] has: the signing key never leaves the
//! node that captured the segment, so an attacker holds no way to FORGE a wrong-but-valid
//! attestation — tampering can only WITHHOLD the identification, falling back to a manual
//! operator choice on restore. [`verify_segment_attestation`] therefore only ever returns
//! the genuinely attested id or `None`, never a guess.
//!
//! The remaining bind — that the named node actually has a genesis on THIS medium, signed
//! by the same key — needs the whole medium (every segment's records, not just one), so it
//! is not this module's job; it lives in `crate::verify`'s chain pass.

use crate::record::MediumRecord;
use crate::segment::{Plane, Segment};
use cairn_event::{event_address, sign, verify_self_described, EventBody, Hlc, SigningKey};

/// Event type of the in-container segment attestation. Like `node.self_attested` it NEVER
/// enters `node_event`, never syncs and is never registered in the in-DB twin registry —
/// it lives in the backup container only, which is what lets it record a local
/// self-distinction that set-union convergence would otherwise erase.
pub const SEGMENT_ATTEST_TYPE: &str = "node.segment_attested";

/// Commitment over a segment's records — over each record's `(event_address(signed_bytes),
/// source_seq)` PAIR. Order-independent, sharing `marker::commitment_over` (the per-record
/// digest is fed straight back through it, so the crate keeps ONE definition of what "a
/// commitment over these bytes" means) — a build that reorders the byte inputs before
/// hashing would defeat that reuse, so don't; frame reordering is harmless under set-union
/// sync and must stay harmless here too.
///
/// WHY `source_seq` IS INCLUDED (C1, #500 final review — this used to commit to
/// `signed_bytes` alone): `source_seq` is the medium's cursor — `watermark()` returns
/// `max(source_seq)` over the verified prefix, and the NEXT capture trusts that value to
/// decide what to write. Before this fix nothing signed or committed to it: a single
/// flipped bit inside an 8-byte in-container field left every signature and every chain
/// link genuinely valid while silently jumping the watermark to ~4.6e18 — every future
/// capture on that medium then writes nothing, forever, while the medium reports itself
/// healthy and growing. That is the exact failure this whole slice exists to prevent.
///
/// The sidecar fields (`attestation`, `attester_key`, `dek_wrapped`) stay OUT of the
/// commitment: a legitimate re-capture may re-wrap that custody (which path owns the
/// re-wrap is a decision the NEXT slice makes, not this one), so committing to it would
/// break a re-capture that changed nothing clinically meaningful. That re-wrap rationale
/// never applied to `source_seq` — it is not custody a re-capture legitimately changes; it
/// is a fixed local fact about the capturing node's own insertion order, recorded once and
/// never revisited.
///
/// RESIDUAL EXPOSURE, named plainly rather than fixed here — and it is ALL THREE sidecars,
/// not just the DEK (an earlier version of this paragraph named only `dek_wrapped`, narrower
/// than the truth already recorded in issue #524):
///   - deleting `dek_wrapped` from an already-verified segment leaves that segment's
///     attestation intact and the medium reporting fully healthy end to end — while a later
///     restore finds a sealed body it can never open;
///   - deleting `attestation` (the human authorship token) is the same shape at lower stakes:
///     the medium still reports healthy, and a suppressing event is then refused fail-closed
///     at the clinical apply door for want of a token that was silently dropped in transit —
///     see `crate::record::MediumRecord::attestation`, which documents that dependence.
///
/// Nothing in this crate catches either today; only a custody-aware check at the slice that
/// owns re-wrap can (#524).
pub fn segment_commitment(records: &[MediumRecord]) -> String {
    let per_record: Vec<Vec<u8>> = records
        .iter()
        .map(|r| {
            let mut item = event_address(&r.signed_bytes);
            item.extend_from_slice(&r.source_seq.to_be_bytes());
            item
        })
        .collect();
    let refs: Vec<&[u8]> = per_record.iter().map(Vec::as_slice).collect();
    crate::marker::commitment_over(&refs)
}

/// Build the signed attestation for one segment.
///
/// NOT pure: it mints a fresh `event_id` (`Uuid::now_v7`), exactly as
/// [`crate::marker::build_self_attestation`] does, so two calls differ. Harmless — the
/// `event_id` is neither committed to nor checked on verify; the authority comes from the
/// signature plus the four binds in the payload.
pub fn build_segment_attestation(
    sk: &SigningKey,
    key_id: &str,
    self_node_id_hex: &str,
    plane: Plane,
    index: u32,
    prev_commitment: &str,
    records: &[MediumRecord],
) -> Vec<u8> {
    // Signing a plane this build cannot name is a programming error, not a runtime
    // condition: an unknown plane only ever arrives by READING a newer Cairn's medium, and
    // nothing in this build can legitimately author one.
    debug_assert!(
        plane.is_known(),
        "refusing to author an attestation for an unknown plane tag {}",
        plane.tag()
    );
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: cairn_event::NIL_PATIENT.into(),
        event_type: SEGMENT_ATTEST_TYPE.into(),
        schema_version: "node/1".into(),
        // Never ordered against anything, so a fixed 0/0 HLC — as the self-attestation does.
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: self_node_id_hex.into(),
        },
        t_effective: None,
        signer_key_id: key_id.into(),
        contributors: serde_json::json!([{"actor_id": key_id, "role": "recorded"}]),
        payload: serde_json::json!({
            "self_node_id_hex": self_node_id_hex,
            // BOTH the human-legible label and the numeric on-disk tag are bound. The tag is
            // the load-bearing one: it is knowable for EVERY plane, including one this build
            // does not recognise, so a segment relabelled to an unknown plane tag still fails
            // its attestation rather than slipping past a conjunct we had to skip.
            "plane": plane.label(),
            "plane_tag": plane.tag(),
            "segment_index": index,
            "record_count": records.len(),
            "segment_commitment": segment_commitment(records),
            "prev_commitment": prev_commitment,
        }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk)
        .expect("segment-attestation signing")
        .signed_bytes
}

/// Verify one segment's attestation against the segment it sits in. Returns the attested
/// `self_node_id_hex` IFF every bind holds, else `None`.
///
/// Fail closed, in the same asymmetry the CAIRNB2 marker has: an attacker holds no private
/// key, so tampering can only WITHHOLD the identification (falling back to a manual
/// choice), never misdirect it. The four binds:
///   - the attestation's own signature verifies and it is a `node.segment_attested`;
///   - its `segment_commitment` matches THIS segment's records;
///   - its `plane`, `segment_index` and `record_count` match this segment's position;
///   - its `prev_commitment` matches this segment's — so a genuine segment replayed
///     elsewhere in the chain fails.
///
/// The remaining bind — that the named node has a genesis on THIS medium, signed by the
/// same key — needs the whole medium and lives in `crate::verify`'s chain pass.
///
/// ON THE `None`/`Some(vec![])` ASYMMETRY WITH `MediumRecord`: `Segment::attestation`
/// collapses an EMPTY attestation chunk into this same `None` at decode time (see
/// `segment::take_section`) — unlike `MediumRecord::attestation`, where `None` (no token
/// travelled) and `Some(vec![])` (an empty token travelled) are deliberately kept
/// distinguishable, because the clinical apply door treats them differently (refuses a
/// suppressing event outright vs. reports an invalid token). Nothing plays that role here:
/// an empty byte string is never a valid signed event, so this function returns `None` for
/// "no attestation chunk" and for "a zero-length one" alike, and no caller — this one
/// included — ever needs to tell them apart. Collapsing the two at the segment layer is
/// therefore not a shortcut that quietly loses information a door needs; it is the honest
/// reflection of a distinction that is load-bearing one layer down and meaningless here.
pub fn verify_segment_attestation(seg: &Segment) -> Option<String> {
    let bytes = seg.attestation.as_deref()?;
    let body = verify_self_described(bytes).ok()?;
    if body.event_type != SEGMENT_ATTEST_TYPE {
        return None;
    }
    let p = &body.payload;
    // The numeric tag binds for EVERY plane, known or not — see `build_segment_attestation`.
    // The label is checked only when this build knows the plane's name; for an unknown plane
    // the tag conjunct still holds, so nothing is skipped, only unnameable.
    let label_ok = match seg.plane.label() {
        Some(known) => p.get("plane")?.as_str()? == known,
        None => true,
    };
    let matches = p.get("segment_commitment")?.as_str()? == segment_commitment(&seg.records)
        && u8::try_from(p.get("plane_tag")?.as_u64()?).ok()? == seg.plane.tag()
        && label_ok
        && p.get("segment_index")?.as_u64()? == u64::from(seg.index)
        && p.get("record_count")?.as_u64()? == seg.records.len() as u64
        && p.get("prev_commitment")?.as_str()? == seg.prev_commitment;
    if !matches {
        return None;
    }
    Some(p.get("self_node_id_hex")?.as_str()?.to_ascii_lowercase())
}

/// Shared test fixtures for building signed segments. `segment.rs`'s own tests use these
/// directly; `container.rs` (the CAIRNB3 container task) and `verify.rs` (the chain-pass
/// task) also need signed segments, and must build them through THIS builder rather than
/// their own copies — every attestation assertion in this crate rests on exactly what
/// `signed` produces, and three independent copies would be three places for that meaning
/// to quietly drift apart.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use cairn_event::SigningKey;

    /// Runtime-derived record bytes, made distinct by `(lineage, n)`.
    ///
    /// `lineage` says which fixture MEDIUM a record belongs to; `n` its position within that
    /// medium. **`lineage` is load-bearing, not decoration.** Without it every fixture chain is
    /// byte-identical, so a segment "spliced from another medium" would carry an identical
    /// predecessor commitment and a cross-medium splice test would pass while asserting
    /// nothing. NEVER a literal byte array — house rule 6 (#146).
    ///
    /// ## Why this parameter is not called `salt` (#527)
    ///
    /// It was, and it cost nineteen critical CodeQL alerts at once. `rust/hard-coded-
    /// cryptographic-value` picks its sink by the NAME of the binding a value flows into, so
    /// every call site passing a constant to a parameter called `salt` was reported as
    /// "this hard-coded value is used as a salt" — even though nothing in this crate derives
    /// a key, and the sibling helpers `testkit::bytes(seed, …)` and
    /// `wire_pins::placeholder(seed, …)` run the identical arithmetic unflagged.
    ///
    /// Runtime derivation does not clear it: the loop below is a pure function of its two
    /// arguments, which the scanner folds straight through. The name is the whole finding.
    /// Guarded by `cairn-node`'s `crypto_sink_names_are_genuine.rs`.
    pub(crate) fn distinct_record(lineage: u8, n: u8) -> MediumRecord {
        let mk = |seed: u8, len: usize| -> Vec<u8> {
            (0..len)
                .map(|i| {
                    seed.wrapping_add(lineage)
                        .wrapping_mul(n.wrapping_add(1))
                        .wrapping_add(i as u8)
                })
                .collect()
        };
        MediumRecord {
            signed_bytes: mk(1, 40),
            attestation: Some(mk(2, 16)),
            attester_key: None,
            dek_wrapped: Some(mk(4, 48)),
            source_seq: i64::from(n) + 1,
        }
    }

    /// One signed segment over `records`, correctly positioned in a chain.
    pub(crate) fn signed(
        sk: &SigningKey,
        self_id: &str,
        plane: Plane,
        index: u32,
        prev: &str,
        records: Vec<MediumRecord>,
    ) -> Segment {
        let kid = hex::encode(sk.verifying_key().to_bytes());
        let attestation =
            build_segment_attestation(sk, &kid, self_id, plane, index, prev, &records);
        Segment {
            plane,
            index,
            prev_commitment: prev.to_string(),
            self_node_id_hex: self_id.to_string(),
            attestation: Some(attestation),
            records,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{enroll, segment, sk};

    /// Shorthand for this module's tests: `n` distinct records under one signed segment.
    /// Delegates to `tests_support::signed` rather than building its own segment — ONE
    /// fixture builder for the whole crate, because every attestation assertion below
    /// rests on exactly what it produces.
    fn signed_segment(
        sk: &SigningKey,
        self_id: &str,
        plane: Plane,
        index: u32,
        prev: &str,
        n: usize,
    ) -> Segment {
        let records = (0..n)
            .map(|i| tests_support::distinct_record(1, i as u8))
            .collect();
        tests_support::signed(sk, self_id, plane, index, prev, records)
    }

    #[test]
    fn a_signed_segment_verifies_and_names_its_node() {
        let sk = sk();
        let seg = signed_segment(&sk, "abcd", Plane::Clinical, 0, "", 3);
        assert_eq!(verify_segment_attestation(&seg), Some("abcd".to_string()));
    }

    /// The commitment is order-independent: frame reordering is harmless under set-union,
    /// so it must not invalidate an attestation.
    ///
    /// `a` and `b` MUST genuinely differ in `signed_bytes` and/or `source_seq` — the only
    /// fields `segment_commitment` covers (see its doc comment above). `testkit::record(flags)`
    /// varies only the sidecar fields (`attestation`, `attester_key`, `dek_wrapped`), which the
    /// commitment deliberately excludes, so two `record(_)` fixtures are byte-identical on the
    /// covered axis — swapping two identical items proves nothing, and this test would still
    /// pass even if a refactor made the commitment order-DEPENDENT. Use
    /// `distinct_record(lineage, n)` (which varies both covered fields by `(lineage, n)`) or
    /// build records inline with distinct
    /// values instead — do NOT simplify this fixture back to `record(...)`.
    #[test]
    fn the_commitment_is_order_independent() {
        let a = tests_support::distinct_record(1, 0);
        let b = tests_support::distinct_record(1, 1);
        // Prove the fixtures are genuinely distinct on exactly the axis that matters: if the
        // single-record commitments differ, `signed_bytes`/`source_seq` differ too — so a
        // future edit that quietly re-vacuums this fixture (e.g. back to `record(flags)`)
        // trips this assertion before the order-independence check below could go vacuous again.
        assert_ne!(
            segment_commitment(std::slice::from_ref(&a)),
            segment_commitment(std::slice::from_ref(&b)),
            "fixture bug: a and b must differ in the fields segment_commitment covers"
        );
        assert_eq!(
            segment_commitment(&[a.clone(), b.clone()]),
            segment_commitment(&[b, a]),
            "reordering records must not change the commitment"
        );
    }

    /// Adding, removing or altering ONE record breaks the attestation.
    #[test]
    fn altering_a_record_breaks_the_attestation() {
        let sk = sk();
        let mut seg = signed_segment(&sk, "abcd", Plane::Node, 0, "", 2);
        seg.records[0].signed_bytes[0] ^= 0xff;
        assert_eq!(verify_segment_attestation(&seg), None, "must fail closed");

        let mut short = signed_segment(&sk, "abcd", Plane::Node, 0, "", 2);
        short.records.pop();
        assert_eq!(verify_segment_attestation(&short), None);
    }

    /// The attestation binds the segment's POSITION as well as its contents: replaying a
    /// genuine segment at another index, or under another plane tag, fails.
    #[test]
    fn the_attestation_binds_plane_index_and_predecessor() {
        let sk = sk();
        let good = signed_segment(&sk, "abcd", Plane::Clinical, 4, "cafe", 2);
        assert!(verify_segment_attestation(&good).is_some());

        for mutated in [
            Segment {
                index: 5,
                ..good.clone()
            },
            Segment {
                plane: Plane::Node,
                ..good.clone()
            },
            Segment {
                prev_commitment: "f00d".into(),
                ..good.clone()
            },
        ] {
            assert_eq!(
                verify_segment_attestation(&mutated),
                None,
                "a genuine attestation must not validate a segment moved in the chain"
            );
        }
    }

    /// C1 (#500 final review): `source_seq` is the medium's cursor — `watermark()` trusts
    /// `max(source_seq)` over the verified prefix — yet it is COSE-signed nowhere (only
    /// `signed_bytes` is) and, before this fix, was not committed to either. Altering it
    /// after signing must now break the segment's attestation exactly like altering any
    /// other field of a record does.
    #[test]
    fn altering_source_seq_breaks_the_attestation() {
        let sk = sk();
        let mut seg = signed_segment(&sk, "abcd", Plane::Clinical, 0, "", 2);
        // Flip the sign bit: the same shape of damage the finding describes — a single bit
        // turning a modest seq into a huge one (~4.6e18), which is what would silently wreck
        // the watermark if this field were uncommitted.
        seg.records[0].source_seq ^= i64::MIN;
        assert_eq!(
            verify_segment_attestation(&seg),
            None,
            "source_seq must be bound into the commitment, or a corrupted cursor verifies clean"
        );
    }

    /// An unsigned segment yields no attested id — never a wrong one. Fail closed.
    #[test]
    fn an_unsigned_segment_attests_nothing() {
        let seg = Segment {
            attestation: None,
            ..segment(Plane::Clinical, 0, 1)
        };
        assert_eq!(verify_segment_attestation(&seg), None);
    }

    /// A tampered attestation withholds the id rather than misdirecting it. The attacker
    /// holds no private key, so a WRONG-but-valid attestation cannot be forged — the only
    /// achievable outcome is withholding, which fails closed.
    #[test]
    fn a_tampered_attestation_fails_closed() {
        let sk = sk();
        let mut seg = signed_segment(&sk, "abcd", Plane::Clinical, 0, "", 1);
        let att = seg.attestation.as_mut().unwrap();
        let last = att.len() - 1;
        att[last] ^= 0x01;
        assert_eq!(verify_segment_attestation(&seg), None);
    }

    /// `event_set_commitment` keeps its exact CAIRNB2 value after being refactored to
    /// share `commitment_over` with `segment_commitment`. A changed value would
    /// invalidate every existing signed medium in the field.
    ///
    /// I9 (#500 final review): the ORIGINAL version of this test derived N=1 independently
    /// but only `assert_ne!`'d N=2 against itself — and N=1 alone does not pin the JOIN.
    /// The most plausible future change (inserting a separator between the concatenated
    /// addresses before hashing) is IDENTICAL to today's value at N=1 (there is only one
    /// address to "separate" from nothing) and also satisfies a bare `assert_ne!` at N=2 (a
    /// different-but-still-wrong value is still different from `one`). So the guard passed
    /// while the exact case it claims to pin — every medium with more than one record —
    /// silently broke. Derive N=2 independently too, by the same recipe `commitment_over`
    /// documents (sort the addresses, concatenate with NO separator, hash), and assert
    /// EQUALITY, not just difference.
    #[test]
    fn event_set_commitment_is_unchanged_by_the_shared_helper() {
        let sk = sk();
        let e1 = enroll(&sk, "a");
        let e2 = enroll(&sk, "b");
        // Pinned by construction: the commitment of a one-event set is the multihash of
        // that event's own address, which we can compute here independently.
        let one = crate::marker::event_set_commitment(std::slice::from_ref(&e1));
        let expected_one =
            hex::encode(cairn_event::event_address(&cairn_event::event_address(&e1)));
        assert_eq!(
            one, expected_one,
            "the CAIRNB2 commitment must not change (N=1)"
        );

        let two = crate::marker::event_set_commitment(&[e1.clone(), e2.clone()]);
        let mut addresses = [
            cairn_event::event_address(&e1),
            cairn_event::event_address(&e2),
        ];
        addresses.sort();
        let expected_two = hex::encode(cairn_event::event_address(&addresses.concat()));
        assert_eq!(
            two, expected_two,
            "the CAIRNB2 commitment must not change (N=2) — a changed join (e.g. an \
             inserted separator) is invisible at N=1 and would still pass a bare assert_ne!"
        );
        assert_ne!(
            one, two,
            "sanity: different sets must commit to different values"
        );
    }

    /// `record_count` is checked as its OWN conjunct, not merely riding along with
    /// `segment_commitment`.
    ///
    /// Mutation testing (task 6 review) found that deleting the `record_count` conjunct
    /// turned no test red: `build_segment_attestation`'s public API always derives
    /// `record_count` and `segment_commitment` from the SAME records slice, so every
    /// existing test that shortens or alters a segment's records changes its
    /// `segment_commitment` too — that conjunct alone already catches them, leaving
    /// `record_count` untested. Per house rule 5, a conjunct no test kills gets a test
    /// added, not quietly deleted. This test signs a hand-built payload (bypassing
    /// `build_segment_attestation`, which cannot express the mismatch) whose
    /// `segment_commitment` DOES match the segment's real records but whose
    /// `record_count` deliberately does not — isolating the one conjunct that can
    /// actually catch it.
    #[test]
    fn record_count_is_checked_independently_of_segment_commitment() {
        let sk = sk();
        let kid = hex::encode(sk.verifying_key().to_bytes());
        let records = vec![
            tests_support::distinct_record(1, 0),
            tests_support::distinct_record(1, 1),
        ];
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: cairn_event::NIL_PATIENT.into(),
            event_type: SEGMENT_ATTEST_TYPE.into(),
            schema_version: "node/1".into(),
            hlc: Hlc {
                wall: 0,
                counter: 0,
                node_origin: "abcd".into(),
            },
            t_effective: None,
            signer_key_id: kid.clone(),
            contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
            payload: serde_json::json!({
                "self_node_id_hex": "abcd",
                "plane": Plane::Clinical.label(),
                "segment_index": 0,
                // Deliberately WRONG: one more than `records.len()`, while
                // `segment_commitment` below is computed honestly over the real records.
                "record_count": records.len() as u64 + 1,
                "segment_commitment": segment_commitment(&records),
                "prev_commitment": "",
            }),
            attachments: vec![],
            plaintext_twin: None,
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
            safety: None,
        };
        let attestation = sign(&body, &sk).expect("signing").signed_bytes;
        let seg = Segment {
            plane: Plane::Clinical,
            index: 0,
            prev_commitment: String::new(),
            self_node_id_hex: "abcd".into(),
            attestation: Some(attestation),
            records,
        };
        assert_eq!(
            verify_segment_attestation(&seg),
            None,
            "record_count must be checked even when segment_commitment matches"
        );
    }

    /// `segment_commitment`'s recipe is pinned by an INDEPENDENT derivation, exactly as
    /// `event_set_commitment` already is.
    ///
    /// `source_seq.to_be_bytes()` -> `to_le_bytes()` survived the whole suite, because every
    /// other assertion computes the expected value with the very function under test. This
    /// value is written ONTO the medium as every segment's `prev_commitment` and is verified
    /// by OTHER NODES ON OTHER BUILDS — principle 11 makes a mixed-version fleet the normal
    /// case, so a changed recipe splits the fleet silently. N=1 and N=2 both, because the
    /// N=2 case is what pins the separator-free sorted join.
    #[test]
    fn segment_commitment_recipe_is_pinned_independently() {
        // Derive the expectation by hand, from the format definition rather than from the
        // function: content-address each record's bytes, append its big-endian source_seq,
        // and feed the pair through the crate's one commitment primitive.
        let by_hand = |records: &[MediumRecord]| -> String {
            let items: Vec<Vec<u8>> = records
                .iter()
                .map(|r| {
                    let mut v = cairn_event::event_address(&r.signed_bytes);
                    v.extend_from_slice(&r.source_seq.to_be_bytes());
                    v
                })
                .collect();
            let refs: Vec<&[u8]> = items.iter().map(Vec::as_slice).collect();
            crate::marker::commitment_over(&refs)
        };

        let one = vec![tests_support::distinct_record(1, 0)];
        assert_eq!(
            segment_commitment(&one),
            by_hand(&one),
            "N=1 recipe drifted"
        );

        let two = vec![
            tests_support::distinct_record(1, 0),
            tests_support::distinct_record(1, 1),
        ];
        assert_eq!(
            segment_commitment(&two),
            by_hand(&two),
            "N=2 recipe drifted"
        );

        // And the two differ, so the N=2 case is not vacuously equal to the N=1 case.
        assert_ne!(segment_commitment(&one), segment_commitment(&two));

        // `source_seq` really is inside the commitment: changing only it must change the
        // value. (This is the C1 fix from the earlier review; pinned here against the
        // endianness mutation as well.)
        let mut moved = one.clone();
        moved[0].source_seq += 1;
        assert_ne!(
            segment_commitment(&one),
            segment_commitment(&moved),
            "source_seq must be bound, or the watermark is authenticated by nothing"
        );
    }

    /// A validly-signed event of the WRONG TYPE is not a segment attestation.
    ///
    /// Deleting the `event_type` check survived the suite: every existing fixture failed on a
    /// later payload-key lookup instead, so the type check was never the thing under test.
    /// It is the one bind that stops a GENUINE signed object being replayed into the wrong
    /// role — the payload keys alone are not a type.
    #[test]
    fn a_signed_event_of_the_wrong_type_is_not_a_segment_attestation() {
        let sk = crate::testkit::sk();
        let key_id = crate::testkit::kid(&sk);
        let records = vec![tests_support::distinct_record(1, 0)];
        let seg = Segment {
            plane: Plane::Clinical,
            index: 0,
            prev_commitment: String::new(),
            self_node_id_hex: "abcd".into(),
            attestation: None,
            records: records.clone(),
        };

        // Every payload field a real attestation carries, correct in every particular —
        // only `event_type` is wrong.
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: cairn_event::NIL_PATIENT.into(),
            event_type: "clinical.medication".into(),
            schema_version: "node/1".into(),
            hlc: Hlc {
                wall: 0,
                counter: 0,
                node_origin: "abcd".into(),
            },
            t_effective: None,
            signer_key_id: key_id.clone(),
            contributors: serde_json::json!([{"actor_id": key_id, "role": "recorded"}]),
            payload: serde_json::json!({
                "self_node_id_hex": "abcd",
                "plane": Plane::Clinical.label(),
                "plane_tag": Plane::Clinical.tag(),
                "segment_index": 0,
                "record_count": records.len(),
                "segment_commitment": segment_commitment(&records),
                "prev_commitment": "",
            }),
            attachments: vec![],
            plaintext_twin: None,
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
            safety: None,
        };
        let impostor = sign(&body, &sk).expect("signing").signed_bytes;

        let seg = Segment {
            attestation: Some(impostor),
            ..seg
        };
        assert_eq!(
            verify_segment_attestation(&seg),
            None,
            "a genuinely signed event whose payload happens to match is still not a \
             `node.segment_attested`; without the type check any signed object with the \
             right keys could be replayed into this role"
        );

        // Positive control: the SAME segment with a real attestation verifies, so the
        // rejection above is about the type and nothing else.
        let real =
            build_segment_attestation(&sk, &key_id, "abcd", Plane::Clinical, 0, "", &records);
        let good = Segment {
            attestation: Some(real),
            ..seg
        };
        assert_eq!(verify_segment_attestation(&good), Some("abcd".into()));
    }
}
