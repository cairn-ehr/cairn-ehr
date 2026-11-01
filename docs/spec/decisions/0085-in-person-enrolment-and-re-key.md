# ADR-0085 — In-person enrolment and re-key: an enroller vouches, a superuser confirms

- **Status:** Accepted
- **Date:** 2026-11-01
- **Spec version at acceptance:** 0.87
- **Amends:** [ADR-0083](0083-the-human-key-vault.md) decision 2 (what recovery is), decision 4 (what
  `not-current` covers) and decision 7 (who may enrol, and the release door's role → purpose map);
  [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md)'s closed actor-event algebra, which gains four
  node-local ops. ADR-0083 is immutable and stays as written: it carries dated errata E1–E3 pointing here.
- **Issues:** [#247](https://github.com/cairn-ehr/cairn-ehr/issues/247) (whose key is this — decided, option (a)),
  [#664](https://github.com/cairn-ehr/cairn-ehr/issues/664) (`rotate-key` and `supersede` in `actor_current` —
  closed), [#666](https://github.com/cairn-ehr/cairn-ehr/issues/666) (which row carries a key — settled),
  [#918](https://github.com/cairn-ehr/cairn-ehr/issues/918) (an attested link of a repudiated key keeps its grade);
  [#919](https://github.com/cairn-ehr/cairn-ehr/issues/919) (M > N against paper);
  [#921](https://github.com/cairn-ehr/cairn-ehr/issues/921) (a repudiated key's past events on restore and sync, and
  repudiation's retroactive reach); [#922](https://github.com/cairn-ehr/cairn-ehr/issues/922) (no path to repudiate a
  rotated-away key); draft PR [#917](https://github.com/cairn-ehr/cairn-ehr/pull/917) (below the
  window) and the stacked window PR. Design, approved by the maintainer on 2026-11-01:
  `docs/superpowers/specs/2026-11-01-sign-in-slice-2-rekey-and-local-enrolment-design.md`; plan:
  `docs/superpowers/plans/2026-11-01-sign-in-slice-2-rekey-and-local-enrolment.md`; every reader of the registry:
  `docs/superpowers/plans/2026-11-01-slice-2-actor-reader-table.md`
- **Builds on:** [ADR-0083](0083-the-human-key-vault.md) (the vault) · [ADR-0084](0084-the-window-never-holds-the-owners-keys.md)
  (the window's own login) · [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) (the key is not a
  determinant) · [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md) /
  [ADR-0046](0046-enroll-fail-closed-on-key-actor-dual-mapping.md) (the whole-history enrol guards) ·
  [ADR-0043](0043-suppression-self-only-disagreement-is-additive.md) (the suppression owner gate) ·
  [ADR-0064](0064-admit-the-claim-withhold-the-power.md) (`cairn_claim_authority`)
- **Code:** `db/004_actors.sql`, `db/005_submit.sql`, `db/006_recall.sql`, `db/020_apply_remote_event.sql`,
  `db/043_deferred_readjudication.sql`, `db/048_sensitivity_stream.sql`, `db/052_restore_doors.sql`,
  `db/061_vault.sql`, `db/063_vault_enrollers.sql`; `crates/cairn-node/src/{vault,vault_admin,actor_enrolment,localstate,localstate_read,main}.rs`;
  `cairn-gui/cairn-gui-tauri/src/{enrol,rekey,reseal,superuser_panel,signer,state,commands}.rs` and
  `src-ui/{signer,enrol,rekey,reseal,superuser}.js`

## Context

Slice 1 (ADR-0083) let only a superuser enrol, and had no way to give a person a new key. Two cases from a Queensland
emergency department, told by the maintainer, show what that costs:

- **A junior doctor's first shift.** He was enrolled, but his first password went to his hospital email, which needs
  a working login to read. Central IT, 400 km away, sent it in the afternoon. Meanwhile: 20 minutes on the phone,
  then work under his supervisor's login, opening each note with "written by Dr X, using Dr Y's login", a sentence that
  was forgotten whenever it got busy.
- **A locum's first night.** A locum is, as a rule, not enrolled on arrival. Shifts start on a Friday night; the
  people who can enrol work office hours. The locum writes on paper, and the stack waits for scanning.

The paper counterpart is a newcomer signing their own name on their first note. Nobody can stop a person who walks in
from producing a paper record; the safety lies in the signature being theirs and in review afterwards. Slice 1
reproduced both failures, and made the shared login **invisible**: the record would say the supervisor wrote it.

Slice 1 also left recovery unbuilt. After a restore the registry comes back with every clinician's old key and the
vault comes back empty, and the only path was to enrol each person under a **new** staff ID, a new actor, losing the
link to what they had signed (ADR-0083 decision 2). A forgotten passphrase had the same answer.

Five questions follow:

1. How does a person get a new key without becoming a new actor?
2. Who may enrol or re-key a person at the terminal, at 2 am, without a superuser?
3. How is an impostor contained, when nobody can be stopped from walking up?
4. Which checks of "is this key current?" must change, now that a key can be retired without its actor?
5. Who does a key in a signed body belong to (#247)?

## Decision

### 1. A re-key is a `rotate-key` row on the SAME actor (db/004)

- The actor registry's ops are now `enroll`, `supersede`, `revoke`, `rotate-key`, `confirm`, `repudiate`,
  `grant-enroller` and `withdraw-enroller`. The last five are new to db/004 (`rotate-key` was already in ADR-0011's
  algebra, but never built). They are **node-local**, like the whole registry,
  until actor-event sync (sign-in slice 5) decides how they travel.
- A per-op shape CHECK, `actor_event_op_shape`, says which row carries what (#666, settled): `enroll` and
  `rotate-key` carry a key and no `superseded_by`; `supersede` carries **no** key and a `superseded_by`; `revoke`
  carries no key; `confirm` and `repudiate` carry the key being judged and an approver; `grant-enroller` and
  `withdraw-enroller` carry no key and an approver. By the convention stated in db/004's header, a `supersede` row
  sits on the OLD actor_id.
- Two new columns, `approved_by` (the approver's hex key, or `'console'`) and `approval` (their attestation token,
  NULL for the console), keep every approval **append-only** on its registry row. The vault's member row is custody
  state that a re-key overwrites, and the cascade (decision 2) must know who approved every key there ever was.
- `actor_current` answers "may this key sign something new?": per actor, the latest `enroll`, `rotate-key` or
  `supersede` row, kept only if it is not a `supersede` (#664: a superseded actor is not current), no `revoke` is at
  or after it, and no `repudiate` names its key. A repudiated latest key leaves the actor with **no** current key: it
  does not fall back to the key the rotation retired.
- `actor_key_vouches` answers "did this key legitimately belong to the actor?": every `enroll` or `rotate-key` row
  whose actor was never revoked and whose key was never repudiated. A rotated-away key stays in it. So do a
  superseded actor's keys: no supersede door exists yet, and the first one owes the decision whether they should.
- `enroll_actor` takes two trailing arguments for the approver and the token. They default to NULL, so a 3-argument
  call still resolves, to the new function; the old 3-argument form is dropped first, so that call is not ambiguous
  between two overloads. `cairn_actor_append` is the only other writer: it appends the five new ops and
  refuses `enroll`, `supersede` and `revoke`. For a `rotate-key` it refuses a revoked or superseded actor (the row
  would outrank the retirement and resurrect it), a key that binds another actor (#166), and a key this actor already
  held (a rotated-away key never comes back), under the same two advisory locks as `enroll_actor`, in the same order.
  For `confirm` and `repudiate` the key must be one the actor holds or held.
- **A clock behind the registry is refused.** `actor_current` and the grant state order by `recorded_at` first, and
  a restore keeps each row's original `recorded_at`. On a replacement machine whose clock is behind the backup, a new
  row would sort before the restored ones: a `rotate-key` would leave the old key current, a withdrawal would change
  nothing, both while appearing to succeed. So `cairn_actor_append` refuses every op when the machine's clock reads
  earlier than the actor's latest row, in a sentence naming both times and saying to set the clock. An equal
  timestamp is allowed: the `seq` tiebreak puts the new row last.
- `enroll_actor` re-run on an actor that has any `rotate-key` or grant row is refused by the existing A-direction
  guard (`cairn_actor_id_key_conflict`): such rows carry another key, or none.

### 2. A key is unconfirmed, confirmed or repudiated, defined in one place

`cairn_key_confirmation(key)` returns one word, and every door and reader asks it:

| Status | Meaning |
|---|---|
| `unknown` | no `enroll` or `rotate-key` row carries this key |
| `repudiated` | a superuser repudiated it (final) |
| `outside` | no row that introduced it names an approver: rows written before this slice, devices, agents |
| `confirmed` | a `confirm` row names it |
| `doubtful` | unconfirmed, and a key that approved it has since been repudiated (the cascade, derived on read) |
| `unconfirmed` | approved by an enroller, not yet confirmed |

- **A superuser's approval confirms at once.** The enrol and re-key doors write the `confirm` row in the same
  transaction, so a key a superuser (or the console) approved is never unconfirmed. An enroller's approval leaves it
  unconfirmed.
- **Unconfirmed never blocks signing by itself.** Only a repudiation or a revoke stops a key from signing. A site's
  own expiry (decision 10) stops the vault from *releasing* it, which is not the same thing. Paper never refuses a
  newcomer's pen.
- **A superuser confirms or repudiates** (`cairn_vault_judge_key`), signed and verified in the database. Confirm
  applies only to an `unconfirmed` or `doubtful` key. Repudiate applies to any person's key not already repudiated, a confirmed
  one included (an impostor found later). **Repudiation is final for the key**: no later confirm undoes it; the remedy
  is a new re-key. The actor is not revoked, so the real person keeps their identity and history and can be re-keyed.
- **The cascade.** When an enroller's key is repudiated, every key it approved that is still unconfirmed reads
  `doubtful`. Keys a superuser already confirmed stay confirmed. No extra act is written.

### 3. An enroller is a grant, not a role, and its acts need a confirmed key (db/061, db/063)

- An actor's role is a pinned determinant: changing it would mint a new actor. So "enroller" is a capability on an
  existing member, granted and withdrawn **only by a superuser** (`cairn_vault_set_enroller`) as an append-only
  `grant-enroller` / `withdraw-enroller` row. The token is signed over the member's current grant state (the `seq` of
  their latest grant row, `cairn_vault_grant_prior`), so a grant token replayed after a withdrawal is refused. A
  superuser cannot hold the grant (they already hold every enroller power), and a no-op grant or withdrawal is
  refused.
- **The approver rule lives in one function**, `cairn_vault_approver_ok`, which every approving door calls. A superuser
  approves if their key is current. Anyone else approves only an enrolment or a re-key, and only while (in this order)
  they hold a current grant, their key is their actor's current key, and that key is `confirmed` or `outside`. That
  last rule blocks chains: an impostor who talks one enroller into re-keying a clinician-enroller cannot then enrol
  or re-key others until a superuser confirms them. The token must be the approver's own valid attestation over the
  request's address.
- **What an enroller may not do:** enrol or re-key a `superuser` or an `admin`; grant or withdraw the role; confirm or
  repudiate anything. Nobody re-keys a superuser from the window: that is the console's
  (`cairn-node vault-rekey-superuser`, `cairn_vault_bootstrap_rekey`, granted to nobody).
- **A new role, `admin`** (non-clinical), for senior local admin staff. Like `superuser`, its key is released for
  administrative acts only. This is a behaviour change: a member enrolled as `admin` before this slice was released
  for `clinical`, and now is not.
- **A clinician enroller keeps one staff ID.** The release door's map (`cairn_vault_purpose_allowed`) is now:
  `reseal` for every role; `superuser` and `admin` for `enrol` only; everyone else for `clinical`, plus `enrol` while
  they pass the enroller rule. What a key signs for an administrative act cannot be replayed as another act or as a
  clinical event, because every administrative act is signed over its own address, each domain-separated by a
  leading tag:
  a re-key binds (staff ID, role, new key); a grant binds (staff ID, grant or withdraw, prior grant `seq`); a
  judgement binds (key, confirm or repudiate); a reseal binds (staff ID, digest of the stored blob, digest of the new
  one).

### 4. In-person enrolment

Slice 1's enrolment, with the approver now a superuser **or** an enroller (`cairn_vault_enrol`). The newcomer, present,
types their own passphrase twice; the key is minted in the window's process and stores only the sealed blob, as
in slice 1. The enroller sees neither the passphrase nor the key. A person on the books with no credential is simply
enrolled: slice 1 has no "enrolled, credential pending" state.

### 5. Re-key in person, and after a restore (db/063)

- `cairn_vault_rekey` takes the approver's token over the re-key address. In one transaction it appends the
  `rotate-key` row (plus a `confirm` row when a superuser approved), replaces the member's key, sealed blob and hint,
  and resets the failure count (the success mark moves up to every attempt counted so far; the counters are sequences
  and never go back, ADR-0083 decision 4). The role given must be the stored one: a re-key never changes the role.
  The old key signs nothing new; what it signed stays the person's (decisions 8 and 9).
- **After a restore the vault is empty and the registry is not.** With no member row, the re-key door re-creates it
  on the existing actor `{role, handle = staff ID}`, which must already be in the registry and be a person's
  (`kind = 'human'`); a display name is then required, and the table's CHECK still requires a phone or an email.
  **Only a superuser (or the console) may approve this branch, never an enroller:** nothing but the `{role, handle}`
  shape ties the staff ID to the actor, and a person enrolled from the command line (`cairn-node enroll`) carries the
  same shape, so an enroller could otherwise take over any such actor. Recovery is: the first superuser re-keyed at
  the console (`vault-rekey-superuser`, confirmed at once), then everyone else re-keyed in person by a superuser. Grants and confirmations live in `actor_event`, which the local-state export carries, so they survive the
  restore. One in-person re-key per person after a disk loss is the price the earlier design already accepted.
- **The window's *Forgotten passphrase* form** carries a display name, a phone and an email. Left blank they keep what a
  surviving member row holds. Before any key is released the window asks whether the staff ID has a vault row
  (`cairn_vault_staff_id_taken`); with none, a form without a name and a contact is refused there, in a plain
  sentence saying to fill them in, so the approver spends no release and no floor message is parsed. The success line
  says whether a superuser must still confirm the key, by who approved it (the grant panel's read lists
  non-superusers only).

### 6. Change my passphrase (db/063)

The current passphrase goes through the **counted release door**, purpose `reseal`, never through a second door that
checks it uncounted. The released key re-seals its own seed under the new passphrase in the window's process, signs
the reseal address as its proof, and is dropped (`vault_admin::change_passphrase`). `cairn_vault_reseal` checks that
proof against the member's own key. The key does not change, no registry row is written, and nothing becomes
unconfirmed. The new passphrase never reaches the database.

### 7. The real owner is told when a sign fails on a replaced key

- When the passphrase is wrong and the member's current key came from a `rotate-key` that is less than 14 days old or
  whose key is still `unconfirmed` or `doubtful`, the release door answers `replaced` instead of `wrong`
  (`cairn_vault_replaced_notice` is the rule's one home). It is **counted exactly like `wrong`** and carries the same
  back-off.
- The window says: *"This passphrase was replaced on {when}, approved by {approver}. If you didn't ask for that, tell
  a superuser."* An approver no longer in the vault reads as "someone no longer enrolled here". Without it the real
  Dr A, typing the passphrase they know after an impostor's re-key, would think they forgot it and burn their tries.
- **On a failure only.** A success proves the signer holds the new passphrase, so on a success the notice would only
  ever reach the person who set it.
- That anyone typing Dr A's staff ID learns of the re-key is a benefit, not a cost: more eyes on a re-key nobody
  asked for (maintainer, 2026-11-01).
- 14 days is this slice's call, long enough to cover a fortnight's roster. It is not a site policy.

### 8. Whose key is this (#247): option (a), resolved when read

- The contributor entry keeps the **signing-key ID** inside the signed body, as now. Nothing is added to the wire and
  no event is rewritten.
- "Who is this?" is answered **when reading**, through `actor_key_vouches` (granted to `cairn_agent`): every key that
  ever legitimately belonged to an actor, the rotated-away ones included, plus the key's status from
  `cairn_key_confirmation`. So Dr A's K1 notes and K2 notes resolve to one person.
- **That is true only where this node's registry is.** Beyond this node the history does not exist until actor-event
  sync (slice 5). Showing the status on each note is slice 3's, with the author display.

### 9. Every reader of the registry asks the question it means

The walk (`docs/superpowers/plans/2026-11-01-slice-2-actor-reader-table.md`) sorted each reader into one of three
questions; each switched reader is pinned by a test that rotates or repudiates a key between the event and the
reading:

- **Local authoring stays on `actor_current`.** `submit_event`'s signer and attester checks (db/005) and the vault's
  doors: a rotated-away or repudiated key authors nothing new here.
- **Re-admission reads `actor_key_vouches`.** `apply_remote_event`'s signer and attester gates (db/020) and the
  deferred re-adjudication (db/043), and with them a restore, which replays the node's own backup through
  `apply_remote_event`. Rotation is not compromise, so an event K1 signed before the re-key still applies after it;
  repudiation is compromise, so a repudiated key is refused. This was a build-time ruling, flagged to the maintainer
  on 2026-11-01. **The residual risk:** a leaked key that was rotated away but NOT repudiated can still bring in
  events through a paired peer or a backup medium. The remedy would be to repudiate it, but neither the window nor
  the console can repudiate a rotated-away key today ([#922](https://github.com/cairn-ehr/cairn-ehr/issues/922)), and
  repudiation reaches backwards: it removes the key's past authority too, the real person's genuine work included
  ([#921](https://github.com/cairn-ehr/cairn-ehr/issues/921)).
- **Re-grading reads `actor_key_vouches`.** `cairn_claim_authority` R1 and R2 (db/005) and the sensitivity-withdrawal
  worklist (db/048): a rotation keeps the key's past authority, a repudiation removes it. R2 joins on the
  withdrawal's own signing key, so a repudiated impostor key loses its self-withdrawals while the owner's other keys
  keep theirs.
- **History readers include `rotate-key`.** The filters that read `op IN ('enroll','supersede')` now read
  `('enroll','rotate-key')`: the recall selector (db/006), the remote attribution stamp (db/020), and the
  suppression owner gate's human-author test (db/005).
- **The suppression owner gate (ADR-0043) now compares people, not keys** (`cairn_suppression_author_ok`): a re-keyed
  Dr A may suppress the note she signed with K1; another clinician may not; a repudiated key may not. A suppressor's
  key must resolve to exactly one vouching actor, or the gate refuses.
- **Not re-graded: an attested `patient_link`.** A repudiated key's links keep their standing; they are shown as
  repudiated only through the author display (slice 3) ([#918](https://github.com/cairn-ehr/cairn-ehr/issues/918)).
- `device_actor_standing` (Rust) reads a superseded, rotated-away or repudiated key as `Retired` (#664's items 2–3).

### 10. A site may make unconfirmed keys expire at the vault (off by default)

- `vault_policy` is a one-row table holding `unconfirmed_expiry_days`, 1 to 36500, or nothing. **The reference
  default is no expiry** (principle 9: Cairn ships the mechanism; the site picks the policy). It is set only at the
  node's console: `cairn-node vault-set-unconfirmed-expiry --days N` or `--none`, on the owner's connection and a
  local one (`require_local`).
- Once a key is still unconfirmed (or doubtful) that many days after the row that introduced it with an approver, the
  release door answers `expired`, for every purpose including `reseal`. It is checked before the attempt is counted,
  like `not-current`. A superuser's confirmation lets the vault release the key again.
- **Expiry stops a release, not a signature.** `submit_event` does not read `vault_policy`. A seed released before
  the stop, or a key the window holds for an act, still signs until it is dropped.
- **Never a surprise at the moment of signing.** The superuser's worklist shows each key's stop date. The signer strip
  warns, within seven days of the stop, when a staff ID is typed and after an act signed with a held key, naming the
  staff ID so the next person on a shared screen never reads it as their own:
  *"Not yet confirmed by a superuser. {staff ID}'s key stops signing on {date}."*

### 11. The window

- **Enrol staff** takes an enroller as the approver and offers the `admin` role. **Forgotten passphrase** re-keys a
  clinician, nurse or admin (decision 5). **Change my passphrase** reseals. **Superuser: enrollers and keys** lists
  every non-superuser member with *Grant* / *Withdraw* and *Repudiate key*, and the unconfirmed and doubtful keys,
  who approved each and when (*Confirm* / *Repudiate*). The member row's *Repudiate key* acts on that member's
  current key, which the row carries (`cairn_vault_member_key`): a confirmed key (an impostor found later) and an
  enroller's own key (which must be confirmed to approve anyone) are never on the worklist, and repudiating an
  enroller's key is what sets off the cascade. A key already repudiated is said so on its row and not offered again.
- The *Enrol staff* and *Forgotten passphrase* forms say, beside their contact fields, that the contact must work
  without any hospital login (a personal mobile or an outside address). A key that is no longer current is told to
  ask a superuser or an enroller for a re-key, never to be enrolled again (which would make a second actor).
- **Repudiate asks no question.** A confirmation dialog fails paper-parity (principle 3) and is not a safety
  mechanism. The act is signed, kept, and repaired by a re-key.
- Each approver's key is released for one act and dropped, never held. Every vault answer that becomes a sentence for
  a person goes through `signer::from_release_noting`, so a replaced key is never told "not right"
  (`a_vault_answer_reaches_the_person_through_from_release_noting`). Every release still goes through
  `vault::release_for_act`, so ADR-0084's over-power check covers the new forms too.

### 12. The doors and their rules change in one transaction

The release and enrol doors call the purpose map, the enroller rule, the approver rule, the `replaced` notice and the
expiry rule, so those are defined in db/061, ahead of the doors, not in db/063. PL/pgSQL binds a called name only
when it runs: a door calling a function that a later file defines would, during an upgrade's replay, answer a
concurrent sign-in with "function does not exist", breaking ADR-0083's promise that the door never raises. This is
why db/061 is long.

## What Cairn cannot verify, and what is not guaranteed

- **Physical presence.** The enroller's signed act is the only evidence that the newcomer stood at the terminal.
  Containment is the unconfirmed status, the `replaced` notice, the superuser's worklist and repudiation, not a check.
- **That a contact is reachable without a hospital login.** The member row still requires a phone or an email. For
  the token re-key to come, it must be reachable without any hospital or Cairn credential (a personal mobile, an
  outside address): the institutional inbox is the very circle in the junior doctor's case. The forms say so;
  Cairn cannot check it.
- **Re-key by token is not built.** Sending a token to that contact, for a clinician off site, waits for an encrypted
  link for thin terminals ([#909](https://github.com/cairn-ehr/cairn-ehr/issues/909)) or slice 5. Until then the only
  usable terminal is the one on the database's machine (ADR-0084), so nobody can re-key from off site anyway.
- **The exports are one-way between builds.** The local-state export now carries `approved_by` and `approval` on
  every registry row, and an older build refuses a row with fields it does not know (`deny_unknown_fields`). An
  export made by this build does not restore on an older one.
- **A restored node has no expiry policy.** `vault_policy`, like every `vault_*` table, is outside Cairn's backups. A
  restore comes back with no expiry, which fails toward availability: every key signs until the site sets its limit
  again.
- **The status is an oracle, accepted as the `replaced` notice and `unknown` already are.** Anyone with the window's
  role can learn whether a staff ID's key is unconfirmed and when it stops (`cairn_vault_unconfirmed_expires_at`, and
  the `expired` answer, given before any guess is counted).
- **A rotated-away key is not a repudiated one** (decision 9's residual). Re-admission accepts it; only a repudiation
  shuts it out — and there is no path in the window or at the console to repudiate a key that is no longer a
  member's current one (the panel acts on the current key, the worklist lists current keys only;
  [#922](https://github.com/cairn-ehr/cairn-ehr/issues/922)).
- **A repudiated key's earlier events do not come back.** A restore or a sync pens every event a repudiated key
  signed, those signed before the repudiation included, as it already pens a revoked actor's. They are missing from a
  restored chart until [#921](https://github.com/cairn-ehr/cairn-ehr/issues/921) is decided.
- **Nothing here travels.** Confirmations, repudiations and grants are local to the facility node until actor-event
  sync (slice 5).
- **The window's DB-gated tests run in no CI job** ([#905](https://github.com/cairn-ehr/cairn-ehr/issues/905)). A
  human runs them, as RUNBOOK §17 says.
- **More steps than paper.** Paper takes no act before a newcomer's first note; Cairn forces two, once per person per
  node (the enroller's form and signature, the newcomer's passphrase typed twice). By rule 7 that is an architecture
  defect, filed as [#919](https://github.com/cairn-ehr/cairn-ehr/issues/919) for the maintainer to accept or close: it buys a signature
  that is attributably the newcomer's, which paper never gives, and it replaces 20 minutes on the phone and an
  afternoon's wait.

## Consequences

- **Easier.** A newcomer signs their own notes within minutes of arriving, under their own name, with no shared login
  and no call to IT. A forgotten passphrase is fixed at the terminal and the person keeps their actor. A restore no
  longer forces new staff IDs. The record never says the supervisor wrote what the junior did.
- **Harder.** "Is this key current?" now has three answers, and a new reader must pick the right one: the reader
  table and `docs/developers/durable-rules.md` say which. The registry has five more ops and two more columns, and the
  restore door's field-by-field check knows them. Thirteen more `SECURITY DEFINER` functions are granted to the
  window's role (one in db/004, two in db/061, ten in db/063); the twelve vault ones are each driven under
  `cairn_window` by `window_role_vault_doors.rs`.
- **What we are betting on, and how we would know it failed.**
  - That an enroller's signed act plus review afterwards contains an impostor as well as paper's review does. We would
    know it failed if a repudiated key were found to have signed for weeks with nobody looking at the worklist; the
    site's optional expiry is the lever.
  - That rotation is not compromise. We would know it failed if a rotated-away, unrepudiated key brought in events
    through a peer or a medium.
  - That 14 days of `replaced` notices, plus the unconfirmed status, are enough for the real owner to notice an
    unasked-for re-key.
- **Owed.** Showing the status per note, and a repudiated author's links as repudiated, is slice 3 (#918). Sealed
  emergency tokens, for a site with no enroller on site at 2 am, are slice 4. Actor-event sync and the token re-key
  are slice 5 (#909).
