# ADR-0108 — The human-key vault's and supervision's doors are the window's role's, never the advisory role's

- **Status:** Accepted (fixes [#1129](https://github.com/cairn-ehr/cairn-ehr/issues/1129))
- **Date:** 2026-11-15
- **Spec version at acceptance:** 0.110
- **Issues:** [#1129](https://github.com/cairn-ehr/cairn-ehr/issues/1129), found by the whole-milestone review
  (`docs/code_reviews/2026-11-15-gui-milestone-review.md`, I1).
- **Supersedes in part:** the grant wording only of [ADR-0083](0083-the-human-key-vault.md) (which names `cairn_agent`
  as the window's runtime role) and of [ADR-0094](0094-supervision-and-the-mandatory-co-sign.md) (which grants
  `cairn_supervision_act`, `_prior` and `_staff` to `cairn_agent`). Every decision of both stands.
- **Builds on:** [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) §1 (`cairn_window` is the window's own role;
  `cairn_agent` is the advisory-actor role, holding the validated submit doors and the projection reads) ·
  [ADR-0053](0053-per-write-human-authorship.md) (an agent never authors as a human) ·
  [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (an in-place db/ edit replays by its content
  hash; `SCHEMA_GENERATION` 82 unchanged)

## Context

The vault (db/061) was written before db/062 gave the window its own role, when `cairn_agent` was the role the window
ran on. Its doors, and slice 2's (db/063), were granted to `cairn_agent`; db/072 followed that pattern for three of its
four supervision doors.

`cairn_agent` is also what every advisory actor's login inherits: the matcher worker (db/017) and any ADR-0030 agent.
So an agent's login could call `cairn_vault_release`, which returns a human's signing seed for a passphrase it has
seen; list every non-superuser staff ID and read any member's passphrase hint; take the terminal lock's Argon2 hash and
guess it offline;
and spend every clinician's tries, holding the whole staff at the 300 s back-off. No signature could be forged by
that, since the floor still checks each act, but the role that must never author as a human held the means to.

## Decision

1. **Every `cairn_vault_*` and `cairn_supervision_*` door the window calls is `cairn_window`'s, and `cairn_agent`
   holds none of them.** The console doors (`cairn_vault_bootstrap`, `_set_terminal_phc`, `_bootstrap_rekey`,
   `_bootstrap_repudiate`, `_set_unconfirmed_expiry`, `cairn_supervision_console_act`) and every internal helper keep no
   grant at all: the owner's alone. The pure functions PUBLIC may execute (the address builders other than
   `cairn_vault_reseal_address`, and helpers such as `cairn_vault_staff_id`) are unchanged.
2. **Each grant is paired with a `REVOKE … FROM cairn_agent`.** A grant persists until revoked, so a node built before
   this fix keeps the old grants until the REVOKE replays. db/061's doors are granted in db/062, because db/061 runs
   before db/062 creates `cairn_window`.
3. **Pinned from the catalogue and by replay:** `advisory_role_holds_no_human_doors.rs` fails if `cairn_agent` can
   execute any function of either family that PUBLIC cannot, and if replaying db/061, db/063 or db/072 over an older
   node's grant leaves it standing. The window still reaches every door it calls (`window_role_vault_doors.rs`).

## Consequences

- A login that is only `cairn_agent` can no longer reach a human's key, the staff list or the back-off. `cairn_window`
  inherits `cairn_agent`, so the window loses nothing.
- No wire change and no event change; db/061, db/062, db/063 and db/072 are edited in place.
- Not covered: `cairn_medication_restore_lineage` (db/080) is still granted to both roles. It is a medication read, the
  kind ADR-0084 gives `cairn_agent`, so it stays.
