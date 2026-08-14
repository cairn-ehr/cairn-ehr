//! §5.9 safety projection — the wire shape of a de-identified safety signal (ADR-0006,
//! ADR-0059 decision 4, ADR-0063).
//!
//! # The two tiers, and why the seal boundary separates them
//!
//! A coded drug's interaction class is a property of the CODE — a drug-knowledge lookup.
//! So a reader cannot re-derive it without a drug database, and making the §5.9 safety
//! floor depend on one would defeat the floor (ADR-0059 decision 4 / #294). The class is
//! therefore computed PRE-SEAL on the coding node, which by construction had a coding
//! authority in hand, and it travels.
//!
//! But the precise class IS the disclosure for exactly the cases §5.9 exists for:
//! "Rh-sensitizing event" in the clear reads as "this patient had a termination".
//! So it travels in TWO tiers:
//!
//!   * `payload.safety` — the precise `{class, severity}`, sealed under the body's own DEK.
//!     A custody-holding node reads it without any drug database.
//!   * `EventBody.safety` — a RUNG, **plus whatever that rung licenses**, in the clear.
//!     The rung is chosen from the sensitivity grade standing at authoring time
//!     (`cairn_prospective_sensitivity`; NOT `cairn_effective_sensitivity`, which needs an
//!     event id the event does not have yet — db/049 section 6). This is what a node
//!     without custody (sequestered, part C) or after a crypto-shred still sees, and it is
//!     the only coarsening that binds a peer's raw-SQL client.
//!
//! READ THAT SECOND BULLET LITERALLY: at rung `precise` — which is what an UNGRADED chart
//! gets, i.e. the default state of every chart — the class and severity travel in the CLEAR
//! on the signed envelope and replicate unconditionally. The seal is not what protects the
//! class in the common case; the GRADE is. That is the whole reason the rung is decided at
//! authoring time and frozen into the signed bytes.
use serde_json::{json, Value};

/// How much of a safety signal is published in the clear. Ordered coarsest-last, mirroring
/// §5.9's ladder: *precise class → "confidential medication, severity X" → "confidential
/// content, break glass"*.
///
/// The rung is chosen at AUTHORING time from the sensitivity grade STANDING on the chart at
/// that moment — db/049's `cairn_prospective_sensitivity` fed through
/// `cairn_safety_rung_for_rank`, never `cairn_effective_sensitivity`, which takes an
/// `event_id` the event about to be authored does not have (db/049 section 6 explains at
/// length why the two must stay separate). It is then frozen into the signed bytes and
/// cannot be revised: bytes on the wire cannot be un-published, which is exactly why the
/// choice has to bind here rather than at read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyRung {
    /// The class itself. Reached when the chart's standing grade ranks <= 0 — i.e. an
    /// explicit `routine` assertion as well as no standing assertion at all (db/049
    /// section 3's `p_rank <= 0` arm).
    Precise,
    /// Severity only. The event TYPE already says "medication" in the clear, so this rung
    /// deliberately adds no `kind` field — it would restate what the row already publishes.
    Kind,
    /// The signal exists; nothing about it is disclosed. Break glass to learn more.
    Existence,
}

impl SafetyRung {
    pub fn as_str(self) -> &'static str {
        match self {
            SafetyRung::Precise => "precise",
            SafetyRung::Kind => "kind",
            SafetyRung::Existence => "existence",
        }
    }

    /// Coarseness rank — higher is coarser. Gaps of 10 leave room to interpose a rung later
    /// without renumbering, the same discipline `cairn_sensitivity_rank` uses.
    pub fn rank(self) -> i32 {
        match self {
            SafetyRung::Precise => 0,
            SafetyRung::Kind => 10,
            SafetyRung::Existence => 20,
        }
    }
}

/// The full safety claim as the coding node established it. Borrowed `&str`s only, so it is
/// `Copy` and costs nothing to pass around.
#[derive(Debug, Clone, Copy)]
pub struct PreciseSafety<'a> {
    /// The coarse safety class — an interaction/allergy class, "rh-sensitizing", a
    /// contraindication flag. Open vocabulary: this crate never enumerates drug knowledge.
    pub class: &'a str,
    /// Open vocabulary; db/049 ranks the named ladder and treats anything else as MAX,
    /// because for a SAFETY signal "unknown" must mean "assume the worst".
    pub severity: &'a str,
}

/// The object that goes INSIDE the sealed payload. Never coarsened — the seal is what
/// protects it, and a custody-holder is entitled to the whole claim (#294: this is the
/// carried class a drugref-less reader depends on).
pub fn precise_safety_body(p: &PreciseSafety) -> Value {
    json!({ "class": p.class, "severity": p.severity })
}

/// The object that goes in the CLEAR on the signed envelope, cut down to `rung`.
///
/// Total and exhaustive over the ladder: adding a rung to `SafetyRung` forces a decision
/// here, which is the point — a rung with no explicit field policy would default to
/// disclosing whatever the previous arm disclosed.
pub fn coarsen(p: &PreciseSafety, rung: SafetyRung) -> Value {
    match rung {
        SafetyRung::Precise => {
            json!({ "rung": rung.as_str(), "class": p.class, "severity": p.severity })
        }
        // Fields are OMITTED, never written as null: an explicit null is an author
        // asserting something about the class, and absence is the honest "withheld".
        SafetyRung::Kind => json!({ "rung": rung.as_str(), "severity": p.severity }),
        SafetyRung::Existence => json!({ "rung": rung.as_str() }),
    }
}

// NO TWIN RENDERER LIVES HERE, AND ITS ABSENCE IS HONEST (2026-08-14 review finding M1).
//
// A `render_safety_twin` used to sit at this spot, rendering "safety: <class> (severity
// <severity>)" for the §3.13 legibility twin. NOTHING CALLED IT: no medication twin renders
// the safety claim, so the legibility obligation it appeared to discharge was not in fact
// discharged. A `pub` function in the wire crate that only its own test calls is worse than
// its absence, because a reviewer reads it as evidence the obligation is met.
//
// Wiring it in would change the rendered twin of every coded medication — a real behaviour
// change, and one that belongs with #379 (rendering the sensitivity grade in the twin),
// not slipped into a fix wave. The gap is recorded in ADR-0063's Known limitations.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rung_ladder_is_ordered_coarsest_last() {
        assert!(SafetyRung::Precise.rank() < SafetyRung::Kind.rank());
        assert!(SafetyRung::Kind.rank() < SafetyRung::Existence.rank());
        assert_eq!(SafetyRung::Precise.as_str(), "precise");
        assert_eq!(SafetyRung::Kind.as_str(), "kind");
        assert_eq!(SafetyRung::Existence.as_str(), "existence");
    }

    #[test]
    fn precise_carries_class_and_severity() {
        let p = PreciseSafety {
            class: "rh-sensitizing",
            severity: "high",
        };
        let v = coarsen(&p, SafetyRung::Precise);
        assert_eq!(v["rung"], "precise");
        assert_eq!(v["class"], "rh-sensitizing");
        assert_eq!(v["severity"], "high");
    }

    #[test]
    fn kind_drops_the_class_and_keeps_the_severity() {
        // "confidential medication, severity X" — the middle rung of §5.9's ladder. The
        // word "medication" is already in the clear on event_log.event_type, so the rung
        // carries only what is genuinely additional.
        let p = PreciseSafety {
            class: "rh-sensitizing",
            severity: "high",
        };
        let v = coarsen(&p, SafetyRung::Kind);
        assert_eq!(v["rung"], "kind");
        assert!(
            v.get("class").is_none(),
            "the class must not survive coarsening"
        );
        assert_eq!(v["severity"], "high");
    }

    #[test]
    fn existence_carries_neither_but_still_exists() {
        // Coarseness varies; EXISTENCE never disappears (§5.9's safety-floor invariant).
        // This rung is the claim "there is a safety-relevant signal here and you are not
        // cleared to see what" — which is what makes break-glass a rational act.
        let p = PreciseSafety {
            class: "rh-sensitizing",
            severity: "high",
        };
        let v = coarsen(&p, SafetyRung::Existence);
        assert_eq!(v["rung"], "existence");
        assert!(v.get("class").is_none());
        assert!(v.get("severity").is_none());
        assert!(v.is_object(), "the signal still exists as an object");
    }

    #[test]
    fn the_sealed_body_always_carries_the_full_precision() {
        // payload.safety is under the DEK, so it is never coarsened: coarsening is what
        // the CLEAR field is for.
        let p = PreciseSafety {
            class: "antiretroviral-interaction",
            severity: "critical",
        };
        let v = precise_safety_body(&p);
        assert_eq!(v["class"], "antiretroviral-interaction");
        assert_eq!(v["severity"], "critical");
        assert!(v.get("rung").is_none(), "the sealed side carries no rung");
    }
}
