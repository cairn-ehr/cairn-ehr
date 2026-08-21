//! #382 — the floor's `REVOKE EXECUTE … FROM PUBLIC` convention, made checkable.
//!
//! # What the convention is
//!
//! Four families of function in `db/*.sql` are meant to be unreachable by `PUBLIC`:
//!
//! * **`cairn_check_*`** — mostly the per-event-type structural validators the `submit_event`
//!   / `apply_remote_event` doors dispatch through, plus two registry triggers
//!   (`cairn_check_twin_registry_fn`, `cairn_check_projection_registry_fn`) that take no
//!   `jsonb` body at all and fire on INSERT into the registry tables. The severity of a
//!   `PUBLIC` caller invoking any of them is genuinely low: **none writes, none grants**, and
//!   the only tables any of them read are open vocabularies already `SELECT`-able by `PUBLIC`
//!   (`event_type_class` in the projection-registry trigger, `contributor_role` in
//!   `cairn_check_contributors`, `medication_coding_system` reached via
//!   `cairn_check_coding_object`). A caller learns strictly less than the door already tells
//!   it by refusing, and the refusal messages are deliberately legible.
//!
//!   The earlier version of this paragraph said "pure `jsonb` shape checks: they read no
//!   table". That was false for four of the twenty-two, which does not change the conclusion
//!   but did make the conclusion unverifiable — so it is spelled out above instead.
//! * **the registered projection appliers** — the functions `cairn_projection_apply` names,
//!   which the `event_log` dispatcher calls. These are the load-bearing half: each one WRITES
//!   a projection table, so a runtime role able to call one directly could forge projection
//!   state that no event supports — the projections would then disagree with the append-only
//!   log they are supposed to be derived from.
//!
//! All are revoked, for different reasons. Stating that difference here rather than
//! flattening it is the point: a reader who thinks they are equally severe will
//! eventually "simplify" one of them away. The first two are described above; the third and
//! fourth joined later and are described below, where the count is kept honest — a header that
//! counts families and then stops counting is the same half-followed convention this file
//! exists to make checkable (#456 review).
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
//! database ended up with, which is the only thing an attacker meets. Grepping would also miss
//! the case that makes ACLs subtle: `CREATE OR REPLACE FUNCTION` **preserves** the existing
//! ACL, so a function first defined (and revoked) in one migration and replaced in a later one
//! stays revoked with no `REVOKE` in the later file at all. Two of these functions are exactly
//! that shape (`cairn_check_demographic_field`, db/011 → db/014; `cairn_check_medication_dose`,
//! db/032 → db/035).
//!
//! How far "it covers whatever a future migration adds" actually goes differs by family, and
//! the difference is the point: the applier half reads the **registry**, so a new applier is
//! covered the moment it becomes reachable. The check half reads a **name pattern**, so a
//! validator renamed out of the `cairn_check_` prefix leaves the family silently. That
//! asymmetry is deliberate — the appliers have an authoritative list and the validators do
//! not — but it is a real limit, not a technicality.
//!
//! # What this guard does NOT claim
//!
//! It says almost nothing about which non-`PUBLIC` roles hold `EXECUTE`. The one exception is
//! `the_declared_twin_provenance_read_surface_still_works`, added with the fourth family: two
//! of those functions need a paired `GRANT … TO cairn_agent` to keep a declared read surface
//! working, so that grant IS asserted, in the positive direction. Everything below is about
//! the other, negative direction. `cairn_node` and
//! `cairn_agent` reach the dispatched validators and the appliers from *inside* the
//! `SECURITY DEFINER` doors, which run as the schema owner; the two registry triggers are
//! reached differently again, by `BEFORE INSERT` triggers during migration replay, where
//! Postgres does not ACL-check the trigger function at all. Nothing here would catch an
//! explicit `GRANT EXECUTE … TO cairn_agent` on an applier — that would need its own
//! assertion, and no migration does it today.
//!
//! A third family joined in #443: `cairn_event_twin`, the dispatcher that routes an event type
//! to its validator, which until then carried default `PUBLIC` EXECUTE. That already failed
//! closed — a `PUBLIC` caller reached the dispatcher and was refused one layer deeper — so the
//! fix bought legibility rather than privilege, which is exactly what the twenty-two bought.
//!
//! A fourth joined in #453: the `cairn_twin_%` functions — `cairn_twin_skeleton` and
//! `cairn_twin_is_present` (db/005), which the dispatcher calls, plus `cairn_twin_is_authored`
//! and `cairn_twin_provenance_of` (db/015), which it does not: those two are the twin
//! READ SURFACE, grouped with the others by name prefix rather than by call graph. Same
//! already-fails-closed reasoning, same legibility purchase — and, uniquely so far, a paired
//! GRANT, because one of them is reached through a view by an invoker-rights path.
//!
//! # Why there is no `db/tests/` SQL mirror
//!
//! For the reason `search_path_pg_temp.rs` gives: the floors below are COUNTS, and this repo
//! has been bitten repeatedly by counts pinned in two places that drift apart (#182, #189).
//! One home, exercised in the gate that runs the whole workspace, beats two that disagree.
//!
//! Every test here self-skips without `$CAIRN_TEST_PG` (see `common::cs`), and a skipped run
//! prints `ok` while proving nothing. CI sets it; `scripts/run-db-gated-tests.sh` bakes it in.
//! That the whole DB-gated suite could go silently green if that variable were ever unset was
//! a suite-wide hole; `db_gate_actually_ran.rs` closes it by failing whenever a gate variable
//! the suite reads is unset, unless the run declares `CAIRN_ALLOW_DB_SKIP=1` (#442, #450 — it
//! fails CLOSED, so an absent opt-out is not permission). It does NOT make a skipped LOCAL run
//! louder line-by-line — the bare `else { return }` sites stay silent until the duplicated
//! `cs()` helper is unified (#327).
mod common;
use common::{cs, NOT_EXTENSION_OWNED, REPO_SCHEMAS};

/// Repo-defined functions selected by `membership` — a SQL predicate over the `pg_proc` alias
/// `p` — each with whether `PUBLIC` can execute it.
///
/// **The ACL, read from `proacl` rather than `has_function_privilege`.** A NULL `proacl`
/// means "nobody has touched the grants", and PostgreSQL's default for a function is
/// `EXECUTE` to `PUBLIC` — so NULL is the *permissive* case, not the empty one. Getting that
/// backwards would invert the whole guard, which is why it is spelled out in SQL here rather
/// than left to a helper whose polarity a reader has to recall. Otherwise the ACL is exploded
/// and grantee `0` — the `PUBLIC` pseudo-role, which has no `pg_authid` row — is looked for
/// directly.
///
/// `membership` is a fragment rather than a name pattern because the two families are
/// identified in genuinely different ways — see [`CHECK_FAMILY`] and [`APPLY_FAMILY`].
fn public_execute(membership: &str) -> String {
    format!(
        "SELECT p.oid::regprocedure::text,
                (p.proacl IS NULL
                 OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a
                             WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'))
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
          WHERE {REPO_SCHEMAS}
            AND ({membership})
            AND {NOT_EXTENSION_OWNED}
          ORDER BY 1"
    )
}

/// The `cairn_check_*` family, identified by NAME, because that is genuinely all it is: a
/// naming convention with no registry behind it. `cairn_event_twin_check.check_fn` names the
/// dispatched validators, but it is not the family — helpers those validators call
/// (`cairn_check_coding_object`) and the two registry triggers appear in no registration at
/// all, and are family members every bit as much. There is nothing authoritative to read, so
/// the prefix is what there is.
///
/// The underscores are escaped as `\_`: unescaped, `_` is LIKE's single-character wildcard, so
/// `cairn_check_%` would also match a hypothetical `cairnXcheckY`. Harmless today, wrong
/// tomorrow.
const CHECK_FAMILY: &str = r"p.proname LIKE 'cairn\_check\_%'";

/// The projection appliers, identified by REGISTRATION rather than by name — the authoritative
/// list is the `cairn_projection_apply` registry (ADR-0057/#208), which is what the `event_log`
/// dispatcher actually calls.
///
/// This started life as `p.proname LIKE '%\_apply'` and that was wrong, in the quiet way: the
/// registry holds 21 appliers and exactly one of them, `medication_dose_seed_initial`
/// (db/032), does not end in `_apply`. It writes `medication_dose_event` — squarely inside the
/// threat this guard names — and was invisible to the pattern. It happens to be revoked, so
/// there was never an exposure; the defect was that the ratchet could not have noticed if it
/// were not, and would not notice the next off-convention name either.
///
/// Reading the registry instead makes the guard self-updating: an applier is covered the
/// moment it is registered, which is the moment it becomes reachable. A name convention only
/// ever covers the names someone remembered to follow.
const APPLY_FAMILY: &str = "p.proname IN (SELECT apply_fn FROM cairn_projection_apply)";

/// The dispatcher that routes an event type to its validator, identified by NAME because a
/// family of one has no list to read (#443).
///
/// Note what it does NOT dispatch to: all twenty-two of them. The `cairn_event_twin_check`
/// registry holds 24 rows naming **16 distinct** `cairn_check_*` functions; the remaining six
/// prefix-siblings are helpers no registration mentions (`cairn_check_coding_object`,
/// `cairn_check_safety_signal` and the like — `db/005_submit.sql` says so where it explains why
/// [`CHECK_FAMILY`] is read from a name prefix rather than from a registry). The twenty-two is
/// the REVOKE *convention's* membership, which is the set this file cares about; the sixteen is
/// the dispatch fan-out, which it does not. Conflating them is what #443's title did, and the
/// arithmetic was wrong in both directions at once.
///
/// Naming it is the honest option here, not a lapse from the [`APPLY_FAMILY`] standard. The
/// registry `cairn_event_twin_check` lists the validators this function dispatches TO; nothing
/// in the schema lists the dispatcher itself, and a derivation invented for it — "the function
/// whose body contains an EXECUTE over the registry" — would be a cleverer way of writing the
/// same single name, with a failure mode (silently matching nothing) the plain name does not
/// have. The `CHECK_FNS_TODAY`-style floor below is what covers the "matched nothing" case.
///
/// Why it is revoked at all, given it fails closed either way: before this, `PUBLIC` could
/// reach the dispatcher and be refused one layer deeper, by `permission denied for function
/// cairn_check_…`. Nothing leaks and nothing is writable — but the refusal came from the wrong
/// place, and told the caller which validator a given event type maps to. More to the point,
/// #382's whole argument was that a convention a reader cannot verify is worth nothing, and
/// "all twenty-two prefix-siblings revoked, the dispatcher that reaches them not" is exactly
/// the half-followed state that argument was about.
const DISPATCHER_FAMILY: &str = "p.proname = 'cairn_event_twin'";

/// The `cairn_twin_%` family, identified by NAME PREFIX (#453).
///
/// `cairn_twin_skeleton` and `cairn_twin_is_present` (db/005), `cairn_twin_is_authored` and
/// `cairn_twin_provenance_of` (db/015). #453 named the first two; the family is four, and
/// revoking two of four would recreate exactly the half-followed state #443 was about — a rule
/// a reader cannot tell from an oversight.
///
/// Grouped by PREFIX, not by call graph, and the distinction is worth keeping straight: only
/// the two db/005 functions are called by `cairn_event_twin`. The db/015 pair are the twin
/// read surface. Calling all four "the dispatcher's helpers", as the first cut did, sends a
/// reader looking for calls that do not exist (#456 review).
///
/// A PREFIX rather than four names, for [`CHECK_FAMILY`]'s reason and not as a lapse from
/// [`APPLY_FAMILY`]'s standard: nothing in the schema lists these, so there is no authoritative
/// registry to read, and a prefix covers the next helper the moment it is added. Underscores
/// escaped as `\_` so `cairn_twin_%` cannot also match a hypothetical `cairnXtwinY`.
///
/// Note it does NOT match `cairn_event_twin` — that is [`DISPATCHER_FAMILY`], asserted
/// separately because its argument (and its live callers) are different.
const TWIN_HELPER_FAMILY: &str = r"p.proname LIKE 'cairn\_twin\_%'";

/// What the loaded schema holds today (2026-08-21), as floors rather than equalities.
///
/// `>=` against today's exact count, in the shape `search_path_pg_temp.rs` established: the
/// count only ever RISES as migrations add functions, so this never needs revising upward —
/// but it trips the moment coverage *shrinks*, which is what would happen if a rename or a
/// failed migration load quietly emptied the query. Without it, "no offenders" and "no rows
/// examined" are the same green.
///
/// The floor is a LIVENESS check on the query, not a definition of the family — the offender
/// scan below iterates every returned row, so fifty functions with twenty-eight unrevoked
/// still fails loudly with twenty-eight names.
const CHECK_FNS_TODAY: usize = 22;
const APPLY_FNS_TODAY: usize = 21;
const DISPATCHER_FNS_TODAY: usize = 1;
const TWIN_HELPER_FNS_TODAY: usize = 4;

/// Run one family's assertion: at least `floor` functions matched, and none of them is
/// `PUBLIC`-executable.
///
/// Pure-ish helper over the connection so each family's test differs only in its inputs and its
/// failure prose — the alternative is two near-identical 30-line bodies where a fix applied to
/// one silently misses the other.
async fn assert_public_cannot_execute(membership: &str, floor: usize, family: &str, fix: &str) {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rows = c.query(&public_execute(membership), &[]).await.unwrap();
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
        CHECK_FAMILY,
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
///
/// Driven off the `cairn_projection_apply` registry, NOT off the `_apply` name suffix. See
/// [`APPLY_FAMILY`] for why the name-based version was one applier short of the truth.
#[tokio::test]
async fn public_cannot_execute_any_projection_applier() {
    assert_public_cannot_execute(
        APPLY_FAMILY,
        APPLY_FNS_TODAY,
        "registered projection applier",
        "An applier WRITES a projection table. Callable by PUBLIC, it lets any role forge \
         projection state no event in the append-only log supports. Add `REVOKE EXECUTE ON \
         FUNCTION <name>(event_log) FROM PUBLIC;` after the definition.",
    )
    .await;
}

/// The dispatcher, closing the gap this file's own header used to declare as open (#443).
///
/// It is one function, so the floor of 1 is doing more work than it looks: it is the only
/// thing standing between a rename of `cairn_event_twin` and a test that passes by examining
/// nothing at all.
#[tokio::test]
async fn public_cannot_execute_the_twin_dispatcher() {
    assert_public_cannot_execute(
        DISPATCHER_FAMILY,
        DISPATCHER_FNS_TODAY,
        "twin-dispatch",
        "Fix: `REVOKE EXECUTE ON FUNCTION cairn_event_twin(text, jsonb) FROM PUBLIC;` in \
         db/005_submit.sql. Every live caller reaches it either from inside a SECURITY \
         DEFINER door (submit_event, apply_remote_event) or as the schema owner \
         (cairn_readjudicate_deferred, itself already revoked), so no runtime role \
         needs a grant.",
    )
    .await;
}

/// The dispatcher's helpers (#453) — the last members of the twin family still holding
/// Postgres's default EXECUTE-to-PUBLIC after #382 revoked the validators and #443 the
/// dispatcher.
///
/// Like #443, this fails closed already: they are pure predicates and formatters over a body a
/// `PUBLIC` caller has no way to submit through any door, so what is bought is legibility — the
/// whole of what #382 and #443 bought too.
///
/// The floor of four is doing real work here. `cairn_twin_is_authored` and
/// `cairn_twin_provenance_of` live in db/015, not db/005, and #453's own text named only the
/// two db/005 helpers; a guard written to that text would have passed while half the family
/// stayed open.
#[tokio::test]
async fn public_cannot_execute_any_twin_helper() {
    assert_public_cannot_execute(
        TWIN_HELPER_FAMILY,
        TWIN_HELPER_FNS_TODAY,
        "cairn_twin_* helper",
        "Fix: add `REVOKE EXECUTE ON FUNCTION <name>(<args>) FROM PUBLIC;` after the \
         definition — db/005 for cairn_twin_skeleton / cairn_twin_is_present, db/015 for \
         cairn_twin_is_authored / cairn_twin_provenance_of. BUT CHECK THE LIVE CALLERS \
         FIRST: event_twin_provenance is a VIEW granted to cairn_agent, and PostgreSQL \
         checks a function called inside a view against the INVOKING user, so two of these \
         need an explicit GRANT alongside the REVOKE — see \
         the_declared_twin_provenance_read_surface_still_works.",
    )
    .await;
}

/// The other half of #453, and the mutation proof for its two `GRANT`s: `cairn_agent` can still
/// read the surface db/015 declares for it.
///
/// **The trap this pins, measured rather than assumed.** PostgreSQL checks *table* access inside
/// a normal view against the VIEW OWNER, but a *function* called inside that view against the
/// INVOKING user — the CREATE VIEW docs say so, and it is easy to carry the table rule across.
/// So a bare `REVOKE … FROM PUBLIC` on the twin helpers breaks `event_twin_provenance` for
/// `cairn_agent`, the one role db/015 grants it to:
///
/// ```text
/// ERROR:  permission denied for function cairn_twin_provenance_of
/// ```
///
/// and granting only the outer function is not enough, because the INNER call is checked too:
///
/// ```text
/// ERROR:  permission denied for function cairn_twin_is_present
/// CONTEXT:  PL/pgSQL function cairn_twin_provenance_of(bytea) line 6 at assignment
/// ```
///
/// Hence two grants, not one — and the inner one needs its own direct call to be pinned at all.
///
/// **Why the view read alone is not enough (#456 review).** `event_twin_provenance` is
/// `CROSS JOIN LATERAL cairn_twin_provenance_of(el.signed_bytes)` over `event_log`. Postgres
/// checks the OUTER function's ACL at executor initialisation, so that grant is pinned however
/// many rows there are; but `cairn_twin_is_present` is called from inside the PL/pgSQL body,
/// so its ACL is checked only when that body actually RUNS — once per row. Nothing in this
/// binary submits an event, and `connect_and_load_schema` seeds none, so whether `event_log`
/// held a row here was inherited from whichever suite last ran against the shared database —
/// and 44 files under `tests/` open with `TRUNCATE event_log CASCADE`. Measured on PG 18.1:
///
/// | grant revoked | `event_log` | result |
/// |---|---|---|
/// | `cairn_twin_provenance_of` | empty | `ERROR: permission denied` — caught |
/// | `cairn_twin_provenance_of` | 2 rows | `ERROR: permission denied` — caught |
/// | `cairn_twin_is_present` | **empty** | **`count = 0`, no error — NOT caught** |
/// | `cairn_twin_is_present` | 2 rows | `ERROR: permission denied` — caught |
///
/// So the inner grant is now driven by a DIRECT call, which has no row-count precondition, and
/// the view read stays as the integration-level check. Delete either grant and this fails,
/// naming which — unconditionally, rather than depending on test-binary ordering.
///
/// The view has no product consumer today (only tests read it, as the schema owner), which is
/// exactly why it needs a test: a DECLARED read surface with no caller is one nobody would
/// notice breaking.
///
/// **One honest limit, found while mutating this.** Deleting a `GRANT … TO cairn_agent` line
/// from db/015 does not revoke an ALREADY-GRANTED privilege: `REVOKE … FROM PUBLIC` in db/005
/// leaves a role grant untouched, and migration replay only ever adds. So on a long-lived
/// developer database this test keeps passing after the line is deleted, and only goes red on
/// a database created after the deletion — which CI is, every run. The same "your dev DB
/// remembers what the schema no longer says" trap as the stale-column-order one; noted here so
/// a local green is not mistaken for proof.
#[tokio::test]
async fn the_declared_twin_provenance_read_surface_still_works() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    // As the runtime role db/015 grants the view to — not as the owner, which would prove
    // nothing: the owner is exempt from every ACL below.
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    // The INNER grant, with no dependence on what `event_log` happens to hold.
    let inner = c
        .query_one("SELECT cairn_twin_is_present($1)", &[&"a twin"])
        .await;
    // ...and the surface as a whole, which is what a consumer would actually issue.
    let read = c
        .query_one("SELECT count(*) FROM event_twin_provenance", &[])
        .await;
    // RESET before asserting: a failed assertion must not leave the connection in a role the
    // teardown cannot escape.
    c.batch_execute("RESET ROLE").await.unwrap();

    let fix = "A function called inside a view is ACL-checked against the INVOKING user, so \
               the cairn_twin_% REVOKEs must keep their paired `GRANT EXECUTE … TO cairn_agent` \
               on BOTH cairn_twin_provenance_of and its inner cairn_twin_is_present call \
               (db/015). Restore the grant rather than removing the REVOKE.";

    if let Err(e) = inner {
        panic!(
            "cairn_agent can no longer execute cairn_twin_is_present, the INNER half of the \
             surface db/015 GRANTs it (#453): {e}\n\n{fix}"
        );
    }
    if let Err(e) = read {
        panic!(
            "cairn_agent can no longer read event_twin_provenance, the surface db/015 GRANTs \
             it (#453): {e}\n\n{fix}"
        );
    }
}
