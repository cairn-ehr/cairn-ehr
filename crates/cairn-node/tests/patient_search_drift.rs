//! #636 — the one-directional invariant db/046's DRIFT NOTE protects.
//!
//! The sweep and the search share key EXTRACTION but not their queries: the sweep blocks
//! all-by-all, the search maps one query to a set. The property that must hold is
//! **sweep-paired ⊆ search-found** — any two charts the background duplicate sweep would put in
//! one block must both be reachable by a search for the key that blocked them.
//!
//! Slice 1 widened search and left the matcher alone, which preserves this. The test exists
//! because the NEXT change might not, and prose cannot fail CI.
mod common;

use cairn_node::db;
use common::{chart_named, cs, setup};
use uuid::Uuid;

/// The projections this suite writes, beyond `common::setup`'s default clinical core
/// (`patient_chart`, `patient_identifier`, `patient_demographic` are already truncated
/// there). `chart_named` (promoted to `common` in Task 2 for exactly this reuse) authors a
/// registration and a legal-name assertion per fixture, so both projections need
/// truncating between runs.
const EXTRA_TABLES: [&str; 2] = ["patient_registration", "patient_name"];

/// Call `cairn_search_candidates` and read back `(patient_id, matched_pass)` for every row.
///
/// Copied from `patient_search.rs` rather than shared: each `tests/*.rs` file compiles as
/// its own binary, so a private helper there is not reachable from here — only
/// `common/mod.rs` items cross that boundary, and this helper (unlike `chart_named`) was
/// never promoted there. It is boilerplate specific to reading this one function back, not
/// the seeding-logic duplication Task 2 fixed.
///
/// UUID / jsonb BINDING: `cairn-node` does not enable tokio-postgres's `with-uuid-1` or
/// `with-serde_json-1` features, so the patient id is read back as `::text` and the
/// identifiers argument is bound as a `text` literal cast with `$3::text::jsonb`
/// (never a bare `$3::jsonb`, which silently no-ops on an untyped parameter and would
/// false-green this helper).
async fn search_candidates(
    c: &tokio_postgres::Client,
    name_tokens: Option<&[&str]>,
    birth_date: Option<&str>,
    identifiers_json: Option<&str>,
) -> Vec<(Uuid, String)> {
    let tokens: Option<Vec<String>> =
        name_tokens.map(|ts| ts.iter().map(|t| t.to_string()).collect());
    let rows = c
        .query(
            "SELECT patient_id::text, matched_pass \
             FROM cairn_search_candidates($1, $2, $3::text::jsonb)",
            &[&tokens, &birth_date, &identifiers_json],
        )
        .await
        .unwrap();
    rows.iter()
        .map(|r| {
            let id: String = r.get(0);
            let pass: String = r.get(1);
            (
                Uuid::parse_str(&id).expect("patient_id is a valid uuid"),
                pass,
            )
        })
        .collect()
}

#[tokio::test]
async fn every_sweep_block_key_is_still_reachable_by_search() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // One shape per way the two sides tokenise differently: plain, short, multi-word,
    // hyphenated, apostrophised, long compound, and an accented multi-word name.
    let fixtures = [
        "Smith",
        "Wu",
        "Ng Wei",
        "Li-Wong",
        "O'Brien-Smith",
        "Fyodorowksi-Eschenbacher",
        "Samantha Michaelowski",
        "García Pérez",
    ];

    let mut seeded: Vec<(&str, Uuid)> = Vec::new();
    for (i, name) in fixtures.iter().enumerate() {
        seeded.push((
            name,
            chart_named(&c, &sk, &kid, (i as i64) * 10, name).await,
        ));
    }

    for (name, id) in &seeded {
        // The sweep's blocking keys for this stored name, extracted exactly as
        // matcher/pipeline/db.py's _GROUPS_SQL does: whitespace split of the NFC-normalised,
        // lower-cased value. Asked of the SERVER — via `lower`/`normalize` — rather than
        // computed in Rust so that at least the NORMALISATION SEMANTICS cannot drift from
        // the matcher's (both sides ask Postgres, not two independent implementations of
        // Unicode case-folding/NFC). This does NOT cover the EXPRESSION itself: the
        // `regexp_split_to_table(lower(normalize($1, NFC)), '\s+')` below is still a
        // hand-copy of `_NAME_TOKENS_CTE`, same as db/046's own comment — see the note on
        // the assertion message below for what that leaves uncovered.
        let keys: Vec<String> = c
            .query(
                "SELECT tok FROM regexp_split_to_table(lower(normalize($1, NFC)), '\\s+') AS tok \
                 WHERE tok <> ''",
                &[name],
            )
            .await
            .unwrap()
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect();

        assert!(
            !keys.is_empty(),
            "fixture {name:?} produced no blocking key — this test would be vacuous for it"
        );

        for k in keys {
            let rows = search_candidates(&c, Some(&[k.as_str()]), None, None).await;
            assert!(
                rows.iter().any(|(found, _)| found == id),
                "the sweep would block {name:?} on key {k:?}, but a search for {k:?} does not \
                 find it — sweep-paired is no longer a subset of search-found (db/046 DRIFT \
                 NOTE), as things stand today. This proves search has not been narrowed away \
                 from THIS test's key extraction — it does NOT prove the matcher was not \
                 WIDENED instead: the `keys` computed above are a hand-copy of \
                 matcher/pipeline/db.py's `_NAME_TOKENS_CTE`, not read from it, so if a future \
                 change widens what the matcher extracts, this file's copy does not move, every \
                 assertion here keeps passing, and the DRIFT NOTE hazard fires silently. Do not \
                 over-trust a green run of this test on that half of the hazard."
            );
        }
    }
}
