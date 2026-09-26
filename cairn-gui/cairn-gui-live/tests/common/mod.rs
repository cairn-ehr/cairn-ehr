//! Shared rigging for this crate's DB-gated suites.
//!
//! Deliberately small. `crates/cairn-node/tests/common/` is the real kit; this is the minimum
//! that lets a `cairn-gui` test open a schema-loaded database and sign as a registered actor,
//! and it should stay that way — a second full kit here would be a second set of fixtures to
//! keep true, and they would drift.

// Each test binary in this crate uses a different subset of these helpers, which is normal
// for a `tests/common` module: Cargo compiles it separately into every binary.
#![allow(dead_code)]

use cairn_event::SigningKey;
use tokio_postgres::Client;

/// The connection string for the single-node test database, or `None` when this run has no
/// database. The SAME variable the root tree's suites read, so one export rigs both.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Does this environment-variable value mean YES?
///
/// Deliberately narrow, and the narrowness is the point (#450): `CAIRN_ALLOW_DB_SKIP=please`
/// or `=false` must NOT read as permission to skip the suite, or the opt-out becomes a way to
/// turn the gate off by typo.
pub fn is_affirmative(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Open the test database with its schema loaded, holding the cluster-wide advisory lock.
///
/// Returns `(reader, guard)`. The GUARD's only job is to hold `db::test_serial_guard`'s
/// advisory lock (`0x4341524E`, the same key every root-tree DB suite takes) for as long as
/// the test lives — bind it, never `_`, or the lock releases at the end of the statement and
/// this suite can interleave its `TRUNCATE` with a concurrent root-tree run.
///
/// The `reader` is a SECOND connection with the schema replayed. It is separate from the one
/// a `LiveData` will own, deliberately: `LiveData::db` is private and must stay so, and a test
/// that needs to read a row back should open its own connection rather than grow an accessor
/// into production code for a test's convenience.
pub async fn connect(cs: &str) -> (Client, Client) {
    let guard = cairn_node::db::test_serial_guard(cs)
        .await
        .expect("the cluster-wide test lock");
    let reader = cairn_node::db::connect_and_load_schema(cs)
        .await
        .expect("schema-loaded connection");
    (reader, guard)
}

/// A second schema-loaded connection, for the `LiveData` under test to own.
pub async fn connect_for_live(cs: &str) -> Client {
    cairn_node::db::connect_and_load_schema(cs)
        .await
        .expect("schema-loaded connection for the port")
}

/// Clear every per-patient projection, and enrol a signer.
///
/// # Why the table list is DERIVED and not written down
///
/// The first cut of this helper copied the root tree's five-table list and left out
/// `patient_name` — the table db/046 pass 3 actually reads. Nothing failed on a clean
/// database; it failed on the SECOND run, when a chart registered by the previous run was
/// still findable and a test asserting "the fixture starts empty" saw last time's patient.
/// That is #583's shape exactly: a DB-gated suite depending on state a predecessor left.
///
/// A hand-written list cannot be right for long, because a new clinical stream adds a
/// projection and nothing points at this file. So the list is derived from the catalogue:
/// **every base table in `public` carrying a `patient_id` column**, plus `actor_event`, which
/// is named by hand because it is keyed on a signer rather than on a patient and the enrolment
/// below would otherwise accumulate across runs. Configuration and seed tables carry no
/// `patient_id` and are left alone — which matters, because truncating a seed table would
/// break the floor rather than clean it. `event_log` is in the set and belongs there: it is
/// the source every projection is derived from.
///
/// # ⚠️ What this does NOT clear, and why that is survivable rather than fine
///
/// *"Per-patient projections all carry a `patient_id`"* is the obvious next sentence and it
/// is **false**. The identity stream keys its per-chart state on differently-named columns —
/// `patient_link` on `low`/`high`, `chart_identity_state` on `subject` — and `chart_dispute`,
/// `name_repudiation`, `match_proposal` and `recall_overlay` are in the same family;
/// `event_dek`/`event_clear` key on `event_id`. None is touched here.
///
/// These two suites survive that: db/046 reads `patient_name` / `patient_demographic` /
/// `patient_identifier` and never consults `patient_link`, and a v7 `patient_id` minted this
/// run cannot collide with a link a previous run left. `hlc_state` is deliberately spared —
/// a monotonic clock must not be reset.
///
/// **But the first suite in this tree that touches identity linking inherits #583's shape
/// from its own predecessor run.** Widen the predicate (any base table with a uuid column in
/// `patient_id`/`subject`/`low`/`high`) before writing that suite, rather than after debugging
/// it. Tracked as [#658](https://github.com/cairn-ehr/cairn-ehr/issues/658).
///
/// `quote_ident` rather than bare interpolation: the names come from `pg_class`, so they are
/// real identifiers already, but quoting keeps the generated SQL obviously safe to a reader.
pub async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "DO $$ \
         DECLARE tables text; \
         BEGIN \
           SELECT string_agg(quote_ident(c.relname), ', ') INTO tables \
           FROM pg_class c \
           JOIN pg_namespace n ON n.oid = c.relnamespace \
           JOIN pg_attribute a ON a.attrelid = c.oid AND a.attname = 'patient_id' \
                              AND a.attnum > 0 AND NOT a.attisdropped \
           WHERE n.nspname = 'public' AND c.relkind = 'r'; \
           IF tables IS NULL THEN \
             RAISE EXCEPTION 'no per-patient projection found — has the schema loaded?'; \
           END IF; \
           EXECUTE 'TRUNCATE ' || tables || ', actor_event CASCADE'; \
         END $$;",
    )
    .await
    .expect("truncate every per-patient projection");

    let seed: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(11).wrapping_add(3));
    let sk = SigningKey::from_bytes(&seed);
    let kid = hex::encode(sk.verifying_key().to_bytes());
    // Enrol as a `device` with role `registration-desk` — the SAME shape
    // `cairn_node::actor_enrolment::enroll_device_actor` gives, which since #654 is the one
    // enrolment path both surfaces use (`cairn-node init` and `cairn-node enroll-device-actor`
    // call it; nothing on a write path does).
    //
    // The alternative (`agent` with a model/skill-epoch blob, which the root tree's `setup`
    // uses) is what a matcher or an advisory agent is, not a registration desk. db/005's
    // actor-kind gates all key on `kind = 'human'` today, so neither choice takes a different
    // path — but the day registration is role-gated, a fixture standing in for the wrong kind
    // keeps passing while the real window is refused.
    c.execute(
        "SELECT enroll_actor('device', \
         '{\"role\":\"registration-desk\",\"node_key\":\"funnel-port-test\"}', $1)",
        &[&kid],
    )
    .await
    .expect("enrol the test signer");
    (sk, kid)
}

/// The node identity a `LiveData` is built with, carrying `origin` as its `node_id_hex`.
///
/// `LiveData::new` takes the whole `Identity` precisely so a caller cannot hand it
/// `fingerprint` or `address` by mistake (see its doc), which means a test needs one too. The
/// three unused fields are blank rather than plausible: nothing in this crate reads them, and
/// a blank is honest where a fabricated hex string would invite someone to assert on it.
pub fn identity(origin: &str) -> cairn_node::identity::Identity {
    cairn_node::identity::Identity {
        node_id_hex: origin.to_string(),
        pubkey_hex: String::new(),
        fingerprint: String::new(),
        address: String::new(),
    }
}

/// An empty candidate list — what a browse search returns before anyone is registered.
///
/// Shared because both suites need exactly this value and two copies of a fixture are two
/// things to keep true.
pub fn nothing_found() -> cairn_patient_search::CandidateList {
    cairn_patient_search::CandidateList {
        candidates: vec![],
        incomplete: false,
        incomplete_reason: None,
    }
}

/// Read the RAW `search.displayed` array back out of the stored event body, in order.
///
/// A query against `event_log.body`, NOT the `patient_registration` projection: the projection
/// stores only `displayed_count` (deliberately — db/045's own comment on why two
/// representations of one number is a lie waiting to happen), so the signed body is the only
/// place the actual LIST can be read back from.
///
/// Goes through `::text` + `serde_json` because this tree does not enable tokio-postgres's
/// `with-serde_json-1` feature — the project-wide convention. Copied from
/// `crates/cairn-node/tests/patient_register.rs`, which is the AUTHORITY on the stored shape:
/// the elements are uuid STRINGS, not native uuids, so they are parsed rather than bound.
pub async fn stored_displayed(c: &Client, patient: uuid::Uuid) -> Vec<uuid::Uuid> {
    let row = c
        .query_one(
            "SELECT (body -> 'search' -> 'displayed')::text AS displayed \
             FROM event_log \
             WHERE patient_id::text = $1 AND event_type = 'identity.registration.asserted'",
            &[&patient.to_string()],
        )
        .await
        .expect("the registration event");
    let raw: String = row.get(0);
    let ids: Vec<String> = serde_json::from_str(&raw).expect("displayed is a JSON array");
    ids.iter()
        .map(|s| uuid::Uuid::parse_str(s).expect("each element is a uuid string"))
        .collect()
}

/// Read the stored attestation's `incomplete` flag back — whether the signed body says the
/// SEARCH was partial.
///
/// # Why this is not a detail
///
/// `displayed` says WHICH candidates were on screen; `incomplete` says whether the search
/// behind them read every chart it matched (ADR-0061's meaning). Since ADR-0075 (#671) a
/// prompt being CUT to `PROMPT_CAP` is not that — it is counted on screen and never signed —
/// so a registration made off an overflowing prompt over a whole search must store `false`.
/// Reading the flag back is what proves the port signed the bounded list's own flag and did
/// not re-derive one from the list's length.
pub async fn stored_incomplete(c: &Client, patient: uuid::Uuid) -> bool {
    let row = c
        .query_one(
            "SELECT (body -> 'search' ->> 'incomplete')::boolean \
             FROM event_log \
             WHERE patient_id::text = $1 AND event_type = 'identity.registration.asserted'",
            &[&patient.to_string()],
        )
        .await
        .expect("the registration event");
    row.get(0)
}

/// Read the stored attestation's QUERY back — the name tokens and the birth date the
/// registration swears it searched on.
///
/// The other half of ADR-0061's pair. `stored_displayed` proves the port reported the right
/// candidates; without this, a port that attested the right LIST against somebody else's
/// QUERY would pass every assertion in this crate. The two travel together inside one
/// `AttestedSearch` precisely so they cannot disagree — this is what checks that the value
/// reaching the signed body is still that pair.
pub async fn stored_query(c: &Client, patient: uuid::Uuid) -> (Vec<String>, Option<String>) {
    let row = c
        .query_one(
            "SELECT (body -> 'search' -> 'query' -> 'name_tokens')::text, \
                    body -> 'search' -> 'query' ->> 'birth_date' \
             FROM event_log \
             WHERE patient_id::text = $1 AND event_type = 'identity.registration.asserted'",
            &[&patient.to_string()],
        )
        .await
        .expect("the registration event");
    let tokens: String = row.get(0);
    let birth_date: Option<String> = row.get(1);
    (
        serde_json::from_str(&tokens).expect("name_tokens is a JSON array of strings"),
        birth_date,
    )
}
