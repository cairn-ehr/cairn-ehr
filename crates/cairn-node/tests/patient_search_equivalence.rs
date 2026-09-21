//! The answer pass 3 gives to every typing gesture, pinned as an EXACT set (#639).
//!
//! `db/046`'s pass 3 was rewritten for cost in #639: the query-token normalisation moved behind an
//! `OFFSET 0` optimisation fence, the lateral's `UNION` became `UNION ALL`, and the
//! alphanumeric-parts branch is skipped for a value that carries no punctuation. All three are
//! claimed to be **semantically neutral** — they change how much work the scan does, never which
//! candidates come back — and a claim of neutrality is worth exactly as much as the test that would
//! catch it being false.
//!
//! That is not a hypothetical here. The parts-branch skip shipped in review with its guard testing
//! `normalize(pn.value, NFC)` while the splitter it guards sees `lower(normalize(pn.value, NFC))`,
//! and one Unicode character — U+0130 `İ` — falls through the gap. The `nce` gesture below is the
//! regression test; `the_subset_argument_holds_for_every_unicode_code_point` is the reason no
//! second such character can exist.
//!
//! ⚠️ **One row of this table is deployment-dependent, and it is asked of the server rather than
//! assumed.** U+0130 only grows a combining mark under FULL Unicode case mapping — an ICU-provider
//! database — so on a simple-case-mapping (libc) server `İnce` lowercases to plain `ince` and `nce`
//! is not a token at all. Both answers are correct; see `full_case_mapping`. The `nce` gesture
//! pinned the ICU answer unconditionally when first written, passed on every local database and
//! failed in CI, which is the lesson: a contract suite must derive its expectation from the
//! contract, and on this row the contract has a locale in it.
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

/// The candidate set for what a clerk typed, as a comparable set of patient ids.
///
/// Only pass 3 can fire here — the corpus asserts no identifier and no date of birth — so the
/// `matched_pass` label is asserted separately and once, rather than being carried through every
/// row of the expectation table.
///
/// **Multiplicity is asserted before the set collapse, and that is not pedantry.** #639 turned the
/// lateral's `UNION` into `UNION ALL`, which makes pass 3 emit a duplicate ROW for a token both
/// sources project (`Smith, John` → `john` twice). That is safe only because the branch's own
/// `SELECT DISTINCT` collapses it — and `db/046`'s dedup block warns in as many words that if a
/// later change makes that `DISTINCT` non-load-bearing, the duplicates reach the caller. A helper
/// that collects straight into a `BTreeSet` throws the evidence away exactly where the change made
/// it observable, so the row count is checked against the distinct count first.
async fn name_candidates(c: &Client, typed: &[String]) -> BTreeSet<Uuid> {
    let rows = c
        .query(
            "SELECT patient_id::text, matched_pass \
             FROM cairn_search_candidates($1, NULL, '[]'::jsonb)",
            &[&typed],
        )
        .await
        .unwrap();
    let ids: Vec<Uuid> = rows
        .iter()
        .map(|r| {
            let pass: String = r.get(1);
            assert_eq!(
                pass, "name",
                "this corpus asserts no identifier and no dob, so every row must come from pass 3"
            );
            let id: String = r.get(0);
            Uuid::parse_str(&id).expect("patient_id is a valid uuid")
        })
        .collect();

    let distinct: BTreeSet<Uuid> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        distinct.len(),
        "typing {typed:?} returned {} rows for {} distinct charts — pass 3 is emitting DUPLICATE \
         rows to its caller. Since #639 the lateral is `UNION ALL`, so the branch's own `SELECT \
         DISTINCT` is the only thing collapsing a token both sources project; this fires when that \
         has stopped doing its job. Rows: {ids:?}",
        ids.len(),
        distinct.len()
    );
    distinct
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
    /// `İnce` — a Turkish surname opening with U+0130 LATIN CAPITAL LETTER I WITH DOT ABOVE, the
    /// one character in Unicode that is `[:alnum:]` but whose lowercase is not (`i` + U+0307
    /// COMBINING DOT ABOVE, under full case mapping). It looks unpunctuated to a guard applied
    /// BEFORE lowering and punctuated to the splitter, which reads the lowered string — so this
    /// chart is the difference between `db/046`'s parts-branch skip being neutral and silently
    /// dropping a token. See the `nce` gesture.
    turkish: Uuid,
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
    // U+0130 written explicitly: an editor or a copy-paste that silently substituted a plain `I`
    // would leave this chart seeded, every assertion green, and the defect it pins untested.
    let turkish = chart_with_name(c, sk, kid, "\u{130}nce").await;

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
        turkish,
        john_doe,
        callsign: callsign.to_lowercase(),
    }
}

/// Does this server apply FULL Unicode case mapping, or the simple one-to-one kind?
///
/// The distinction has exactly one consequence for pass 3, and the `nce` gesture below turns on it.
/// Under full case mapping — what an ICU-provider database does — `lower('İ')` is `i` + U+0307
/// COMBINING DOT ABOVE, two characters, the second of which is not `[:alnum:]`. Under simple case
/// mapping — libc, which is what a default `initdb` on the CI runner gives — `lower('İ')` is plain
/// `i`, and the combining mark never appears.
///
/// So a chart named `İnce` genuinely has DIFFERENT tokens on the two, and both are correct:
///
/// | | lowered value | whole tokens | parts tokens | parts branch |
/// |---|---|---|---|---|
/// | ICU  | `i̇nce` | `{i̇nce}` | `{nce}` | runs |
/// | libc | `ince`  | `{ince}`  | `{ince}` | skipped, and rightly |
///
/// **This is asked of the server rather than assumed, because assuming it is what broke the suite
/// once already.** The `nce` gesture first shipped pinning the ICU answer unconditionally; it passed
/// on every local database (all ICU) and failed in CI (libc) — where `nce` had never been a token at
/// all, before the rewrite or after, so the neutrality claim held trivially. A contract suite must
/// derive its expectation from the contract, and on this one row the contract depends on a property
/// of the deployment.
async fn full_case_mapping(c: &Client) -> bool {
    c.query_one(
        "SELECT lower(normalize('\u{130}', NFC)) ~ '[^[:alnum:][:space:]]' AS full_mapping",
        &[],
    )
    .await
    .unwrap()
    .get(0)
}

/// What a clerk types, and the EXACT set of charts that must come back.
///
/// `why` is not decoration: it is the contract each row is derived from, so a reviewer can check
/// the expectation against the design rather than against the code that produced it.
struct Gesture {
    /// The token ARRAY handed to `cairn_search_candidates`, not a single string: `SearchQuery::new`
    /// emits whole words AND their parts, so the real caller shape is multi-token. Every gesture in
    /// this suite was single-token until #639's review found that the rewritten query-token join —
    /// whose whole purpose is the (stored token × query token) cross product — had never been
    /// exercised with more than one query token anywhere in the repository.
    typed: Vec<String>,
    expected: BTreeSet<Uuid>,
    why: &'static str,
}

fn gesture(typed: &str, expected: impl IntoIterator<Item = Uuid>, why: &'static str) -> Gesture {
    tokens(&[typed], expected, why)
}

/// A gesture carrying several query tokens, as `SearchQuery::new` actually emits them.
fn tokens(typed: &[&str], expected: impl IntoIterator<Item = Uuid>, why: &'static str) -> Gesture {
    Gesture {
        typed: typed.iter().map(|t| t.to_string()).collect(),
        expected: expected.into_iter().collect(),
        why,
    }
}

/// The table the whole suite is. Every arm of pass 3 appears at least once, and — the half that
/// makes this a neutrality guard rather than a findability one — several gestures expect the EMPTY
/// set, which is what a widening rewrite would break first.
fn gestures(k: &Corpus, full_mapping: bool) -> Vec<Gesture> {
    vec![
        gesture(
            "smith",
            [k.plain, k.comma],
            "EXACT arm reaches the unpunctuated chart's whole token. The comma chart is reached \
             TWICE OVER — the parts source strips its comma, and `starts_with('smith,', 'smith')` \
             is true as well — which is why mutation M1 did not move this row and `esch` is the \
             gesture that pins the parts source. Note the two paths reach it by DIFFERENT tokens \
             (`smith` from the parts source, `smith,` from the whole one), so plain `UNION` would \
             never have collapsed them either: this row says nothing about `UNION ALL`. The row \
             that rides that change is `john`.",
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
            "The second word of `Smith, John` — and THE gesture that rides `UNION ALL`. `john` is \
             an unpunctuated word inside a punctuated value, so both lateral sources project it \
             and the lateral now emits it TWICE. Exactly one row must still reach the caller, \
             which `name_candidates` asserts on multiplicity before collapsing to a set.",
        ),
        if full_mapping {
            gesture(
                "nce",
                [k.turkish],
                "THE U+0130 REGRESSION (#639 review), on a server that applies FULL case mapping. \
                 `İnce` lowercases to `i` + U+0307 COMBINING DOT ABOVE, and a combining mark is \
                 not `[:alnum:]`, so the parts split projects `nce` while the whole-token split \
                 projects only `i̇nce`. This chart is therefore reachable ONLY through the parts \
                 branch — and `db/046`'s skip guard must decide whether to run that branch by \
                 looking at the LOWERED value. A guard that tests the un-lowered \
                 `normalize(pn.value, NFC)` sees an unpunctuated string, skips the branch, and \
                 this gesture returns EMPTY: a chart findable by `nce` before the rewrite is not \
                 after it, which falsifies the whole neutrality claim.",
            )
        } else {
            gesture(
                "nce",
                [],
                "EMPTY, and CORRECTLY so: this server applies SIMPLE case mapping, so `İnce` \
                 lowercases to plain `ince`, the two splits coincide on it, and `nce` was never a \
                 projected token — not before the rewrite and not after. Pass 3 cannot return a \
                 chart by a token that does not exist, and neutrality holds here trivially. \
                 ⚠️ This row therefore does NOT guard the U+0130 defect on such a server; on a \
                 simple-case-mapping database the guard is \
                 `the_subset_probe_still_describes_the_query_db046_runs`, which pins the composed \
                 `lower(normalize(pn.value, NFC)) ~ …` as a literal and is locale-independent. \
                 That layering is the point: the gesture bites where the defect is real, the \
                 literal bites everywhere.",
            )
        },
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
            "sm",
            [],
            "EMPTY: two Latin bytes is below the prefix gate. `sm` is a REAL prefix of the seeded \
             tokens `smith` and `smith,`, which is what makes this row bite — relax the gate to \
             two bytes and three charts appear. (It read `mi` until #639's review: no seeded name \
             began `mi`, so the row it claimed to pin was green under exactly the widening it \
             named.) The gate must not loosen for Latin when it admits two CJK characters.",
        ),
        gesture(
            "",
            [],
            "EMPTY: a caller's naive split can hand pass 3 a blank token, and it must match \
             nothing. Note this is DEFENCE IN DEPTH, not the only thing standing: no empty token \
             is ever projected either, since the whole-token source filters `w <> ''` and the \
             parts source requires `length(p) > 1`. Any ONE of the three suffices, so this row \
             cannot go red alone — it goes red when the last of them is removed.",
        ),
        gesture(
            "zzz",
            [],
            "EMPTY: the floor case. A search that finds nothing must find nothing.",
        ),
        tokens(
            &["smith", "李小"],
            [k.plain, k.comma, k.cjk],
            "MULTI-TOKEN, which is what `SearchQuery::new` actually emits and what the rewritten \
             query-token join exists to serve. The tokens are OR-ed, so the answer is the union of \
             the two single-token answers — never their intersection, because a clerk typing two \
             fragments is widening the net, not narrowing it. Two tokens of very different byte \
             length in one call, since the #639 hoist is precisely about work done per (stored \
             token × query token) pair.",
        ),
        tokens(
            &["smith", "smith"],
            [k.plain, k.comma],
            "A REPEATED query token. `OFFSET 0` fences the normalisation subquery but must not \
             de-duplicate it — it is not a `DISTINCT` — so this produces two join rows per stored \
             token, and exactly the same two charts, once each. A fence that silently collapsed \
             duplicates would pass this; the multiplicity assertion in `name_candidates` is what \
             makes the row count part of the claim.",
        ),
    ]
}

/// Pass 3 answers every typing gesture with exactly the charts the contract names — no more.
///
/// The ONE test in this suite, deliberately: the corpus is expensive to seed (eight charts through
/// the real `submit_event` door) and every gesture reads the same seeded state, so splitting this
/// into one test per gesture would seed it once per gesture and assert nothing extra.
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
    let full_mapping = full_case_mapping(&c).await;
    for g in gestures(&corpus, full_mapping) {
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
/// them trivially true — the empty set equals the empty set. This asserts the eight charts are
/// distinct and present in `patient_name` before any of that reasoning is worth anything.
#[tokio::test]
async fn the_corpus_seeds_eight_distinct_charts() {
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
        k.turkish,
        k.john_doe,
    ]
    .into_iter()
    .collect();
    assert_eq!(ids.len(), 8, "the corpus must seed eight DISTINCT charts");

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
        named, 8,
        "every seeded chart must hold a name row for pass 3 to tokenise"
    );
}

/// Values a name can legitimately hold, chosen to stress the separator classes rather than the
/// clinical paths — scripts with combining marks, non-breaking and vertical whitespace, digits, and
/// the punctuated forms the parts branch exists for.
const SEPARATOR_PROBES: [&str; 14] = [
    "Anne A Smith",
    "李小明",
    "Ng",
    "  Wu  ",
    "Patient 2",
    "Иванов Иван",
    "สมชาย ใจดี",
    "Anne\u{00A0}Smith",
    "Anne\u{000B}Smith",
    "Jose\u{301} A\u{301}lvarez",
    "O'Brien-Smith",
    "Smith, John",
    "Müller Groß",
    // U+0130: the case-mapping probe. Added by #639's review, where a guard applied before
    // lowering let it through. It is the one value in this list that was ever actually wrong.
    "\u{130}nce",
];

/// The subset argument that makes #639's parts-branch skip neutral — **proved, not sampled.**
///
/// db/046 skips the alphanumeric-parts branch for a value whose lowered, NFC-normalised form matches
/// nothing outside `[[:alnum:][:space:]]`. The claim is that for such a value the parts split
/// (`[^[:alnum:]]+`) yields a SUBSET of the whole-token split (`\s+`), so the branch contributes
/// nothing and skipping it loses no chart.
///
/// **The claim reduces to two facts about single characters.** Let `S` be the string both splitters
/// are handed, `lower(normalize(value, NFC))`. If `S` matches nothing outside `[[:alnum:][:space:]]`
/// then every character of `S` is alnum or space, so:
///
///   * the parts split's separators are the characters of `S` that are NOT `[:alnum:]` — i.e. its
///     space characters, provided no character is both;
///   * the whole split's separators are the characters of `S` matching `\s`.
///
/// The two therefore coincide — and the parts branch additionally drops length-1 tokens and excludes
/// callsigns — **iff** (a) `\s` is exactly `[[:space:]]`, and (b) no character is both `[:space:]`
/// and `[:alnum:]`. Nothing else is load-bearing.
///
/// So this checks (a) and (b) over **every Unicode code point** rather than over a handful of
/// sampled names. A sample can only ever fail to find a counterexample; this enumerates the space in
/// which one could exist. It costs well under a second and it is the whole argument: a `0` in both
/// columns means no name in any script, present or future, can be a counterexample.
///
/// This matters because the documented facts it rests on are exactly the kind a locale, an ICU
/// version or a server upgrade is entitled to move underneath a deployment — and because a sampled
/// version of this test shipped green while `İnce` was broken.
#[tokio::test]
async fn the_subset_argument_holds_for_every_unicode_code_point() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // The surrogate range is excluded because `chr()` refuses it under a UTF-8 encoding — those
    // code points cannot appear in a `text` value at all, so they are not part of the space.
    let row = c
        .query_one(
            "WITH cps AS ( \
               SELECT chr(g) AS ch \
                 FROM generate_series(1, 1114111) g \
                WHERE g NOT BETWEEN 55296 AND 57343 \
             ) \
             SELECT \
               count(*) FILTER (WHERE (ch ~ '\\s') IS DISTINCT FROM (ch ~ '[[:space:]]')) \
                 AS space_class_divergences, \
               count(*) FILTER (WHERE ch ~ '[[:space:]]' AND ch ~ '[[:alnum:]]') \
                 AS both_space_and_alnum \
               FROM cps",
            &[],
        )
        .await
        .unwrap();
    let divergences: i64 = row.get(0);
    let overlap: i64 = row.get(1);

    assert_eq!(
        divergences, 0,
        "this server's `\\s` is NOT `[[:space:]]` for {divergences} code point(s), so db/046's two \
         separator classes no longer coincide on an unpunctuated value and skipping the parts \
         branch can drop a token. Re-derive the subset argument before shipping against it."
    );
    assert_eq!(
        overlap, 0,
        "{overlap} code point(s) on this server are BOTH `[:space:]` and `[:alnum:]`. Such a \
         character is a separator to the whole-token split and part of a token to the parts split, \
         so `ab<c>cd` splits to {{ab, cd}} in one and {{abccd}} in the other — a token the parts \
         branch alone projects, which db/046's skip would then drop."
    );
}

/// The same claim end to end, on real names — the belt to the proof's braces.
///
/// `the_subset_argument_holds_for_every_unicode_code_point` establishes the property from the
/// character classes up. This one runs the predicate and both splits, COMPOSED as db/046 composes
/// them, over values a name can legitimately hold — so a defect in the composition rather than in
/// any one class shows up as a concrete name that loses a token.
///
/// It holds a COPY of db/046's predicate, not the shipped one: the property is about the SQL the
/// splitter is handed, which no call to `cairn_search_candidates` can expose. Keeping the copy
/// honest is `the_subset_probe_still_describes_the_query_db046_runs`'s whole job — so if you change
/// the predicate here, that test is the one that will tell you db/046 disagrees.
#[tokio::test]
async fn skipping_the_parts_branch_can_never_drop_a_token() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let mut skipped = 0;
    for value in SEPARATOR_PROBES {
        let row = c
            .query_one(
                "SELECT lower(normalize($1, NFC)) ~ '[^[:alnum:][:space:]]' AS branch_runs, \
                 NOT EXISTS ( \
                   SELECT 1 \
                     FROM regexp_split_to_table(lower(normalize($1, NFC)), '[^[:alnum:]]+') p \
                    WHERE length(p) > 1 \
                      AND p NOT IN (SELECT w \
                                      FROM regexp_split_to_table( \
                                             lower(normalize($1, NFC)), '\\s+') w \
                                     WHERE w <> '') \
                 ) AS parts_subset_of_whole",
                &[&value],
            )
            .await
            .unwrap();
        let branch_runs: bool = row.get(0);
        let subset: bool = row.get(1);

        if !branch_runs {
            skipped += 1;
            assert!(
                subset,
                "db/046 skips the parts branch for {value:?} (its LOWERED NFC form holds nothing \
                 outside [[:alnum:][:space:]]), but the parts split projects a token the \
                 whole-token split does NOT — so the skip drops it and the chart stops being \
                 findable by it"
            );
        }
    }

    // Anti-vacuity: a probe set that no longer exercises the skipped path would pass this test
    // while proving nothing. TEN of the fourteen probes carry no punctuation once lowered (all but
    // `สมชาย ใจดี`, `O'Brien-Smith`, `Smith, John` and — only because of its combining dot —
    // `İnce`); the floor is set below that measured count so that trimming one probe fails loudly
    // here rather than quietly weakening the check.
    assert!(
        skipped >= 9,
        "only {skipped} probes exercised the SKIPPED path; the subset claim is untested below that"
    );
}

/// db/046, embedded so the test above cannot go on proving something about a query the shipped
/// function no longer runs.
const DB046: &str = include_str!("../../../db/046_patient_search.sql");

/// The four expressions the subset argument is about, exactly as db/046 must still spell them.
///
/// `skipping_the_parts_branch_can_never_drop_a_token` re-states db/046's predicate rather than
/// calling the function, because the property is about the SQL the splitter is handed, which no
/// call can expose. That is one invariant written twice — the shape this project distrusts — so
/// this guard ties the copy to the original: change either separator class, or the punctuation
/// test, and it fails and names the test that must be re-derived.
///
/// **The `lower(` in the first entry is the whole point of this guard, and it earned its place the
/// hard way.** The list originally pinned the un-lowered form, which is what the shipped predicate
/// read — while the splitter read the lowered one. The two agreed on every probe in the list, so
/// nothing was red, and `İnce` lost a token. Pinning the exact composed expression, rather than its
/// pieces, is what makes that class of drift a compile-time-ish failure instead of a silent one.
#[test]
fn the_subset_probe_still_describes_the_query_db046_runs() {
    for expression in [
        // the punctuation test that decides whether the parts branch runs at all — applied to the
        // LOWERED, normalised value, because that is the string the splitters below are handed
        "lower(normalize(pn.value, NFC)) ~ '[^[:alnum:][:space:]]'",
        // the string both splitters actually receive
        "lower(normalize(pn.value, NFC))",
        // the parts separator class
        "'[^[:alnum:]]+'",
        // the whole-token separator class
        r"'\s+'",
    ] {
        assert!(
            DB046.contains(expression),
            "db/046 no longer contains {expression:?}, so the subset argument in \
             `skipping_the_parts_branch_can_never_drop_a_token` is about a query that is no \
             longer run. Re-derive that argument against the new expression before editing this \
             list — the claim it protects is that skipping the parts branch drops no token, and \
             a dropped token is a chart that silently stops being findable."
        );
    }
}
