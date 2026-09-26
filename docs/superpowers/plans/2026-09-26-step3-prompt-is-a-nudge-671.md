# The step-3 prompt is a nudge (#671) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop signing display truncation as `incomplete`, show it as a quiet on-screen count instead, and rank the step-3 prompt by name tokens matched and a DOB near-miss so a wrong-DOB duplicate reaches the five rows.

**Architecture:** The ranking becomes a pure module in `cairn-patient-search` (`rank.rs`) that `cairn-node`'s `search_patients` feeds after its per-candidate reads; Postgres does the NFC/lower normalisation, Rust does tokenising and ordering. `cairn-gui-funnel`'s `bound_for_prompt` stops OR-ing truncation into `incomplete` and carries a `withheld` count; the window's `prompt_summary` sentence says it. No `db/` file, wire, or `SCHEMA_GENERATION` change.

**Tech Stack:** Rust (workspace + the separate `cairn-gui` tree), PostgreSQL 18 (DB-gated tests on `$CAIRN_TEST_PG`), Python 3 stdlib rig run through `uv`.

**Spec:** `docs/superpowers/specs/2026-09-26-step3-prompt-is-a-nudge-671-design.md` · ADR: `docs/spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md`

## Global Constraints

- No change to `db/045`, `db/046`, any `db/*.sql`, the wire shape, or `SCHEMA_GENERATION`.
- `PROMPT_CAP` stays `5`.
- Ranking only REORDERS: it never adds or removes a candidate.
- Ranking reads the RETAINED name set (`patient_name`, repudiated values included — #349), never `patient_name_current`.
- A partial-precision DOB (`YYYY`, `YYYY-MM`) is never a near-miss.
- `search.incomplete` is set only by the node's own partiality (ADR-0075 decision 3).
- No new dependency in any tree (so no lockfile moves).
- House rules: TDD, junior-legible doc comments on every non-trivial fn, pure fns, files under ~500 lines where feasible.
- `export CAIRN_ALLOW_DB_SKIP=1` for any DB-free cargo run (both trees). Never `| tail` a cargo command (masks the exit code).

## Review Focus

1. **A node-partial list that also truncates** — the node's reason must survive intact and `incomplete` stay `true`; the truncation count must not replace or be merged into it. (Task 3 test `a_partial_search_that_also_truncates_keeps_the_nodes_word`.)
2. **Calendar-impossible "swap"** — `1980-02-30` swapped is not a date; `parse_ymd` must reject both sides, never panic. (Task 1 test `an_impossible_date_is_never_a_near_miss`.)
3. **A candidate with NO retained name row** (a DOB-only or identifier-only match) — ranks with `tokens_matched = 0`, is not dropped. (Task 1 `a_candidate_with_no_stored_names_matches_no_tokens`; Task 2 keeps `unwrap_or_default`.)
4. **Duplicate query tokens** (`"John John"`, or whole+parts of a punctuated word) — counted once each distinct token. (Task 1 `repeated_query_tokens_count_once`.)
5. **A zero-candidate prompt** — `withheld` is 0 and the "No existing chart matched" sentence is unchanged. (Task 4 `an_empty_prompt_still_licenses_a_new_chart`.)

---

### Task 1: The pure ranking module in `cairn-patient-search`

**Files:**
- Modify: `crates/cairn-patient-search/src/query.rs` (extract the tokeniser)
- Modify: `crates/cairn-patient-search/src/candidate.rs:85-101` (extract the date parser)
- Create: `crates/cairn-patient-search/src/rank.rs`
- Modify: `crates/cairn-patient-search/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub fn cairn_patient_search::name_tokens(raw_name: &str) -> Vec<String>` (sorted, deduped — exactly what `SearchQuery::new` puts in `name_tokens`)
  - `pub(crate) fn candidate::parse_ymd(s: &str) -> Option<(i32, u32, u32)>`
  - `pub struct cairn_patient_search::RankKey { pub id: Uuid, pub passes: u32, pub tokens_matched: usize, pub dob_near_miss: bool }`
  - `pub fn tokens_matched(query_tokens: &[String], stored_names: &[String]) -> usize`
  - `pub fn is_dob_near_miss(query: &str, candidate: &str) -> bool`
  - `pub fn rank_candidates(keys: Vec<RankKey>) -> Vec<Uuid>`

- [ ] **Step 1: Extract the tokeniser and the date parser (pure refactor, existing tests cover it)**

In `query.rs`, move the body that builds `name_tokens` into a free function above `impl SearchQuery`, and call it from `new`. Move the explanatory "TWO KINDS OF TOKEN" doc onto the free function; `new`'s doc keeps one line pointing at it.

```rust
/// Split a typed (or stored) name into the tokens db/046 pass 3 blocks on — see the rule
/// below. Public because the ranking (`crate::rank`) must tokenise a STORED name by exactly
/// the rule the query was tokenised by; two tokenisers would be two answers to "did this
/// token match?".
///
/// (… the existing TWO KINDS OF TOKEN explanation, moved here verbatim …)
pub fn name_tokens(raw_name: &str) -> Vec<String> {
    let mut tokens: Vec<String> = raw_name
        .split_whitespace()
        .flat_map(|word| {
            let whole = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            let parts: Vec<String> = word
                .split(|c: char| !c.is_alphanumeric())
                .filter(|p| p.chars().count() > 1)
                .map(str::to_lowercase)
                .collect();
            std::iter::once(whole).chain(parts)
        })
        .filter(|t| !t.is_empty())
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}
```

In `new`: build the struct literal with `name_tokens: name_tokens(raw_name),` (no `let` of the same name — shadowing the fn reads badly), otherwise as before (keep the inline comments that explain each field's trimming).

In `candidate.rs`, lift the `ymd` closure out of `age_years` into:

```rust
/// Parse an ISO `YYYY-MM-DD` date into `(year, month, day)`, or `None` for a partial date,
/// anything unparseable, or a calendrically impossible day (`2026-02-30`). Shared by
/// `age_years` and the ranking's DOB near-miss (`crate::rank`), so the two can never disagree
/// about what counts as a real date.
pub(crate) fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let y = it.next()?.parse::<i32>().ok()?;
    let m = it.next()?.parse::<u32>().ok()?;
    let d = it.next()?.parse::<u32>().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) {
        return None;
    }
    // Real per-month validation (leap years included), not a blanket 1..=31.
    if !(1..=days_in_month(y, m)).contains(&d) {
        return None;
    }
    Some((y, m, d))
}
```

and replace `ymd(...)` calls in `age_years` with `parse_ymd(...)`.

Run: `cd crates/cairn-patient-search && cargo test` — Expected: all existing tests PASS (pure refactor).

- [ ] **Step 2: Write the failing tests for `rank.rs`**

Create `crates/cairn-patient-search/src/rank.rs` with only the module doc, the signatures returning `todo!()`, and this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn key(n: u128, passes: u32, tokens_matched: usize, dob_near_miss: bool) -> RankKey {
        RankKey { id: Uuid::from_u128(n), passes, tokens_matched, dob_near_miss }
    }

    #[test]
    fn a_two_token_match_counts_two() {
        assert_eq!(tokens_matched(&s(&["john", "smith"]), &s(&["john smith"])), 2);
        assert_eq!(tokens_matched(&s(&["john", "smith"]), &s(&["john brown"])), 1);
    }

    #[test]
    fn tokens_are_matched_across_every_retained_name() {
        // A chart known as "Jane Doe" and later "Jane Smith": both names are evidence.
        assert_eq!(
            tokens_matched(&s(&["jane", "smith"]), &s(&["jane doe", "jane smith"])),
            2
        );
    }

    #[test]
    fn repeated_query_tokens_count_once() {
        assert_eq!(tokens_matched(&s(&["john", "john"]), &s(&["john smith"])), 1);
    }

    #[test]
    fn a_candidate_with_no_stored_names_matches_no_tokens() {
        assert_eq!(tokens_matched(&s(&["john"]), &[]), 0);
    }

    #[test]
    fn stored_names_are_tokenised_by_the_query_rule() {
        // "o'brien-smith" stored whole must match the query's whole token AND its parts.
        let q = crate::query::name_tokens("O'Brien-Smith");
        assert_eq!(tokens_matched(&q, &s(&["o'brien-smith ann"])), q.len());
    }

    #[test]
    fn day_and_month_swapped_is_a_near_miss() {
        assert!(is_dob_near_miss("1980-03-07", "1980-07-03"));
    }

    #[test]
    fn a_year_off_by_one_is_a_near_miss_either_way() {
        assert!(is_dob_near_miss("1980-03-07", "1981-03-07"));
        assert!(is_dob_near_miss("1980-03-07", "1979-03-07"));
    }

    #[test]
    fn transposed_last_two_year_digits_is_a_near_miss() {
        assert!(is_dob_near_miss("1967-05-20", "1976-05-20"));
        assert!(!is_dob_near_miss("1967-05-20", "1977-05-20"));
    }

    #[test]
    fn an_exact_dob_is_not_a_near_miss() {
        // The exact match is already rewarded by db/046's DOB pass (`passes`).
        assert!(!is_dob_near_miss("1980-03-07", "1980-03-07"));
    }

    #[test]
    fn a_partial_date_is_never_a_near_miss() {
        assert!(!is_dob_near_miss("1980", "1981"));
        assert!(!is_dob_near_miss("1980-03", "1980-03-07"));
    }

    #[test]
    fn an_impossible_date_is_never_a_near_miss() {
        assert!(!is_dob_near_miss("1980-02-30", "1980-30-02"));
        assert!(!is_dob_near_miss("1980-13-01", "1980-01-13"));
    }

    #[test]
    fn an_unrelated_date_is_not_a_near_miss() {
        assert!(!is_dob_near_miss("1980-03-07", "1955-11-21"));
        assert!(!is_dob_near_miss("1980-03-07", "1982-03-07"), "two years is not a slip");
    }

    #[test]
    fn more_passes_rank_first() {
        let r = rank_candidates(vec![key(1, 1, 2, true), key(2, 2, 0, false)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn within_equal_passes_more_tokens_rank_first() {
        let r = rank_candidates(vec![key(1, 1, 1, false), key(2, 1, 2, false)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn within_equal_tokens_a_dob_near_miss_ranks_first() {
        let r = rank_candidates(vec![key(1, 1, 2, false), key(2, 1, 2, true)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn full_ties_keep_chart_age_order() {
        let r = rank_candidates(vec![key(20, 1, 1, false), key(10, 1, 1, false)]);
        assert_eq!(r, vec![Uuid::from_u128(10), Uuid::from_u128(20)]);
    }

    #[test]
    fn ranking_never_adds_or_drops_a_candidate() {
        let keys = vec![key(3, 1, 0, false), key(1, 2, 1, true), key(2, 1, 2, false)];
        let mut r = rank_candidates(keys);
        r.sort();
        assert_eq!(r, vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)]);
    }
}
```

Add `pub mod rank;` and `pub use rank::{is_dob_near_miss, rank_candidates, tokens_matched, RankKey}; pub use query::{name_tokens, SearchQuery};` to `lib.rs`.

Run: `cd crates/cairn-patient-search && cargo test rank` — Expected: FAIL (panics at `todo!()`).

- [ ] **Step 3: Implement `rank.rs`**

```rust
//! The ORDER of a candidate list — which five the step-3 prompt shows.
//!
//! # Why ranking matters, and why it is only ever an order
//!
//! `db/046` is a disjunction (any name token OR the exact DOB OR an identifier), so a
//! registration search returns ~100 candidates and the prompt shows five (ADR-0075). The
//! prompt is a best-effort nudge, not a completeness claim, so the one lever that helps
//! without asking the person at the desk for anything is putting the likeliest duplicate
//! first. This module only REORDERS: it never adds or drops a candidate, so the search's
//! set — and the drift invariant *sweep-paired ⊆ search-found* — is untouched.
//!
//! # The keys, strongest first
//!
//! 1. `passes` — how many of db/046's passes matched (identifier / DOB / name).
//! 2. `tokens_matched` — how many DISTINCT query name tokens the chart's retained names
//!    contain. db/046's name pass counts ONCE however many tokens matched, so without this
//!    key "John Brown" ties "John Smith" for a "John Smith" query.
//! 3. `dob_near_miss` — the chart's DOB is the query's with a typical slip (day/month
//!    swapped, year ±1, last two year digits transposed). This is what lifts a duplicate
//!    typed with a WRONG date of birth, which matches the name pass alone.
//! 4. `id` ascending — UUIDv7, so chart age: a stable, deterministic final tie-break.
//!
//! # Stated limit
//!
//! Keys 2 and 3 are computed here, in Rust, over names Postgres has normalised. They may
//! drift from db/046's own SQL expression. That can only worsen the ORDER, never lose a
//! candidate — which is why it is stated rather than pinned by a cross-language twin.
use crate::candidate::parse_ymd;
use crate::query::name_tokens;
use std::collections::HashSet;
use uuid::Uuid;

/// Everything the order is decided on, for one candidate. Built by the caller (the node's
/// `search_patients`) from its reads; nothing here touches a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankKey {
    pub id: Uuid,
    /// Distinct db/046 passes this chart matched (1..=3).
    pub passes: u32,
    /// See [`tokens_matched`].
    pub tokens_matched: usize,
    /// See [`is_dob_near_miss`].
    pub dob_near_miss: bool,
}

/// How many DISTINCT `query_tokens` appear among the tokens of ANY of `stored_names`.
///
/// `stored_names` are the chart's retained names, already lowercased and NFC-normalised by
/// Postgres; each is tokenised by [`name_tokens`] — the SAME rule the query was — so a
/// punctuated compound matches by its whole form and by its parts alike.
pub fn tokens_matched(query_tokens: &[String], stored_names: &[String]) -> usize {
    let stored: HashSet<String> = stored_names.iter().flat_map(|n| name_tokens(n)).collect();
    let distinct_query: HashSet<&String> = query_tokens.iter().collect();
    distinct_query.into_iter().filter(|t| stored.contains(*t)).count()
}

/// True when `candidate` is `query` with one of the commonest date-of-birth slips.
///
/// Both must be full, real ISO dates ([`parse_ymd`]); a partial-precision date is an honest
/// "only the year is known" (principle 4), not a slip, and never counts. An EXACT match is
/// not a near-miss — db/046's DOB pass already rewards it through `passes`.
pub fn is_dob_near_miss(query: &str, candidate: &str) -> bool {
    let (Some(q), Some(c)) = (parse_ymd(query), parse_ymd(candidate)) else {
        return false;
    };
    if q == c {
        return false;
    }
    let (qy, qm, qd) = q;
    let (cy, cm, cd) = c;
    let same_day_and_month = cm == qm && cd == qd;
    let day_month_swapped = cy == qy && cm == qd && cd == qm;
    let year_off_by_one = same_day_and_month && (cy - qy).abs() == 1;
    let year_digits_transposed = same_day_and_month && last_two_digits_transposed(qy, cy);
    day_month_swapped || year_off_by_one || year_digits_transposed
}

/// 1967 ↔ 1976: same century, the last two digits swapped. Different years only (a
/// palindromic pair like 1977 is excluded by the `a != b` check).
fn last_two_digits_transposed(a: i32, b: i32) -> bool {
    a != b && a / 100 == b / 100 && (a % 100) / 10 == b % 10 && a % 10 == (b % 100) / 10
}

/// Order candidates strongest-first by the four keys in the module doc; return their ids.
pub fn rank_candidates(mut keys: Vec<RankKey>) -> Vec<Uuid> {
    keys.sort_by(|a, b| {
        b.passes
            .cmp(&a.passes)
            .then(b.tokens_matched.cmp(&a.tokens_matched))
            .then(b.dob_near_miss.cmp(&a.dob_near_miss))
            .then(a.id.cmp(&b.id))
    });
    keys.into_iter().map(|k| k.id).collect()
}
```

Run: `cd crates/cairn-patient-search && cargo test` — Expected: all PASS. Then `cargo clippy -p cairn-patient-search --all-targets -- -D warnings` and `cargo fmt --all -- --check`.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-patient-search
git commit -m "feat(search): a pure ranking by passes, name tokens and DOB near-miss (#671)"
```

---

### Task 2: `search_patients` ranks by the new keys

**Files:**
- Modify: `crates/cairn-node/src/patient/search.rs` (`search_patients`, `read_candidate_ids`, delete `rank_by_passes_matched` + its two unit tests, add two reads)
- Test: `crates/cairn-node/tests/patient_search_ranking.rs`

**Interfaces:**
- Consumes: `cairn_patient_search::{RankKey, rank_candidates, tokens_matched, is_dob_near_miss}` (Task 1)
- Produces: `search_patients` returns candidates in the new order (signature unchanged).

- [ ] **Step 1: Write the failing DB-gated test**

Append to `crates/cairn-node/tests/patient_search_ranking.rs` (add a local helper above it; it is file-local, NOT in `tests/common`, so no helper registry changes):

```rust
/// Assert a day-precision date of birth for `patient` (file-local helper).
async fn assert_dob(
    c: &tokio_postgres::Client,
    sk: &ed25519_dalek::SigningKey,
    kid: &str,
    patient: uuid::Uuid,
    dob: &str,
    wall: i64,
) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body(dob, "day", None, "patient-stated"),
            plaintext_twin: Some(render_dob_twin(dob, "day", "patient-stated")),
            wall,
        },
    )
    .await
    .expect("dob accepted");
}

/// ADR-0075 / #671: the duplicate typed with a WRONG date of birth. It matches db/046's name
/// pass only, so passes alone tie it with every namesake; the two new keys must lift it —
/// both its name tokens match, and its stored DOB is the typed one with day and month swapped.
#[tokio::test]
async fn a_wrong_dob_duplicate_outranks_namesakes_by_tokens_and_near_miss() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Six OLDER one-token namesakes — more than the cap, so chart-age order would bury the dup.
    let mut older = Vec::new();
    for i in 0..6 {
        older.push(chart_named(&c, &sk, &kid, 10 * i, &format!("Smith Other{i}")).await);
    }
    // An older full namesake with an unrelated DOB: two tokens, no near-miss.
    let namesake = chart_named(&c, &sk, &kid, 80, "John Smith").await;
    assert_dob(&c, &sk, &kid, namesake, "1950-06-15", 82).await;
    // The duplicate, newest: stored 1980-02-01, typed 1980-01-02 (day/month swapped).
    let dup = chart_named(&c, &sk, &kid, 100, "John Smith").await;
    assert_dob(&c, &sk, &kid, dup, "1980-02-01", 102).await;

    let query = SearchQuery::new("John Smith", Some("1980-01-02"), &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-09-26")
        .await
        .expect("search succeeds");

    let ids: Vec<_> = list.candidates.iter().map(|c| c.patient_id).collect();
    assert_eq!(ids.len(), 8, "the SET is unchanged: {list:?}");
    assert_eq!(ids[0], dup, "two tokens AND a DOB near-miss come first: {list:?}");
    assert_eq!(ids[1], namesake, "two tokens beat one: {list:?}");
    assert_eq!(ids[2..].to_vec(), older, "one-token ties keep chart-age order");
}
```

(If `ed25519_dalek::SigningKey` is not the type `setup` returns, use the type named in `tests/common/mod.rs`'s `setup` signature.)

Run the DB-gated test (see memory "PG test substrate": `CAIRN_TEST_PG` must be set): `cargo test -p cairn-node --test patient_search_ranking` — Expected: the new test FAILS at `ids[0] == dup` (today's order puts the dup last among 1-pass ties).

- [ ] **Step 2: Implement the ranking in `search_patients`**

1. `read_candidate_ids` → rename `read_candidate_passes`, returning `Vec<(Uuid, u32)>` sorted by id (no ranking); keep its SQL and the comment about the per-pass collapse. Update its doc: it returns passes per chart; the ORDER is decided later by `cairn_patient_search::rank_candidates`.
2. Delete `rank_by_passes_matched` and its two unit tests (their intent now lives in `cairn-patient-search/src/rank.rs`'s tests). Move the useful history paragraph ("Why this exists (funnel UI slice 2c)") into the doc of the new ranking step below, updated to cite ADR-0075.
3. Add two reads:

```rust
/// Every RETAINED name of each candidate, lowercased and NFC-normalised BY POSTGRES — the
/// same normalisation db/046 applies before matching — for the ranking's token count.
///
/// Reads `patient_name`, NOT `patient_name_current`, for db/046's own reason (#349): a
/// repudiated alias is exactly how a fabricated persona's chart is FOUND, so it must also
/// count toward how strongly it matched. A candidate with no row simply gets no entry
/// (the caller treats that as zero tokens matched — never as a reason to drop it).
async fn read_retained_names<C: GenericClient + Sync>(
    client: &C,
    ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, Vec<String>>> {
    let id_strs: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let sql = "SELECT patient_id::text AS patient_id, lower(normalize(value, NFC)) AS value \
               FROM patient_name \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.entry(id).or_default().push(row.get::<_, String>("value"));
    }
    Ok(out)
}

/// The query's name tokens, lowercased and NFC-normalised by Postgres, so both sides of the
/// token comparison went through the same server-side normalisation.
async fn normalise_query_tokens<C: GenericClient + Sync>(
    client: &C,
    tokens: &[String],
) -> anyhow::Result<Vec<String>> {
    let row = client
        .query_one(
            "SELECT coalesce(array_agg(lower(normalize(t, NFC))), '{}') \
             FROM unnest($1::text[]) AS t",
            &[&tokens],
        )
        .await?;
    Ok(row.get::<_, Vec<String>>(0))
}
```

4. In `search_patients`, after `let dobs = read_dob(...)`, add:

```rust
    // ADR-0075: rank before assembling, so `candidates` is built in display order. Ranking
    // only reorders `ids` — every id read above is still here afterwards.
    let retained = read_retained_names(client, &ids).await?;
    let query_tokens = normalise_query_tokens(client, &query.name_tokens).await?;
    let keys: Vec<RankKey> = passes
        .iter()
        .map(|(id, n)| RankKey {
            id: *id,
            passes: *n,
            tokens_matched: tokens_matched(
                &query_tokens,
                retained.get(id).map(Vec::as_slice).unwrap_or_default(),
            ),
            dob_near_miss: match (query.birth_date.as_deref(), dobs.get(id)) {
                (Some(q), Some((stored, _basis))) => is_dob_near_miss(q, stored),
                _ => false,
            },
        })
        .collect();
    let ids = rank_candidates(keys);
```

where `passes` is the `Vec<(Uuid, u32)>` from `read_candidate_passes`, and the earlier `ids` (used by the reads) is `passes.iter().map(|(id, _)| *id).collect::<Vec<_>>()`. Keep the reads that follow (`trust_states`, …) using either `ids`; the `candidates` map must iterate the RANKED `ids`.

5. Update the module-level doc's list of reads (now nine projections-worth of queries) in one sentence.

Run: `cargo test -p cairn-node --test patient_search_ranking` — Expected: BOTH tests PASS. Then the rest of the search suites, which pin the SET and must be unaffected:
`cargo test -p cairn-node --test patient_search --test patient_search_drift --test patient_search_equivalence --test search_path_pg_temp`
Expected: PASS. (If a test asserted exact ORDER among single-pass ties with differing token counts, it pinned the old limit; update it only if it is asserting what ADR-0075 changed, and say so in the commit.)

- [ ] **Step 3: Clippy, fmt, doc**

`cargo clippy -p cairn-node --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `RUSTDOCFLAGS="-D warnings" cargo doc -p cairn-node -p cairn-patient-search --no-deps` (an intra-doc link to a now-deleted `rank_by_passes_matched` fails here — fix every reference: `grep -rn rank_by_passes_matched crates cairn-gui scripts docs` and update them to `rank_candidates`).

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node crates/cairn-patient-search scripts
git commit -m "feat(search): rank by name tokens matched and DOB near-miss (#671, ADR-0075)"
```

---

### Task 3: `bound_for_prompt` stops signing truncation as `incomplete`

**Files:**
- Modify: `cairn-gui/cairn-gui-funnel/src/prompt.rs` (module doc, `PromptList`, `bound_to`, delete `withheld_reason` + `combine_reasons`, tests)
- Modify: `cairn-gui/cairn-gui-live/tests/attestation_through_the_port.rs:241-260`
- Modify: `cairn-gui/cairn-gui-live/tests/common/mod.rs:197-212` (the `stored_incomplete` doc)

**Interfaces:**
- Produces: `PromptList::withheld(&self) -> usize` (count the node returned but the prompt did not show); `PromptList::as_list()` unchanged. `PromptList.as_list().incomplete` is now exactly the node's `incomplete`.

- [ ] **Step 1: Rewrite the tests in `prompt.rs` first**

Replace the truncation tests with (keep `the_prompt_cap_is_five`, `a_list_that_fits_comes_back_untouched`, `a_longer_list_keeps_the_first_cap_candidates_in_display_order`, `a_bare_partiality_flag_from_the_node_keeps_its_own_attribution`, `a_partial_search_that_fits_inside_the_cap_stays_partial`, `an_empty_search_result_is_complete_not_truncated`; delete `the_withheld_sentence_names_the_count…` and `combining_reasons_never_drops_one_of_them`):

```rust
    /// ADR-0075 decision 3: cutting the list to the cap is NOT an incompleteness of the
    /// search. It is counted (for the on-screen line) and never signed.
    #[test]
    fn truncation_is_counted_not_signed_as_incomplete() {
        let bounded = bound_to(&list_of(8, None), 5);
        assert!(!bounded.as_list().incomplete, "truncation is the prompt's normal state");
        assert_eq!(bounded.as_list().incomplete_reason, None);
        assert_eq!(bounded.withheld(), 3);
    }

    #[test]
    fn a_list_one_longer_than_the_cap_withholds_exactly_one() {
        let bounded = bound_to(&list_of(6, None), 5);
        assert_eq!(bounded.withheld(), 1);
        assert_eq!(bounded.as_list().candidates.len(), 5);
    }

    #[test]
    fn a_list_exactly_the_size_of_the_cap_withholds_nothing() {
        let bounded = bound_to(&list_of(5, None), 5);
        assert_eq!(bounded.withheld(), 0);
        assert!(!bounded.as_list().incomplete);
    }

    /// Review focus 1: the node's partiality survives truncation untouched — same flag, same
    /// words, nothing appended.
    #[test]
    fn a_partial_search_that_also_truncates_keeps_the_nodes_word() {
        let bounded = bound_to(&list_of(8, Some("2 candidates could not be read")), 5);
        assert!(bounded.as_list().incomplete);
        assert_eq!(
            bounded.as_list().incomplete_reason.as_deref(),
            Some("2 candidates could not be read")
        );
        assert_eq!(bounded.withheld(), 3);
    }

    #[test]
    fn a_cap_of_zero_shows_nobody_and_counts_everyone_withheld() {
        let bounded = bound_to(&list_of(4, None), 0);
        assert!(bounded.as_list().candidates.is_empty());
        assert_eq!(bounded.withheld(), 4);
        assert!(!bounded.as_list().incomplete);
    }
```

Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-funnel prompt` — Expected: FAIL to compile (`withheld` does not exist).

- [ ] **Step 2: Implement**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptList {
    list: CandidateList,
    /// How many candidates the node returned that this prompt did not show. Shown on
    /// screen (ADR-0075 decision 4), never signed.
    withheld: usize,
}

impl PromptList {
    pub fn as_list(&self) -> &CandidateList {
        &self.list
    }
    /// See the field. Read-only, like `as_list`.
    pub fn withheld(&self) -> usize {
        self.withheld
    }
}

fn bound_to(list: &CandidateList, cap: usize) -> PromptList {
    PromptList {
        list: CandidateList {
            candidates: list.candidates.iter().take(cap).cloned().collect(),
            // ONLY the node's own partiality (ADR-0075 decision 3, restoring ADR-0061's
            // meaning): the search could not read some candidate. Truncation is `withheld`.
            incomplete: list.incomplete,
            // `node_reason` so a bare node flag still arrives with words.
            incomplete_reason: node_reason(list).map(str::to_string),
        },
        withheld: list.candidates.len().saturating_sub(cap),
    }
}
```

Keep the existing doc on `PromptList` (why it is a type) — update its sentences that say the private field is a `CandidateList` tuple. Rewrite the module doc section **"What `incomplete` must never become"** to: two partialities still exist and are still kept apart, but now in two fields — `incomplete` (the search, signed) and `withheld` (the display, shown, never signed) — citing ADR-0075. Update `PROMPT_CAP`'s doc: replace the "If it turns out the prompt is *routinely* truncating, the cap is wrong…" paragraph with: *"Measured 2026-09-23: it routinely truncates (92% of registrations), and ADR-0075 decided that is the prompt's normal state — it is a nudge, not a completeness claim. So five is now a layout choice, not a completeness boundary; do not raise it to make a number look better."* Fix every `PromptList(...)` tuple construction the compiler reports (there are none outside `bound_to` by design).

Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-funnel` — Expected: PASS.

- [ ] **Step 3: Invert the live attestation assertion**

In `attestation_through_the_port.rs`, replace the final `assert!(common::stored_incomplete(...))` block and its comment with:

```rust
    // ADR-0075 (#671): the prompt was CUT, but the SEARCH was whole — so the signed body must
    // say `incomplete: false`. Truncation is shown on screen, never signed. The raw-list
    // bypass the old assertion guarded against is still caught above: a port forwarding the
    // node's raw list would store all `PROMPT_CAP + 3` ids, and `stored.len() == PROMPT_CAP`
    // fails on it.
    assert!(
        !common::stored_incomplete(&reader, created).await,
        "truncation is not an incompleteness of the search (ADR-0075 decision 3)"
    );
```

Rewrite `common::stored_incomplete`'s doc to say what it now proves (the flag carries only the node's partiality; a truncating prompt must store `false`). Search the tree for other tests asserting truncation ⇒ incomplete: `grep -rn "incomplete" cairn-gui --include=*.rs | grep -v "src/prompt.rs"` and update any that encode the old rule.

Run: `cd cairn-gui && CAIRN_TEST_PG=… cargo test -p cairn-gui-live --test attestation_through_the_port` — Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add cairn-gui/cairn-gui-funnel cairn-gui/cairn-gui-live
git commit -m "feat(funnel): truncation is counted, never signed as incomplete (#671, ADR-0075)"
```

---

### Task 4: The window says how many were not shown

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/funnel/view.rs:174-202` (`prompt_summary`) + its tests
- Modify: `cairn-gui/cairn-gui-tauri/src/funnel/commands.rs:199-211` (call site) and the test at ~line 715 (`the_prompt_is_bounded_and_the_registration_attests_exactly_its_rows`)

**Interfaces:**
- Consumes: `PromptList::withheld()` (Task 3)
- Produces: `pub fn prompt_summary(shown: usize, withheld: usize, incomplete: bool) -> String`

- [ ] **Step 1: Failing tests in `view.rs`**

Update existing calls to the 3-arg form (`prompt_summary(3, 0, false)`, `prompt_summary(0, 0, false)`, `prompt_summary(0, 0, true)`, `prompt_summary(2, 0, true)`), and add:

```rust
    /// ADR-0075 decision 4: truncation is said quietly, as a count and a way to narrow —
    /// not as "incomplete", which is reserved for a search that did not finish.
    #[test]
    fn a_cut_prompt_says_how_many_closest_of_how_many() {
        let s = prompt_summary(5, 98, false);
        assert!(s.contains("5 closest of 103"), "{s}");
        assert!(s.contains("type more to narrow"), "{s}");
        assert!(!s.contains("not complete"), "a cut list is not a partial search: {s}");
        assert!(s.contains("none of these"), "{s}");
    }

    #[test]
    fn a_cut_prompt_over_a_partial_search_says_both() {
        let s = prompt_summary(5, 3, true);
        assert!(s.contains("5 closest of 8"), "{s}");
        assert!(s.contains("not complete"), "{s}");
    }

    /// Review focus 5.
    #[test]
    fn an_empty_prompt_still_licenses_a_new_chart() {
        assert!(prompt_summary(0, 0, false).contains("No existing chart matched"));
    }
```

Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri view` — Expected: FAIL to compile.

- [ ] **Step 2: Implement**

```rust
/// … (keep the existing doc) …
///
/// `withheld` is how many further candidates matched but are not shown (ADR-0075): said as
/// "the N closest of M", a count and a way to narrow, never as "incomplete" — that word is
/// reserved for a search that did not finish (`incomplete`), because it changes whether
/// "no match" can be trusted, and truncation does not.
pub fn prompt_summary(shown: usize, withheld: usize, incomplete: bool) -> String {
    let partial = if incomplete {
        ", but the list is not complete (the reason follows)"
    } else {
        ""
    };
    match shown {
        0 if incomplete => "The search did not finish, and showed nobody — this is NOT a \
                            \"no match\" (the reason follows). Registering now records that \
                            incomplete search."
            .to_string(),
        0 => "No existing chart matched what is typed. Registering will record that search."
            .to_string(),
        n if withheld > 0 => format!(
            "{n} existing patient(s) might be this person — the {n} closest of {} matches, \
             listed below{partial}; type more to narrow. Pressing Register now means none of \
             these.",
            n + withheld
        ),
        n => format!(
            "{n} existing patient(s) might be this person — listed below{partial}. Pressing \
             Register now means none of these."
        ),
    }
}
```

In `commands.rs`: `summary: Some(prompt_summary(bounded.candidates.len(), prompt.withheld(), bounded.incomplete)),` — read `prompt.withheld()` into a local BEFORE `record(&form, prompt)` moves `prompt`. In `the_prompt_is_bounded_and_the_registration_attests_exactly_its_rows`, replace the `incomplete_reason.is_some()` assertion with:

```rust
        assert!(p.incomplete_reason.is_none(), "a cut prompt is not a partial search: {p:?}");
        assert!(
            p.summary.as_deref().unwrap_or("").contains("closest of"),
            "a cut prompt must say it was cut: {p:?}"
        );
```

and update that test's doc sentence ("says it is partial" → "says how many it did not show").

Run: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri` — Expected: PASS. Then the full cairn-gui gate (memory "Three Cargo trees"): `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --all -- --check`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — all from `cairn-gui/`.

- [ ] **Step 3: Commit**

```bash
git add cairn-gui/cairn-gui-tauri
git commit -m "feat(window): the prompt says 'the 5 closest of N', not 'incomplete' (#671)"
```

---

### Task 5: The rig measures the new ranking, and a name-typo arm

**Files:**
- Modify: `scripts/measure_prompt_truncation.py`
- Create: `cairn-gui/cairn-gui-tauri/results/2026-09-26-funnel-prompt-ranking.md`

**Interfaces:**
- Consumes: the Rust rule of Tasks 1–2 (twinned in Python).

- [ ] **Step 1: Self-tests first**

Add to `self_test()`:

```python
    # Twin of cairn_patient_search::rank (Task 1's Rust examples).
    assert tokens_matched(["john", "smith"], ["john smith"]) == 2
    assert tokens_matched(["john", "john"], ["john smith"]) == 1
    assert tokens_matched(["john"], []) == 0
    assert is_dob_near_miss("1980-03-07", "1980-07-03")
    assert is_dob_near_miss("1980-03-07", "1979-03-07")
    assert is_dob_near_miss("1967-05-20", "1976-05-20")
    assert not is_dob_near_miss("1980-03-07", "1980-03-07")
    assert not is_dob_near_miss("1980", "1981")
    assert not is_dob_near_miss("1980-02-30", "1980-30-02")
    assert rank([("1", 1, 2, True), ("2", 2, 0, False)]) == ["2", "1"]
    assert rank([("1", 1, 1, False), ("2", 1, 2, False)]) == ["2", "1"]
    assert rank([("1", 1, 2, False), ("2", 1, 2, True)]) == ["2", "1"]
    assert rank([("20", 1, 1, False), ("10", 1, 1, False)]) == ["10", "20"]
    # The name-typo arm: exactly one character of the LAST word changes, deterministically.
    t = perturb_name("John Smith", random.Random(1))
    assert t.split()[0] == "John" and t != "John Smith" and len(t) == len("John Smith"), t
```

Run: `uv run --no-project python scripts/measure_prompt_truncation.py --self-test` — Expected: FAIL (NameError).

- [ ] **Step 2: Implement the twins and the arm**

- `tokens_matched(query_tokens, stored_names)`: distinct query tokens found in the union of `query_tokens(unicodedata.normalize("NFC", n).lower())` over `stored_names`.
- `parse_ymd(s)` → `(y, m, d)` or `None` (full date, month 1–12, real day incl. leap years via `calendar.monthrange`), and `is_dob_near_miss(q, c)` exactly as the Rust.
- `rank(rows)` now takes `(id, passes, tokens, near)` tuples: `sorted(rows, key=lambda r: (-r[1], -r[2], not r[3], r[0]))`. Docstring: the Python twin of `rank_candidates`.
- `perturb_name(name, rng)`: replace one character at a random index `1..len(last)-1` of the last word with a different lowercase ASCII letter (preserving length); docstring says it models the commonest real duplicate (maintainer: typos in hard-to-spell names) and that `db/046` cannot find it by construction.
- `--perturb` choices become `["none", "dob", "name"]`.
- In `main`: keep an `id -> (name, dob)` dict of the population; for each result row compute `(pid, passes, tokens_matched(query_tokens(queried_name), [name_of[pid]]), is_dob_near_miss(queried_dob, dob_of[pid]))`.
- The `missing` check: for `--perturb name` a search not finding its own chart is a RESULT, not a rig error — count it as `self_not_in_candidate_set` in the summary; for the other arms keep the `SystemExit`.
- `summarise` reports `self_in_cap_ranked` with the new `rank`, plus `self_in_cap_passes_only` (the 2c rank: passes then id) so the file shows before/after in one run.

Run: `--self-test` — Expected: `self-test: ok`.

- [ ] **Step 3: Measure all three arms** (DB `cairn_test` on :5532, real-name pool)

```bash
for arm in none dob name; do
  uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
    --samples 500 --perturb $arm --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3
done
```

Do NOT run while a `cargo test` suite is using `cairn_test` (the rig refuses to report if the population changes mid-run).

Then the latency check the §1.2 section owes (Task 2 added two reads to the search path):
`uv run --no-project python scripts/measure_patient_search.py --help` for its flags, run its default
gesture set once on this branch and once on `main` (`git stash` is NOT needed — build each in its own
worktree or run `main`'s numbers from the most recent results file under `cairn-gui/cairn-gui-tauri/results/`
or `docs/`), and record the delta in the result file. A visible regression (> ~50 ms p50 on the Mac) is a
finding to report, not to hide.

- [ ] **Step 4: Write the result file** `cairn-gui/cairn-gui-tauri/results/2026-09-26-funnel-prompt-ranking.md` in the 2026-09-23 file's format: rig table, one table per arm (passes-only vs new ranking, self in cap, median position), what it means, reproduce block. State plainly that the name arm is the case only the §5.2 matcher and the repair path can catch. Add a one-line forward pointer at the top of the 2026-09-23 file: *"Superseded in part by 2026-09-26-funnel-prompt-ranking.md (ADR-0075)."*

- [ ] **Step 5: Commit**

```bash
git add scripts/measure_prompt_truncation.py cairn-gui/cairn-gui-tauri/results
git commit -m "measure(funnel): the new ranking on all three arms, and the name-typo case (#671)"
```

---

### Task 6: Issues, docs, PR

- [ ] File the repair-path issues (ADR-0075 decision 2), each citing ADR-0075 and the name-arm figure: (a) commit-time local duplicate check by the §5.2 matcher, (b) the duplicate worklist in the reference UI, (c) the link gesture. Check wording with `scripts/check_closing_keywords.py` before committing any message that names them.
- [ ] Update HANDOVER (⇒ NEXT: #671 done → the repair-path brainstorm; the durable rule about `rank_by_passes_matched` becomes `rank_candidates` with the four keys; the "Do not change `PROMPT_CAP`" note stays) and ROADMAP (one condensed entry). Prune both toward 500 lines, verifying no issue number is lost.
- [ ] Full gates in CI's order AFTER the final edit (memory: a gate is evidence only for the tree it ran against): root `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, targeted DB-gated suites; the cairn-gui tree's gate; `mkdocs build` via the pinned requirements; `cargo test -p cairn-node --test paper_parity_plan_section`.
- [ ] Push; mark PR #678 ready with the description of what changed; body says `Closes #671` only once #671 is fully resolved (it is, by this plan).

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the clerk glancing at the alphabetical card index before writing a new card — the index drawer shows whatever cards sit near the name; nobody reads every card in the drawer.

**Steps:** paper acts N = 1 (glance at the neighbouring cards). Architecture-forced M = 1 (read the five rows the prompt shows — unchanged by this slice). UI bundling target K = 1. `M ≤ N`; no architecture defect. This slice adds no act: the ranking is invisible, and the on-screen count replaces a sentence rather than adding one.

**Time + cognitive load:** no new time budget — the slice-2c budgets stand (find ≤ 5 s, register ≤ 20 s; stopwatch still owed as a human act). Cognitive load goes DOWN: the prompt no longer announces "the list is not complete" on 92% of registrations (a warning that is nearly always on trains the reader to ignore it), and the likeliest duplicate is more often in the first rows. The ranking's extra cost is two small reads over ≤ ~1000 ids on the search path; Task 2 must confirm it adds no visible latency by re-running `scripts/measure_patient_search.py`'s default gesture set once and noting the delta in the result file.
