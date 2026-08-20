//! #382 — the floor's `REVOKE EXECUTE … FROM PUBLIC` convention, made checkable.
//!
//! # What the convention is
//!
//! Two families of function in `db/*.sql` are meant to be unreachable by `PUBLIC`:
//!
//! * **`cairn_check_*`** — the per-event-type structural validators the `submit_event` /
//!   `apply_remote_event` doors dispatch through. These are pure `jsonb` shape checks: they
//!   read no table, write nothing and grant nothing, so the severity of a `PUBLIC` caller
//!   invoking one is genuinely low — it learns strictly less than the door already tells it
//!   by refusing, and the refusal messages are deliberately legible.
//! * **`*_apply`** — the projection appliers the `event_log` triggers call. These are the
//!   load-bearing half: each one WRITES a projection table, so a runtime role able to call
//!   one directly could forge projection state that no event supports — the projections
//!   would then disagree with the append-only log they are supposed to be derived from.
//!
//! Both are revoked, for different reasons. Stating that difference here rather than
//! flattening it is the point: a reader who thinks the two are equally severe will
//! eventually "simplify" one of them away.
//!
//! # Why the guard exists at all, given one half is low-severity
//!
//! Before this file, 17 of the 22 `cairn_check_*` functions carried no `REVOKE` and five did.
//! That is worse than either extreme. A reader cannot tell whether a missing `REVOKE` is
//! deliberate or an oversight, so the signal is unusable — and on the §9 safety-critical
//! surface, "looks inconsistent, probably fine" is the wrong resting state. The value of
//! closing it is not the privilege removed; it is that the convention becomes **checkable**,
//! so the next omission is a failing test rather than a thing someone might notice.
//!
//! # Why it is written over the catalogue, not over `db/*.sql`
//!
//! A guard that greps the migration text proves what someone typed. This one proves what the
//! database ended up with, which is the only thing an attacker meets — and it covers whatever
//! a future migration adds, the moment it loads. Grepping would also miss the case that makes
//! ACLs subtle: `CREATE OR REPLACE FUNCTION` **preserves** the existing ACL, so a function
//! first defined (and revoked) in one migration and replaced in a later one stays revoked
//! with no `REVOKE` in the later file at all. Two of these functions are exactly that shape.
//!
//! # What this guard does NOT claim
//!
//! It says nothing about which non-`PUBLIC` roles hold `EXECUTE`. `cairn_node` and
//! `cairn_agent` reach these functions the only way they should: from *inside* the
//! `SECURITY DEFINER` doors, which run as the schema owner. Nothing here would catch an
//! explicit `GRANT EXECUTE … TO cairn_agent` on an applier — that would need its own
//! assertion, and no migration does it today.
//!
//! # Why there is no `db/tests/` SQL mirror
//!
//! For the reason `search_path_pg_temp.rs` gives: the floors below are COUNTS, and this repo
//! has been bitten repeatedly by counts pinned in two places that drift apart (#182, #189).
//! One home, exercised in the gate that runs the whole workspace, beats two that disagree.
//!
//! Every test here self-skips without `$CAIRN_TEST_PG` (see `common::cs`), and a skipped run
//! prints `ok` while proving nothing. CI sets it; `scripts/run-db-gated-tests.sh` bakes it in.
mod common;
use common::{cs, NOT_EXTENSION_OWNED, REPO_SCHEMAS};

/// Repo-defined functions whose name matches `pattern`, each with whether `PUBLIC` can
/// execute it.
///
/// **The ACL, read from `proacl` rather than `has_function_privilege`.** A NULL `proacl`
/// means "nobody has touched the grants", and PostgreSQL's default for a function is
/// `EXECUTE` to `PUBLIC` — so NULL is the *permissive* case, not the empty one. Getting that
/// backwards would invert the whole guard, which is why it is spelled out in SQL here rather
/// than left to a helper whose polarity a reader has to recall. Otherwise the ACL is exploded
/// and grantee `0` — the `PUBLIC` pseudo-role, which has no `pg_authid` row — is looked for
/// directly.
///
/// `pattern` is matched with `\\_` escaping the underscores: unescaped, `_` is LIKE's
/// single-character wildcard, so `cairn_check_%` would also match a hypothetical
/// `cairnXcheckY`. Harmless today, wrong tomorrow.
fn public_execute(pattern: &str) -> String {
    format!(
        "SELECT p.oid::regprocedure::text,
                (p.proacl IS NULL
                 OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a
                             WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'))
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
          WHERE {REPO_SCHEMAS}
            AND p.proname LIKE '{pattern}'
            AND {NOT_EXTENSION_OWNED}
          ORDER BY 1"
    )
}

/// What the loaded schema holds today (2026-08-20), as floors rather than equalities.
///
/// `>=` against today's exact count, in the shape `search_path_pg_temp.rs` established: the
/// count only ever RISES as migrations add functions, so this never needs revising upward —
/// but it trips the moment coverage *shrinks*, which is what would happen if a rename or a
/// failed migration load quietly emptied the query. Without it, "no offenders" and "no rows
/// examined" are the same green.
const CHECK_FNS_TODAY: usize = 22;
const APPLY_FNS_TODAY: usize = 20;

/// Run one family's assertion: at least `floor` functions matched, and none of them is
/// `PUBLIC`-executable.
///
/// Pure-ish helper over the connection so the two tests differ only in their inputs and their
/// failure prose — the alternative is two near-identical 30-line bodies where a fix applied to
/// one silently misses the other.
async fn assert_public_cannot_execute(pattern: &str, floor: usize, family: &str, fix: &str) {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rows = c.query(&public_execute(pattern), &[]).await.unwrap();
    assert!(
        rows.len() >= floor,
        "expected at least the schema's {floor} {family} functions, found {} — has the \
         migration set failed to load, or the catalogue query gone stale?",
        rows.len()
    );

    let offenders: Vec<String> = rows
        .iter()
        .filter(|r| r.get::<_, bool>(1))
        .map(|r| format!("  {}", r.get::<_, String>(0)))
        .collect();

    assert!(
        offenders.is_empty(),
        "PUBLIC can EXECUTE these {family} functions (#382):\n{}\n{fix}",
        offenders.join("\n")
    );
}

/// The low-severity half, and the reason this file exists: the convention was followed by
/// five of twenty-two.
#[tokio::test]
async fn public_cannot_execute_any_floor_check_function() {
    assert_public_cannot_execute(
        r"cairn\_check\_%",
        CHECK_FNS_TODAY,
        "cairn_check_*",
        "Fix: add `REVOKE EXECUTE ON FUNCTION <name>(<args>) FROM PUBLIC;` after the \
         definition in db/*.sql. Note that CREATE OR REPLACE preserves an existing ACL, so a \
         function first defined in an earlier migration needs the REVOKE only there.",
    )
    .await;
}

/// The load-bearing half. This one holds already — it is a **ratchet**, not a fix: it stops
/// the next projection applier from shipping callable by every role on the node.
#[tokio::test]
async fn public_cannot_execute_any_projection_applier() {
    assert_public_cannot_execute(
        r"%\_apply",
        APPLY_FNS_TODAY,
        "*_apply",
        "An applier WRITES a projection table. Callable by PUBLIC, it lets any role forge \
         projection state no event in the append-only log supports. Add `REVOKE EXECUTE ON \
         FUNCTION <name>(event_log) FROM PUBLIC;` after the definition.",
    )
    .await;
}
