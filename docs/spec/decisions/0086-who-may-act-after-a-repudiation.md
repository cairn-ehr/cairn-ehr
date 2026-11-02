# ADR-0086 — Who may act after a repudiation, and two limits of ADR-0085 accepted

- **Status:** Accepted (refines [ADR-0085](0085-in-person-enrolment-and-re-key.md))
- **Date:** 2026-11-02
- **Spec version at acceptance:** 0.88
- **Issues:** [#926](https://github.com/cairn-ehr/cairn-ehr/issues/926) (who may re-key a repudiated member; may a
  superuser repudiate a superuser — decided), [#924](https://github.com/cairn-ehr/cairn-ehr/issues/924) (an enroller's
  re-key resets the expiry clock — accepted), [#919](https://github.com/cairn-ehr/cairn-ehr/issues/919) (`M > N`
  against paper — accepted). All three were decided by the maintainer on 2026-11-02.
- **Builds on:** [ADR-0085](0085-in-person-enrolment-and-re-key.md) (enrollers, re-key, confirm/repudiate, the
  optional expiry) · [ADR-0083](0083-the-human-key-vault.md) (the vault and its console verbs)

## Context

ADR-0085 left four questions in its "not guaranteed" section, and the floor (db/063) allowed both answers to two of
them:

1. **Re-keying after a repudiation.** A superuser who repudiates a key (the impostor case) is told "give the member a
   new key". Any *enroller* could do that a minute later, so the impostor's handle had a fresh, signing key again
   before any superuser looked.
2. **Repudiating a superuser.** `cairn_vault_judge_key` did not look at the target's role. A superuser could
   repudiate their own key, or the last superuser's, which locked the window out until someone ran
   `vault-rekey-superuser`. Worse, one stolen superuser key could repudiate every other superuser and leave itself
   as the only one.
3. **The expiry clock (#924).** Each unconfirmed key ages from its own introducing row, so an enroller who re-keys an
   unconfirmed person gives them another N days.
4. **More steps than paper (#919).** Enrolment and re-key cost `M = 2` human acts, and change of passphrase and confirm
   cost `M = 1`, against paper's `N = 0`. Rule 7 calls that an architecture defect until it is accepted or removed.

## Decision

1. **Only a superuser may re-key a member whose current key a superuser repudiated.** `cairn_vault_rekey` refuses an
   enroller in that case ("ask a superuser"). The check runs under the actor lock that `cairn_vault_judge_key` takes,
   in `cairn_actor_append`'s key-then-actor order, so a repudiation landing at the same moment is either seen by the
   re-key or refused by the judgement as "not the current key". A key that is only *doubtful* (approved by a
   repudiated enroller) is not covered: an enroller may still re-key that person.
2. **A superuser's key is repudiated only at the node's console.** `cairn_vault_judge_key` refuses to repudiate any
   key whose actor's pinned role is `superuser`, the caller's own included. The owner runs
   `cairn-node vault-repudiate-superuser-key --staff-id <id>` (db/063 `cairn_vault_bootstrap_repudiate`, approved by
   `console`, no token, owner-only like `cairn_vault_bootstrap_rekey`), which repudiates that superuser's *current*
   key and prints it. The remedy is the console re-key, `vault-rekey-superuser`. Confirming a superuser's key was
   already refused: only an unconfirmed or doubtful key is confirmed, and a superuser's key is neither (the
   vault confirms it as it issues it; one enrolled outside the vault is `outside`).
3. **#924 is accepted as a limit, not fixed.** An enroller's re-key restarts the expiry clock for an unconfirmed
   person. Expiry is off by default. A forgotten passphrase at 2 am must stay fixable without a superuser. The new
   key is still unconfirmed and still on the superuser's worklist. So the expiry is a prompt to confirm, not a bound
   an enroller cannot pass.
4. **#919 is accepted.** `M = 2` for enrolment and re-key and `M = 1` for change of passphrase and confirm are the
   price of a signature that is attributably the person's own. They are paid once per person per node, or rarely,
   never per note. The paper they replace in practice is a call to central IT and an afternoon's wait. RUNBOOK §17's
   measured times are still owed.

## What is not guaranteed

- **The console is the only judge of a superuser.** A site whose owner cannot reach the node's console cannot
  repudiate a stolen superuser key at all. This is the same reach `vault-rekey-superuser` already needs.
- **Only the current key.** The console verb, like the window's judgement, repudiates a superuser's *current* key.
  A rotated-away key still needs the door #922 asks for.
- **Only a `{role, handle}` superuser.** The console verb finds the actor from the staff ID, so a superuser enrolled
  from the command line with another pinned shape (a `registration_id`, an un-normalised handle) can be repudiated
  by no door ([#929](https://github.com/cairn-ehr/cairn-ehr/issues/929)).
- **Doubtful is not repudiated.** Decision 1 does not stop an enroller from re-keying a member whose key is doubtful.
- **The expiry bounds nothing against an enroller** (decision 3). A site that needs a hard bound must confirm or
  repudiate from the worklist.

## Consequences

- **Easier.** A superuser's repudiation of an impostor holds until a superuser acts again. No stolen superuser key
  can take the window's superuser role away from the others.
- **Harder.** A repudiated member waits for a superuser, not for any enroller on shift. A stolen superuser key needs
  the owner at the console. Both are rare and both are the point.
- **Pinned by** `crates/cairn-node/tests/vault_rekey.rs` (`rekey_after_repudiation_restores_the_actor`,
  `a_repudiation_landing_mid_rekey_is_seen`, `an_enrollers_rekey_takes_the_row_lock_first`,
  `a_superusers_key_is_repudiated_only_at_the_console` — own key and a fellow superuser's, both ways round),
  `vault_enrollers.rs` (`an_enroller_may_rekey_a_doubtful_member`), `vault_admin_lib.rs`
  (`the_console_repudiates_a_superusers_key`) and `db/tests/063_vault_enrollers_test.sql` (the console door is the
  owner's alone).
