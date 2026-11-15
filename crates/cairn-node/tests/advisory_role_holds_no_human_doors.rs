//! #1129 — the advisory-actor role holds none of the human-key vault's or supervision's doors.
//!
//! `cairn_agent` is the role an advisory actor's login inherits (the matcher worker, any ADR-0030
//! agent; db/004, db/017). It carries the validated submit doors and the projection reads
//! (ADR-0084 §1), and nothing that belongs to a person at the window. Before #1129 the vault's and
//! three supervision doors were granted to it, a leftover from before db/062 gave the window its
//! own `cairn_window` role. An agent login could then release a human's signing seed for any
//! passphrase it had seen, list every staff ID and hint, take the terminal lock's hash offline,
//! and hold every clinician at the back-off cap with wrong guesses. Now these doors are
//! `cairn_window`'s (db/062 for db/061's; db/063 and db/072 their own), and the window still
//! reaches them (walked in `window_role_vault_doors.rs`).
//!
//! What this pins, read from the catalogue: no `cairn_vault_*` or `cairn_supervision_*` function
//! is executable by `cairn_agent`, except the few PUBLIC may execute (the pure address builders),
//! which any role reaches. DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
mod common;
use common::cs;

#[tokio::test]
async fn the_advisory_role_cannot_execute_a_vault_or_supervision_door() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    // (name, whether cairn_agent may execute it, whether PUBLIC may) per function of the families.
    let rows = c
        .query(
            "SELECT p.oid::regprocedure::text,
                    has_function_privilege('cairn_agent', p.oid, 'EXECUTE'),
                    EXISTS (SELECT 1 FROM aclexplode(coalesce(p.proacl, acldefault('f', p.proowner))) a
                             WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE')
               FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
              WHERE n.nspname = 'public'
                AND (p.proname LIKE 'cairn\\_vault\\_%' OR p.proname LIKE 'cairn\\_supervision\\_%')
              ORDER BY 1",
            &[],
        )
        .await
        .unwrap();
    assert!(
        rows.len() >= 30,
        "expected the vault and supervision families, found {} functions — has the schema failed \
         to load, or the query gone stale?",
        rows.len()
    );
    let offenders: Vec<String> = rows
        .iter()
        .filter(|r| r.get::<_, bool>(1) && !r.get::<_, bool>(2))
        .map(|r| format!("  {}", r.get::<_, String>(0)))
        .collect();
    assert!(
        offenders.is_empty(),
        "the advisory role cairn_agent can EXECUTE these human doors (#1129):\n{}\nFix: grant \
         them to cairn_window (db/062 for db/061's doors) and `REVOKE EXECUTE … FROM cairn_agent`.",
        offenders.join("\n")
    );
}
