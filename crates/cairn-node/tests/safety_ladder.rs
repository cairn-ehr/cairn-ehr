//! §5.9 part B (ADR-0063) — the two rank ladders, the rung map, the structural floor check,
//! and the empty class map. Pure in-DB functions: no doors, no events, no signing.
mod common;
use common::{cs, db_msg};

/// Open a serialized connection, or `None` when the suite should self-skip.
///
/// Returns the GUARD alongside the client: the guard is a second `Client` holding a
/// cluster-wide advisory lock, so dropping it inside this helper would un-serialize every
/// caller. Callers bind it as `let Some((_g, c)) = connect().await else { return };`.
async fn connect() -> Option<(tokio_postgres::Client, tokio_postgres::Client)> {
    let base = cs()?;
    let guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    Some((guard, c))
}

#[tokio::test]
async fn an_unrecognised_severity_ranks_max() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    let row = c
        .query_one(
            "SELECT cairn_safety_severity_rank('none'), cairn_safety_severity_rank('critical'),
                    cairn_safety_severity_rank('severity:novel'), cairn_safety_severity_rank(NULL)",
            &[],
        )
        .await
        .expect("severity ranks");
    let (none, critical, novel, null): (i32, i32, i32, i32) =
        (row.get(0), row.get(1), row.get(2), row.get(3));
    assert_eq!(none, 0, "'none' is the floor");
    assert!(critical > none, "the ladder is ordered");
    // For a SAFETY signal, unknown must mean "assume the worst" — the opposite of muting a
    // warning nobody here can interpret.
    assert_eq!(novel, i32::MAX, "an unrecognised severity ranks MAX");
    assert_eq!(null, i32::MAX, "NULL lands on the safe side");
}

#[tokio::test]
async fn an_unrecognised_rung_ranks_coarsest() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    let row = c
        .query_one(
            "SELECT cairn_safety_rung_rank('precise'), cairn_safety_rung_rank('kind'),
                    cairn_safety_rung_rank('existence'), cairn_safety_rung_rank('rung:novel'),
                    cairn_safety_rung_rank(NULL)",
            &[],
        )
        .await
        .expect("rung ranks");
    let (p, k, e, novel, null): (i32, i32, i32, i32, i32) =
        (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4));
    assert!(p < k && k < e, "coarsest last");
    assert_eq!(
        novel,
        i32::MAX,
        "an unrecognised rung is treated as coarsest"
    );
    assert_eq!(null, i32::MAX);
}

#[tokio::test]
async fn the_rung_map_is_monotone_non_decreasing_in_grade_rank() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    // A higher sensitivity grade may never disclose MORE. Checked across the whole ladder
    // including the MAX sentinel, so a future grade interposed at any rank inherits a rung
    // no finer than its neighbour's.
    let rows = c
        .query(
            "SELECT r, cairn_safety_rung_rank(cairn_safety_rung_for_rank(r))
             FROM unnest(ARRAY[0, 5, 10, 15, 20, 30, 2147483647]) AS r ORDER BY r",
            &[],
        )
        .await
        .expect("rung map");
    let ranks: Vec<i32> = rows.iter().map(|r| r.get(1)).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "the rung map must be monotone non-decreasing in grade rank: {ranks:?}"
    );

    let named = c
        .query_one(
            "SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank('routine')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('sensitive')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('restricted')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('sequestered')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('grade:protected-witness'))",
            &[],
        )
        .await
        .expect("named grades");
    assert_eq!(
        named.get::<_, String>(0),
        "precise",
        "no grade discloses fully"
    );
    assert_eq!(named.get::<_, String>(1), "kind");
    assert_eq!(named.get::<_, String>(2), "existence");
    assert_eq!(named.get::<_, String>(3), "existence");
    assert_eq!(
        named.get::<_, String>(4),
        "existence",
        "an unrecognised grade ranks MAX (ADR-0062 decision 2), hence coarsest here"
    );
}

#[tokio::test]
async fn the_floor_check_admits_absence_and_every_well_formed_rung() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    for body in [
        // The two DISTINCT absence arms of the early RETURN, pinned separately because they
        // are different SQL values that happen to share a meaning. `{}` makes `b -> 'safety'`
        // yield SQL NULL (the key is absent); `{"safety": null}` makes it yield the jsonb
        // scalar `null`, for which `jsonb_typeof` returns the STRING 'null'. Both are legal
        // and both mean "this event carries no safety signal". Pinning only the first would
        // let a future reordering of the guard drop the second silently — and an explicit
        // JSON null is exactly what a serializer emits for an absent optional field.
        r#"{}"#,
        r#"{"safety": null}"#,
        r#"{"safety": {"rung": "precise", "class": "rh-sensitizing", "severity": "high"}}"#,
        r#"{"safety": {"rung": "kind", "severity": "high"}}"#,
        r#"{"safety": {"rung": "existence"}}"#,
        // A future peer's rung is ADMITTED, not refused — the floor gates effect, not
        // presence (ADR-0056). The read model treats it as coarsest.
        r#"{"safety": {"rung": "rung:novel"}}"#,
    ] {
        // `$1::text::jsonb`, never a bare `$1::jsonb` — the established codebase idiom
        // (twin_registry.rs, sensitivity_floor.rs, floor_properties.rs all carry this
        // note). With a bare cast Postgres infers the parameter's type as `jsonb`, and
        // tokio-postgres then refuses to serialize a Rust `&str` into it: the call never
        // reaches the database. Here that surfaces as a loud failure, but in the
        // `expect_err` tests below it is a FALSE GREEN waiting to happen — the test would
        // "pass" on a client-side serialization error while the floor was never exercised.
        // The `msg.contains(needle)` assertions are what keep that honest.
        c.execute(
            "SELECT cairn_check_safety_signal($1::text::jsonb)",
            &[&body],
        )
        .await
        .unwrap_or_else(|e| panic!("must admit {body}: {}", db_msg(&e)));
    }
}

#[tokio::test]
async fn the_floor_check_refuses_a_class_the_rung_does_not_license() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    // A body claiming "existence" while carrying the class publishes what it asserts is
    // concealed. Refused where it is AUTHORED — the only place a door can help.
    let e = c
        .execute(
            "SELECT cairn_check_safety_signal($1::text::jsonb)",
            &[&r#"{"safety": {"rung": "existence", "class": "rh-sensitizing"}}"#],
        )
        .await
        .expect_err("a class at a coarser rung must be refused");
    let msg = db_msg(&e);
    assert!(
        msg.contains("class"),
        "the message names the offending key: {msg}"
    );
}

#[tokio::test]
async fn the_floor_check_refuses_a_missing_rung_and_a_precise_without_a_class() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    for (body, needle) in [
        (r#"{"safety": {"severity": "high"}}"#, "rung"),
        (r#"{"safety": {"rung": ""}}"#, "rung"),
        (
            r#"{"safety": {"rung": "precise", "severity": "high"}}"#,
            "class",
        ),
        (
            r#"{"safety": {"rung": "precise", "class": "  ", "severity": "high"}}"#,
            "class",
        ),
        (r#"{"safety": "not-an-object"}"#, "object"),
    ] {
        let e = c
            .execute(
                "SELECT cairn_check_safety_signal($1::text::jsonb)",
                &[&body],
            )
            .await
            .expect_err(&format!("must refuse {body}"));
        let msg = db_msg(&e);
        assert!(msg.contains(needle), "message must name `{needle}`: {msg}");
    }
}

#[tokio::test]
async fn the_floor_check_refuses_a_severity_at_the_coarsest_rung() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    // 2026-08-14 review finding: the door and the read model disagreed about this shape.
    //
    // db/049 section 7 is explicit — "AT 'existence' NEITHER class NOR severity SURVIVES.
    // 'existence' is the claim 'there is a safety-relevant signal here and you are not
    // cleared to see what' — a severity beside it would narrow exactly that." The read
    // model therefore gates severity off at this rung. But the door admitted it, so the
    // bytes could be MINTED and replicated permanently while every reader declined to
    // show them. Decision 6's own logic says the door is where this must bind: emission
    // is the only coarsening that binds a peer's raw-SQL client.
    //
    // ADR-0060 SAFETY ARGUMENT FOR ADDING A REFUSAL AT THE STRICT DOOR: this shape is
    // unreachable from any in-repo builder — `cairn_event::safety::coarsen` is total over
    // three fixed shapes and its `Existence` arm emits `{"rung":"existence"}` with no
    // other key. So the guard cannot fail a clinical write, exactly like the sibling
    // class guard it now mirrors.
    let e = c
        .execute(
            "SELECT cairn_check_safety_signal($1::text::jsonb)",
            &[&r#"{"safety": {"rung": "existence", "severity": "critical"}}"#],
        )
        .await
        .expect_err("a severity at the coarsest rung must be refused");
    let msg = db_msg(&e);
    assert!(
        msg.contains("severity"),
        "the message names the offending key: {msg}"
    );

    // The CONTROL that stops this passing for the wrong reason: severity is legal at both
    // finer rungs, so the guard must be keyed on the RUNG and not merely on the key.
    for body in [
        r#"{"safety": {"rung": "kind", "severity": "high"}}"#,
        r#"{"safety": {"rung": "precise", "class": "rh-sensitizing", "severity": "high"}}"#,
    ] {
        c.execute(
            "SELECT cairn_check_safety_signal($1::text::jsonb)",
            &[&body],
        )
        .await
        .unwrap_or_else(|e| panic!("must still admit {body}: {}", db_msg(&e)));
    }
}

#[tokio::test]
async fn the_safety_functions_are_revoked_from_public_and_granted_deliberately() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    // 2026-08-14 review finding: db/049 section 8 REVOKEs EXECUTE from PUBLIC on five
    // functions and re-GRANTs to cairn_agent, citing #382 — "an un-REVOKEd function is
    // directly callable by a below-the-floor adversary with raw SQL". None of it was
    // asserted anywhere. Postgres grants EXECUTE to PUBLIC by DEFAULT, so any future
    // migration doing `CREATE OR REPLACE` on one of these silently restores the default
    // and no test notices. The control was enforced only by the migration's own prose.
    //
    // `public` is a real role name here, so has_function_privilege resolves it.
    //
    // `cairn_record_safety_overclaim_flag(bytea, uuid, text, text)` joined in #405 part 2
    // (2026-08-15 review, Minor #5): same posture as `cairn_check_safety_signal` just
    // below — a writer called only from inside `submit_event`'s SECURITY DEFINER context.
    for f in [
        "cairn_check_safety_signal(jsonb)",
        "cairn_record_safety_overclaim_flag(bytea, uuid, text, text)",
        "cairn_safety_class_candidate(jsonb)",
        "cairn_prospective_sensitivity(uuid, uuid)",
        "cairn_event_safety(uuid)",
        "cairn_patient_safety(uuid)",
    ] {
        let public_can: bool = c
            .query_one(
                "SELECT has_function_privilege('public', $1, 'EXECUTE')",
                &[&f],
            )
            .await
            .unwrap_or_else(|e| panic!("privilege probe for {f}: {}", db_msg(&e)))
            .get(0);
        assert!(
            !public_can,
            "{f} must be REVOKEd from PUBLIC — every role is a member of PUBLIC, so an \
             un-REVOKEd function is reachable by a below-the-floor adversary (#382)"
        );
    }

    // …and the four the daemon actually calls are granted back. `cairn_check_safety_signal`
    // is deliberately NOT in this list: it is called only from inside `submit_event`, which
    // is SECURITY DEFINER, so cairn_agent needs no grant on it at all.
    for f in [
        "cairn_safety_class_candidate(jsonb)",
        "cairn_prospective_sensitivity(uuid, uuid)",
        "cairn_event_safety(uuid)",
        "cairn_patient_safety(uuid)",
    ] {
        let agent_can: bool = c
            .query_one(
                "SELECT has_function_privilege('cairn_agent', $1, 'EXECUTE')",
                &[&f],
            )
            .await
            .unwrap_or_else(|e| panic!("privilege probe for {f}: {}", db_msg(&e)))
            .get(0);
        assert!(agent_can, "{f} must be granted to cairn_agent");
    }
}

#[tokio::test]
async fn every_clinical_event_type_is_thread_bearing_so_the_missing_gate_cannot_bite() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    // THE TRIPWIRE FOR #404's ONE REMAINING PIECE. Read this before adding a clinical verb.
    //
    // `cairn_prospective_sensitivity`'s `p_thread IS NULL` arm coarsens chart-wide — the
    // conservative bound for "this event MAY be on that thread". db/048 section 11's
    // equivalent arm additionally gates on `cairn_event_type_has_no_thread`, so an event
    // type that CANNOT have a thread does not take the bound. The prospective form has no
    // such gate, because it takes no event type.
    //
    // That is harmless only while every event type reaching the emission seam is
    // thread-BEARING. The day a thread-free clinical verb writes `payload.safety`, a
    // standing thread-scoped grade will coarsen it to `existence` at emission — permanently,
    // in signed bytes — while `cairn_effective_sensitivity` computes `routine` for that same
    // event. That is exactly the emission/read divergence #404 was, reopened through a door
    // nobody was watching.
    //
    // The fix at that point is to give `cairn_prospective_sensitivity` an event-type
    // parameter and gate the NULL arm. That is deliberately NOT done today: it would change
    // nothing observable (see the assertion below), and a signature change on this function
    // is its own hazard — Postgres OVERLOADS rather than replaces, migration replay never
    // drops what a file stops creating, and a stale 2-arg definition would silently keep
    // serving every un-updated caller, this suite's `has_function_privilege` pins and
    // safety_emission.rs's staged-outage rig included. It needs db/005's
    // `DROP FUNCTION IF EXISTS` idiom and all five call sites in one pass.
    //
    // WHAT THIS TEST DOES NOT COVER, said plainly: it keys on the `clinical.` prefix, which
    // is a proxy for "reaches the emission seam", not the thing itself. A thread-free verb
    // named OUTSIDE that prefix that writes `payload.safety` would slip past. There is no
    // queryable "can carry safety" property to key on — if you are adding such a verb, this
    // comment is the warning, not the assertion.
    let types: Vec<String> = c
        .query(
            "SELECT event_type FROM cairn_event_twin_check
             WHERE event_type LIKE 'clinical.%' ORDER BY event_type",
            &[],
        )
        .await
        .expect("the twin-check registry is the mechanical list of registered event types")
        .iter()
        .map(|r| r.get(0))
        .collect();

    // Non-vacuous: an empty or renamed registry must fail loudly rather than pass by
    // asserting nothing at all.
    assert!(
        types.len() >= 9,
        "expected the registered clinical event types (9 medication verbs at the time of \
         writing); got {types:?}. If the registry moved, fix this query — do not delete \
         the test"
    );

    for t in &types {
        let thread_free: bool = c
            .query_one("SELECT cairn_event_type_has_no_thread($1)", &[t])
            .await
            .unwrap_or_else(|e| panic!("thread-gate probe for {t}: {}", db_msg(&e)))
            .get(0);
        assert!(
            !thread_free,
            "event type {t:?} is declared THREAD-FREE by db/048, but it is clinical content \
             and so may reach the §5.9 emission seam. `cairn_prospective_sensitivity` has no \
             event-type gate on its `p_thread IS NULL` arm, so a standing thread-scoped \
             grade will coarsen this event to `existence` at emission while every read of it \
             computes `routine` — the #404 divergence, reopened. Give that function an \
             event-type parameter (and read this test's comment first: the signature change \
             has its own overload hazard)."
        );
    }
}

#[tokio::test]
async fn the_class_map_ships_empty() {
    let Some((_g, c)) = connect().await else {
        return;
    };
    // RULING (task-2-brief.md's count-empty half dropped): counting rows in the long-lived
    // `cairn_test` database is flaky by construction — later tasks (5, 6) seed fixture rows
    // there, and Rust test order across files is not guaranteed. The shipped-empty invariant
    // is asserted instead in db/tests/049_safety_projection_test.sql (Task 7), which runs
    // against a freshly created scratch database and is the only place that tests the
    // *migration* rather than the runtime.
    //
    // What DOES belong here, and holds regardless of what other tests have seeded: the
    // candidate lookup is honest about a coding it has no row for. Cairn ships the LOOKUP,
    // never the drug knowledge — the same discipline sensitivity_category_map keeps.
    let hit: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_safety_class_candidate(
                 '{\"system\":\"drugref-moiety\",\"code\":\"a-code-that-cannot-be-mapped-yet\"}'::jsonb)",
            &[],
        )
        .await
        .expect("candidate lookup runs")
        .get(0);
    assert_eq!(hit, 0, "no row in the map for this coding ⇒ no candidate");
}
