//! A CAIRNB3 backup medium, answering as a sync peer (slice 2b, #500).
//!
//! This is the half of ADR-0026 decision 2 that makes "backup is a configuration of the sync
//! daemon" mechanical rather than aspirational: slice 2d's restore drives `cairn-sync`'s own
//! puller — same cursor, same quarantine pen, same custody handling — against a file.
//!
//! PURE. It reads a `MediumImage` someone else loaded; it opens no file and touches no
//! database. The caller owns the I/O.
//!
//! WHAT IT IS NOT. It serves; it does not capture. Nothing here writes a medium — that is
//! slice 2c — and `cairn-node`'s `backup.rs` still exports `node_event` and nothing else, so
//! a medium built today has an EMPTY clinical plane and this transport will truthfully say so.
//! #500 is not closed by this module existing.

use cairn_medium::{assess, chain_report, MediumHealth, MediumImage, MediumRecord, Plane};

use crate::transport::{Transport, TransportError};
use crate::wire::{EventsResponse, Request};

#[derive(Debug)]
pub struct MediumTransport {
    label: String,
    /// Clinical records within `verified_through`, ascending by `source_seq`. Materialised at
    /// construction because every request re-scans the same set, and because sorting once is
    /// what makes `request` a pure lookup.
    servable: Vec<MediumRecord>,
    health: MediumHealth,
}

impl MediumTransport {
    pub fn new(label: impl Into<String>, image: MediumImage) -> Result<Self, TransportError> {
        let label = label.into();
        let MediumImage::V3(m) = image else {
            return Err(TransportError::Unsupported {
                label,
                reason: "this medium predates the two-plane format (CAIRNB1/CAIRNB2). Those \
                         revisions carry the FEDERATION plane and no clinical event at all, so \
                         there is nothing here to restore a patient record from — see issue \
                         #500. Re-capture with a build that writes CAIRNB3."
                    .into(),
            });
        };
        let chain = chain_report(&m);
        // TRUST STOPS AT `verified_through` (2a invariant 5). Serving past it would hand a
        // puller records whose chain link never held — and the puller's cursor would then
        // advance over them. `None` (nothing verified) yields an empty set, never "all".
        let through = chain.verified_through;
        let mut servable: Vec<MediumRecord> = m
            .segments
            .iter()
            .take(through.map_or(0, |t| t + 1))
            .filter(|s| s.plane == Plane::Clinical)
            .flat_map(|s| s.records.iter().cloned())
            .collect();
        // Segments sit in CAPTURE order, which stops matching source_seq order after a
        // re-capture. The puller's contiguous-prefix cursor RELIES on strictly ascending
        // arrival, so a medium serving capture order would advance a cursor past events it
        // had not yet delivered. `partition_point` in `request` below depends on this sort —
        // the two must not drift apart.
        servable.sort_by_key(|r| r.source_seq);
        Ok(Self {
            label,
            servable,
            health: assess(&m),
        })
    }

    /// Everything this medium can say about its own soundness. Handed out so a restore can
    /// report SCOPE honestly — a torn tail, a missing plane, records this build cannot route —
    /// rather than inferring it from an event count.
    pub fn health(&self) -> &MediumHealth {
        &self.health
    }

    /// The highest clinical `source_seq` this medium can be trusted to serve.
    ///
    /// `None` — never `Some(0)` — when it carries no verified clinical record. Zero is a
    /// claim; absence is the honest answer (2a invariant 8), and the two lead an operator to
    /// opposite conclusions about whether a restore recovered anything.
    pub fn clinical_watermark(&self) -> Option<i64> {
        self.servable.last().map(|r| r.source_seq)
    }
}

impl Transport for MediumTransport {
    fn label(&self) -> &str {
        &self.label
    }

    fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError> {
        let (after_seq, limit, unwrap_cert) = match req {
            Request::EventsAfterSeq {
                after_seq,
                limit,
                unwrap_cert,
            } => (*after_seq, *limit, unwrap_cert.as_deref()),
            Request::EventsAfter { .. } => {
                return Err(TransportError::Unsupported {
                    label: self.label.clone(),
                    reason: "records on a medium are keyed by the capturing node's source_seq; \
                             there is no HLC index here. Use EventsAfterSeq."
                        .into(),
                })
            }
            Request::BlobSlice { .. } => {
                return Err(TransportError::Unsupported {
                    label: self.label.clone(),
                    reason: "a backup medium carries no byte tier — attachment bytes replicate \
                             by election, on their own resource-isolated path (ADR-0013). This \
                             is NOT 'blob absent': fetch it from a peer that holds it."
                        .into(),
                })
            }
        };

        if unwrap_cert.is_some() {
            // NEVER SILENTLY. The cert asks the server to re-wrap each DEK for the requester,
            // and a medium holds no secret, so it cannot. Saying so is the difference between
            // an operator who knows why a chart will not render and one who does not.
            eprintln!(
                "{}: the unwrap cert in this request is IGNORED. A medium holds no secret and \
                 cannot re-wrap; every DEK travels wrapped to the CAPTURING node's unwrap key, \
                 verbatim. That is correct only because ADR-0066 makes `restore` ADOPT the \
                 exported unwrap secret, so the restoring node's secret IS the capturing \
                 node's. If that adoption path ever changes, this stops working and a restored \
                 node will hold DEKs it cannot open.",
                self.label
            );
        }

        // `servable` is sorted ascending by `source_seq` (see `new`), which is what makes
        // `partition_point` a valid binary search for "strictly greater than `after_seq`".
        let start = self.servable.partition_point(|r| r.source_seq <= after_seq);
        let rest = &self.servable[start..];
        let (page, complete) = match limit {
            None => (rest, true),
            Some(n) => {
                let n = n as usize;
                (&rest[..n.min(rest.len())], rest.len() <= n)
            }
        };

        let resp = EventsResponse {
            events: page.iter().map(|r| hex::encode(&r.signed_bytes)).collect(),
            attestations: page
                .iter()
                .map(|r| r.attestation.as_ref().map(hex::encode))
                .collect(),
            attester_keys: page
                .iter()
                .map(|r| r.attester_key.as_ref().map(hex::encode))
                .collect(),
            seqs: page.iter().map(|r| r.source_seq).collect(),
            // A medium does not record the ADR-0040 signing context its records were minted
            // under, so it declares none rather than guessing. The puller falls back to its
            // all-unverifiable heuristic (#108) — a degraded diagnosis, not a wrong answer.
            // Slice 2c writes the segments and could carry it.
            signing_context: None,
            // Verbatim: wrapped to the CAPTURING node's key. See the unwrap_cert note above.
            wrapped_deks: page
                .iter()
                .map(|r| r.dek_wrapped.as_ref().map(hex::encode))
                .collect(),
            // Nothing was withheld. `None` here means "custody travelled, or there was none to
            // send" — which is exactly true of a pass-through.
            custody_withheld: None,
            complete,
        };
        serde_json::to_vec(&resp).map_err(|e| TransportError::Exchange {
            label: self.label.clone(),
            source: Box::new(e),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medium::{append_segment, parse_any, serialize_v3, MediumRecord, Plane, Segment};

    /// One record with deterministic, runtime-derived bytes.
    ///
    /// `lineage` distinguishes one record from another and is NOT cryptographic. It must not
    /// be called `salt`/`nonce`/`iv`: CodeQL picks its sink by the NAME of the binding a
    /// constant flows into, so those three words mint a critical alert PER CALL SITE no matter
    /// how the value is computed (house rule 6b, #527). Nothing here derives a key.
    fn record(lineage: u8, seq: i64) -> MediumRecord {
        MediumRecord {
            signed_bytes: std::array::from_fn::<u8, 32, _>(|i| lineage ^ (i as u8)).to_vec(),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: seq,
        }
    }

    /// An UNSIGNED segment per group. Unsigned is a declared limitation, never a fault (2a
    /// invariant 7), it chains on `prev_commitment` alone, and it keeps these fixtures free of
    /// a signing key none of these properties depend on.
    fn segments(groups: &[(Plane, Vec<MediumRecord>)]) -> Vec<Segment> {
        let mut out: Vec<Segment> = Vec::new();
        for (index, (plane, records)) in groups.iter().enumerate() {
            // `segment_commitment` takes the RECORDS, not the segment.
            let prev = out
                .last()
                .map(|s: &Segment| cairn_medium::segment_commitment(&s.records))
                .unwrap_or_default();
            out.push(Segment {
                plane: *plane,
                index: index as u32,
                prev_commitment: prev,
                self_node_id_hex: String::new(),
                attestation: None,
                records: records.clone(),
            });
        }
        out
    }

    fn transport_over(groups: &[(Plane, Vec<MediumRecord>)]) -> MediumTransport {
        let bytes = serialize_v3(&segments(groups)).expect("serialize");
        MediumTransport::new("medium /tmp/test.b3", parse_any(&bytes).expect("parse"))
            .expect("a CAIRNB3 image is servable")
    }

    /// The same, with the last segment's bytes cut short — an interrupted append.
    fn transport_over_torn(groups: &[(Plane, Vec<MediumRecord>)]) -> MediumTransport {
        let segs = segments(groups);
        let mut bytes = serialize_v3(&segs[..segs.len() - 1]).expect("serialize");
        let mut tail = Vec::new();
        append_segment(&mut tail, segs.last().expect("a last segment")).expect("append");
        // Keep a prefix of the final section: fewer bytes than its length prefix claims.
        bytes.extend_from_slice(&tail[..tail.len() / 2]);
        MediumTransport::new("medium /tmp/torn.b3", parse_any(&bytes).expect("parse"))
            .expect("a torn tail is a MILD fault — never a refusal")
    }

    /// A CAIRNB2 image: no marker, no events. The argument order is `(marker, events)`.
    fn legacy_image() -> cairn_medium::MediumImage {
        let bytes = cairn_medium::serialize_container(None, &[]).expect("serialize CAIRNB2");
        parse_any(&bytes).expect("parse")
    }

    /// Ask for a page and decode it. Every test below is about the response, not the framing.
    fn events(t: &MediumTransport, after_seq: i64, limit: Option<u32>) -> EventsResponse {
        let raw = t
            .request(&Request::EventsAfterSeq {
                after_seq,
                unwrap_cert: None,
                limit,
            })
            .expect("a clinical page");
        serde_json::from_slice(&raw).expect("decode")
    }

    #[test]
    fn a_legacy_medium_is_refused_by_name_not_answered_empty() {
        // CAIRNB1/B2 carry the federation plane and NO clinical event. Answering "0 events,
        // complete" would be #500's exact signature reproduced inside the machinery built to
        // close it: an operator would read a clean, complete restore of nothing.
        let err = MediumTransport::new("medium /tmp/x", legacy_image()).expect_err("must refuse");
        assert!(matches!(err, TransportError::Unsupported { .. }), "{err}");
        assert!(
            err.to_string().contains("#500"),
            "the refusal must name the issue: {err}"
        );
    }

    #[test]
    fn a_blob_slice_is_unsupported_not_not_found() {
        let t = transport_over(&[(Plane::Clinical, vec![record(1, 1)])]);
        let err = t
            .request(&Request::BlobSlice {
                addr_hex: "aa".into(),
                offset: 0,
                len: 1,
            })
            .expect_err("a medium has no byte tier");
        assert!(matches!(err, TransportError::Unsupported { .. }), "{err}");
    }

    #[test]
    fn the_legacy_hlc_cursor_is_unsupported() {
        let t = transport_over(&[(Plane::Clinical, vec![record(1, 1)])]);
        let err = t
            .request(&Request::EventsAfter {
                wall: 0,
                counter: 0,
            })
            .expect_err("records are keyed by source_seq, not HLC");
        assert!(matches!(err, TransportError::Unsupported { .. }), "{err}");
    }

    #[test]
    fn records_are_served_in_ascending_source_seq_whatever_order_the_segments_are_in() {
        // THE test for the sort. Segments sit in CAPTURE order, which is not source_seq order
        // after a re-capture. The puller's contiguous-prefix cursor RELIES on strictly
        // ascending arrival; a medium serving capture order would advance a cursor past
        // events it had not yet delivered. A fixture already in order would pass either way.
        let t = transport_over(&[
            (Plane::Clinical, vec![record(1, 5), record(2, 6)]),
            (Plane::Clinical, vec![record(3, 2), record(4, 3)]),
        ]);
        let resp = events(&t, 0, None);
        assert_eq!(resp.seqs, vec![2, 3, 5, 6]);
    }

    #[test]
    fn only_the_clinical_plane_is_served() {
        let t = transport_over(&[
            (Plane::Node, vec![record(9, 1)]),
            (Plane::Clinical, vec![record(1, 2)]),
        ]);
        assert_eq!(events(&t, 0, None).seqs, vec![2]);
    }

    #[test]
    fn after_seq_is_strict_and_the_limit_is_honoured_with_a_truthful_complete() {
        let t = transport_over(&[(
            Plane::Clinical,
            vec![record(1, 1), record(2, 2), record(3, 3)],
        )]);
        assert_eq!(events(&t, 1, None).seqs, vec![2, 3]); // STRICTLY greater

        let page = events(&t, 0, Some(2));
        assert_eq!(page.seqs, vec![1, 2]);
        assert!(!page.complete);

        let exact = events(&t, 0, Some(3)); // the boundary
        assert_eq!(exact.seqs, vec![1, 2, 3]);
        assert!(
            exact.complete,
            "the limit did not bite; there is nothing above"
        );

        assert!(events(&t, 3, Some(2)).complete); // drained
    }

    #[test]
    fn wrapped_deks_pass_through_byte_identical() {
        // The medium holds no secret and cannot re-wrap. This is correct ONLY because
        // ADR-0066 / DR slice 1 make `restore` ADOPT the exported unwrap secret, so the
        // restoring node's secret IS the capturing node's.
        let mut r = record(1, 1);
        r.dek_wrapped = Some(vec![7, 7, 7]);
        let t = transport_over(&[(Plane::Clinical, vec![r])]);
        let resp = events(&t, 0, None);
        assert_eq!(resp.wrapped_deks, vec![Some("070707".to_string())]);
        assert!(
            resp.custody_withheld.is_none(),
            "nothing was withheld — custody travelled"
        );
    }

    #[test]
    fn nothing_beyond_verified_through_is_served() {
        // A torn tail is a MILD fault with an intact prefix, so the medium is not refused —
        // refusing a recoverable medium mid-disaster is what BackupError's three-way split
        // exists to prevent. Trust simply stops at `verified_through` (2a invariant 5).
        let t = transport_over_torn(&[
            (Plane::Clinical, vec![record(1, 1)]),
            (Plane::Clinical, vec![record(2, 2)]), // in the torn tail
        ]);
        assert_eq!(events(&t, 0, None).seqs, vec![1]);
    }

    #[test]
    fn an_empty_clinical_plane_has_no_watermark_and_never_zero() {
        // 2a invariant 8: zero is a claim, absence is the honest answer.
        let t = transport_over(&[(Plane::Node, vec![record(9, 1)])]);
        assert_eq!(t.clinical_watermark(), None);
        let resp = events(&t, 0, None);
        assert!(resp.events.is_empty());
        assert!(resp.complete);
    }

    #[test]
    fn a_medium_never_declares_a_signing_context_it_does_not_record() {
        let t = transport_over(&[(Plane::Clinical, vec![record(1, 1)])]);
        assert!(events(&t, 0, None).signing_context.is_none());
    }
}
