# Sign-in slice 2 — every reader of the actor registry, after rotate-key and repudiate

Task 2 of [the slice 2 plan](2026-11-01-sign-in-slice-2-rekey-and-local-enrolment.md) (design §5.5, §6).
Once a key can be **rotated** (`rotate-key`, same actor) and **repudiated** (the key, not the actor),
"is this key current?" splits into three questions, and each reader must ask the one it means:

- **ADMISSION** — *may this key sign something NEW?* A write door, or a gate that decides whether a
  new or deferred event gets in. Reads `actor_current`. A rotated-away or repudiated key is refused;
  a K1 event that arrives after the rotation is refused or stays deferred. That is the safe answer,
  because when it was actually signed cannot be known.
- **RE-GRADE** — *did this key legitimately belong to the actor, and is it not repudiated?* A reader
  that re-judges an event already in the log. Reads `actor_key_vouches`, so a rotation keeps K1's
  past authority, and a repudiation removes what the repudiated key signed.
- **HISTORY** — *which actor did this key ever mean?* A reader of the raw `actor_event` history.
  Since #666 the key-bearing ops are `enroll` and `rotate-key`, and `supersede` carries no key. So a
  filter `op IN ('enroll','supersede')` silently drops every rotated-in key.

Paper-parity: not clinical-surface — a reference table for one plan task. It classifies the registry readers in the in-DB floor and adds no clinical workflow. The slice's benchmark is in the plan itself.

Walk: `grep -rn 'actor_current\|FROM actor_event\|JOIN actor_event'` over `db/`, `crates/*/src` and
`cairn-gui/`, excluding tests and `target/`. Line numbers are as of the Task 2 commit.

| Site | Class | Verdict | Why |
|---|---|---|---|
| `db/005_submit.sql:1306` `submit_event` step 2 (signer resolution) | ADMISSION | kept | A new event's signer must be current. Pinned: `a_rotated_away_key_cannot_submit`, `a_repudiated_key_cannot_submit`. |
| `db/005_submit.sql:1333` `submit_event` step 4 (attester is a human) | ADMISSION | kept | The attester of a NEW event must be current. |
| `db/005_submit.sql:710` `cairn_suppression_author_ok` (signer was ever human) | HISTORY | **switched** `('enroll','supersede')` → `('enroll','rotate-key')` | A rotated-in key has no enroll row. Without the switch its notes have no human author and anyone can hide them (over-permission on the ADR-0043 floor). A repudiated key stays human here, which is the over-refusal direction. Pinned: `a_rotated_in_keys_note_cannot_be_hidden_by_another_human`. |
| `db/005_submit.sql:1058` `cairn_claim_authority` R1 (attester) | RE-GRADE | **switched** → `actor_key_vouches` | It re-judges a stored withdrawal. `attester_key IS NOT NULL` and `count(*) = 1 AND bool_and(kind = 'human')` are kept, and the arity is unchanged. Pinned: `a_rotated_attester_still_vouches_for_past_claims`, `a_repudiated_key_loses_its_past_authority_but_the_actor_survives`. |
| `db/005_submit.sql:1067` `cairn_claim_authority` R2 (self-withdrawal) | RE-GRADE | **switched** → `actor_key_vouches`, joined on the withdrawal's own `signer_key_id` | The R2 actor must still be human, and the key that signed the withdrawal must still vouch. Then a repudiated impostor key loses its self-withdrawals, and the actor's other keys keep theirs. Before this, a repudiated LATEST key dropped the whole actor from `actor_current`, which demoted the real owner's earlier self-withdrawals too. |
| `db/048_sensitivity_stream.sql:1009` `sensitivity_withdrawal_worklist` (accountable actor) | RE-GRADE | **switched** → `actor_key_vouches` | It must name the same attester R1 accepted. Otherwise a re-keyed attester's strip falls back to the relaying device's actor, and the "stranger-attested" row vanishes. Pinned: `the_worklist_still_names_a_rotated_attester`. |
| `db/006_recall.sql:101` `events_by_actor_epoch` (epoch registrations) | HISTORY | **switched** `('enroll','supersede')` → `('enroll','rotate-key')` | Without it, a recall of (rotated-in key, epoch) selects nothing, which is the dangerous under-selection (#99). The `rotate-key` row copies the actor's pinned set, so the epoch matches. The rotated-away key keeps its enroll row, so its events stay selectable. A repudiated key stays selectable, because a recall over-selects. Pinned: `a_recall_selects_what_both_keys_of_a_rotated_actor_signed`. |
| `db/020_apply_remote_event.sql:183` remote signer gate | ADMISSION | kept | Same as `submit_event` step 2. Pinned: `a_repudiated_key_cannot_submit` (remote arm). |
| `db/020_apply_remote_event.sql:188` remote attribution stamp | HISTORY | **switched** `('enroll','supersede')` → `('enroll','rotate-key')` | Without it, every replicated event by a rotated-in key is stamped `actor_id = NULL`. That loses attribution and breaks R2 `'self'` for it. Pinned: `a_remote_event_by_a_rotated_in_key_is_attributed_to_its_actor`. |
| `db/020_apply_remote_event.sql:265` remote attester gate | ADMISSION | kept | A new event's attester must be current. |
| `db/043_deferred_readjudication.sql:198` deferred gate 1 (attester is a human) | ADMISSION | kept | It decides whether a deferred event is admitted. A K1-attested event re-adjudicated after the rotation stays deferred (safe; it is never dropped). |
| `db/061_vault.sql:322` `cairn_vault_release` (`not-current`) | ADMISSION | kept (comment updated) | A key that cannot sign is not released. |
| `db/061_vault.sql:462` `cairn_vault_enrol` (superuser's key) | ADMISSION | kept | The approving superuser's key must be current for a new act. |
| `db/004_actors.sql` (`cairn_actor_id_key_conflict`, `cairn_key_actor_id_conflict`, `cairn_actor_append`, `cairn_key_confirmation`) | HISTORY | settled in Task 1 | Already `('enroll','rotate-key')` or whole-history. |
| `db/052_restore_doors.sql:158,177` restore door | HISTORY (replay) | Task 3 | The op list widening belongs to the export/restore task. |
| `crates/cairn-node/src/actor_enrolment.rs:161,163` `device_actor_standing` | ADMISSION (+ HISTORY arm) | kept; docs rewritten (#664 items 2–3) | Superseded, rotated-away and repudiated keys all read `Retired`, and the successor reads `Enrolled`. Pinned: `a_superseded_key_classifies_as_retired`, `a_rotated_away_key_classifies_as_retired_and_its_successor_as_enrolled`. |
| `crates/cairn-node/src/enroll.rs:149` `enroll_human_actor` guard 1 | ADMISSION (advisory) | kept | It mirrors the floor for a NEW enrolment, and the db/004 floor refuses a rotated-away key regardless. |
| `crates/cairn-node/src/enroll.rs:100` `determinant_already_claimed` | HISTORY (by actor_id) | kept | It asks whether the actor_id has any row; it does not filter by op. |
| `crates/cairn-node/src/identify.rs:193` `attester_is_enrolled_human` | ADMISSION (advisory) | kept | It previews `submit_event` step 4 for a NEW link. |
| `crates/cairn-node/src/matcher_actor.rs:87` matcher live check | ADMISSION | kept | Only a current key is reused. |
| `crates/cairn-node/src/matcher_actor.rs:99` matcher "ever enrolled" | HISTORY | kept (`op = 'enroll'`) | A matcher key is minted per epoch and is never the target of a `rotate-key`. If one ever were, this would fall through to `enroll_actor`, and the db/004 floor refuses it anyway (a worse message, the same refusal). |
| `crates/cairn-node/src/localstate_read.rs:136` local-state export | HISTORY (all rows) | kept; Task 3 | It exports every row. The new columns are Task 3's. |
| `crates/cairn-node/src/main.rs:3947`, `restore/clinical.rs:286` | — | kept | "Is the registry empty?" checks. |
| `crates/cairn-event/src/contributor.rs:127`, `localstate.rs`, `seed_measurement_corpus.rs` | — | n/a | These only mention it in doc comments. |

**Not changed and worth knowing.** `cairn_suppression_author_ok` builds its human-author set from
KEYS, and the attester must be one of those keys. So after a rotation, Dr A's new key K2 cannot
suppress a note Dr A signed with K1. That is over-refusal, the safe direction, but it costs
paper-parity. Fixing it means comparing actors rather than keys, which is a floor change outside
this task.
