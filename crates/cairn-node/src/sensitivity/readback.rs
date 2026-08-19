//! §5.9 — what an authoring verb reports back after it has written (#388, #435).
//!
//! # Why a read-back exists at all
//!
//! Both §5.9 orchestrators mint a local `Uuid` and return it without reading anything back,
//! so an act that took full effect and an act that changed nothing produced byte-identical
//! output. That is the defect ADR-0064's §1.2 budget is about: the floor deliberately
//! ADMITS a claim it will not honour — a withdrawal below the authority bar lands,
//! converges and stays re-assertable, it simply removes no protection — and a surface that
//! prints "withdrew" over that state tells the operator the opposite of what happened.
//!
//! # The one rule this module exists to hold
//!
//! NEVER COLLAPSE TWO OUTCOMES INTO ONE SENTENCE. `sensitivity_withdrawal_worklist` is a
//! union of two arms with OPPOSITE meanings (db/048 section 11), and its `inert` arm
//! further merges two states its own comment separates. The read-back therefore observes
//! the accountability fact and the effect fact SEPARATELY and reports both — see
//! [`WithdrawOutcome`]. The Slice 69 review found the collapsed version of exactly this on
//! the chart report, where a completed, unaccountable removal of protection was counted
//! under "did NOT take effect".
//!
//! Reads only — nothing here authors, and nothing here withholds. The wording lives in
//! `super::render`, which is pure; this module supplies it with facts.
use super::subject_kind_phrase;
use cairn_event::sensitivity::SubjectKind;
use uuid::Uuid;

/// The effective grade standing over ONE subject, as a read-back reports it.
///
/// Replaces the bare `(String, String, &'static str)` the assert read-back used to return.
/// Two of those three fields are interchangeable by type, and a call site that prints all
/// three in one sentence is exactly where a transposition survives review unnoticed.
pub struct SubjectReading {
    /// The effective grade now standing over the subject. PEER TEXT — unconstrained `TEXT`
    /// copied from a body, so the renderer escapes it (`render::peer`).
    pub grade: String,
    /// Which subject produced that grade: [`subject_kind_phrase`]'s output.
    pub winning_subject: String,
    /// What the grade was read OVER — "this chart" | "that event" | "that thread". The
    /// three subject kinds resolve against three DIFFERENT things, and saying "on this
    /// chart" after a thread-scoped act is a precise untruth about an act that did exactly
    /// what was asked of it (the bug the #388 review found in the assert read-back).
    pub scope: &'static str,
}

/// Whether a withdrawal's target can be READ on this node, once it is known to be held.
///
/// A sum type rather than an `Option<SubjectReading>` beside a loose `subject_kind: String`:
/// these are two different sentences an operator reads, and an unrecognised kind must NAME
/// itself rather than degrade into an absent value that reads like "nothing applies".
pub enum SubjectResolution {
    /// The subject kind is one this build understands, and this is what now stands over it.
    Resolved(SubjectReading),
    /// A subject kind from a newer peer — ADR-0056 admits it without understanding it.
    /// Carried BY NAME so the read-back can say which one it could not resolve, instead of
    /// guessing at a grade (principle 4: an imprecise near-truth beats a precise untruth).
    Unrecognised(String),
}

/// What this node can say about the assertion a withdrawal targeted.
pub enum TargetState {
    /// Not in `sensitivity_assertion` here at all. Set-union sync has no ordering, so a
    /// withdrawal legitimately precedes its target — db/048 keeps NO foreign key for
    /// exactly this reason ("standing" is a set difference evaluated at read). Nothing can
    /// be said about what it graded, in either direction.
    NotHeldHere,
    /// Held on this node, but recorded on a DIFFERENT chart than the withdrawal was
    /// stamped with — ADR-0064's KNOWN GAP, which its own text records as "not fixed, and
    /// not exercised by any test".
    ///
    /// `cairn_sensitivity_standing` is patient-scoped on BOTH sides, and that is
    /// load-bearing: without it a withdrawal authored on chart B could strip chart A's
    /// protection. The cost is that a mis-stamped withdrawal is permanently inert AND
    /// falls out of the worklist's `inert` arm too, which asks whether the target still
    /// stands on the WITHDRAWAL's own chart, where it never did.
    ///
    /// It MUST NOT collapse into `Held { still_standing: false }`. The target genuinely is
    /// absent from this chart's standing set, so a naive membership test reports the
    /// withdrawal as effective — a precise untruth in the reassuring direction, on a
    /// confidentiality surface.
    ///
    /// Deliberately carries NO patient id. The operator needs to know their act is
    /// mis-targeted, not who the other chart belongs to; naming it would answer a question
    /// nobody asked with another patient's identifier, on the one surface whose entire
    /// purpose is minimising disclosure.
    ///
    /// SCOPE: this catches the shape at the AUTHORING node, at the moment of authoring. A
    /// mis-chart withdrawal that arrives by replication is still invisible to every §5.9
    /// read surface — **#436**, which also explains why the honest fix is visibility rather
    /// than a door refusal (refusing at apply would fork the event set, and "on another
    /// chart" is indistinguishable from "not arrived yet" on a node holding neither).
    OnAnotherChart,
    /// Held here, on this chart. `still_standing` is a set-membership test over
    /// `cairn_sensitivity_standing`: the DIRECT effect of the withdrawal, exactly
    /// determinable and — note — independent of whether the subject kind is understood.
    Held {
        still_standing: bool,
        subject: SubjectResolution,
    },
}

/// What a `sensitivity-withdraw` actually achieved, read back from the database (#435).
///
/// TWO INDEPENDENTLY OBSERVED FACTS, deliberately not merged. They are correlated but not
/// redundant, and each answers a question the other cannot:
///
/// * `worklist_reason` is the ACCOUNTABILITY fact — which arm of db/048 section 11 this
///   withdrawal landed on, if any. `None` covers two benign cases: it cleared the bar and
///   the author has prior presence here, or it was unaccountable but the target has since
///   been stripped by an accountable route, so the row self-cleared.
/// * `target` is the EFFECT fact. The worklist's `inert` arm merges "nobody accountable"
///   with "the target has not replicated here yet" — its own comment says so — and only a
///   direct look at `sensitivity_assertion` separates them.
///
/// The two can honestly disagree in direction: `stranger-attested` means the withdrawal
/// TOOK EFFECT and is still worth a look. Collapsing them into one verdict would reproduce,
/// one verb over, the union-view defect the Slice 69 review found on the chart report.
pub struct WithdrawOutcome {
    /// The worklist arm, verbatim. OPEN VOCABULARY — a build that has never seen a value
    /// must still surface it rather than drop the row (see
    /// `render::withdrawal_reason_explanation`).
    pub worklist_reason: Option<String>,
    pub target: TargetState,
}

/// Parse a stored `subject_kind` into the enum a reading resolves against.
///
/// Pure and total. `None` is NOT an error: db/048 admits an unrecognised subject kind from
/// a future peer and interprets it conservatively (ADR-0062/ADR-0056), so this build must
/// be able to hold "a kind I do not know" as an ordinary value.
///
/// TODO(#387) — collapses into `SubjectKind: TryFrom<&str>` along with the clap value
/// parser and the CLI's hand-rolled match, which are the same closed set written three
/// times.
fn parse_subject_kind(kind: &str) -> Option<SubjectKind> {
    match kind {
        "event" => Some(SubjectKind::Event),
        "thread" => Some(SubjectKind::Thread),
        "patient" => Some(SubjectKind::Patient),
        _ => None,
    }
}

/// Read what now stands over ONE subject.
///
/// The scope label is what makes a read-back honest: the three subject kinds resolve
/// against three DIFFERENT things, so this must never be replaced by a chart-wide read.
///
/// * `Patient` — the chart-wide reading, off the chart's registration event.
/// * `Event` — `cairn_effective_sensitivity` over that event itself.
/// * `Thread` — over the thread's currently projected head. A node holding no DEK custody
///   has no such row, and this says so rather than reporting a grade for something it
///   cannot see (principle 4).
pub async fn subject_reading(
    client: &mut tokio_postgres::Client,
    patient: Uuid,
    kind: SubjectKind,
    subject_id: Uuid,
) -> anyhow::Result<SubjectReading> {
    match kind {
        SubjectKind::Patient => {
            let after = super::chart_sensitivity(client, patient).await?;
            Ok(SubjectReading {
                grade: after.chart_grade,
                winning_subject: after.chart_source,
                scope: "this chart",
            })
        }
        SubjectKind::Event => {
            let row = client
                .query_one(
                    "SELECT grade, subject_kind FROM cairn_effective_sensitivity($1::text::uuid)",
                    &[&subject_id.to_string()],
                )
                .await?;
            let kind_s: String = row.get(1);
            Ok(SubjectReading {
                grade: row.get(0),
                winning_subject: subject_kind_phrase(&kind_s).to_string(),
                scope: "that event",
            })
        }
        SubjectKind::Thread => {
            // The thread's current head is the row `medication_statement` holds for it — an
            // `ON CONFLICT (medication_id) DO UPDATE` table (db/031), so this names a real,
            // locally-resolvable event whenever this node can open the thread at all.
            let head = client
                .query_opt(
                    "SELECT ces.grade, ces.subject_kind
                       FROM medication_statement ms
                       JOIN event_log e ON e.content_address = ms.content_address,
                            LATERAL cairn_effective_sensitivity(e.event_id) ces
                      WHERE ms.medication_id = $1::text::uuid",
                    &[&subject_id.to_string()],
                )
                .await?;
            Ok(match head {
                Some(row) => {
                    let kind_s: String = row.get(1);
                    SubjectReading {
                        grade: row.get(0),
                        winning_subject: subject_kind_phrase(&kind_s).to_string(),
                        scope: "that thread",
                    }
                }
                // NOT an error: the act landed and is perfectly valid. This node simply
                // cannot project the thread it grades, which is ordinary on a node without
                // DEK custody. Say that, rather than reporting a grade for a thread this
                // node cannot see or implying the act did nothing.
                None => SubjectReading {
                    grade: "(not readable here)".to_string(),
                    winning_subject: "no locally projected row for that thread — this node \
                                      may hold no DEK custody (#383)"
                        .to_string(),
                    scope: "that thread",
                },
            })
        }
    }
}

/// Read back what a withdrawal achieved (#435).
///
/// TWO READS HERE, plus whatever the subject reading costs (a chart-wide target goes
/// through `chart_sensitivity`, which is six more). Each is load-bearing:
///
/// 1. The worklist arm, keyed on the WITHDRAWAL's own `event_id` rather than on its target
///    — two withdrawals of the same assertion are legal and would otherwise collide, and
///    reporting another act's verdict for this one is the worst available answer.
/// 2. The target: whether this node holds it, whether it is on THIS chart, whether it
///    still stands, and with what subject. Those four facts split db/048's single `inert`
///    arm into the genuinely different operator stories it merges.
///
/// Then, when the target is resolvable, what now stands over its OWN subject — never the
/// chart-wide grade. That is what stops "no longer stands" from reading as "this subject is
/// now open" when a SECOND assertion still grades it.
pub async fn withdraw_readback(
    client: &mut tokio_postgres::Client,
    patient: Uuid,
    withdrawal_event_id: Uuid,
    withdraws_hex: &str,
) -> anyhow::Result<WithdrawOutcome> {
    // `query_opt`, not `query`: at most one row can match. `event_log.event_id` is the
    // PRIMARY KEY (db/001), so a second signed body reusing an event_id is refused at the
    // door and can never project a second `sensitivity_withdrawal` row for it — the
    // one-row assumption is enforced upstream, not merely expected here.
    let worklist_reason: Option<String> = client
        .query_opt(
            "SELECT reason FROM sensitivity_withdrawal_worklist WHERE event_id = $1::text::uuid",
            &[&withdrawal_event_id.to_string()],
        )
        .await?
        .map(|row| row.get(0));

    // The target as this node holds it. `sensitivity_assertion` keeps every assertion ever
    // applied here; `cairn_sensitivity_standing` is the set difference that survives
    // withdrawal. Reading BOTH is what separates "gone because withdrawn" from "never here".
    // `withdraws_hex` reached this point through `withdraw_sensitivity`, which submitted it
    // past db/048's own `cairn_decode_hex_or_raise` — so `decode` here cannot be reached
    // with malformed hex by the CLI path. A caller that fed it something else gets a
    // database error, which the CLI surfaces as a WARNING beside the landed write rather
    // than as a failed command (see `main.rs`).
    let held = client
        .query_opt(
            "SELECT a.subject_kind, a.subject_id::text,
                    EXISTS (SELECT 1 FROM cairn_sensitivity_standing($2::text::uuid) s
                             WHERE s.content_address = a.content_address),
                    a.patient_id = $2::text::uuid AS on_this_chart
               FROM sensitivity_assertion a
              WHERE a.content_address = decode($1, 'hex')",
            &[&withdraws_hex, &patient.to_string()],
        )
        .await?;

    let target = match held {
        None => TargetState::NotHeldHere,
        // The chart check comes FIRST and short-circuits: on a mis-stamped withdrawal the
        // `still_standing` column below is a perfectly well-formed `false` that means
        // something else entirely, and reading it as the effect is the whole defect.
        Some(row) if !row.get::<_, bool>(3) => TargetState::OnAnotherChart,
        Some(row) => {
            let kind_s: String = row.get(0);
            let subject_id: String = row.get(1);
            let still_standing: bool = row.get(2);
            let subject = match parse_subject_kind(&kind_s) {
                Some(kind) => {
                    let subject_id =
                        Uuid::parse_str(&subject_id).expect("subject_id column is a valid UUID");
                    SubjectResolution::Resolved(
                        subject_reading(client, patient, kind, subject_id).await?,
                    )
                }
                None => SubjectResolution::Unrecognised(kind_s),
            };
            TargetState::Held {
                still_standing,
                subject,
            }
        }
    };

    Ok(WithdrawOutcome {
        worklist_reason,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_known_subject_kinds_parse() {
        assert_eq!(parse_subject_kind("event"), Some(SubjectKind::Event));
        assert_eq!(parse_subject_kind("thread"), Some(SubjectKind::Thread));
        assert_eq!(parse_subject_kind("patient"), Some(SubjectKind::Patient));
    }

    #[test]
    fn a_future_peers_subject_kind_parses_to_none_rather_than_erroring() {
        // ADR-0056: an unrecognised kind is ADMITTED, so this build must hold it as an
        // ordinary value. Erroring here would turn a read-back on a perfectly legal
        // federated state into a failure, and the caller would report the withdrawal
        // itself as broken.
        assert_eq!(parse_subject_kind("episode"), None);
        assert_eq!(parse_subject_kind(""), None);
    }
}
