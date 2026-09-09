//! The two preconditions the restore ceremony's ORDER rests on (#554 slice 2d, design §3).
//!
//! # What the order is, and why it is load-bearing
//!
//! ```text
//! mint new key → apply node plane → install custody + registry → apply CLINICAL plane
//!              → finalize_identity   (LAST)
//! ```
//!
//! Custody precedes the clinical apply because `apply_remote_event` wraps each event's DEK to
//! the **registered** public half. The actor registry precedes it for a different reason:
//! every apply door resolves its author through `actor_current`, so without the registry the
//! door refuses this node's own history with *"signer … is not an enrolled, non-revoked
//! actor"* — the zero-patients outcome wearing a different costume. And `finalize_identity`
//! runs **last** so the whole restore happens inside the un-enrolled fence: a clinical apply
//! that fails catastrophically then leaves a database with **no genesis written**, still
//! legitimately restorable from the same medium into the same database. Under the minimal
//! reordering (finalize where it was, clinical after it) the same failure leaves a node
//! already identity-minted and already fenced, whose only recovery is a fresh database.
//!
//! # Why these are SOURCE guards rather than behavioural ones
//!
//! Both steps now run against an **un-enrolled** database, and they only work because neither
//! reads `local_node`. That was verified by reading the SQL when the ceremony was reordered —
//! and a verification that happened once, in a session, is not a verification. A future
//! migration adding a `local_node` read to either would break the ceremony **silently**: the
//! restore would fail at a step whose error names a table nobody would connect to the order of
//! operations, and it would do so only on a real disaster-recovery run.
//!
//! Same discipline slice 2c demanded for the shred predicate, and the same shape as
//! `shred_predicate_has_one_home.rs`. A source guard is the cheapest way to make a fact that
//! is currently true stay true, when the cost of it silently ceasing to be true is a clinic
//! that cannot restore.
//!
//! **When one of these fails, do not delete it.** It means the ceremony's order no longer
//! holds and `main.rs`'s `Cmd::Restore` arm has to change with the migration.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/cairn-node → repo root")
        .to_path_buf()
}

/// Read a migration, with comment lines stripped.
///
/// Comments are stripped for the same reason `hex_decode_helper.rs` strips them: these files
/// DISCUSS `local_node` at length in prose — db/020's header does not, but a future one might,
/// and a guard that counted a sentence about a table as a read of it would fail on correct
/// code. Failing closed on the code and open on the prose is the safe direction here, because
/// the prose cannot break a ceremony.
fn migration_code(name: &str) -> String {
    let path = repo_root().join("db").join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// **Precondition 1.** `apply_remote_event` never reads `local_node`, so the clinical plane
/// can be applied while the database is still un-enrolled.
///
/// Its HLC merge goes through `cairn_node_hlc_merge`, which updates `hlc_state` — created in
/// db/001 and not gated on enrollment. If a future revision of this door starts consulting
/// `local_node` (to stamp an origin, to check a peer, to gate anything), the clinical apply
/// must move to AFTER `finalize_identity` — and moving it there costs the property that a
/// failed restore leaves a restorable database, so it is a decision, not a patch.
#[test]
fn the_clinical_apply_door_does_not_read_local_node() {
    let code = migration_code("020_apply_remote_event.sql");
    assert!(
        !code.contains("local_node"),
        "db/020_apply_remote_event.sql now references `local_node`. The restore ceremony \
         applies the clinical plane BEFORE `finalize_identity` writes that row, precisely \
         because this door did not need it — so this door reading it means a restore now \
         fails at a step whose error names a table nobody would connect to the ORDER of \
         operations, and only ever on a real disaster-recovery run. Fix the ceremony in \
         `main.rs`'s `Cmd::Restore` arm together with the migration; do not delete this guard."
    );
}

/// **Precondition 2.** `cairn_register_unwrap_key` never reads `local_node`, so custody can be
/// registered on an un-enrolled database.
///
/// It is a singleton registrar guarding only against a DIFFERING key (db/037). The restore
/// installs and registers the dead node's inherited unwrap secret before any clinical event
/// lands, because the apply door wraps each DEK to the registered public half — so if this
/// registrar ever required an enrolled node, custody could not be installed before the events
/// that need it, and every restored sealed body would land without a key.
#[test]
fn the_unwrap_key_registrar_does_not_read_local_node() {
    let code = migration_code("037_born_sealed.sql");
    let body = code
        .split("cairn_register_unwrap_key")
        .nth(1)
        .expect("db/037_born_sealed.sql must define cairn_register_unwrap_key — if it moved, move this guard");
    // Bounded to this function's own body: db/037 is a whole migration and another object in
    // it may legitimately read `local_node`. The `$$` that closes the function body is the
    // boundary — a whole-file check would fail on code that has nothing to do with this
    // precondition, which is how a guard gets deleted instead of read.
    let end = body.find("$$;").unwrap_or(body.len());
    assert!(
        !body[..end].contains("local_node"),
        "`cairn_register_unwrap_key` now references `local_node`. The restore ceremony \
         registers the inherited custody key while the database is still UN-ENROLLED, so \
         this would make custody impossible to install before the clinical events that need \
         it — every restored sealed body would land without a key, present and unreadable. \
         Fix the ceremony in `main.rs`'s `Cmd::Restore` arm together with the migration."
    );
}

/// **The ceremony's order itself, pinned in `main.rs`.**
///
/// The two guards above prove the preconditions HOLD; this one proves the ceremony still
/// RELIES on them in the order that needs them. A reordering that moved `finalize_identity`
/// back above the clinical apply would leave both preconditions true and the guarantee gone —
/// the failure mode is a restore that fails half-way and leaves a node already identity-minted
/// and already fenced, recoverable only into a fresh database.
///
/// Positional rather than behavioural because the alternative is to stage a mid-restore
/// crash against a live database, and this is the property a reviewer would check by eye.
#[test]
fn finalize_identity_runs_after_the_clinical_apply() {
    let main_rs = repo_root().join("crates/cairn-node/src/main.rs");
    let text = std::fs::read_to_string(&main_rs).expect("reading main.rs");
    let restore_arm = text
        .split("Cmd::Restore {")
        .nth(1)
        .expect("main.rs must have a Cmd::Restore arm");

    let clinical = restore_arm
        .find("apply_clinical_plane")
        .expect("the restore arm must apply the clinical plane — that is #554 itself");
    let finalize = restore_arm
        .find("restore::finalize_identity")
        .expect("the restore arm must mint the new identity");
    assert!(
        clinical < finalize,
        "`finalize_identity` must run AFTER the clinical apply. It writes `local_node`, which \
         permanently fences the self-trusting restore door — so with it first, a clinical \
         apply that fails catastrophically leaves a node already identity-minted and already \
         fenced, whose only recovery is a fresh database. Running it last means the whole \
         restore happens inside the un-enrolled fence and the SAME database can be restored \
         again from the SAME medium."
    );

    let custody = restore_arm
        .find("apply_local_state_export")
        .expect("the restore arm must install the inherited custody key");
    assert!(
        custody < clinical,
        "custody and the actor registry must be installed BEFORE the clinical apply. The \
         apply door wraps each event's DEK to the REGISTERED public half, and it resolves \
         every author through `actor_current` — with this order reversed, a restore admits \
         events it cannot open and refuses this node's own history as unenrolled."
    );
}
