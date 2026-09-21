//! The answer pass 3 gives to every typing gesture, pinned as an EXACT set (#639).
//!
//! `db/046`'s pass 3 is about to be rewritten for cost: the query-token normalisation moves behind
//! an `OFFSET 0` optimisation fence, the lateral's `UNION` becomes `UNION ALL`, and the
//! alphanumeric-parts branch is skipped for a value that carries no punctuation. All three are
//! claimed to be **semantically neutral** — they change how much work the scan does, never which
//! candidates come back — and a claim of neutrality is worth exactly as much as the test that would
//! catch it being false.
//!
//! **Why this suite exists rather than more cases in `patient_search.rs`.** Every test there asserts
//! that a chart IS found (`rows.iter().any(...)`), which is the right shape for "this gesture must
//! work" and the wrong shape for "nothing else changed": a rewrite that started returning EXTRA
//! charts would pass all of them. Here the assertion is **set equality** over one seeded corpus —
//! nothing lost and nothing gained — which is the slice-1 reviewer's `EXCEPT`-both-ways method made
//! standing instead of one-off.
//!
//! **Each expectation is derived from the contract, not from a run.** A pin copied out of the
//! shipped code's output pins whatever the code does, including its defects; every row in
//! `GESTURES` below carries the reason the clerk's gesture must land where it does. The suite was
//! also run red under three mutations before being trusted — deleting the parts branch, deleting
//! the prefix arm, and dropping the parts branch's callsign guard — recorded in the #639 plan.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`,
//! the same shared-DB + TRUNCATE pattern every suite in this directory uses.
mod common;

use cairn_event::demographics::{name_assertion_body, render_name_twin};
use cairn_event::SigningKey;
use cairn_node::{db, john_doe};
use common::{cs, setup, submit_registration, submit_signed, EventSpec};
use std::collections::BTreeSet;
use tokio_postgres::Client;
use uuid::Uuid;

/// Mirrors `patient_search.rs`'s list: `patient_name` is the retained name set pass 3 tokenises,
/// and the John Doe this corpus registers writes `chart_identity_state` and `patient_registration`.
/// Truncated for the same leave-no-residue discipline, even where this suite does not read them.
const EXTRA_TABLES: [&str; 3] = [
    "patient_name",
    "chart_identity_state",
    "patient_registration",
];

/// Submit one §4.2 name assertion — the only event shape this corpus needs.
async fn submit_name(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    value: &str,
) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: name_assertion_body(value, Some("legal"), "patient-stated"),
            plaintext_twin: Some(render_name_twin(value, Some("legal"), "patient-stated")),
            wall,
        },
    )
    .await
    .expect("name assertion accepted by the floor");
}

/// Register a chart and give it one name.
async fn chart_with_name(c: &Client, sk: &SigningKey, kid: &str, value: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    submit_name(c, sk, kid, p, 1, value).await;
    p
}

/// The candidate set for one typed token, as a comparable set of patient ids.
///
/// Only pass 3 can fire here — the corpus asserts no identifier and no date of birth — so the
/// `matched_pass` label is asserted separately and once, rather than being carried through every
/// row of the expectation table.
async fn name_candidates(c: &Client, typed: &str) -> BTreeSet<Uuid> {
    let tokens: Vec<String> = vec![typed.to_string()];
    let rows = c
        .query(
            "SELECT patient_id::text, matched_pass \
             FROM cairn_search_candidates($1, NULL, '[]'::jsonb)",
            &[&tokens],
        )
        .await
        .unwrap();
    rows.iter()
        .map(|r| {
            let pass: String = r.get(1);
            assert_eq!(
                pass, "name",
                "this corpus asserts no identifier and no dob, so every row must come from pass 3"
            );
            let id: String = r.get(0);
            Uuid::parse_str(&id).expect("patient_id is a valid uuid")
        })
        .collect()
}

/// One seeded chart, named for the pass-3 path it exercises.
struct Corpus {
    /// `Anne A Smith` — UNPUNCTUATED and multi-word, and the reason the "skip the parts branch
    /// when the value has no punctuation" change is a subset argument rather than a guess. Its
    /// single-character word `A` is kept by the whole-token source and dropped by the parts
    /// source, so a rewrite that skipped the WHOLE branch instead of the parts branch loses it.
    plain: Uuid,
    /// `Fyodorowksi-Eschenbacher` — punctuated, so the parts branch must still run for it. This is
    /// the chart slice 1a exists for.
    compound: Uuid,
    /// `Smith, John` — the registration-desk convention `register_patient` stores raw. Its whole
    /// token is `smith,` with the comma; only the parts branch makes `smith` reach it (#348).
    comma: Uuid,
    /// `李小明` — one token, no whitespace and no punctuation to split on, so it is reachable only
    /// by the prefix arm's BYTE gate (#638).
    cjk: Uuid,
    /// `José Álvarez` stored DECOMPOSED — the NFC path. Without `normalize`, a composed query
    /// string is a different byte sequence and the chart is silently unfindable.
    decomposed: Uuid,
    /// `Wu` with surrounding whitespace — a two-character surname, below the prefix arm's byte
    /// gate, findable only because the EXACT arm carries no length rule at all.
    short: Uuid,
    /// A John Doe. Its callsign must be findable in full and must never fragment.
    john_doe: Uuid,
    /// That John Doe's callsign, lower-cased, as a clerk would type it back.
    callsign: String,
}

/// Seed one chart per pass-3 path. Returns the ids so each expectation can name its charts.
async fn seed(c: &mut Client, sk: &SigningKey, kid: &str) -> Corpus {
    let plain = chart_with_name(c, sk, kid, "Anne A Smith").await;
    let compound = chart_with_name(c, sk, kid, "Fyodorowksi-Eschenbacher").await;
    let comma = chart_with_name(c, sk, kid, "Smith, John").await;
    let cjk = chart_with_name(c, sk, kid, "李小明").await;
    // Written with COMBINING ACUTE ACCENT (U+0301) rather than the composed letters, so the stored
    // bytes differ from what a clerk's keyboard emits.
    let decomposed = chart_with_name(c, sk, kid, "Jose\u{301} A\u{301}lvarez").await;
    let short = chart_with_name(c, sk, kid, "  Wu  ").await;

    let (john_doe, callsign, _ord) = john_doe::register_john_doe(
        c,
        sk,
        kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .expect("john doe registration accepted by the floor");

    Corpus {
        plain,
        compound,
        comma,
        cjk,
        decomposed,
        short,
        john_doe,
        callsign: callsign.to_lowercase(),
    }
}

/// What a clerk types, and the EXACT set of charts that must come back.
///
/// `why` is not decoration: it is the contract each row is derived from, so a reviewer can check
/// the expectation against the design rather than against the code that produced it.
struct Gesture {
    typed: String,
    expected: BTreeSet<Uuid>,
    why: &'static str,
}

fn gesture(typed: &str, expected: impl IntoIterator<Item = Uuid>, why: &'static str) -> Gesture {
    Gesture {
        typed: typed.to_string(),
        expected: expected.into_iter().collect(),
        why,
    }
}

/// The table the whole suite is. Every arm of pass 3 appears at least once, and — the half that
/// makes this a neutrality guard rather than a findability one — several gestures expect the EMPTY
/// set, which is what a widening rewrite would break first.
fn gestures(k: &Corpus) -> Vec<Gesture> {
    vec![
        gesture(
            "smith",
            [k.plain, k.comma],
            "EXACT arm reaches the unpunctuated chart's whole token. The comma chart is reached \
             TWICE OVER — the parts source strips its comma, and `starts_with('smith,', 'smith')` \
             is true as well — which is why mutation M1 did not move this row and `esch` is the \
             gesture that pins the parts source. Kept because the DOUBLE path is exactly what \
             `UNION ALL` stops de-duplicating inside the lateral.",
        ),
        gesture(
            "a",
            [k.plain],
            "A single character is below the prefix arm's byte gate, so this can only be the EXACT \
             arm equalling the stored whole token `a`. A rewrite that skipped the WHOLE-token \
             source for an unpunctuated value — rather than the parts source — loses exactly this.",
        ),
        gesture(
            "anne",
            [k.plain],
            "The whole-token source again, on a value whose parts branch is the one being skipped.",
        ),
        gesture(
            "esch",
            [k.compound],
            "PREFIX of a PART: four bytes, above the gate, matching `eschenbacher` only after 1a \
             has split the compound. Both new arms of slice 1 in one gesture.",
        ),
        gesture(
            "fyodorowksi-eschenbacher",
            [k.compound],
            "The compound typed back exactly as printed — the whole-token source, unsplit.",
        ),
        gesture(
            "john",
            [k.comma],
            "The second word of `Smith, John`, reached by the whole-token source.",
        ),
        gesture(
            "李小",
            [k.cjk],
            "Two characters and six bytes: the #638 gesture. Above the gate BECAUSE the gate counts \
             bytes, and the only arm that can reach a single unpunctuated CJK token.",
        ),
        gesture(
            "álvarez",
            [k.decomposed],
            "Typed COMPOSED against a chart stored DECOMPOSED: only NFC normalisation on both sides \
             makes these the same token.",
        ),
        gesture(
            "wu",
            [k.short],
            "Two characters, below the byte gate, so the PREFIX arm refuses it — and the chart comes \
             back anyway, because the EXACT arm has no length rule. Short surnames stay findable.",
        ),
        gesture(
            &k.callsign,
            [k.john_doe],
            "A callsign typed back in full: the whole-token source deliberately keeps callsigns, \
             punctuation intact, so the John Doe in front of the clerk is findable.",
        ),
        gesture(
            "unknown",
            [],
            "EMPTY, and this is the load-bearing empty. `unknown` is the callsign's leading word, \
             so without the callsign guards on BOTH new arms one typed word surfaces every John Doe \
             on the node.",
        ),
        gesture(
            "mi",
            [],
            "EMPTY: two Latin bytes is below the gate, and nothing stores `mi` as a whole token. \
             The gate must not loosen for Latin script when it admits two CJK characters.",
        ),
        gesture(
            "",
            [],
            "EMPTY: an empty query token must not equal the empty string a stored value's \
             surrounding whitespace projects — `  Wu  ` is a legitimately admitted value.",
        ),
        gesture(
            "zzz",
            [],
            "EMPTY: the floor case. A search that finds nothing must find nothing.",
        ),
    ]
}

/// Pass 3 answers every typing gesture with exactly the charts the contract names — no more.
///
/// The ONE test in this suite, deliberately: the corpus is expensive to seed (seven charts through
/// the real `submit_event` door) and every gesture reads the same seeded state, so splitting this
/// into fourteen tests would seed it fourteen times and assert nothing extra.
#[tokio::test]
async fn every_typing_gesture_returns_exactly_the_charts_it_should() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let corpus = seed(&mut c, &sk, &kid).await;

    // Collected rather than asserted one at a time: a rewrite that breaks several gestures should
    // report all of them in one run, not send the next session round the loop once per gesture.
    let mut failures: Vec<String> = Vec::new();
    for g in gestures(&corpus) {
        let got = name_candidates(&c, &g.typed).await;
        if got != g.expected {
            let lost: Vec<_> = g.expected.difference(&got).collect();
            let gained: Vec<_> = got.difference(&g.expected).collect();
            failures.push(format!(
                "typing {:?}: lost {lost:?}, gained {gained:?}\n    contract: {}",
                g.typed, g.why
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "pass 3's candidate set changed for {} of the pinned gestures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// A guard against the corpus quietly emptying itself.
///
/// Every expectation above is a set comparison, and a corpus that failed to seed would make most of
/// them trivially true — the empty set equals the empty set. This asserts the seven charts are
/// distinct and present in `patient_name` before any of that reasoning is worth anything.
#[tokio::test]
async fn the_corpus_seeds_seven_distinct_charts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let k = seed(&mut c, &sk, &kid).await;
    let ids: BTreeSet<Uuid> = [
        k.plain,
        k.compound,
        k.comma,
        k.cjk,
        k.decomposed,
        k.short,
        k.john_doe,
    ]
    .into_iter()
    .collect();
    assert_eq!(ids.len(), 7, "the corpus must seed seven DISTINCT charts");

    // Bound and compared as TEXT: `cairn-node` does not enable tokio-postgres's `with-uuid-1`
    // feature (project-wide convention), so a `Uuid` cannot be a bind parameter here.
    let as_text: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
    let named: i64 = c
        .query_one(
            "SELECT count(DISTINCT patient_id) FROM patient_name WHERE patient_id::text = ANY($1)",
            &[&as_text],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        named, 7,
        "every seeded chart must hold a name row for pass 3 to tokenise"
    );
}
