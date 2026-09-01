//! Tests for `crate::chain`, kept in this sibling file (Rust's standard non-`mod.rs`
//! layout — `chain.rs` declares `mod tests;`, which resolves here) purely to keep
//! `chain.rs` itself under the crate's 500-line cap (house rule 4). This is a file-layout
//! move only: no production code moved, no public API changed, and `use super::*` below
//! still reaches everything `chain.rs` defines exactly as it did when this module was
//! inline. See `chain.rs`'s module docs for why the CONTENT lives together as one module.

use super::*;
use crate::attest::{segment_commitment, tests_support};
use crate::record::MediumRecord;
use crate::segment::Segment;
use crate::testkit::{enroll, sk};

#[test]
fn a_well_formed_chain_verifies_through_its_last_segment() {
    let (m, _) = crate::testkit::chain_of(3, 1);
    let r = chain_report(&m);
    assert!(r.chain_intact(), "faults: {:?}", r.faults);
    assert_eq!(r.verified_through, Some(2));
    assert_eq!((r.segments, r.signed_valid, r.unsigned), (3, 3, 0));
}

/// A break is located by plane AND index. "chain invalid" sends an operator nowhere.
///
/// I8 (#500 final review): mangling `prev_commitment` fires BOTH `ChainBroken` (the direct
/// prev-mismatch check) and `AttestationInvalid` (the attestation's SIGNED `prev_commitment`
/// no longer matches the mutated field) — the ORIGINAL assertion here was an OR over the
/// two, which passes on either and so never proved `ChainBroken` was constructed at all,
/// let alone with correct payload. Assert `ChainBroken` specifically, with its `plane`,
/// `expected` and `found` — nothing else in the crate asserts those fields, so a swapped
/// `expected`/`found` would ship silently and mislead an operator at the exact moment they
/// need the truth.
#[test]
fn a_chain_break_is_located_not_merely_counted() {
    let (mut m, _) = crate::testkit::chain_of(4, 1);
    let real_predecessor_commitment = segment_commitment(&m.segments[1].records);
    m.segments[2].prev_commitment = "deadbeef".into();
    let r = chain_report(&m);
    assert!(!r.chain_intact());
    assert_eq!(
        r.verified_through,
        Some(1),
        "verified through the last GOOD segment"
    );
    let broken = r.faults.iter().find_map(|f| match f {
        SegmentFault::ChainBroken {
            plane,
            position,
            index: 2,
            expected,
            found,
        } => Some((plane, position, expected, found)),
        _ => None,
    });
    match broken {
        Some((plane, position, expected, found)) => {
            assert_eq!(*plane, Plane::Clinical);
            assert_eq!(
                *position, 2,
                "located by the TRUSTED coordinate — where the reader found it — as well as \
                 by the segment's own self-declared index"
            );
            assert_eq!(
                *expected, real_predecessor_commitment,
                "expected must be the REAL predecessor's commitment"
            );
            assert_eq!(
                found, "deadbeef",
                "found must be the mutated field, not swapped with expected"
            );
        }
        None => panic!("ChainBroken at index 2 must be constructed: {:?}", r.faults),
    }
}

/// A genuine segment spliced from ANOTHER medium fails on its predecessor, even though
/// its own signature and commitment are perfectly valid.
///
/// I8 (#500 final review): the ORIGINAL test asserted only `!r.chain_intact()`, which is true for
/// a dozen unrelated reasons and proves nothing about WHICH check caught the splice. This
/// asserts the property the test's own name claims: the spliced segment's own attestation
/// verifies fine in isolation (it is not a forgery), and `chain_report` flags it
/// specifically via `ChainBroken`, not `AttestationInvalid` — it fails on its PREDECESSOR.
#[test]
fn a_spliced_segment_fails_on_its_predecessor() {
    let (mut mine, _) = crate::testkit::chain_of(2, 1);
    let (theirs, _) = crate::testkit::chain_of(2, 2);
    let spliced = theirs.segments[1].clone();
    assert!(
        crate::attest::verify_segment_attestation(&spliced).is_some(),
        "a spliced segment's OWN signature and commitment must still be genuine"
    );
    mine.segments[1] = spliced;
    let r = chain_report(&mine);
    assert!(
        !r.chain_intact(),
        "a foreign segment must not validate here"
    );
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, SegmentFault::ChainBroken { index: 1, .. })),
        "must fail specifically on its PREDECESSOR (ChainBroken): {:?}",
        r.faults
    );
    assert!(
        !r.faults
            .iter()
            .any(|f| matches!(f, SegmentFault::AttestationInvalid { index: 1, .. })),
        "the spliced segment's own attestation is genuine, so it must NOT also be flagged \
         AttestationInvalid — that would muddy which check actually caught this: {:?}",
        r.faults
    );
}

/// The watermark comes from the last VERIFIED segment, so a torn or broken tail costs
/// exactly one increment: its records are re-captured, never lost.
///
/// I8 (#500 final review): `after < good` also passes if a regression collapses the
/// watermark to `None` — `Option`'s derived ordering makes `None < Some(_)` true — so the
/// ORIGINAL assertion could not tell "correctly retreated to segment 1's seq" apart from
/// "silently lost the watermark entirely". Assert the exact expected value instead.
#[test]
fn the_watermark_ignores_everything_after_the_last_verified_segment() {
    let (mut m, seqs) = crate::testkit::chain_of(3, 1);
    let good = watermark(&m, &chain_report(&m), Plane::Clinical);
    assert_eq!(good, Some(seqs.last().copied().unwrap()));
    m.segments[2].records[0].signed_bytes[0] ^= 0xff; // breaks segment 2
    let after = watermark(&m, &chain_report(&m), Plane::Clinical);
    assert_eq!(
        after,
        Some(seqs[1]),
        "must retreat to exactly the last VERIFIED segment's seq, not merely 'something smaller'"
    );
}

/// A plane with no verified segment has NO watermark — `None`, never `Some(0)`. Zero is
/// a claim ("I hold everything up to seq 0"); the honest answer is "I do not know".
///
/// I8 (#500 final review): the ORIGINAL fixture used a medium with no Node-plane segment AT
/// ALL, so it never exercised what the invariant actually claims — "a segment exists but
/// did not verify" is a different case from "no such segment exists", and a regression
/// conflating the two would slip past. Adds that case: a genuine Node segment is present but
/// tampered, sitting after a verified Clinical segment — the watermark for `Plane::Node`
/// must still be `None`, not the seq of the unverified segment's record.
#[test]
fn a_plane_with_no_verified_segment_has_no_watermark() {
    let (m, _) = crate::testkit::chain_of(1, 1); // clinical only
    assert_eq!(watermark(&m, &chain_report(&m), Plane::Node), None);

    let sk = sk();
    let clinical = tests_support::signed(
        &sk,
        "abcd",
        Plane::Clinical,
        0,
        "",
        vec![tests_support::salted_record(1, 0)],
    );
    let prev = segment_commitment(&clinical.records);
    let mut node = tests_support::signed(
        &sk,
        "abcd",
        Plane::Node,
        1,
        &prev,
        vec![tests_support::salted_record(2, 0)],
    );
    node.records[0].signed_bytes[0] ^= 0xff; // tamper: breaks its own attestation
    let m2 = crate::testkit::medium_v3(vec![clinical, node]);
    let r2 = chain_report(&m2);
    assert_eq!(
        r2.verified_through,
        Some(0),
        "sanity: the break stops the walk at segment 1"
    );
    assert_eq!(
        watermark(&m2, &r2, Plane::Node),
        None,
        "a Node segment EXISTS but did not verify — still None, never the unverified seq"
    );
}

/// Self-identification takes the LAST verified attestation and binds the named id to a
/// genesis present on this medium, signed by the same key.
#[test]
fn self_id_binds_the_named_node_to_a_genesis_on_this_medium() {
    let (m, _) = crate::testkit::chain_with_genesis();
    let r = chain_report(&m);
    assert!(self_id_from_chain(&m, &r).is_some());
}

/// Covers the EARLY-RETURN path only: this fixture clears the genesis segment's own
/// records, which also breaks THAT segment's own attestation (its commitment no longer
/// matches the now-empty records), so `chain_report`'s `verified_through` comes back
/// `None` for the whole medium and `self_id_from_chain` returns via
/// `report.verified_through?` before its loop — and the signer-bind conjunct inside
/// that loop — ever runs. This is real, distinct coverage of that early-exit gate, but
/// it does NOT exercise the signer bind; see
/// `a_forged_self_id_naming_a_real_genesis_is_withheld_not_misdirected` below for a
/// test that reaches the loop with an intact chain and isolates that property instead.
#[test]
fn an_unbound_self_id_is_withheld_not_guessed() {
    let (mut m, _) = crate::testkit::chain_with_genesis();
    m.segments[0].records.clear(); // remove the genesis; the attestation now mismatches
    let r = chain_report(&m);
    assert_eq!(self_id_from_chain(&m, &r), None);
}

/// Mutation testing (task 8 step 5) found that the test above never reaches the SIGNER
/// bind at all — see its doc comment. Per house rule 5, a conjunct no test kills gets a
/// test added, not quietly waved through.
///
/// This test reaches the loop with a fully INTACT, correctly-chained medium instead, so
/// the signer bind is the ONLY thing standing between a forged self-id and a false
/// positive: an attacker holding no key for the real node can still sign a *valid*
/// segment attestation that simply CLAIMS the real node's genuine node-id
/// (`self_node_id_hex` is attacker-supplied, not derived) — the id half of the match
/// succeeds on its own. The genesis segment is deliberately left UNSIGNED, so the only
/// signed attestation anywhere on the medium naming `self_id` is the forged one; that
/// isolates the signer-key comparison from the genuine-owner segment that would
/// otherwise legitimately self-identify further back in the walk.
///
/// Since task 8 review FIX 1, `chain_report` ALSO locates this as a `SelfIdUnbound`
/// fault (it no longer merely returns a bare `None` from `self_id_from_chain`) — this
/// test checks both: the fault is located, and identification is still withheld.
#[test]
fn a_forged_self_id_naming_a_real_genesis_is_withheld_not_misdirected() {
    let owner = sk();
    let genesis = enroll(&owner, "real-node");
    let self_id = hex::encode(cairn_event::event_address(&genesis));
    let node_records = vec![MediumRecord {
        signed_bytes: genesis,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 1,
    }];
    let s0 = Segment {
        plane: Plane::Node,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: self_id.clone(),
        attestation: None, // unsigned: no genuine attestation for `self_id` exists here
        records: node_records,
    };
    let prev = segment_commitment(&s0.records);
    // An attacker, holding no key for `owner`, signs a genuinely-valid attestation that
    // simply claims the real node's self_id.
    let attacker = sk();
    let s1 = tests_support::signed(
        &attacker,
        &self_id,
        Plane::Clinical,
        1,
        &prev,
        vec![tests_support::salted_record(9, 0)],
    );
    let m = crate::testkit::medium_v3(vec![s0, s1]);
    let r = chain_report(&m);
    assert!(
        r.faults.iter().any(|f| matches!(
            f,
            SegmentFault::SelfIdUnbound {
                plane: Plane::Clinical,
                index: 1,
                ..
            }
        )),
        "the forged claim must be LOCATED as a fault, not silently absorbed into a \
         bare None: {:?}",
        r.faults
    );
    assert_eq!(
        self_id_from_chain(&m, &r),
        None,
        "the claimed id matches a real genesis, but the signer does not — must withhold"
    );
}

/// FIX 1 (task 8 review, #500): a signed segment naming an id with no matching genesis,
/// on a medium that DOES carry a genesis, is the single most security-relevant failure
/// this file can report — a forged identity claim — and it must be LOCATED, never
/// merely counted. Unlike the forgery test above (which isolates the SIGNER half of the
/// bind), this isolates the ID half: the claimed id itself matches no genesis anywhere,
/// regardless of who signed the claim.
#[test]
fn a_signed_segment_naming_an_unbound_id_is_located_as_a_fault() {
    let owner = sk();
    let real_genesis = enroll(&owner, "real-node");
    let real_id = hex::encode(cairn_event::event_address(&real_genesis));
    let node_records = vec![MediumRecord {
        signed_bytes: real_genesis,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 1,
    }];
    let s0 = tests_support::signed(&owner, &real_id, Plane::Node, 0, "", node_records);
    let prev = segment_commitment(&s0.records);

    // A genuinely different node's id — no genesis for IT exists anywhere on this
    // medium — claimed by an otherwise perfectly validly-signed clinical segment.
    let stranger = sk();
    let unbound_id = hex::encode(cairn_event::event_address(&enroll(&stranger, "Ghost")));
    let s1 = tests_support::signed(
        &owner,
        &unbound_id,
        Plane::Clinical,
        1,
        &prev,
        vec![tests_support::salted_record(9, 0)],
    );

    let m = crate::testkit::medium_v3(vec![s0, s1]);
    let r = chain_report(&m);
    let fault = r.faults.iter().find(|f| {
        matches!(
            f,
            SegmentFault::SelfIdUnbound {
                plane: Plane::Clinical,
                index: 1,
                ..
            }
        )
    });
    match fault {
        Some(SegmentFault::SelfIdUnbound {
            self_node_id_hex, ..
        }) => {
            assert_eq!(
                *self_node_id_hex, unbound_id,
                "the fault must NAME the claimed id"
            );
        }
        other => panic!(
            "expected a located SelfIdUnbound fault on clinical segment 1, got {other:?} \
             (all faults: {:?})",
            r.faults
        ),
    }
}

/// I5 (#500 final review): pins that `verified_through` is STRUCTURAL chain continuity,
/// not tamper-evidence — `SelfIdUnbound` is the clearest proof. The records in a
/// forged-self-id segment ARE validly signed and correctly chained; only the identity
/// CLAIM is forged. So `chain_report` must NOT retract `verified_through` past it —
/// retracting would treat a records-trustworthy segment as if its records were suspect
/// too, which they are not. See `ChainReport::verified_through`'s doc for the full nuance.
#[test]
fn self_id_unbound_does_not_retract_verified_through() {
    let owner = sk();
    let real_genesis = enroll(&owner, "real-node");
    let real_id = hex::encode(cairn_event::event_address(&real_genesis));
    let node_records = vec![MediumRecord {
        signed_bytes: real_genesis,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 1,
    }];
    let s0 = tests_support::signed(&owner, &real_id, Plane::Node, 0, "", node_records);
    let prev = segment_commitment(&s0.records);
    let stranger = sk();
    let unbound_id = hex::encode(cairn_event::event_address(&enroll(&stranger, "Ghost")));
    let s1 = tests_support::signed(
        &owner,
        &unbound_id,
        Plane::Clinical,
        1,
        &prev,
        vec![tests_support::salted_record(9, 0)],
    );
    let m = crate::testkit::medium_v3(vec![s0, s1]);
    let r = chain_report(&m);
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, SegmentFault::SelfIdUnbound { index: 1, .. })),
        "sanity: the forged claim must still be located: {:?}",
        r.faults
    );
    assert_eq!(
        r.verified_through,
        Some(1),
        "a forged self-id claim must NOT retract verified_through — the records themselves \
         are genuinely signed and chained, only the identity claim is forged"
    );
}

/// I5 (#500 final review): the flip side of the pin above. An UNSIGNED segment's
/// `prev_commitment` is checked against the same public, derivable `segment_commitment`
/// the chain always compares against — no signature required — so a well-formed unsigned
/// segment advances `verified_through` (and hence the watermark) purely on chain-link
/// shape, with ZERO tamper-evidence over its `source_seq` or its records. Operationally
/// necessary (principle 7: an unavailable signing key must never block a backup) and a
/// pinned, deliberate limitation, not a bug.
#[test]
fn an_unsigned_segment_advances_verified_through_with_no_tamper_evidence() {
    let m = crate::testkit::unsigned_chain_of(2);
    let r = chain_report(&m);
    assert_eq!(
        r.verified_through,
        Some(1),
        "a well-formed unsigned chain-link is enough to advance verified_through"
    );
    assert_eq!(
        watermark(&m, &r, Plane::Clinical),
        Some(2),
        "the watermark trusts an unsigned segment's source_seq just as much as a signed one's"
    );
}

/// FIX 1's guard: with NO node-plane segment (hence no genesis at all) on the medium,
/// the honest verdict for every signed segment's self-id claim is "cannot determine",
/// not "failed" — a partial capture may legitimately not carry the node plane yet, and
/// flagging that as a fault would red-flag a healthy, still-partial medium.
#[test]
fn no_genesis_on_the_medium_raises_no_self_id_unbound_fault() {
    let (m, _) = crate::testkit::chain_of(3, 1);
    let r = chain_report(&m);
    assert!(
        !r.faults
            .iter()
            .any(|f| matches!(f, SegmentFault::SelfIdUnbound { .. })),
        "no genesis anywhere means 'cannot determine', not a fault: {:?}",
        r.faults
    );
}

/// Every record's SIGNATURE is checked, not merely its commitment.
///
/// This is a distinct property and it is easy to lose: the chain pass verifies
/// attestations and commitments, and a commitment is over content ADDRESSES — which a
/// tampered blob still has. So in a SIGNED segment tampering is caught twice (the
/// address changes, so the commitment fails), but in an UNSIGNED segment there is no
/// attestation at all, and without this pass nothing would check the bytes.
#[test]
fn a_tampered_record_is_caught_even_in_an_unsigned_segment() {
    let mut m = crate::testkit::unsigned_chain_of(2);
    assert!(chain_report(&m).chain_intact(), "unsigned but well-formed");
    let clean = verify_records(&m);
    assert_eq!(
        clean.first_bad, None,
        "the fixture's records must verify to begin with"
    );

    m.segments[1].records[0].signed_bytes[0] ^= 0xff;
    let report = verify_records(&m);
    assert_eq!(report.total, 2);
    assert_eq!(report.intact, 1);
    assert_eq!(
        report.first_bad,
        Some(1),
        "and it must NAME which record failed"
    );
}

/// An all-unsigned medium identifies nobody, and says so without inventing a fault.
#[test]
fn an_unsigned_medium_identifies_nobody_and_is_not_a_fault() {
    let m = crate::testkit::unsigned_chain_of(2);
    let r = chain_report(&m);
    assert_eq!((r.signed_valid, r.unsigned), (0, 2));
    assert!(
        r.chain_intact(),
        "unsigned is not a FAULT — it is a declared limitation"
    );
    assert_eq!(self_id_from_chain(&m, &r), None);
}

// ---------------------------------------------------------------------------
// Mutation-killers (#500 slice 2a review). Each test below was written because a
// single-line mutation to production code survived the whole suite.
// ---------------------------------------------------------------------------

/// `self_id_from_chain` must return the ATTESTED id — never the segment's untrusted
/// plaintext `self_node_id_hex`.
///
/// The mutation `return Some(id)` -> `return Some(seg.self_node_id_hex.clone())` survived
/// every test in this crate, because every fixture set the plaintext field EQUAL to the
/// attested id, making the two indistinguishable. `segment.rs` documents this footgun in
/// prose ("a future caller easily could reach for the obviously-named field") — this is the
/// test that makes the prose enforceable. If it ever regressed, restore would record an
/// immutable supersede edge against an attacker-supplied node id, which is the single
/// failure issue #53 exists to prevent.
#[test]
fn self_id_returns_the_attested_id_not_the_plaintext_field() {
    let (mut m, _) = crate::testkit::chain_with_genesis();
    let attested = self_id_from_chain(&m, &chain_report(&m)).expect("the fixture identifies");

    // Make the plaintext field DISAGREE with the signed one. Nothing re-signs, so the
    // attestation still binds the real id; only the untrusted field changes.
    // Runtime-derived, not a literal: a 64-hex-char string in an identity slot is the shape
    // CodeQL's hard-coded-cryptographic-value rule reacts to (house rule 6, #146), and a
    // derived value is exactly as deterministic.
    let decoy: String = (0..32u8)
        .map(|i| format!("{:02x}", i.wrapping_mul(7).wrapping_add(3)))
        .collect();
    for seg in &mut m.segments {
        seg.self_node_id_hex.clone_from(&decoy);
    }
    assert_ne!(
        attested, decoy,
        "the fixture must actually distinguish the two"
    );

    let r = chain_report(&m);
    assert_eq!(
        self_id_from_chain(&m, &r),
        Some(attested),
        "the identification must come from the SIGNED payload; returning the plaintext field \
         would hand a caller an attacker-supplied string as a node identity"
    );
}

/// A broken chain link retracts `verified_through` even on an UNSIGNED segment.
///
/// Deleting `ok = false` from the `ChainBroken` arm survived the whole suite: every test that
/// broke a chain link did so on a SIGNED segment, where the attestation independently failed
/// and set `ok = false` anyway. So the chain-link half of the retraction was never the thing
/// under test. On an unsigned segment there is no attestation to mask it, and a spliced
/// predecessor would otherwise advance the watermark past records nothing verified.
#[test]
fn a_broken_link_retracts_verified_through_even_when_unsigned() {
    let mut m = crate::testkit::unsigned_chain_of(3);
    m.segments[1].prev_commitment = "deadbeef".into();
    let r = chain_report(&m);
    assert_eq!(
        (r.signed_valid, r.signed_invalid, r.unsigned),
        (0, 0, 3),
        "the fixture must be entirely unsigned, or an attestation could mask the break"
    );
    assert_eq!(
        r.verified_through,
        Some(0),
        "the walk must stop at the last segment before the break, on the chain link ALONE"
    );
    assert_eq!(
        watermark(&m, &r, Plane::Clinical),
        Some(1),
        "and the watermark must retreat with it, not run on to the file's tail"
    );
}

/// The watermark is `max`, not "the last segment's seq".
///
/// `.max()` -> `.last()` survived the suite because every fixture had exactly ONE record per
/// segment with `source_seq` ascending in lockstep with segment order, making `max`, `last`
/// and "the last segment's only record" indistinguishable. Records within a segment are NOT
/// ordered — frame reordering is harmless under set-union sync, and `segment_commitment` is
/// explicitly order-independent — so a watermark that depended on order would be wrong for
/// any real capture batch.
#[test]
fn the_watermark_is_the_maximum_not_the_last_record() {
    let sk = sk();
    // One segment, several records, seqs deliberately NOT in ascending order.
    let records = vec![
        MediumRecord {
            signed_bytes: enroll(&sk, "a"),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: 40,
        },
        MediumRecord {
            signed_bytes: enroll(&sk, "b"),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: 7,
        },
    ];
    let seg = tests_support::signed(&sk, "abcd", Plane::Clinical, 0, "", records);
    let m = crate::testkit::medium_v3(vec![seg]);
    let r = chain_report(&m);
    assert_eq!(
        watermark(&m, &r, Plane::Clinical),
        Some(40),
        "the highest seq held, regardless of the order records happen to sit in"
    );
}

/// A record legitimately AT seq 0 yields `Some(0)` — invariant 8 cuts both ways.
///
/// Every fixture used seqs >= 1, so `.max()` -> `.max().filter(|v| *v != 0)` survived. The
/// invariant is that `None` and `Some(0)` are DIFFERENT statements ("I do not know" versus
/// "I hold through seq 0"); a test that only ever exercises the `None` side proves half of it.
#[test]
fn a_watermark_of_zero_is_some_zero_not_none() {
    let sk = sk();
    let records = vec![MediumRecord {
        signed_bytes: enroll(&sk, "a"),
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 0,
    }];
    let seg = tests_support::signed(&sk, "abcd", Plane::Clinical, 0, "", records);
    let m = crate::testkit::medium_v3(vec![seg]);
    let r = chain_report(&m);
    assert_eq!(
        watermark(&m, &r, Plane::Clinical),
        Some(0),
        "zero is a CLAIM ('I hold through 0'); collapsing it to None would report a medium \
         that holds something as holding nothing"
    );
    assert_eq!(
        watermark(&m, &r, Plane::Node),
        None,
        "while a plane with no verified segment at all really does hold nothing knowable"
    );
}

/// A hole in a plane's seq run is reported, so `watermark` can never be mistaken for a
/// completeness claim.
///
/// `watermark` is `max`: for seqs 1,2,3,5,6 it returns `Some(6)`, and a caller using that as
/// a cursor would start after 6 and never capture seq 4 — while the medium reported itself
/// complete through 6. `seq_gaps` is the mechanism that makes the hole visible; the policy
/// (refuse? re-capture? warn?) belongs to the slice that owns capture.
#[test]
fn a_gap_in_the_seq_run_is_reported_not_absorbed() {
    let sk = sk();
    let mk = |seq: i64, name: &str| MediumRecord {
        signed_bytes: enroll(&sk, name),
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: seq,
    };
    // 1,2,3 then 5,6 — seq 4 is missing.
    let records = vec![mk(1, "a"), mk(2, "b"), mk(3, "c"), mk(5, "e"), mk(6, "f")];
    let seg = tests_support::signed(&sk, "abcd", Plane::Clinical, 0, "", records);
    let m = crate::testkit::medium_v3(vec![seg]);
    let r = chain_report(&m);
    assert_eq!(watermark(&m, &r, Plane::Clinical), Some(6));
    assert_eq!(
        seq_gaps(&m, &r, Plane::Clinical),
        vec![(3, 5)],
        "the hole between 3 and 6 must be named; a caller trusting the watermark alone would \
         skip seq 4 forever while the medium reported itself complete"
    );
    // A contiguous run has no gaps — without this the function could return everything.
    let (clean, _) = crate::testkit::verifiable_chain_of(3);
    let cr = chain_report(&clean);
    assert!(seq_gaps(&clean, &cr, Plane::Clinical).is_empty());
}

/// A segment whose self-declared index disagrees with where it sits is a located fault.
///
/// Nothing checked this, so on an unsigned segment `index` was free for an attacker to set —
/// and EVERY other fault is "located" by it. `IndexMismatch` is what makes the other faults'
/// locations trustworthy, and it turns #522 (two crates deriving the next index with no
/// shared helper) from a silent divergence into a loud one.
#[test]
fn a_segment_that_lies_about_its_index_is_caught() {
    let mut m = crate::testkit::unsigned_chain_of(2);
    m.segments[1].index = 4_000_000_000;
    let r = chain_report(&m);
    assert!(
        r.faults.iter().any(|f| matches!(
            f,
            SegmentFault::IndexMismatch {
                position: 1,
                declared: 4_000_000_000,
                ..
            }
        )),
        "the fault must carry BOTH the trusted position and the declared value: {:?}",
        r.faults
    );
    assert_eq!(
        r.verified_through,
        Some(0),
        "and an unverifiable ordering claim must not advance the cursor"
    );
}

/// Two planes interleaved in ONE medium keep independent watermarks, and the single global
/// chain runs across both.
///
/// Every existing fixture was node@0 then clinical@1, never interleaved, so nothing proved
/// the per-plane filter worked rather than both reads returning the file's tail.
#[test]
fn interleaved_planes_keep_independent_watermarks() {
    let sk = sk();
    let mk = |seq: i64, name: &str| MediumRecord {
        signed_bytes: enroll(&sk, name),
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: seq,
    };
    // node@0, clinical@1, node@2 — the interleaving the fixtures never had.
    //
    // The node-plane records are real `node.enrolled` events, so this medium carries a
    // genesis and the self-id bind is live: the segments must therefore name the genesis's
    // own content address (which is what a node id IS), not an arbitrary string, or every
    // segment would correctly fail `SelfIdUnbound` and the test would be measuring that
    // instead of the watermarks.
    let genesis = mk(10, "n0");
    let self_id = hex::encode(cairn_event::event_address(&genesis.signed_bytes));
    let mut segments: Vec<Segment> = Vec::new();
    let mut prev = String::new();
    for (i, (plane, rec)) in [
        (Plane::Node, genesis.clone()),
        (Plane::Clinical, mk(500, "n1")),
        (Plane::Node, mk(11, "n2")),
    ]
    .into_iter()
    .enumerate()
    {
        let seg = tests_support::signed(&sk, &self_id, plane, i as u32, &prev, vec![rec]);
        prev = segment_commitment(&seg.records);
        segments.push(seg);
    }
    let m = crate::testkit::medium_v3(segments);
    let r = chain_report(&m);
    assert!(r.chain_intact(), "{:?}", r.faults);
    assert_eq!(
        r.verified_through,
        Some(2),
        "one chain spans BOTH planes in file order — that is what detects a cross-plane splice"
    );
    assert_eq!(
        watermark(&m, &r, Plane::Node),
        Some(11),
        "the node plane's own maximum, not the file's last record"
    );
    assert_eq!(
        watermark(&m, &r, Plane::Clinical),
        Some(500),
        "and the clinical plane's, independently"
    );
}

/// A failing record is resolved from its flat ordinal back to the segment holding it.
///
/// `VerifyReport::first_bad` is an index into a synthetic flattened list that exists nowhere
/// on the medium — "record 14372 of 20000" sends an operator nowhere. Invariant 9 requires a
/// location, and `locate_record` is it. The fixture uses UNEVEN segment sizes deliberately:
/// with one record per segment the flat ordinal and the segment position coincide by
/// accident, and the mapping would prove nothing.
#[test]
fn a_failing_record_is_located_within_its_segment() {
    let sk = sk();
    let mk = |name: &str| MediumRecord {
        signed_bytes: enroll(&sk, name),
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 1,
    };
    let mut segments = vec![
        tests_support::signed(
            &sk,
            "abcd",
            Plane::Node,
            0,
            "",
            vec![mk("a"), mk("b"), mk("c")],
        ),
        Segment {
            plane: Plane::Clinical,
            index: 1,
            prev_commitment: String::new(),
            self_node_id_hex: "abcd".into(),
            attestation: None,
            records: vec![mk("d"), mk("e")],
        },
    ];
    segments[1].prev_commitment = segment_commitment(&segments[0].records);
    // Corrupt the SECOND record of the SECOND segment: flat ordinal 4.
    segments[1].records[1].signed_bytes[0] ^= 0xff;
    let m = crate::testkit::medium_v3(segments);

    let report = verify_records(&m);
    assert_eq!(report.first_bad, Some(4), "the flat ordinal");
    assert_eq!(
        locate_record(&m, 4),
        Some((1, Plane::Clinical, 1, 1)),
        "which must resolve to (position, plane, declared index, ordinal WITHIN the segment)"
    );
    assert_eq!(locate_record(&m, 0), Some((0, Plane::Node, 0, 0)));
    assert_eq!(
        locate_record(&m, 5),
        None,
        "past the end is None, not a panic"
    );
}

/// `first_bad` is the FIRST bad record, not the last.
///
/// Changing `else if first_bad.is_none()` to a bare `else` survived, because every fixture
/// had exactly one bad record. An operator pointed at the LAST corruption instead of the
/// first would cut a partial recovery in the wrong place.
#[test]
fn first_bad_is_the_first_not_the_last() {
    let sk = sk();
    let mut m = crate::testkit::unsigned_chain_of(3);
    let _ = &sk;
    for i in [1usize, 2] {
        m.segments[i].records[0].signed_bytes[0] ^= 0xff;
    }
    let report = verify_records(&m);
    assert_eq!(report.total, 3);
    assert_eq!(report.intact, 1);
    assert_eq!(
        report.first_bad,
        Some(1),
        "with corruption at ordinals 1 AND 2, the report must name the FIRST"
    );
}

/// `signed_valid` counts attestations that VERIFIED, not attestation blobs that were present.
///
/// The old single `signed` tally incremented on `Some(att)` before verification and was never
/// decremented, so a medium whose every attestation had been tampered with reported "fully
/// signed" beside a list of faults. Counts are what an operator surface renders.
#[test]
fn signed_counts_distinguish_verified_from_merely_present() {
    let (mut m, _) = crate::testkit::verifiable_chain_of(2);
    // Tamper the first segment's attestation bytes: still PRESENT, no longer valid.
    let att = m.segments[0].attestation.as_mut().unwrap();
    let mid = att.len() / 2;
    att[mid] ^= 0xff;

    let r = chain_report(&m);
    assert_eq!(
        (r.signed_valid, r.signed_invalid, r.unsigned),
        (1, 1, 0),
        "a present-but-invalid attestation must not be counted as signed: {:?}",
        r.faults
    );
}

/// A segment carrying no records is a located fault.
///
/// `put_segment` refuses to write one, so finding one means the medium came from somewhere
/// else — and it is dangerous: `segment_commitment(&[])` is the multihash of the empty
/// string, the same constant on every medium, so anything chaining off it can be spliced in
/// from another medium with its predecessor matching.
#[test]
fn an_empty_segment_is_a_fault_and_does_not_anchor_a_chain() {
    let sk = sk();
    let empty = Segment {
        plane: Plane::Clinical,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: "abcd".into(),
        attestation: None,
        records: vec![],
    };
    let _ = &sk;
    // Built directly, not via `medium_v3`: `put_segment` REFUSES to write an empty segment,
    // so this shape can only arrive from a writer that is not us — which is precisely the
    // case the read-side fault exists to catch.
    let m = crate::container::MediumV3 {
        segments: vec![empty],
        truncated_tail: false,
        complete_bytes: 0,
    };
    let r = chain_report(&m);
    assert!(
        r.faults
            .iter()
            .any(|f| matches!(f, SegmentFault::EmptySegment { position: 0, .. })),
        "an empty segment must be named: {:?}",
        r.faults
    );
    assert_eq!(
        r.verified_through, None,
        "and must anchor nothing — its commitment is identical on every medium ever written"
    );
}
