//! Who may make a link/unlink judgement from this node, and which chart its event is FILED
//! under. **Pure** — no database — so every rule here is unit-tested on its own; `judge` (in
//! `judge.rs`) reads the facts (is each chart held? does a record contain the pair?) and asks.
//!
//! "Filed under" is the event ENVELOPE's `patient_id`: the chart whose `event_log` stream the
//! event sits in. It is not what the event is ABOUT — db/018 reads the pair from the payload's
//! `subject_a`/`subject_b`, never from the envelope (audit, R2b-2 plan). db/005 step 8b refuses a
//! local event filed under a chart with no history here; this module goes further and requires
//! the filed-under chart to be HELD (a `patient_chart` row, which always has history) — R2a's
//! rule, so a judgement is only ever filed in a stream this node itself carries.
use super::{canonical_pair, LinkVerb};
use crate::db_diagnosis::RefusalScope;
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// Which chart a judgement's event is filed under, and WHY that chart may carry it.
///
/// Typed rather than a bare `Uuid` so the one relaxation #699 (a) makes cannot leak: only an
/// UNLINK may be filed under a chart that is neither subject ([`filing_for`] refuses the rest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiledUnder {
    /// One of the two charts being judged (the C1 convention: `low`, or the held one).
    Subject(Uuid),
    /// The chart the clinician has OPEN, held here, whose record reads both subjects as part of
    /// it (#699 (a)): the far link B–C of an A–B–C record, when neither B nor C is held here.
    RecordOf(Uuid),
}

impl FiledUnder {
    /// The chart the envelope names, whichever reason admitted it.
    pub fn chart(self) -> Uuid {
        match self {
            FiledUnder::Subject(c) | FiledUnder::RecordOf(c) => c,
        }
    }
}

/// The chart the clinician has OPEN when they judge — the record the judgement is made from.
/// Consulted for an UNLINK whenever it is not one of the two subjects: it may carry the filing
/// when neither subject is held here (#699 (a): the far link B–C of an A–B–C record, judged
/// while reading A), and it is always checked (held? record holds both?) because its record is
/// what the caller reports back (Ruling R5). Its two facts are a snapshot read before the
/// judgement's transaction — for a legible refusal; the signing core re-reads the record under
/// the link lock (CARNLK) before a third-chart filing is signed ([`super::assert_link_in_tx`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenedChart {
    /// The open chart itself.
    pub chart: Uuid,
    /// A `patient_chart` row: db/005 step 8b will admit an event filed under it.
    pub held: bool,
    /// Its record (`person_charts`) reads BOTH subjects as part of it, here — see
    /// [`record_holds_both`].
    pub holds_both: bool,
}

impl OpenedChart {
    /// Why a judgement cannot be made FROM this chart, or `None` if it can: it must be held
    /// here, and its record must read both subjects.
    fn unusable(self) -> Option<Unusable> {
        match (self.held, self.holds_both) {
            (true, true) => None,
            (true, false) => Some(Unusable::LacksPair),
            (false, _) => Some(Unusable::NotHeld),
        }
    }
}

/// Why an open chart cannot be judged from — two facts of different KINDS, so two refusal
/// scopes (PR #711 review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unusable {
    /// No `patient_chart` row here. A fact about this NODE, as R2a's "not held here" is for a
    /// subject: sync delivering the chart makes the identical call succeed.
    NotHeld,
    /// Held, but its record does not read both subjects. A fact about the PICTURE judged from:
    /// the record changed since the clinician read it, or the wrong chart was named. Waiting
    /// changes nothing; re-reading does — the same class the signing core's re-check gives the
    /// same fact.
    LacksPair,
}

impl Unusable {
    /// Completes "the chart you have open (X) …".
    fn why(self) -> &'static str {
        match self {
            Unusable::NotHeld => "is not held here",
            Unusable::LacksPair => "does not read both as part of its record",
        }
    }

    /// What the refusal is a verdict about — see the variants.
    fn scope(self) -> RefusalScope {
        match self {
            Unusable::NotHeld => RefusalScope::NodeState,
            Unusable::LacksPair => RefusalScope::Input,
        }
    }

    /// The way forward, appended after the rule. A chart not held here has none a clinician
    /// can take (sync brings it); a record that no longer holds the pair is re-read.
    fn remedy(self) -> &'static str {
        match self {
            Unusable::NotHeld => "",
            Unusable::LacksPair => ". Nothing was done; reload the chart and judge again",
        }
    }
}

/// A judgement [`admit_judgement`] refused: the sentence, and what it is a verdict ABOUT — so
/// the caller marks it with the right [`RefusalScope`] and a surface offers the right way
/// forward (wait for this node, or re-read and judge again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmitRefusal {
    /// What the refusal is a verdict about.
    pub scope: RefusalScope,
    /// The refusal, naming the chart(s) at fault and the rule.
    pub text: String,
}

impl AdmitRefusal {
    /// A refusal about what this node holds — R2a's held-chart rules.
    fn node_state(text: String) -> Self {
        AdmitRefusal {
            scope: RefusalScope::NodeState,
            text,
        }
    }
}

/// Does `record` (a chart's `person_charts`) read BOTH subjects `a` and `b` as part of it?
/// **Pure**, and the one predicate behind [`OpenedChart::holds_both`]: `judge` asks it before
/// the transaction (for a legible refusal) and [`super::assert_link_in_tx`] asks it again under
/// the link lock (CARNLK) before a third-chart filing is signed, so the two cannot drift.
pub fn record_holds_both(record: &ChartSet, a: Uuid, b: Uuid) -> bool {
    record.contains(&a) && record.contains(&b)
}

/// Check a filing against the pair before anything is signed, and return the envelope chart.
/// `Subject` must name `low` or `high` (a wrong one would misfile the event in an unrelated
/// patient's stream, invisibly to the database floor); `RecordOf` is refused for a LINK and for
/// a chart that IS a subject (that is `Subject`, and saying otherwise hides which rule admitted
/// it). `Err(text)` names what is wrong.
///
/// Being pure, it CANNOT check that a `RecordOf` chart's record really contains both subjects,
/// nor that the chart is held: [`super::assert_link_in_tx`] re-reads the record under the
/// link lock (CARNLK) before signing, and db/005 step 8b refuses a chart with no history here.
pub fn filing_for(
    verb: LinkVerb,
    low: Uuid,
    high: Uuid,
    filed: FiledUnder,
) -> Result<Uuid, String> {
    match filed {
        FiledUnder::Subject(c) if c == low || c == high => Ok(c),
        FiledUnder::Subject(c) => Err(format!(
            "a judgement about ({low}, {high}) cannot be filed under chart {c} as one of its subjects"
        )),
        FiledUnder::RecordOf(_) if verb == LinkVerb::Link => Err(format!(
            "a link between {low} and {high} must be filed under one of them, never under a third chart"
        )),
        FiledUnder::RecordOf(c) if c == low || c == high => Err(format!(
            "chart {c} is a subject of the judgement, not a third chart"
        )),
        FiledUnder::RecordOf(c) => Ok(c),
    }
}

/// Whether a judgement may be made on this pair, given what this node holds — and if so,
/// which chart its event is filed under. **Pure**, so the rule is unit-testable apart from
/// the database.
///
/// - LINK needs BOTH charts held (a `patient_chart` row — see
///   `patient::person::ChartIdentity::held`). The floor admits a link naming a chart that
///   has not synced yet, correctly (offline-first); a human's deliberate act from this node
///   has no such excuse, and a typo would otherwise attach a stranger's future chart to
///   this person.
/// - UNLINK attaches nothing, so that risk does not apply. It needs one of the two charts
///   held and the other either held too or already part of that chart's record here
///   (`shared_record`) — the member line R1 displays for a chart whose registration has not
///   reached this node. A never-linked stranger is still refused.
/// - UNLINK with NEITHER chart held (#699 (a)): admitted only when judged from an `opened`
///   chart that is held here and whose record reads both as part of it — the far link B–C of
///   an A–B–C record, judged while reading A. It is filed under that open chart
///   ([`FiledUnder::RecordOf`]). A held subject is still preferred whenever there is one, and
///   `opened` never admits a link.
/// - An UNLINK's `opened` chart that is NOT one of the subjects is checked whichever chart
///   the event is filed under: a judgement "made from" a chart that is not held here, or
///   whose record does not hold both, is refused — otherwise a mistyped `--from` would be
///   reported back as a record ("chart X now reads as: X") that does not exist here.
///
/// `Ok(filed)`: the chart to file the event under, and why — `low` by the C1 convention when
/// both are held, else the one held chart (db/005 step 8b refuses a local event about a chart
/// with no history here), else the open chart. `Err`: the refusal, naming the chart(s) at
/// fault, and what it is a verdict about ([`AdmitRefusal`]).
pub fn admit_judgement(
    verb: LinkVerb,
    (a, a_held): (Uuid, bool),
    (b, b_held): (Uuid, bool),
    shared_record: bool,
    opened: Option<OpenedChart>,
) -> Result<FiledUnder, AdmitRefusal> {
    let (low, _) = canonical_pair(a, b);
    let rule = "a link needs both charts held on this node; an unlink needs one of the two \
                held here and the other held too or already read as part of its record, or — \
                when neither is held — the chart you have open held here with both in its \
                record";
    // A refusal about the open chart: its sentence, scope and way forward (see `Unusable`).
    let open_refusal = |situation: String, why: Unusable| AdmitRefusal {
        scope: why.scope(),
        text: format!("{situation} — {rule}{}", why.remedy()),
    };
    let filed = match (verb, a_held, b_held) {
        (_, true, true) => Ok(FiledUnder::Subject(low)),
        (LinkVerb::Unlink, false, false) => match opened {
            Some(o) => match o.unusable() {
                None => Ok(FiledUnder::RecordOf(o.chart)),
                Some(why) => Err(open_refusal(
                    format!(
                        "neither chart {a} nor chart {b} is held on this node, and the chart \
                         you have open ({}) {}",
                        o.chart,
                        why.why()
                    ),
                    why,
                )),
            },
            None => Err(AdmitRefusal::node_state(format!(
                "neither chart {a} nor chart {b} is held on this node — {rule}"
            ))),
        },
        (_, false, false) => Err(AdmitRefusal::node_state(format!(
            "neither chart {a} nor chart {b} is held on this node — {rule}"
        ))),
        (LinkVerb::Unlink, true, false) if shared_record => Ok(FiledUnder::Subject(a)),
        (LinkVerb::Unlink, false, true) if shared_record => Ok(FiledUnder::Subject(b)),
        // Exactly one chart unheld, and not admitted above. Say only what is true: for an
        // unlink that means it is also outside the other's record; for a link, whether it
        // is inside is beside the point.
        (_, a_held, _) => {
            let (unheld, other) = if a_held { (b, a) } else { (a, b) };
            let outside = match verb {
                LinkVerb::Unlink => format!(" and is not part of chart {other}'s record here"),
                LinkVerb::Link => String::new(),
            };
            Err(AdmitRefusal::node_state(format!(
                "chart {unheld} is not held on this node{outside} — {rule}"
            )))
        }
    }?;
    // Reached only with a HELD subject deciding the filing (a third chart that decided it was
    // checked above). An open chart that is not a subject must still be one this judgement can
    // honestly be made from — the caller reports its record back as the result.
    if let (LinkVerb::Unlink, FiledUnder::Subject(_), Some(o)) = (verb, filed, opened) {
        if o.chart != a && o.chart != b {
            if let Some(why) = o.unusable() {
                return Err(open_refusal(
                    format!(
                        "this judgement cannot be made from the chart you have open ({}): it {}",
                        o.chart,
                        why.why()
                    ),
                    why,
                ));
            }
        }
    }
    Ok(filed)
}

#[cfg(test)]
#[path = "admit_tests.rs"]
mod tests;
