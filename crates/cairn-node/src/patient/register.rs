//! §5.3/§5.8 STANDARD patient registration — the funnel's create act. Composes the `cairn-event`
//! wire shape (`RegistrationAssertion`) with the `cairn-patient-search` read model
//! (`SearchAttestation`) `search::search_patients` produced, and authors the
//! `identity.registration.asserted` act PLUS the name/dob/identifier facts that make the chart
//! findable (#350 / Task 8b and the final review's C1 — see below), through `submit_event`.
//!
//! # Why this module exists, and what it deliberately does NOT do
//!
//! §5.8 requires the act that mints a chart to record the search that preceded it — see
//! `cairn_event::registration`'s module doc for why that must be an ACT (not a side effect
//! of whatever event happens to carry the `patient_id` first) and why the attestation NAMES
//! candidates rather than counting them. This module is where that requirement is discharged
//! for the STANDARD path: a clerk typed a search, saw a `CandidateList`, and chose to create
//! anyway. `register_patient` therefore mints ONLY `RegistrationClass::Standard` — the §5.4
//! John Doe path (`john_doe::register_john_doe`) has nothing to search with and is registered
//! elsewhere, and the §5.6 pseudonymous path is likewise out of scope here.
//!
//! No human-author requirement is added here, and none should ever be added: ADR-0061 decision 4
//! records the REJECTED alternative at length — authorship confidence is a GRADE, not a gate
//! (spec §5.11) — and `db/045_patient_registration.sql`'s
//! own header explains why a gate would block care documentation at 03:00 when a clerk's key
//! is not unlocked. The contributor set below is the same "the NODE recorded it" shape every
//! additive event in this codebase uses (see `john_doe::build_callsign_name_body`) — a human
//! registrar, when one signs, is added by the CALLER as a bearing role.
//!
//! # #350 / Task 8b — the funnel could not catch its own duplicate, and why
//!
//! Task 8 wired `patient-search`/`patient-register` together and found, by running the real
//! CLI end to end, that `register_patient` authored ONLY the registration act: it never wrote
//! to `patient_name`/`patient_demographic`, which is exactly what `search_patients` reads. So
//! "register Jane Smith" -> "search Jane Smith" found nothing, and a second registration
//! silently minted a duplicate chart. Filed as issue #350; this module now ALSO authors a
//! `demographic.field.asserted` name and/or dob event, in the SAME transaction as the
//! registration act (a chart is never half-registered — see `register_patient`'s doc for the
//! atomicity argument), and ONLY when actually supplied (principle 4 — a fabricated
//! placeholder would be a precise untruth, worse than the honest gap left by an
//! identifier-only registration). Provenance is `registrar-entered`, not `patient-stated` —
//! see `REGISTRATION_DEMOGRAPHIC_PROVENANCE`'s own doc for why. And `SearchQuery` cannot
//! supply the raw name a name assertion needs (it retains only normalised tokens) without
//! changing shape for one caller's convenience — see `register_patient`'s doc for why it
//! takes `name` as a separate parameter instead.
//!
//! **Review round 1 (#350) found two further gaps, both fixed here.** Important 1: this
//! module used to hardcode dob precision as "day" regardless of what was typed — see
//! `dob_precision`'s own doc for the fix (derive the precision from the value's SHAPE,
//! refuse an unrecognised one, never guess). Important 2: the blank-name filter below was
//! tested only against the LIBRARY shape (`None`) — the live CLI always sends `Some("")`
//! for an empty `--name` (never `None`), and that shape had no test; see
//! `tests/patient_register_demographics.rs` for the added coverage.
//!
//! # Final review, C1 — the identifiers were searched on, attested, and then thrown away
//!
//! The #350 fix above closed passes 2 and 3 and left pass 1 — the HIGHEST-precision one —
//! wide open. `patient-register` accepts repeatable `--identifier system=value`, parses it
//! strictly, searches on it and signs it into the permanent attestation, and this module
//! never wrote a `demographic.identifier.asserted` event. db/046 pass 1 reads
//! `patient_identifier`, which only that event type writes (db/010), and `--identifier` on
//! `patient-register` is the only place in the entire CLI an operator can enter an MRN. So a
//! clerk who registered off an MRN card and later ran `patient-search --identifier MRN=…` —
//! the precise, correct gesture db/045 blesses as "a complete and often better search" — got
//! `no candidates found` and minted a duplicate carrying a signed attestation that reads as
//! perfectly diligent. Worse, an identifier-only registration (no name, no dob — explicitly
//! supported) produced a chart with NO searchable content on any of the three passes:
//! permanently unreachable. `build_identifier_body` closes it, under the same
//! "assert only what was actually supplied" rule, in the same transaction.
//!
//! Split, mirroring `john_doe.rs`: pure body assembly (`build_registration_body`/
//! `build_name_body`/`build_dob_body`/`build_identifier_body`, plus the pure
//! `supplied_identifiers` selection rule — all unit-tested, no DB) plus the async
//! `register_patient` orchestrator. And the one-home rule for the cross-crate conversion:
//!
//! `cairn_event::registration::SearchAttestationInput` takes borrowed primitives (`cairn-event`
//! is the wire core and must not depend on a read-model crate — see that module's own doc for
//! why). `cairn_patient_search::SearchAttestation` is the owned, already-derived read-model
//! value the search surface hands back. Converting between the two has EXACTLY ONE home: here,
//! inside `build_registration_body`. If that conversion existed in two places they could drift
//! apart silently, and a drift here means a registration swearing to candidates the clerk
//! never actually saw — the load-bearing round-trip test in `tests/patient_register.rs` is
//! what keeps this the only place it happens.

use cairn_event::demographics::{
    dob_assertion_body, identifier_assertion_body, name_assertion_body, render_dob_twin,
    render_identifier_twin, render_name_twin, IdentifierAssertion,
};
use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{sign, ClockGrade, EventBody, Hlc, SigningKey};
use cairn_patient_search::{CandidateList, SearchAttestation, SearchQuery};
use tokio_postgres::Client;
use uuid::Uuid;

/// schema_version for a `demographic.field.asserted` event (mirrors `john_doe.rs`'s own
/// private copy — no shared location exports one today).
const DEMOGRAPHIC_FIELD_SCHEMA_VERSION: &str = "demographic.field/1";

/// schema_version for a `demographic.identifier.asserted` event (§4.4, db/010). Same
/// "no shared location exports one today" caveat as the field version above.
const DEMOGRAPHIC_IDENTIFIER_SCHEMA_VERSION: &str = "demographic.identifier/1";

/// The §4.1 provenance stamped on the name/dob/identifier asserted at STANDARD registration.
/// See this
/// module's own doc ("#350 / Task 8b") for the full reasoning; in short: `patient-stated`
/// would often be a precise untruth (the speaker at a registration desk is frequently a
/// third party — a parent, a carer), and `registrar-entered` is the honest description of
/// what actually happened — a clerk wrote down what they were told, unverified.
///
/// Deliberately NOT added to `cairn_provenance_rank` (db/011): that function's own doc
/// states its safe default for an unrecognised term is the LOWEST rank (0, below even
/// "inferred") so a newer/unknown term "can never DISPLACE a known-provenance value" and
/// degrades to "lowest, never highest" — exactly right for a brand-new term today; ranking
/// it explicitly is a separate decision for whoever next audits the whole ladder.
pub const REGISTRATION_DEMOGRAPHIC_PROVENANCE: &str = "registrar-entered";

/// Derive the honest §4.2 dob precision from the SHAPE of a raw date string — YEAR
/// (`YYYY`), MONTH (`YYYY-MM`), or DAY (`YYYY-MM-DD`) — and NEVER guesses: an unrecognised
/// shape is refused, not silently coerced. (Review round 1, #350, Important 1: this module
/// used to hardcode "day" regardless of input, so a clerk who typed only `--birth-date 1980`
/// got a permanent, signed twin reading "Date of birth (registrar-entered): 1980 (day)" —
/// asserting a precision nobody has, exactly the principle-4 failure this task otherwise
/// honours, and immutable once signed.)
///
/// Shape-only, not calendar-valid — mirrors db/011's own "the floor does NOT parse the date
/// value" stance (principle 12, culture-neutral): `"1980-13-40"` passes as day precision.
/// Full calendar validation is a separate, later concern; this function's only job is to
/// stop the *precision label* from being fabricated. Used both here (`register_patient`,
/// before any HLC tick) and at the CLI edge (`main.rs`, before any I/O) — ONE function, so
/// the two call sites can never silently drift into different opinions of "valid".
pub fn dob_precision(value: &str) -> anyhow::Result<&'static str> {
    fn all_digits(s: &str, len: usize) -> bool {
        s.len() == len && s.bytes().all(|b| b.is_ascii_digit())
    }
    match value.split('-').collect::<Vec<_>>().as_slice() {
        [y] if all_digits(y, 4) => Ok("year"),
        [y, m] if all_digits(y, 4) && all_digits(m, 2) => Ok("month"),
        [y, m, d] if all_digits(y, 4) && all_digits(m, 2) && all_digits(d, 2) => Ok("day"),
        _ => anyhow::bail!(
            "birth date {value:?} is not a recognised shape (expected YYYY, YYYY-MM, or \
             YYYY-MM-DD) — refusing rather than asserting a precision nobody actually gave"
        ),
    }
}

/// Assemble the `identity.registration.asserted` `EventBody`. Pure: every input is supplied
/// by the caller (including the freshly-minted `patient_id` and the HLC-derived `event_id`),
/// so the whole wire shape is unit-testable with no clock, no database, and no key.
///
/// `attestation` is `Some` for a `Standard` registration (the db/045 floor requires it
/// present) and `None` for the non-standard classes (the floor requires it structurally
/// ABSENT — see `cairn_event::registration`'s own doc for why absence is not merely
/// optional). This function does not itself enforce that pairing: like the typed builder it
/// wraps, it deliberately PERMITS the illegal states too (twelfth founding principle — the
/// unbypassable enforcement point is the database, not one client's types). A caller that
/// gets the pairing backwards is refused at submit time, never silently admitted.
///
/// THE ONE HOME for the `SearchAttestation` -> `SearchAttestationInput` conversion (see the
/// module doc). `cairn-event`'s builder takes borrowed slices, so this borrows straight out
/// of `attestation` rather than cloning — the borrow only needs to live for this call, which
/// is all `registration_assertion_body`/`render_registration_twin` require.
pub fn build_registration_body(
    event_id: Uuid,
    patient_id: Uuid,
    class: RegistrationClass,
    basis: Option<&str>,
    attestation: Option<&SearchAttestation>,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    let search = attestation.map(|a| SearchAttestationInput {
        terms: SearchTerms {
            name_tokens: &a.query.name_tokens,
            birth_date: a.query.birth_date.as_deref(),
            identifiers: &a.query.identifiers,
        },
        // Borrowed straight out of `a.displayed` — no re-ordering, no re-deriving. Keeping
        // the SAME `Vec<Uuid>` (not, say, sorting it "for tidiness") is exactly what the
        // round-trip test in tests/patient_register.rs pins: the stored order must be the
        // display order, because that is the order the clerk actually read it in.
        displayed: &a.displayed,
        incomplete: a.incomplete,
    });
    let assertion = RegistrationAssertion {
        class,
        basis,
        search,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: REGISTRATION_EVENT_TYPE.into(),
        schema_version: REGISTRATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        // "recorded", never "authored": the NODE recorded this registration. A human
        // registrar, when one signs, is added by the CALLER as a bearing role — see the
        // module doc for why no such requirement belongs in this builder.
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: registration_assertion_body(&assertion),
        attachments: vec![],
        plaintext_twin: Some(render_registration_twin(&assertion)),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// Assemble a §4.2 NAME `demographic.field.asserted` `EventBody` for the registration path.
/// Pure, mirroring `john_doe::build_callsign_name_body`'s shape. `value` is the RAW name text
/// as the clerk typed it — not `SearchQuery`'s normalised tokens (see the module doc's
/// "signature problem" section for why `register_patient` takes it as a separate parameter).
///
/// `use_` is left `None`: this is not asserted as any particular category (legal/alias/…),
/// just "the name given at registration" — no claim about legal status is being made. The
/// db/012 display-winner still picks it up as the display winner via its documented
/// fallback ("when no legal name exists, the newest name of ANY use wins"), which is exactly
/// right for a chart that, at the moment of registration, has no other name on file yet.
pub fn build_name_body(
    event_id: Uuid,
    patient_id: Uuid,
    value: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: name_assertion_body(value, None, REGISTRATION_DEMOGRAPHIC_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_name_twin(
            value,
            None,
            REGISTRATION_DEMOGRAPHIC_PROVENANCE,
        )),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// Assemble a §4.2 DOB `demographic.field.asserted` `EventBody` for the registration path.
/// Pure, mirroring `build_name_body` above. `value` is the raw ISO date string —
/// `SearchQuery.birth_date` already carries it untouched (tokenisation is a NAME-only
/// concern), so no separate raw-value plumbing was needed for this field, unlike the name.
/// `precision` is caller-supplied, not derived here: this stays a pure, unconditional
/// assembler (like `build_name_body`) — `register_patient` is the one place that calls
/// `dob_precision` and PERMITS `precision` to be anything (twelfth founding principle);
/// the caller getting it wrong is a caller bug, not something this builder gatekeeps.
pub fn build_dob_body(
    event_id: Uuid,
    patient_id: Uuid,
    value: &str,
    precision: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: dob_assertion_body(value, precision, None, REGISTRATION_DEMOGRAPHIC_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_dob_twin(
            value,
            precision,
            REGISTRATION_DEMOGRAPHIC_PROVENANCE,
        )),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// The `(system, value)` pairs a registration should actually ASSERT, out of everything the
/// query carried. Pure, so the rule is checkable without a database.
///
/// Two filters, and each one has a reason:
///
///   * **Blank after trimming is "nothing supplied"** — the same principle-4 boundary rule
///     `register_patient` applies to a blank `--name`. The db/010 floor refuses a
///     whitespace-only `system` or `value` outright, so without this filter a stray pair
///     from a library caller would fail the WHOLE registration inside the transaction rather
///     than simply not being asserted.
///   * **Exact duplicates collapse** — the same claim supplied twice is one claim, and two
///     identical signed events in an append-only log are permanent noise. Mirrors
///     `SearchQuery::new`'s own dedup of name tokens ("no duplicate tokens belong in that
///     record"). Order is otherwise preserved: the clerk's entry order is the submission
///     order, exactly as with the attested candidate list.
///
/// **The surviving values are returned TRIMMED — maintainer decision, final review (N3),
/// reversing the original "leave it untrimmed" call.** That call read: "`SearchQuery` stores
/// identifier values verbatim and db/046 pass 1 compares EXACTLY, so trimming here would store
/// a value the very query it came from can no longer match." That was only true because the
/// QUERY side did not trim either — the fix is to trim BOTH sides, the same shape `birth_date`
/// already uses (`SearchQuery::new`, `cairn-patient-search/src/query.rs:84-87` — "a clerk's
/// stray leading/trailing space … must not silently defeat it"). `SearchQuery::new` now trims
/// `system`/`value` the same way, so the stored value and the value a later search compares
/// against agree bit-for-bit. Before this fix, registering with a pasted
/// `--identifier "MRN= 12345"` (trailing space intact) and later searching
/// `--identifier MRN=12345` (no space) did not match — db/046 pass 1's exact compare saw two
/// different strings. Trimming is no longer merely how emptiness is decided; it is also what
/// gets asserted.
pub fn supplied_identifiers(identifiers: &[(String, String)]) -> Vec<(&str, &str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for (system, value) in identifiers {
        let system = system.trim();
        let value = value.trim();
        if system.is_empty() || value.is_empty() {
            continue;
        }
        let pair = (system, value);
        if !out.contains(&pair) {
            out.push(pair);
        }
    }
    out
}

/// Assemble a §4.4 IDENTIFIER `demographic.identifier.asserted` `EventBody` for the
/// registration path. Pure, mirroring `build_name_body`/`build_dob_body`.
///
/// # Why this exists (final review, C1) — do not remove it
///
/// `patient-register` accepts repeatable `--identifier system=value`, parses it strictly,
/// SEARCHES on it and signs it into the permanent §5.8 attestation — and used to discard it.
/// db/046 pass 1 reads `patient_identifier`, which ONLY this event type writes (db/010), and
/// `patient-register --identifier` is the only place in the whole CLI an operator can enter
/// an MRN. So the highest-precision pass could never find a chart the funnel itself had
/// created, and an identifier-only registration produced a chart with zero searchable
/// content on ANY pass — unreachable, permanently.
///
/// `normalized` and `profile` are both `None`, and that is a correctness choice, not a stub:
/// a registration desk holds no §4.4 comparator profile (ADR-0014/ADR-0033), so naming one
/// would be a fabrication, and db/010's floor refuses a `normalized` key that does not name
/// the profile which produced it. With both absent, db/010's projection sets
/// `match_key = COALESCE(normalized, value) = value` — precisely what db/046 pass 1's
/// `pi.match_key = ...` arm (and its `OR pi.value = ...` arm) compares a clerk's typed value
/// against.
///
/// `use_` is `None` for the same reason `build_name_body` claims no name `use`: no category
/// (primary/secondary/…) is being asserted, only "the identifier given at registration".
pub fn build_identifier_body(
    event_id: Uuid,
    patient_id: Uuid,
    system: &str,
    value: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    let assertion = IdentifierAssertion {
        value,
        system,
        provenance: REGISTRATION_DEMOGRAPHIC_PROVENANCE,
        normalized: None,
        profile: None,
        use_: None,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: DEMOGRAPHIC_IDENTIFIER_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: identifier_assertion_body(&assertion),
        attachments: vec![],
        plaintext_twin: Some(render_identifier_twin(&assertion)),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// Register a standard patient: mint a UUID, derive the §5.8 search attestation from the
/// query and candidate list the clerk actually saw, and author the
/// `identity.registration.asserted` act PLUS a name and/or dob `demographic.field.asserted`
/// event and one `demographic.identifier.asserted` event per supplied identifier — for
/// whatever was actually supplied — ALL through the real `submit_event` door,
/// inside ONE transaction, mirroring `register_john_doe` (a chart is never half-registered;
/// see the module doc's "#350 / Task 8b" section for why that means "never
/// registered-but-unfindable").
///
/// `query`/`displayed` are exactly what `patient::search::search_patients` produced and the
/// clerk was shown — `SearchAttestation::from_displayed` derives the attestation FROM them
/// (see `cairn_patient_search::attestation`'s module doc for why it must be derived, never
/// independently constructed).
///
/// `name` is the RAW name text as typed (see the module doc's "signature problem" for why
/// `SearchQuery` cannot supply this) — `None`, or blank after trimming, when nothing was
/// typed (e.g. identifier-only). No name event is authored in that case (principle 4). The
/// dob needs no separate parameter — `query.birth_date` already carries the raw ISO string,
/// and the identical "only if actually supplied" rule applies to it. Nor do the identifiers:
/// `query.identifiers` carries the `(system, value)` pairs `SearchQuery::new` already trimmed,
/// which is exactly what must be asserted so a later search on the same value matches (see
/// `supplied_identifiers`).
///
/// **`name` MUST be the same typed string `query` was built FROM** (i.e. from the very same
/// clerk keystroke that fed `SearchQuery::new`'s `raw_name` argument). Nothing in this
/// function's TYPES enforces that — a caller could structurally attest a search for "Smith"
/// while asserting the name "Jones", and this function would sign both without complaint
/// (the twelfth founding principle again: the type system permits the illegal state; only a
/// disciplined caller prevents it). The one real caller, `main.rs`'s `PatientRegister`
/// handler, satisfies this by construction — `name` and `query` are built from the SAME
/// `name: String` CLI argument, one line apart — but a future caller must preserve that.
///
/// Returns the minted `patient_id`; care can proceed against it immediately, as with
/// `register_john_doe`.
pub async fn register_patient(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    name: Option<&str>,
    query: &SearchQuery,
    displayed: &CandidateList,
) -> anyhow::Result<Uuid> {
    let patient_id = Uuid::now_v7();
    let attestation = SearchAttestation::from_displayed(query, displayed);

    // Principle 4 at the boundary: blank-after-trim counts as "nothing supplied", same as
    // `None` (an empty `--name` CLI default must not assert an empty-string name). Filtering
    // HERE, once, means everything below can treat `Some` as a genuine value.
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    let birth_date = query
        .birth_date
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());

    // Derive + validate the dob precision from its SHAPE up front — before ticking ANY HLC
    // or authoring the registration act (review round 1, #350, Important 1). A malformed
    // shape must refuse the WHOLE call with zero side effects, never partially proceed (tick
    // a clock, author a registration) and only then discover dob's turn fails. Bundling the
    // value with its derived precision here means every later use of `birth_date` already
    // carries an honest, validated precision — never re-derived, never re-guessed.
    let birth_date: Option<(&str, &str)> = birth_date
        .map(|d| dob_precision(d).map(|p| (d, p)))
        .transpose()?;

    // The identifiers to assert, decided BEFORE any HLC tick (same discipline as the dob
    // precision above) so the number of ticks matches the number of events exactly. See
    // `supplied_identifiers` for the blank/duplicate rules and for why the values are NOT
    // trimmed on their way into the record.
    let identifiers = supplied_identifiers(&query.identifiers);

    // Tick the HLC once per event actually being authored, in submission order: the
    // registration act FIRST (the chart's birth act — #345 is expected to require the FIRST
    // event on any patient_id to be a registration, no carve-out here either), then name,
    // then dob, then one per identifier. These self-commit outside the transaction below; a
    // rollback merely leaves a monotonic HLC gap, which is fine (the same shape
    // `register_john_doe`/`auto_apply` use).
    let h_registration = crate::db::next_hlc(client, node_origin).await?;
    let h_name = match name {
        Some(_) => Some(crate::db::next_hlc(client, node_origin).await?),
        None => None,
    };
    let h_dob = match birth_date {
        Some(_) => Some(crate::db::next_hlc(client, node_origin).await?),
        None => None,
    };
    let mut h_identifiers = Vec::with_capacity(identifiers.len());
    for _ in &identifiers {
        h_identifiers.push(crate::db::next_hlc(client, node_origin).await?);
    }

    let registration_body = build_registration_body(
        Uuid::now_v7(),
        patient_id,
        RegistrationClass::Standard,
        None,
        Some(&attestation),
        kid,
        h_registration,
    );
    let registration_signed = sign(&registration_body, sk)?;

    // `.zip` pairs each supplied value with its already-ticked HLC (one is `Some` iff the
    // other is); `.transpose()?` signs iff present, propagating a real signing error.
    let name_signed = name
        .zip(h_name)
        .map(|(n, h)| sign(&build_name_body(Uuid::now_v7(), patient_id, n, kid, h), sk))
        .transpose()?;
    let dob_signed = birth_date
        .zip(h_dob)
        .map(|((d, precision), h)| {
            sign(
                &build_dob_body(Uuid::now_v7(), patient_id, d, precision, kid, h),
                sk,
            )
        })
        .transpose()?;
    // One signed identifier event per supplied pair, paired with its already-ticked HLC.
    // `zip` is length-safe by construction (`h_identifiers` was built by iterating the same
    // vec), and `collect::<Result<_,_>>()` propagates a real signing error rather than
    // silently authoring a short list.
    let identifier_signed = identifiers
        .iter()
        .zip(h_identifiers)
        .map(|((system, value), h)| {
            sign(
                &build_identifier_body(Uuid::now_v7(), patient_id, system, value, kid, h),
                sk,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    // ONE transaction for every event this call authors — the #350 fix: a registration with
    // no matching demographic facts is exactly the half-registered state to avoid. The
    // identifiers (final review, C1) are part of that same rule: a registration that searched
    // on an MRN and then did not record it is unfindable on the highest-precision pass.
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT submit_event($1)",
        &[&registration_signed.signed_bytes],
    )
    .await?;
    if let Some(s) = &name_signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes])
            .await?;
    }
    if let Some(s) = &dob_signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes])
            .await?;
    }
    for s in &identifier_signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes])
            .await?;
    }
    tx.commit().await?;

    Ok(patient_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_patient_search::{Candidate, TrustState};

    fn hlc(wall: i64) -> Hlc {
        Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        }
    }

    fn candidate(id: Uuid) -> Candidate {
        Candidate {
            patient_id: id,
            display_name: "Some One".into(),
            age: None,
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    #[test]
    fn a_standard_body_carries_the_attestation_and_no_basis() {
        let pid = Uuid::from_u128(1);
        let displayed_ids = [Uuid::from_u128(10), Uuid::from_u128(11)];
        let list = CandidateList {
            candidates: displayed_ids.iter().map(|id| candidate(*id)).collect(),
            incomplete: false,
            incomplete_reason: None,
        };
        let query = SearchQuery::new("smith", None, &[]);
        let attestation = SearchAttestation::from_displayed(&query, &list);
        let body = build_registration_body(
            Uuid::from_u128(2),
            pid,
            RegistrationClass::Standard,
            None,
            Some(&attestation),
            "kid",
            hlc(1),
        );
        assert_eq!(body.event_type, "identity.registration.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["class"], "standard");
        assert!(
            body.payload.get("basis").is_none(),
            "a standard registration carries no basis"
        );
        assert_eq!(
            body.payload["search"]["displayed"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(
            !twin.trim().is_empty(),
            "the demographic floor HARD-requires a non-empty twin"
        );
    }

    #[test]
    fn a_non_standard_body_carries_no_search_key() {
        // Pure-assembly proof that this builder permits the illegal pairing rather than
        // gatekeeping it itself — the twelfth founding principle applied. The DB floor
        // (db/045, exercised in tests/patient_registration.rs) is the actual enforcement
        // point; this test only pins what the BUILDER does with mismatched inputs.
        let body = build_registration_body(
            Uuid::from_u128(2),
            Uuid::from_u128(1),
            RegistrationClass::Unidentified,
            Some("unconscious, no ID"),
            None,
            "kid",
            hlc(1),
        );
        assert!(body.payload.get("search").is_none());
        assert_eq!(body.payload["basis"], "unconscious, no ID");
    }

    #[test]
    fn contributors_are_recorded_only_no_responsibility_claimed() {
        let body = build_registration_body(
            Uuid::from_u128(2),
            Uuid::from_u128(1),
            RegistrationClass::Standard,
            None,
            None,
            "kid",
            hlc(1),
        );
        let c = &body.contributors[0];
        assert_eq!(c["role"], "recorded");
        assert!(
            c.get("responsibility").is_none(),
            "no attestation is demanded of a recorded-only contributor"
        );
    }

    // --- #350 / Task 8b: the name and dob demographic assertions ---

    #[test]
    fn name_body_asserts_field_name_with_registrar_entered_provenance() {
        let pid = Uuid::from_u128(1);
        let body = build_name_body(
            Uuid::from_u128(2),
            pid,
            "O'Brien-Smith, Jane",
            "kid",
            hlc(1),
        );
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["field"], "name");
        assert_eq!(body.payload["value"], "O'Brien-Smith, Jane");
        // Literal, not the imported constant: comparing against the constant would make this
        // assertion vacuous against a change to the constant's OWN value.
        assert_eq!(
            body.payload["provenance"], "registrar-entered",
            "must NOT be patient-stated — see REGISTRATION_DEMOGRAPHIC_PROVENANCE's doc for why"
        );
        assert!(
            body.payload.get("facets").is_none(),
            "no use category is claimed for a registration-desk name"
        );
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(
            !twin.trim().is_empty(),
            "the demographic floor HARD-requires a non-empty twin"
        );
        assert!(twin.contains("O'Brien-Smith, Jane"));
    }

    #[test]
    fn dob_body_carries_the_supplied_precision_and_registrar_entered_provenance() {
        let pid = Uuid::from_u128(1);
        // "day" here is a CALLER-SUPPLIED precision, not derived by this builder — the
        // point of `dob_body_asserts_the_caller_supplied_precision_verbatim` below (and the
        // `dob_precision_*` suite) is that `build_dob_body` never re-derives or second-
        // guesses it, so this test uses a fixed literal to pin only the wiring.
        let body = build_dob_body(Uuid::from_u128(2), pid, "1980-01-01", "day", "kid", hlc(1));
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["field"], "dob");
        assert_eq!(body.payload["value"], "1980-01-01");
        assert_eq!(body.payload["provenance"], "registrar-entered"); // literal — see above
        assert_eq!(body.payload["facets"]["precision"], "day");
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(!twin.trim().is_empty());
        assert!(twin.contains("1980-01-01"));
    }

    #[test]
    fn dob_body_asserts_the_caller_supplied_precision_verbatim() {
        // `build_dob_body` must not hardcode or re-derive a precision (review round 1,
        // #350, Important 1) — whatever the caller passes lands unchanged, for every shape
        // `dob_precision` recognises.
        let pid = Uuid::from_u128(1);
        for (value, precision) in [
            ("1980", "year"),
            ("1980-06", "month"),
            ("1980-06-15", "day"),
        ] {
            let body = build_dob_body(Uuid::from_u128(2), pid, value, precision, "kid", hlc(1));
            assert_eq!(
                body.payload["facets"]["precision"], precision,
                "value={value}"
            );
            let twin = body.plaintext_twin.as_deref().unwrap();
            assert!(
                twin.contains(precision),
                "twin must legibly state the precision: {twin}"
            );
        }
    }

    #[test]
    fn name_and_dob_bodies_are_recorded_only_no_responsibility_claimed() {
        let pid = Uuid::from_u128(1);
        for body in [
            build_name_body(Uuid::from_u128(2), pid, "Jane", "kid", hlc(1)),
            build_dob_body(Uuid::from_u128(2), pid, "1980-01-01", "day", "kid", hlc(1)),
        ] {
            let c = &body.contributors[0];
            assert_eq!(c["role"], "recorded");
            assert!(
                c.get("responsibility").is_none(),
                "additive events demand no attestation"
            );
        }
    }

    // --- final review, C1: the §4.4 identifier assertion ---

    #[test]
    fn identifier_body_asserts_field_identifier_with_registrar_entered_provenance() {
        let pid = Uuid::from_u128(1);
        let body = build_identifier_body(Uuid::from_u128(2), pid, "MRN", "12345", "kid", hlc(1));
        assert_eq!(body.event_type, "demographic.identifier.asserted");
        assert_eq!(body.schema_version, "demographic.identifier/1");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["field"], "identifier");
        assert_eq!(body.payload["system"], "MRN");
        assert_eq!(body.payload["value"], "12345");
        // Literal, not the imported constant — see `name_body_asserts_…` above for why.
        assert_eq!(body.payload["provenance"], "registrar-entered");
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(
            !twin.trim().is_empty(),
            "the demographic floor HARD-requires a non-empty twin"
        );
        assert!(twin.contains("12345"), "{twin}");
        assert!(twin.contains("MRN"), "{twin}");
    }

    #[test]
    fn identifier_body_claims_no_normalized_key_no_profile_and_no_use() {
        // Not decoration: db/010 sets `match_key = COALESCE(normalized, value)`, so a
        // `normalized` key here would move the stored match_key AWAY from the value db/046
        // pass 1 compares a clerk's typed identifier against — and a `profile` would claim a
        // §4.4 comparator bundle a registration desk does not hold (ADR-0014/ADR-0033).
        let body = build_identifier_body(
            Uuid::from_u128(2),
            Uuid::from_u128(1),
            "NHS",
            "943 476 5919",
            "kid",
            hlc(1),
        );
        assert!(
            body.payload.get("normalized").is_none(),
            "no comparator profile is held, so no materialised key may be claimed"
        );
        assert!(body.payload.get("profile").is_none());
        assert!(
            body.payload.get("use").is_none(),
            "no use category is claimed for a registration-desk identifier"
        );
    }

    #[test]
    fn identifier_body_is_recorded_only_no_responsibility_claimed() {
        let body = build_identifier_body(
            Uuid::from_u128(2),
            Uuid::from_u128(1),
            "MRN",
            "1",
            "kid",
            hlc(1),
        );
        let c = &body.contributors[0];
        assert_eq!(c["role"], "recorded");
        assert!(c.get("responsibility").is_none());
    }

    #[test]
    fn supplied_identifiers_keeps_entry_order_and_trims_the_value() {
        // Maintainer decision, final review (N3): reverses the earlier "keep it verbatim"
        // test. `SearchQuery::new` now trims identifiers on the way in too (query.rs), so
        // trimming here as well is what keeps the STORED value and a later QUERY's value in
        // agreement — db/046 pass 1's exact compare would otherwise be defeated by a pasted
        // MRN's stray whitespace on one side but not the other.
        let input = vec![
            ("MRN".to_string(), " 12345 ".to_string()),
            ("NHI".to_string(), "ZZZ9999".to_string()),
        ];
        assert_eq!(
            supplied_identifiers(&input),
            vec![("MRN", "12345"), ("NHI", "ZZZ9999")],
            "the value must be asserted TRIMMED — db/046 pass 1 compares it with `=`, and \
             the query side (SearchQuery::new) now trims too, so a mismatch here would make \
             the chart unfindable by the very query it was registered from"
        );
    }

    #[test]
    fn supplied_identifiers_drops_blank_pairs_rather_than_failing_the_registration() {
        let input = vec![
            ("".to_string(), "12345".to_string()),
            ("MRN".to_string(), "".to_string()),
            ("   ".to_string(), "12345".to_string()),
            ("MRN".to_string(), "   ".to_string()),
            ("MRN".to_string(), "real".to_string()),
        ];
        assert_eq!(
            supplied_identifiers(&input),
            vec![("MRN", "real")],
            "a blank system or value is 'nothing supplied' (principle 4), not an identifier \
             — and the db/010 floor would refuse it, failing the whole registration"
        );
    }

    #[test]
    fn supplied_identifiers_collapses_exact_duplicates_only() {
        let input = vec![
            ("MRN".to_string(), "12345".to_string()),
            ("MRN".to_string(), "12345".to_string()),
            // Same system, DIFFERENT value: two distinct claims, both must survive (§4.4
            // keeps both as the veto SIGNAL — see db/010's projection comment).
            ("MRN".to_string(), "67890".to_string()),
        ];
        assert_eq!(
            supplied_identifiers(&input),
            vec![("MRN", "12345"), ("MRN", "67890")]
        );
    }

    // --- review round 1, #350, Important 1: `dob_precision` must derive, never fabricate ---

    #[test]
    fn dob_precision_recognises_year_month_and_day_shapes() {
        assert_eq!(dob_precision("1980").unwrap(), "year");
        assert_eq!(dob_precision("1980-06").unwrap(), "month");
        assert_eq!(dob_precision("1980-06-15").unwrap(), "day");
    }

    #[test]
    fn dob_precision_refuses_an_unrecognised_shape_rather_than_guessing() {
        for bad in [
            "not-a-date",
            "1980/06/15", // slash-separated — a real shape a clerk might type, still refused
            "80-06-15",   // two-digit year
            "1980-6-15",  // un-padded month
            "1980-06-15-extra",
            "",
            "1980-",
        ] {
            assert!(
                dob_precision(bad).is_err(),
                "{bad:?} is not one of YYYY / YYYY-MM / YYYY-MM-DD and must be refused, not \
                 silently coerced to a guessed precision"
            );
        }
    }
}
