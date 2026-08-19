//! §5.9 — the chart sensitivity READ model.
//!
//! Split out of `sensitivity/mod.rs` (which keeps the authoring verbs) when the operator
//! surface grew more reads. This is the ONLY file in the module that READS from a database
//! (`mod.rs` keeps the authoring verbs and writes through `submit_event`): the wording an
//! operator actually reads lives in `render.rs` and is pure, so
//! the honesty claims this surface makes are unit-testable without a cluster. That split
//! is the point of the file boundary, not merely a line-count fix.
//!
//! This module REPORTS; it does not enforce. Nothing here may start withholding content on
//! the strength of a grade — a projection-layer filter with no floor beneath it is security
//! theatre, since a client talking raw SQL walks straight past it. Real enforcement is
//! custody narrowing (#232 part C / #376).
use super::winner::WinningSubject;
use uuid::Uuid;

// NO `#[derive(Debug)]` ON THE ROW TYPES BELOW, DELIBERATELY. An earlier draft of this
// slice derived it on all six; nothing consumed it, and `WithdrawalWorklistRow.rationale`
// is peer-authored clear text that replicates and can never be unsaid (#378). In the one
// module whose whole subject is confidentiality, a derive nothing needs turns
// `tracing::debug!("{report:?}")` into a one-keystroke disclosure. Add it back only with a
// caller that needs it, and only on a type that carries no clinical text.

/// One row of `sensitivity_withdrawal_worklist` (db/048 section 11) — a withdrawal this
/// node admitted that a human should look at.
///
/// NOT "a withdrawal that failed". The view has TWO DISJOINT ARMS and only one of them is
/// ineffective, which an earlier name (`IneffectiveWithdrawal`) and header got wrong:
///   * `inert` — `verdict = 'unverified'`. Nobody this node can hold responsible stands
///     behind it, so the grade did NOT move.
///   * `stranger-attested` — `verdict <> 'unverified'`. It cleared the bar and the grade
///     DID move; db/048 says so in terms ("as SALIENCE it blocks nothing and delays
///     nothing — the withdrawal has already taken effect"). It is on the worklist because
///     protection was removed by someone with no prior presence on this chart, which is a
///     thing to see, not a failure to fix.
///
/// A withdrawal lands, converges and stays re-assertable either way — ADR-0064 gates
/// EFFECT, never admission, so nothing here is a refusal.
pub struct WithdrawalWorklistRow {
    /// Hex `content_address` of the assertion this withdrawal targeted.
    pub withdraws: String,
    /// `inert` (no accountable human stands behind the claim) or `stranger-attested`
    /// (attested, but by an actor with no prior presence on this chart). Open vocabulary —
    /// see `render::withdrawal_reason_explanation`.
    pub reason: String,
    pub node_origin: String,
    /// NOT `Option`: `sensitivity_withdrawal.rationale` is `TEXT NOT NULL` (db/048),
    /// because db/048's ceremony refuses a withdrawal that does not state why.
    pub rationale: String,
    /// Hex `actor_id` of whoever is accountable (#421). `None` when the attester key maps
    /// to no single human — the view's `count(*) = 1` guard deliberately yields NULL rather
    /// than picking one arbitrarily.
    pub responsible_actor_id: Option<String>,
}

/// One assertion standing on this chart, read from `cairn_sensitivity_standing` — which
/// needs NO custody, because sensitivity bodies are plaintext by necessity (ADR-0062
/// decision 4: a node must READ a grade in order to coarsen by it).
///
/// That is the whole point of carrying these separately from `threads`: the per-thread
/// breakdown comes from `medication_statement`, whose rows are opened through
/// `cairn_clear_payload`, so a node with no DEK custody projects none of them and the
/// report used to print "no medication threads on this chart" while honouring standing
/// thread-scoped grades on those very threads (#383).
pub struct StandingAssertion {
    pub content_address: String,
    pub subject_kind: String,
    pub subject_id: Uuid,
    pub grade: String,
}

/// One `sensitivity.%` event this node admitted but cannot interpret — ADR-0056's
/// admit-and-defer, a DESIGNED state given that there is no lockstep fleet upgrade.
///
/// It is a grade this node is FAILING TO APPLY. It projects nothing, so the chart reads
/// 'routine', and the only trace is a row in `event_deferred` that nothing in the §5.9 read
/// path consulted before this slice.
pub struct DeferredSensitivityEvent {
    pub event_id: Uuid,
    pub event_type: String,
    /// Rendered in SQL (`::text`) rather than formatted in Rust: TIMESTAMPTZ::text gives
    /// ISO-8601 with the session offset and costs no new dependency — the same idiom the
    /// `deferred` CLI verb already uses.
    pub admitted_at: String,
    /// `None` until a re-adjudication attempt has run and FAILED; then the verbatim refusal.
    pub adjudication_error: Option<String>,
}

/// One event on this chart whose emitted safety rung was FINER than the standing grade
/// licensed (#405 part 2, recorded by db/049 at the LOCAL door only).
///
/// A LEDGER, not a view — ADR-0064 decision 6: flag what cannot self-heal, view what can.
/// A published byte can never improve, so unlike the withdrawal worklist this row will
/// never stop being true.
pub struct SafetyOverclaim {
    pub content_address: String,
    pub emitted_rung: String,
    pub licensed_rung: String,
}

/// One chart's grades, as `patient-sensitivity` renders them.
///
/// `chart_grade`/`chart_subject` is the CHART-WIDE reading: the effective grade computed
/// off the chart's own registration event (its birth act). Exactly one such event is read
/// because `patient_registration_current` is a `SELECT DISTINCT ON (patient_id) ... ORDER BY
/// ... ASC` view (db/045) — NOT because #345 forbids a second registration, which it does
/// not: db/005 step 8b refuses only a chart whose FIRST event is not a registration, and
/// db/045 deliberately retains later duplicates as the evidence that something went wrong.
/// Reading the view rather than the raw table is therefore load-bearing, not stylistic.
/// `identity.%` event types can never carry a medication thread, so resolving there can only
/// ever pick up a chart-wide or a coarsening assertion, never a thread's.
///
/// `threads` is the per-thread breakdown: one entry per medication thread that has a
/// LOCALLY-PROJECTED `medication_statement` row, each resolved through ITS OWN representative
/// event — so a thread whose own grade is outranked by a chart-wide assertion reports the TRUE
/// winning subject, not merely its own standing row (see
/// `the_chart_report_lists_each_medication_thread_with_its_own_winning_subject` in
/// `tests/sensitivity_ladder.rs`). It is NOT "every thread on the chart", and the difference
/// is visible in ordinary operation: `medication_statement_apply` opens its payload through
/// `cairn_clear_payload`, so a node holding no DEK custody projects no rows and reports NO
/// threads at all, and an orphan thread carrying only a cessation or dose event (a state
/// db/031 explicitly designs for) never appears either.
///
/// A NAMED struct for `threads`, not a bare tuple: `sensitivity-withdraw --withdraws`
/// documents its argument as "the hex content_address, as `patient-sensitivity` prints
/// it" — a promise that only holds if the report actually CARRIES that address, which a
/// hand exercise of the CLI (running `patient-sensitivity` then trying to copy a value
/// into `sensitivity-withdraw`) caught was missing from an earlier draft of this struct.
/// Without it, withdrawing anything through the CLI alone would be impossible — an
/// operator would have to fall back to raw SQL, defeating the point of this surface.
pub struct ChartReport {
    pub chart_grade: String,
    /// Which subject produced `chart_grade`, and the hex `content_address` to feed
    /// `sensitivity-withdraw --withdraws` when one applies. One value, not two correlated
    /// ones — see [`WinningSubject`] for why that matters here specifically.
    pub chart_subject: WinningSubject,
    pub threads: Vec<ThreadGrade>,
    /// Withdrawals this node holds that a human should look at — the §1.2 budget's
    /// subject. Empty on a healthy chart, so the renderer stays silent there. See
    /// `WithdrawalWorklistRow`: only the `inert` arm actually failed to take effect.
    pub withdrawals_needing_review: Vec<WithdrawalWorklistRow>,
    /// Every assertion standing on this chart, readable WITHOUT custody. Carried
    /// unconditionally — the custody-blind case has a perfectly good registration and still
    /// projects no threads, so gating this on the no-registration fallback would miss it.
    pub standing: Vec<StandingAssertion>,
    /// Sensitivity events admitted but not interpreted here — grades this node is failing
    /// to apply, invisible to `cairn_effective_sensitivity` by construction.
    pub deferred: Vec<DeferredSensitivityEvent>,
    /// Safety rungs published finer than the grade licensed. An EMPTY vec is not a clean
    /// bill — see `render`'s footer and #414.
    pub overclaims: Vec<SafetyOverclaim>,
    /// How many sealed medication events on this chart this node holds but cannot OPEN.
    ///
    /// The custody fact, MEASURED rather than inferred. `threads` being empty was
    /// previously read as "custody-blind" via the proxy `standing.is_empty()`, which is
    /// wrong in both directions: grading is opt-in, so most custody-blind charts carry no
    /// standing assertion at all and were reported as genuinely empty (#383 surviving in
    /// its own fix), and a chart with no medications but one standing assertion drew a
    /// custody warning it had not earned. Worse, a node holding SOME custody projected a
    /// plausible truncated list and warned about nothing.
    ///
    /// `event_log` keeps the sealed row regardless of custody while `event_clear` does
    /// not, so this is exactly determinable. `> 0` means the thread list above is
    /// incomplete, whatever its length.
    pub sealed_medication_events_without_custody: usize,
}

/// One medication thread's effective grade, as `chart_sensitivity` reports it.
pub struct ThreadGrade {
    pub thread_id: Uuid,
    pub grade: String,
    /// Which subject won, and its withdrawable address — see
    /// [`ChartReport::chart_subject`].
    pub subject: WinningSubject,
}

/// Read `patient`'s current §5.9 sensitivity report. No key, no HLC tick, nothing authored
/// — every query below is a read.
///
/// SIX READS, THREE MIGRATIONS. The chart-wide grade, the per-thread breakdown, the
/// withdrawal worklist and the standing set come from db/048's projections; the deferred
/// list comes from db/043's chart-scoped definer; the overclaim ledger comes from db/049.
/// Anyone reasoning about grants or migration ordering from this doc needs all three.
pub async fn chart_sensitivity(
    client: &mut tokio_postgres::Client,
    patient: Uuid,
) -> anyhow::Result<ChartReport> {
    let patient_s = patient.to_string();

    // The chart-wide reading, resolved off the chart's own registration event (its birth
    // act; `patient_registration_current` is the DISTINCT ON view, so at most one row even
    // if a duplicate registration exists). Reusing `cairn_effective_sensitivity` here,
    // rather than re-deriving "which standing row wins" in Rust, means this report can
    // never silently disagree with the read model every other caller of that function
    // uses (db/048 section 11's own "ONE definition" argument — the same reason
    // `register.rs` never hand-rolls the search-attestation shape it borrows instead).
    // `encode(ces.content_address, 'hex')` on a SQL NULL yields NULL, which
    // tokio-postgres reads straight into `Option<String>` — exactly the "nothing to
    // withdraw" signal db/048 section 11 documents (content_address is left NULL, never
    // coalesced to a sentinel, precisely when no assertion won).
    let chart_row = client
        .query_opt(
            "SELECT ces.grade, ces.subject_kind, encode(ces.content_address, 'hex')
               FROM patient_registration_current r
               JOIN event_log e ON e.content_address = r.content_address,
                    LATERAL cairn_effective_sensitivity(e.event_id) ces
              WHERE r.patient_id = $1::text::uuid",
            &[&patient_s],
        )
        .await?;
    let (chart_grade, chart_subject) = match chart_row {
        Some(row) => {
            let kind: String = row.get(1);
            (
                row.get::<_, String>(0),
                WinningSubject::from_row(&kind, row.get::<_, Option<String>>(2)),
            )
        }
        // NO REGISTRATION ON FILE — REACHABLE IN ORDINARY FEDERATED OPERATION, and the
        // fallback must not answer 'routine' here.
        //
        // An earlier draft called this unreachable "through the real doors" on the strength
        // of #345. That is wrong: db/005 step 8b says in terms that the precedence rule is
        // STRICT-DOOR ONLY and that apply_remote_event must never enforce it, because
        // set-union sync has no ordering and a peer's event legitimately precedes the
        // registration that licenses it. apply_remote_event IS a real door. So a chart whose
        // events arrived by sync ahead of its registration lands here routinely.
        //
        // Answering 'routine' would then be a precise untruth in the disclosure direction:
        // this node may be holding a standing chart-wide 'sequestered' assertion for exactly
        // this patient while the report says nothing applies. The standing set needs no
        // registration event to be readable, so read it directly and report the highest grade
        // standing on the chart. `cairn_sensitivity_standing` is patient-scoped and the
        // ordering mirrors section 11's own (rank first, content_address as the deterministic
        // tie-break), so this can only ever agree with the read model or over-state it —
        // never under-state it.
        None => {
            let standing = client
                .query_opt(
                    "SELECT s.grade, encode(s.content_address, 'hex')
                       FROM cairn_sensitivity_standing($1::text::uuid) s
                      ORDER BY cairn_sensitivity_rank(s.grade) DESC, s.content_address ASC
                      LIMIT 1",
                    &[&patient_s],
                )
                .await?;
            match standing {
                Some(row) => (
                    row.get::<_, String>(0),
                    // Not a specific subject: nothing anchors these assertions to a
                    // registration event here, so the honest phrase is the coarsening one.
                    //
                    // `coarsened`, NOT `from_row`: winner-ness is already settled here by
                    // the row existing, and `cairn_sensitivity_standing` selects a BYTEA
                    // PRIMARY KEY, so there is no NULL to interpret. Reading column 1 as a
                    // non-optional `String` makes a future widening of that query a loud
                    // type error rather than a silent collapse to "nothing applies" — see
                    // `WinningSubject::coarsened`.
                    WinningSubject::coarsened(row.get::<_, String>(1)),
                ),
                // Genuinely nothing: no registration AND no standing assertion.
                None => (
                    cairn_event::sensitivity::GRADE_ROUTINE.to_string(),
                    WinningSubject::None,
                ),
            }
        }
    };

    // The per-thread breakdown: every medication thread with a locally-projected
    // medication_statement row (see the struct doc — that is NOT every thread on the chart
    // when this node holds no custody), each resolved through that table's CURRENT winning
    // content_address — an `ON CONFLICT (medication_id) DO UPDATE` table (db/031), so this
    // always names a real, locally-resolvable event whose `cairn_event_thread` will find
    // exactly this thread (db/048 section 10's "what this resolves, and what it does not"
    // note explains why that resolution is precise only for the CURRENT assert, which is
    // exactly the row this join reads).
    let thread_rows = client
        .query(
            "SELECT ms.medication_id::text, ces.grade, ces.subject_kind,
                    encode(ces.content_address, 'hex')
               FROM medication_statement ms
               JOIN event_log e ON e.content_address = ms.content_address,
                    LATERAL cairn_effective_sensitivity(e.event_id) ces
              WHERE ms.patient_id = $1::text::uuid",
            &[&patient_s],
        )
        .await?;
    let threads = thread_rows
        .into_iter()
        .map(|row| {
            let thread_id: String = row.get(0);
            let grade: String = row.get(1);
            let kind: String = row.get(2);
            ThreadGrade {
                thread_id: Uuid::parse_str(&thread_id)
                    .expect("medication_id column is a valid UUID"),
                grade,
                subject: WinningSubject::from_row(&kind, row.get(3)),
            }
        })
        .collect();

    // The worklist already knows WHY a withdrawal was ineffective — reading it here rather
    // than re-deriving the verdict in Rust is the same "ONE definition" discipline the
    // chart-wide read follows: a second implementation of authority in this file could
    // disagree with db/048 and would do so silently.
    let withdrawal_rows = client
        .query(
            "SELECT encode(withdraws, 'hex'), reason, node_origin, rationale,
                    encode(responsible_actor_id, 'hex')
               FROM sensitivity_withdrawal_worklist
              WHERE patient_id = $1::text::uuid
              ORDER BY reason, withdraws",
            &[&patient_s],
        )
        .await?;
    let withdrawals_needing_review = withdrawal_rows
        .into_iter()
        .map(|row| WithdrawalWorklistRow {
            withdraws: row.get(0),
            reason: row.get(1),
            node_origin: row.get(2),
            rationale: row.get(3),
            responsible_actor_id: row.get(4),
        })
        .collect();

    // Read UNCONDITIONALLY, not only in the no-registration fallback: the custody-blind
    // case has a perfectly good registration and still projects no threads.
    let standing_rows = client
        .query(
            "SELECT encode(s.content_address, 'hex'), s.subject_kind, s.subject_id::text,
                    s.grade
               FROM cairn_sensitivity_standing($1::text::uuid) s
              ORDER BY cairn_sensitivity_rank(s.grade) DESC, s.content_address ASC",
            &[&patient_s],
        )
        .await?;
    let standing = standing_rows
        .into_iter()
        .map(|row| StandingAssertion {
            content_address: row.get(0),
            subject_kind: row.get(1),
            subject_id: Uuid::parse_str(&row.get::<_, String>(2))
                .expect("subject_id column is a valid UUID"),
            grade: row.get(3),
        })
        .collect();

    // Through the chart-scoped definer, never a direct event_deferred read: that table is
    // granted to cairn_node, not cairn_agent (see db/043's own note and #425).
    let deferred_rows = client
        .query(
            "SELECT event_id::text, event_type, admitted_at::text, adjudication_error
               FROM cairn_patient_deferred_sensitivity($1::text::uuid)",
            &[&patient_s],
        )
        .await?;
    let deferred = deferred_rows
        .into_iter()
        .map(|row| DeferredSensitivityEvent {
            event_id: Uuid::parse_str(&row.get::<_, String>(0))
                .expect("event_id column is a valid UUID"),
            event_type: row.get(1),
            admitted_at: row.get(2),
            adjudication_error: row.get(3),
        })
        .collect();

    // The one PERMANENT list on this report. Every other block describes a state that can
    // still improve — a withdrawal can be re-asserted, a deferred event re-adjudicated, a
    // DEK granted. A published byte cannot be clawed back, so an overclaim row never stops
    // being true (ADR-0064 decision 6: flag what cannot self-heal, view what can). Ordered
    // with content_address as the tie-break because recorded_at defaults from
    // clock_timestamp() and two rows written in one transaction can share it.
    let overclaim_rows = client
        .query(
            "SELECT encode(content_address, 'hex'), emitted_rung, licensed_rung
               FROM safety_overclaim_flag
              WHERE patient_id = $1::text::uuid
              ORDER BY recorded_at, content_address",
            &[&patient_s],
        )
        .await?;
    let overclaims = overclaim_rows
        .into_iter()
        .map(|row| SafetyOverclaim {
            content_address: row.get(0),
            emitted_rung: row.get(1),
            licensed_rung: row.get(2),
        })
        .collect();

    // The custody fact, measured not inferred (db/048 section 11b). A definer, because
    // event_clear is deliberately unreadable by the runtime role; it returns a count, never
    // a body. `> 0` means the thread list above is incomplete WHATEVER its length — the
    // partial-custody case, which no proxy over `threads`/`standing` can detect at all.
    let unopenable: i64 = client
        .query_one(
            "SELECT cairn_patient_sealed_medication_without_custody($1::text::uuid)",
            &[&patient_s],
        )
        .await?
        .get(0);

    Ok(ChartReport {
        chart_grade,
        chart_subject,
        threads,
        withdrawals_needing_review,
        standing,
        deferred,
        overclaims,
        sealed_medication_events_without_custody: unopenable as usize,
    })
}
