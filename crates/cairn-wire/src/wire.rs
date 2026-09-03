//! The clinical-plane sync protocol: one JSON request, one JSON response, per connection.
//!
//! WHY THIS IS A LIBRARY AND NOT `cairn-sync`'s `main.rs`: from slice 2b a backup medium is
//! addressed through this exact protocol (ADR-0026 decision 2 — "backup is a configuration of
//! the sync daemon"), so `cairn-node`'s backup and restore paths need these types. `cairn-sync`
//! is a binary-only crate that dev-depends on `cairn-node`, so it cannot grow a `lib.rs` without
//! a dependency cycle. Same reasoning as `cairn-keystore` (#503) and `cairn-medium` (slice 2a).
//!
//! THE EXTRACTION was verbatim: Task 1 moved these types out of `cairn-sync/src/main.rs` with
//! every field, doc comment and serde attribute byte-for-byte what it was, and its proof is
//! that every call site compiled untouched.
//!
//! THE FILE IS NOT VERBATIM ANY MORE, and saying so matters more than the tidy sentence it
//! replaces (final review). Later tasks in the same slice ADDED two fields here —
//! `EventsAfterSeq::limit` and `EventsResponse::complete`, the whole of #101 item 1's wire
//! change. A reader who took the old unqualified claim at face value would conclude those two
//! predate slice 2b and that an older peer already speaks them. It does not: both are additive
//! with serde defaults precisely because older peers do NOT.
//!
//! EVOLUTION IS ADDITIVE (principle 12, ADR-0021). A new field arrives with `#[serde(default)]`
//! and a default that is safe when it is ABSENT; an existing field never changes meaning.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op")]
pub enum Request {
    /// Clinical plane, HLC-cursored (legacy). KEPT so an older puller still works;
    /// a new puller uses EventsAfterSeq. Every event at or after this HLC watermark.
    EventsAfter { wall: i64, counter: i32 },
    /// Clinical plane, seq-cursored (issue #196): every event whose serving-node
    /// `seq` is strictly greater than `after_seq`, in `seq` order. `after_seq = 0`
    /// returns the full set (the full-sweep path). `seq` is the server's LOCAL
    /// insertion order — the only ordering where newly-learned events always sort
    /// above a puller's cursor, so incremental can never silently skip (#196).
    /// Additive (principle 12): the older EventsAfter variant stays served.
    ///
    /// `unwrap_cert` (ADR-0052 custody sidecar) is the puller's signed unwrap-key
    /// certificate (hex CBOR): it binds the puller's X25519 unwrap public key to its
    /// Ed25519 identity. When present, the server re-wraps each sealed event's DEK
    /// for that key so the puller gains crypto-shred custody of what it replicates
    /// (see rewrap_custody_for_peer). Additive (serde default): an old puller omits
    /// it and the server serves the events with no custody — sealed rows still admit
    /// structurally at the apply door, so nothing fails to sync.
    EventsAfterSeq {
        after_seq: i64,
        #[serde(default)]
        unwrap_cert: Option<String>,
        /// The maximum number of events to return in this response (slice 2b, #101 item 1).
        ///
        /// `None` means UNPAGINATED — the whole suffix in one frame, which is what this
        /// protocol did before paging existed and what a caller that has no reason to page
        /// still gets. A serving node applies it as a plain SQL `LIMIT`.
        ///
        /// Additive (serde default): the field's ABSENCE is the old behaviour, so a request
        /// that predates paging means exactly what it always meant.
        #[serde(default)]
        limit: Option<u32>,
    },
    /// Byte tier: a BLAKE3 verified-streaming slice of a blob.
    BlobSlice {
        addr_hex: String,
        offset: u64,
        len: u64,
    },
}

#[derive(Serialize, Deserialize)]
pub struct EventsResponse {
    /// Verbatim signed_bytes, hex-encoded (skeleton simplification; the real
    /// tier ships raw). The receiver reconstructs everything from these bytes.
    pub events: Vec<String>,
    /// Per-event attestation token (hex), PARALLEL to `events` (issue #91). A
    /// suppressing event (or asserted responsibility) is admitted at the in-DB
    /// apply door only against its human attestation token, so the token must
    /// travel with the event or a legitimately-attested suppress could never
    /// replicate. Additive field (serde default): an older peer's response
    /// decodes with empty arrays, which simply means "no attestation shipped" —
    /// its suppressing events are then refused fail-closed at the door.
    #[serde(default)]
    pub attestations: Vec<Option<String>>,
    /// Per-event attester public key (hex), parallel to `attestations`.
    #[serde(default)]
    pub attester_keys: Vec<Option<String>>,
    /// Per-event serving-node `seq` (issue #196), PARALLEL to `events`. The puller
    /// checkpoints its per-peer cursor on the max handled seq. Additive (serde
    /// default): an older peer's response decodes with an empty vec — a new puller
    /// that sent EventsAfterSeq treats an events-without-seqs response as a
    /// wire-format error rather than checkpointing blindly (see do_pull).
    #[serde(default)]
    pub seqs: Vec<i64>,
    /// The ADR-0040 signing context this server's events are minted under
    /// (issue #108). Lets the puller tell deterministic wire-format skew ("your
    /// events are signed for a context I don't speak") from tampering BEFORE
    /// burning a whole batch on per-event verify failures. Additive (serde
    /// default): a response from a peer predating this field decodes as None —
    /// "undeclared" — and the puller falls back to the all-unverifiable
    /// heuristic for the mixed-version diagnosis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_context: Option<String>,
    /// Per-event wrapped DEK (hex), PARALLEL to `events` (ADR-0052 custody sidecar).
    /// `wrapped_deks[i]` is the sealed event's data-encryption key RE-WRAPPED for the
    /// pulling peer's unwrap key (from the cert in its request) — the puller opens it
    /// with its own unwrap secret and hands it to the apply door as the 4th arg, so a
    /// replicated sealed event becomes crypto-shreddable on the puller too. A slot is
    /// None whenever no custody travels: the event is unsealed, this node holds no
    /// DEK for it, it has been SHREDDED here (the serve SQL nulls a shredded row's DEK
    /// — the wire-level half of the shred guarantee), or the peer sent no/invalid
    /// cert. Additive (serde default): an old peer omits the field entirely and it
    /// decodes to an empty vec — the puller then applies every event without custody
    /// (sealed rows still admit structurally at the door).
    #[serde(default)]
    pub wrapped_deks: Vec<Option<String>>,
    /// WHY no custody travelled, when the server deliberately withheld it (issue #231
    /// review). `None` means either "custody was granted" or "there was nothing to
    /// grant" — an empty `wrapped_deks` alone cannot tell those apart, which is exactly
    /// how the puller went blind.
    ///
    /// The serving node prints this on its own stderr, but the node that experiences
    /// the consequence is the PULLER: its sealed bodies will not render, and the
    /// remedy names steps its operator must run, at what is usually another site. So
    /// the reason travels with the refusal. It is operator prose, never a control
    /// signal: the puller prints it and counts it, and applies exactly the events it
    /// would have applied anyway (withhold the key, never the bytes).
    ///
    /// **It is sent to an UNADMITTED peer, deliberately.** The line does disclose a
    /// little about this node — whether it has peers, whether its node plane is
    /// provisioned — to a party the trust set just refused. Accepted, because that
    /// party has already been served the entire event log, including every UNSEALED
    /// event in plaintext (this pin protects sealed bodies; it is not an authorisation
    /// layer over replication). Against that, "this node has admitted no peers yet" is
    /// not the disclosure worth guarding, and an operator who cannot see why a chart is
    /// blank is a real safety cost. Revisit if replication itself ever becomes gated.
    ///
    /// Additive (serde default): an older peer omits it and it decodes as None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custody_withheld: Option<String>,
    /// Did this response drain the peer's log above `after_seq`? (slice 2b, #101 item 1.)
    ///
    /// A paged puller loops until this is `true`. The default is FALSE — "there may be more" —
    /// and the DIRECTION is the decision, not an accident. A server that fails to set it makes
    /// a puller ask once more: wasted work. A `true` default would make the same omission stop
    /// the puller early and SILENTLY LOSE EVENTS, with the cursor checkpointed as though the
    /// log had been drained. Principle 4 applied to a protocol field: an imprecise near-truth
    /// beats a precise untruth.
    ///
    /// An empty response that does not set this is neither an end nor a continuation, and a
    /// puller must REFUSE it rather than guess — see `cairn_wire::page_decision`.
    #[serde(default)]
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The op tag is a WIRE CONSTANT. A mirrored rename of the variant and its `#[serde(tag)]`
    /// would be invisible to a round-trip through this same enum, so the expectation is a
    /// literal JSON string, not a re-encode.
    #[test]
    fn events_after_seq_encodes_its_op_tag_literally() {
        let req = Request::EventsAfterSeq {
            after_seq: 7,
            unwrap_cert: None,
            limit: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(
            json.contains(r#""op":"EventsAfterSeq""#),
            "op tag must be the literal wire string, got {json}"
        );
        assert!(json.contains(r#""after_seq":7"#), "got {json}");
    }

    /// The two paging fields are WIRE CONSTANTS too, and nothing pinned them (final review).
    ///
    /// The sibling test above pins `op` and `after_seq` against a mirrored rename; `limit` and
    /// `complete` had only the absent-default test below, which decodes from a literal
    /// containing NEITHER name. A mirrored rename of `limit` (the Rust field and its serde
    /// attribute together) is invisible to any round trip through this same enum — both sides
    /// of a same-version pair agree — but a new puller's page request then decodes at an
    /// existing peer as `limit: None`, i.e. UNPAGINATED: the whole log suffix in one frame,
    /// refused at `write_frame`'s cap, and the link stops converging. That is #101 item 1
    /// un-fixed, silently, by a rename.
    #[test]
    fn the_paging_fields_encode_under_their_literal_wire_names() {
        let req = Request::EventsAfterSeq {
            after_seq: 0,
            unwrap_cert: None,
            limit: Some(500),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains(r#""limit":500"#), "got {json}");

        let resp = EventsResponse {
            events: vec![],
            attestations: vec![],
            attester_keys: vec![],
            seqs: vec![],
            signing_context: None,
            wrapped_deks: vec![],
            custody_withheld: None,
            complete: true,
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        assert!(json.contains(r#""complete":true"#), "got {json}");
    }

    /// A response from a peer that omits every additive field must still decode — that is
    /// what `#[serde(default)]` is FOR, and it is the property principle 12 rests on.
    #[test]
    fn a_minimal_response_decodes_through_the_serde_defaults() {
        let minimal = r#"{"events":["aa"]}"#;
        let resp: EventsResponse = serde_json::from_str(minimal).expect("decode");
        assert_eq!(resp.events, vec!["aa".to_string()]);
        assert!(resp.attestations.is_empty());
        assert!(resp.attester_keys.is_empty());
        assert!(resp.seqs.is_empty());
        assert!(resp.signing_context.is_none());
        assert!(resp.wrapped_deks.is_empty());
        assert!(resp.custody_withheld.is_none());
    }

    /// The additive fields must decode ABSENT, and to the values §3 of the design specifies.
    /// This is principle 12's whole guarantee, and a default that drifted would be silent.
    #[test]
    fn the_paging_fields_decode_absent_to_their_documented_defaults() {
        let old_req = r#"{"op":"EventsAfterSeq","after_seq":0}"#;
        match serde_json::from_str::<Request>(old_req).expect("decode") {
            Request::EventsAfterSeq {
                limit,
                unwrap_cert,
                after_seq,
            } => {
                assert_eq!(after_seq, 0);
                assert!(unwrap_cert.is_none());
                assert!(
                    limit.is_none(),
                    "an absent limit means UNPAGINATED, not zero"
                );
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let old_resp = r#"{"events":[]}"#;
        let resp: EventsResponse = serde_json::from_str(old_resp).expect("decode");
        assert!(
            !resp.complete,
            "an absent `complete` must mean THERE MAY BE MORE. The opposite default would let a \
             server that omits the field stop a puller early and silently lose events, with the \
             cursor checkpointed as if the log had been drained."
        );
    }
}
