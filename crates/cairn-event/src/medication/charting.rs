//! Charting fields on a medication assertion (charting slice 1a, design §3.4–§3.5).
//!
//! Charting a medication adds five OPTIONAL keys to `clinical.medication.asserted` —
//! top-level `charted_as`, `route`, `warning_override`, and `substance.strength`,
//! `substance.components`. The event type and schema version do not change: an absent key
//! is the old event, byte for byte (principle 11), so a `Charting` with nothing in it writes
//! nothing. Every key and every twin word here is wire, frozen once merged.
//!
//! `assert.rs` owns the payload and the twin; it calls [`insert_charting`] and
//! [`charting_twin_parts`] so the charting shape lives in one place. The floor that judges
//! these keys is `cairn_check_medication_charting` (db/077).
//!
//! Pure: no clock, no randomness, no I/O.
use serde_json::{json, Map, Value};

use super::assert::SubstanceCoding;

/// How the medication came to be on the chart. A closed set at the authoring door; a peer's
/// newer word is admitted at the remote door (ADR-0056), so a reader must not assume these
/// three are the only strings it will ever see on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartedAs {
    /// Started by this clinician, now ("Start medication").
    StartedHere,
    /// The patient was already taking it ("Already taking").
    PreExisting,
    /// Nobody said which — an honest unknown, never a guess (principle 4).
    Unknown,
}

impl ChartedAs {
    /// The wire word.
    pub fn as_str(self) -> &'static str {
        match self {
            ChartedAs::StartedHere => "started-here",
            ChartedAs::PreExisting => "pre-existing",
            ChartedAs::Unknown => "unknown",
        }
    }

    /// The wire word back into the enum; `None` for any word outside the closed set.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "started-here" => Some(ChartedAs::StartedHere),
            "pre-existing" => Some(ChartedAs::PreExisting),
            "unknown" => Some(ChartedAs::Unknown),
            _ => None,
        }
    }
}

/// One component of a combination ("amox + clav"): the term as typed and, when picked, its
/// drugref moiety. `coding: None` is written as an explicit `null` — inside the array every
/// element has the same two keys, so a reader never has to tell "absent" from "uncoded".
#[derive(Debug, Clone, Copy)]
pub struct Component<'a> {
    pub term: &'a str,
    pub coding: Option<SubstanceCoding<'a>>,
}

/// The `source` word of an override naming an allergy record. The vocabulary is open: later
/// sources (`interaction`, `drug-disease`, `renal-dose`, …) arrive as new words, never a new key.
pub const ALLERGY_SOURCE: &str = "allergy";

/// One warning a line was charted against. `source` says what kind of warning it was;
/// `reference` (wire key `ref` — a Rust keyword) points at it. For [`ALLERGY_SOURCE`] the
/// reference is the allergy record's event uuid; each future source defines its own.
#[derive(Debug, Clone, Copy)]
pub struct OverrideRef<'a> {
    pub source: &'a str,
    pub reference: &'a str,
}

/// Charting despite a warning: the clinician's reason, and which warning(s) the line was
/// charted against. General on purpose — warnings that need a reason will also come from
/// interactions, drug–disease and renal/hepatic dosing, and the wire is frozen once merged.
/// The window requires the reason; the floor checks the shape (design §3.5, db/077 §1).
#[derive(Debug, Clone, Copy)]
pub struct WarningOverride<'a> {
    pub reason: &'a str,
    pub against: &'a [OverrideRef<'a>],
}

/// The charting fields of one assertion. Every field is optional; `Default` is "none of
/// them", which writes nothing to the payload and nothing to the twin.
#[derive(Debug, Clone, Copy, Default)]
pub struct Charting<'a> {
    pub charted_as: Option<ChartedAs>,
    /// Open lower-case token (`po`, `iv`, `im`, …); `None` = unknown.
    pub route: Option<&'a str>,
    /// Free strength text (`500 mg`, `250 mg/5 mL`); `None` = unknown.
    pub strength: Option<&'a str>,
    /// A combination's components (≥ 2 when present). Empty = not a combination; the
    /// assertion's single `coding` is then the identity, as before.
    pub components: &'a [Component<'a>],
    pub warning_override: Option<WarningOverride<'a>>,
    /// The `version_id` of a set-aside version this line restores (slice 2b): a restored
    /// version becomes a charted line by NEW assertion; the original set-aside event is never
    /// edited. Top-level key `restores`; `None` writes nothing (principle 11).
    pub restores: Option<&'a str>,
}

/// Write the charting keys into an assertion's `payload` and its `substance` object.
/// Each key is inserted only when present, never as null (principle 11).
pub fn insert_charting(
    payload: &mut Map<String, Value>,
    substance: &mut Map<String, Value>,
    c: &Charting,
) {
    if let Some(k) = c.charted_as {
        payload.insert("charted_as".into(), json!(k.as_str()));
    }
    if let Some(r) = c.route {
        payload.insert("route".into(), json!(r));
    }
    if let Some(o) = c.warning_override {
        let against: Vec<Value> = o
            .against
            .iter()
            .map(|r| json!({ "source": r.source, "ref": r.reference }))
            .collect();
        payload.insert(
            "warning_override".into(),
            json!({ "reason": o.reason, "against": against }),
        );
    }
    if let Some(v) = c.restores {
        payload.insert("restores".into(), json!(v));
    }
    if let Some(s) = c.strength {
        substance.insert("strength".into(), json!(s));
    }
    if !c.components.is_empty() {
        let list: Vec<Value> = c
            .components
            .iter()
            .map(|p| {
                let coding = p.coding.map_or(
                    Value::Null,
                    |k| json!({ "system": k.system, "code": k.code, "display": k.display }),
                );
                json!({ "term": p.term, "coding": coding })
            })
            .collect();
        substance.insert("components".into(), Value::Array(list));
    }
}

/// The strings the assertion twin splices in for charting. Empty / `None` when absent, so an
/// assertion without charting renders exactly as before.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TwinParts {
    /// `{ strength}{ route}`, rendered right after the formulation.
    pub after_formulation: String,
    /// The components joined `" + "` (a coded one by its display, an uncoded one by its
    /// term). When `Some`, it replaces the single-coding `[display]` bracket and is always shown.
    pub components: Option<String>,
    /// `{ · started here | · already taking}{ · charted despite a warning ({sources}): {reason}}{ · restores a set-aside version}`,
    /// `{sources}` being the distinct non-blank sources in first-seen order, joined `", "`;
    /// when no source is named, the parenthetical is omitted. Rendered at the very end.
    pub tail: String,
}

/// Build the [`TwinParts`] for one `Charting`. The word order is frozen (twin = wire).
pub fn charting_twin_parts(c: &Charting) -> TwinParts {
    let mut after_formulation = String::new();
    for v in [c.strength, c.route].into_iter().flatten() {
        after_formulation.push(' ');
        after_formulation.push_str(v);
    }
    let components = (!c.components.is_empty()).then(|| {
        c.components
            .iter()
            .map(|p| p.coding.map_or(p.term, |k| k.display))
            .collect::<Vec<_>>()
            .join(" + ")
    });
    let mut tail = String::new();
    match c.charted_as {
        Some(ChartedAs::StartedHere) => tail.push_str(" · started here"),
        Some(ChartedAs::PreExisting) => tail.push_str(" · already taking"),
        Some(ChartedAs::Unknown) | None => {}
    }
    if let Some(o) = c.warning_override {
        // Each source once, in the order first met: the reader learns which KINDS of warning
        // were overridden; the refs themselves stay in the payload. A blank source names no
        // kind, so it is skipped, and with none left the parenthetical is omitted, never "()".
        let mut sources: Vec<&str> = Vec::new();
        for r in o.against {
            if !r.source.trim().is_empty() && !sources.contains(&r.source) {
                sources.push(r.source);
            }
        }
        tail.push_str(" · charted despite a warning");
        if !sources.is_empty() {
            tail.push_str(" (");
            tail.push_str(&sources.join(", "));
            tail.push(')');
        }
        tail.push_str(": ");
        tail.push_str(o.reason);
    }
    // After the override clause, so the tail still ends the drug line; a source note follows it.
    if c.restores.is_some() {
        tail.push_str(" · restores a set-aside version");
    }
    TwinParts {
        after_formulation,
        components,
        tail,
    }
}

#[cfg(test)]
#[path = "charting_tests.rs"]
mod tests;
