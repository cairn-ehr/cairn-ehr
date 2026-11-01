# GUI sign-in slice 2 — in-person enrolment, re-key, change of passphrase (design)

**Date:** 2026-11-01 · **Status:** design, APPROVED by the maintainer 2026-11-01 (§9 records the two decisions taken after the builder check) · **Builds on:** the sign-in design
[`2026-10-31-gui-sign-in-and-authorship-design.md`](2026-10-31-gui-sign-in-and-authorship-design.md) (§5.1, §5.2,
§5.5 and the slice table), [ADR-0083](../../spec/decisions/0083-the-human-key-vault.md) (the vault, `db/061`),
[ADR-0084](../../spec/decisions/0084-the-window-never-holds-the-owners-keys.md) (the window's own login, `db/062`),
ADR-0011 §5 (the actor algebra), ADR-0007/0008 (compositional authorship). Fixes **#247** and **#664** first.

## 1. Why — the failure this slice exists to remove

Told by the maintainer (2026-11-01), both cases common in a Queensland ED:

- **A junior doctor's first shift.** He was enrolled, but his initial password was sent by internal email, which
  needs a working login to read: a **circular impossibility** that happens often. Central IT, 400 km away, issued it
  in the afternoon. In the meantime: 20 minutes lost on the phone while patients piled up; then he worked under his
  supervisor's login, opening every note with *"written by Dr X, using Dr Herb's login"*, a sentence forgotten
  whenever it got busy; then the supervisor lost more time cycling his own credentials.
- **A locum's first night.** It is the RULE, not the exception, that a locum is not enrolled on arrival. Shifts
  often start on a Friday night; the staff who can enrol work office hours. The locum writes on paper, and the stack
  waits for scanning, which helps nobody.

**Paper counterpart:** the newcomer signs their own name on the first note. Nobody can stop a person who walks in
from producing a paper record; the safety lies in the signature being theirs and in review afterwards. Cairn as
designed (enrolment by superuser only; re-key by a token from IT, sent to a contact) reproduces both failures, and
worse, makes the login sharing **invisible**: the record would say the supervisor wrote it.

## 2. Goals

1. **In-person enrolment by an *enroller*.** A role a superuser assigns — to clinicians or to senior local admin
   staff. With the newcomer physically present at the terminal, an enroller can enrol a person not known to the
   system at all, or give a working credential to one who is enrolled but has none.
2. **Re-key in person** (a forgotten passphrase): an enroller or a superuser, with the clinician present, issues a
   new key on the **same actor** (`rotate-key`). The old key signs nothing new; everything it signed stays valid
   and resolves to the same person.
3. **Unconfirmed until a superuser confirms.** Whatever an enroller issues — an enrolment or a re-key — is
   *unconfirmed*. It **never blocks signing by itself**. A superuser confirms or repudiates it later.
4. **Change my passphrase** (the clinician knows the current one): re-seal the same key under a new passphrase.
5. **Re-key after a restore** (owed to ADR-0083, which names it slice 2's job): the registry comes back, the vault
   comes back empty, so every clinician keeps their actor and gets a new key (§5.8).
6. **#247 decided and recorded; #664 closed; #666's supersede convention settled with it.**

## 3. Non-goals (this slice)

- **The remote token path (§5.5) and its SMS/email gateway.** Until #909 or slice 5 lands, the only usable
  terminal is on the database's machine (ADR-0084), so nobody can re-key from off site anyway. It waits for them.
  When it comes, its contact must be reachable **without any hospital or Cairn credential** (a personal mobile, an
  outside address): the institutional inbox is exactly the circular failure above.
- **`suspend`.** Nothing in the cases needs it.
- **Sealed emergency tokens** (slice 4) — the remaining answer for "no enroller on site at 2 am".
- **Actor-event sync** (slice 5). Everything here is local to the facility node, as in slice 1.

## 4. Anti-goals — failure even if it technically works

- **Anything that stops a newcomer signing before IT acts** (pushes them back to paper or to a shared login).
- **Anything that lets a person's work land under another person's name.**
- **A confirmation dialog as the safety mechanism** (CLAUDE.md principle 3). The safety is: the newcomer's own key,
  the enroller's signed act, the visible unconfirmed status, and repudiation by overlay.
- **Weakening slice 1's promises:** the enroller sees neither the newcomer's passphrase nor their key; the sealed
  blob never leaves the database; every guess counts where a rollback cannot undo it (ADR-0083 promise 4); the
  strict door's authorship binding (db/005 step 4b) is untouched.

## 5. The design, as decided

### 5.1 The enroller role

- **A grant, not an actor role.** An actor's `role` is a pinned determinant (`{role, handle}` → `actor_id`), so
  changing it would mint a new actor. "Enroller" is therefore a separate capability held by an existing member,
  granted and withdrawn **only by a superuser**, as a signed, append-only act. *(Builder's call: the shape of the
  grant record.)*
- **Who may hold it:** a clinician member, or a non-clinical admin member. *My call:* a new member role **`admin`**
  (non-clinical). Like `superuser`, it releases its key only for administrative acts, never for a clinical event —
  extend the release door's role → purpose map (db/061 promise 3), do not bypass it.
- **A clinician enroller keeps ONE staff ID.** *My call:* the clinician's own key may also be released for the
  administrative purpose when they hold the grant. Two staff IDs per consultant (the superuser pattern) is friction
  at 2 am, and the act an enroller signs is domain-separated from a clinical event (its own address, as
  `cairn_vault_enrol_address` is), so the key cannot be tricked into signing one for the other.
- **Hard policy, in the floor:** every enroller act is accepted only with a valid signature by a member who holds a
  current grant, verified in the database (the slice-1 pattern for superuser enrolment). Hiding a button is not
  enough.
- **What an enroller may NOT do** *(my calls)*: grant or withdraw the enroller role; enrol or re-key a `superuser`
  or an `admin`; confirm or repudiate anything. A superuser keeps all of those.
- **An enroller act needs a CONFIRMED key** *(my call, enforced in the floor)*. Otherwise an impostor who talks
  one enroller into re-keying a clinician-enroller could enrol and re-key others, a chain the cascade (§5.5) only
  catches once someone repudiates its root. Cost: an enroller who was themselves re-keyed by another enroller
  cannot enrol until a superuser confirms them.

### 5.2 In-person enrolment

As slice 1's §5.2 — the enroller enters name, staff ID, role and a contact; the newcomer, present, sets their own
passphrase and optional hint; the node generates the key and seals it — with these differences:

- The approving signature is the **enroller's** (or a superuser's, as today).
- An enrolment approved by an enroller is **unconfirmed** (§5.5). One approved by a superuser is confirmed at once.
- **The pre-enrolled case** (a person on the books with no credential): *my call* — for slice 2 this is simply an
  enrolment. Slice 1 has no "enrolled, credential pending" state (the passphrase is set in person), so the only
  way to be in the junior doctor's position is to be **not yet in this vault**, which an enrolment covers. If a
  future pre-enrolment feed (HR, central IT) creates members without a passphrase, it reuses §5.3's act.

### 5.3 Re-key in person (forgotten passphrase)

1. The enroller (or a superuser) and the clinician are at the terminal. The enroller picks the clinician's
   member record and signs the re-key act.
2. The clinician sets a new passphrase (and optional hint). The node generates a **new key pair** and seals it.
3. In ONE transaction: the vault member's sealed key and signing key are replaced (custody state, mutable — as a
   key file always was, ADR-0083), and an append-only **`rotate-key`** row is written to `actor_event` on the
   **same `actor_id`**, carrying the new signing key. The member's failure count is reset *(builder's call how)*.
   **The approver's signed act must be kept APPEND-ONLY**, not on the member row: slice 1 keeps an enrolment's
   approval only in `vault_member.enrol_token`, which a re-key overwrites, and the cascade (§5.5) needs to know who
   approved every key. *(Builder's call where — a new `actor_event` column, which then fixes the row shape slice 5
   syncs and must be taught to the restore door's field-by-field comparison (db/052), or a sibling append-only
   table.)*
4. The old key can sign **nothing new**; what it signed stays valid and resolves to the same actor (§5.6).
5. **A re-key by an enroller is unconfirmed (§5.5) — the key, not just the person.** A re-key is more dangerous
   than a fresh enrolment: an impostor who talks an enroller into re-keying "Dr A" would sign as an established,
   confirmed clinician. Two defences: the new key is unconfirmed until a superuser confirms it, and the real Dr A
   is told. **At their next sign, success or not**: a wrong passphrase on a member whose current key came from a
   re-key still unconfirmed answers **"this passphrase was replaced on <date>, approved by <approver> — if you did
   not ask for that, tell a superuser"**, not a bare "wrong" (which would make Dr A think they forgot it, and burn
   their tries into the back-off). That anyone typing Dr A's staff ID learns of the re-key is **a benefit, not a
   cost** — more eyes on an unasked-for re-key *(maintainer, 2026-11-01)*. *Builder's call:* the release door's
   new outcome, still counted like a wrong guess (promise 4).
- *My call:* a superuser's own re-key is not offered here (a locked-out superuser is the console's
  `vault-bootstrap-superuser` path or slice 4's sealed token).

### 5.4 Change my passphrase

- The clinician, at the terminal, gives staff ID, current passphrase and a new one. The current passphrase goes
  through the **release door** (counted, backed off — ADR-0083 promise 4; the change is never a second door that
  checks a passphrase uncounted). The same seed is re-sealed under the new passphrase; the key does not change.
- No approver, no registry row: it is custody state only, like re-sealing a key file. Not unconfirmed. *(My call.)*
- Slice 1's passphrase rules (an empty passphrase never reaches a counting door, etc.) apply to the new one.

### 5.5 Unconfirmed, confirmed, repudiated

- **Status belongs to a KEY** (an enrolment's first key, or a re-key's new key): `unconfirmed` if an enroller
  approved it, else `confirmed`.
- **Unconfirmed never blocks signing by itself** (anti-goal 1). This matches paper (maintainer, 2026-11-01).
- **A superuser confirms or repudiates**, each a signed, append-only act verified in the floor.
  - **Confirm:** the key stops being shown as unconfirmed. Nothing else changes.
  - **Repudiate:** the key can sign **nothing new**, and everything it signed is shown as repudiated — repaired by
    overlay, **never erased** (principle 2). If it was the actor's latest key (an impostor's re-key of Dr A), the
    actor has no usable key until a proper re-key; the actor itself is NOT revoked, so the real Dr A keeps their
    identity and history. *(Builder's call: the mechanism, e.g. new `actor_event` ops beside a widened CHECK, or a
    sibling append-only table; it must be append-only, node-local like `actor_event`, and every "is this key
    current" check must honour it.)*
  - **Repudiation lowers what the floor already granted** *(my call, the consequence of "everything it signed is
    shown as repudiated")*: a reader that re-judges a stored event asks "was the key current WHEN RECORDED **and is
    it not repudiated**". So a rotation keeps K1's legitimate work, and a repudiation demotes an impostor's
    sensitivity withdrawals (`cairn_claim_authority`, db/005, the only re-grade of an attested act). An
    attested `patient_link` is **not** re-graded: a repudiated key's links are shown as repudiated only
    through the author display (slice 3), not demoted *(corrected during the build, Task 2)*.
- **Cascade:** if a superuser repudiates an **enroller's** key, every still-unconfirmed key that enroller approved
  is shown as **doubtful** — derived when read, no extra acts. Keys a superuser already confirmed stay confirmed.
- **A superuser's worklist** in the window lists every unconfirmed key: who, approved by whom, when (and on which
  terminal, if the window knows one — builder's call), with Confirm / Repudiate.
- **Optional site expiry (hard policy, mechanism not policy — principle 9).** A site may set a limit after which an
  unconfirmed key stops signing. **The reference default is no expiry.** A site that sets one shows a warning on
  screen days ahead, never a surprise at the moment of signing.

### 5.6 #247 — who a key belongs to (decided: option (a))

- The contributor entry keeps the **signing-key id** inside the signed body, as now. Nothing is added to the wire;
  no event is rewritten.
- "Who is this" is answered **when reading**, by looking the key up in the node's `actor_event` HISTORY (every
  `enroll` / `rotate-key` row ever written for the actor, not only the current one), plus the key's status (§5.5).
  One read-side function does this; the #245 grader and any author display use it.
- **Beyond this node** that history does not exist until actor-event sync (slice 5). Say so in the ADR.

### 5.7 #664 — closed

- `rotate-key` is its own op on the **same** `actor_id`; once `actor_current` reads it, latest-row-wins makes the
  old key non-current. That needs FOUR widenings in db/004/db/052, each a test: the op CHECK (db/004:14), the view's
  op list (db/004:64), the one-key-one-actor check (db/004:127 — or a rotated-in key could be enrolled to a second
  actor, the #166 hole), and the restore door's op list (db/052:176-181).
- `supersede` (a **new** actor replacing an old one) still has no door, but `actor_current` must stop treating a
  superseded actor as current *(the one-line `NOT EXISTS` beside the revoke one)*, so the hole is closed before any
  writer exists. `ActorStanding`'s three-way classification (`actor_enrolment.rs`) is revisited in the same change,
  with a test for a superseded key, per #664's items 2–3. The `NOT EXISTS` is right only if a supersede row sits
  on the OLD `actor_id`, which #666 says is still unsettled (and db/052 already replays supersede rows); so this
  slice settles #666's convention and adds its per-op CHECK.

## 6. Constraints and risks the builder must handle

- **⚠️ "Current" is asked after the fact in several places.** `actor_current` is read by db/005 (submit, and
  `cairn_claim_authority` at db/005:1046/1053, which re-grades stored events), db/020 (remote apply), db/043
  (deferred re-adjudication), db/048, db/061's release door (061:321), and Rust (`identify.rs`, `localstate*.rs`,
  `restore/clinical.rs`, `contributor.rs`, …); db/006 reads the registry's history on purpose and db/052 only
  mentions the view. Re-list them with `grep` when planning. After a re-key, any path that
  re-checks an OLD event's signer against `actor_current` would refuse an event K1 signed legitimately before the
  rotation. Walk every reader: a write door asks "is the signer current NOW" (correct — refuse new K1 events); a
  path re-judging a stored event must ask "was it current WHEN RECORDED". Pin each with a test that rotates a key
  between the event and the re-judgement.
- **A repudiated key must fail every write door,** not just the window's — the same walk.
- **Pinned-count guards:** a new actor op or table, new definer doors, new grants each have their registries
  (durable-rules; memory notes on pinned counts and `db/062`'s `cairn_window` grant walk, which must reach each
  new read's deep path, not run it on empty data).
- **ADR-0084's posture check** must still hold: no new grant gives `cairn_window` an owner power; each new door is
  `SECURITY DEFINER` with `SET search_path = public, pg_temp`.
- **The enroller's act is the only proof of physical presence** — Cairn cannot verify presence. Say so in the ADR;
  containment is §5.5.
- **The contact field.** Slice 1's CHECK still requires a phone or email. The form says it must be reachable
  without a hospital login; Cairn cannot verify that either.
- **One ADR** records: the enroller role and its limits, unconfirmed/confirm/repudiate and the cascade, rotate-key
  as an op on the same actor, #247 option (a), #664's view change, and the optional expiry.

### 5.8 Re-key after a restore

- After a restore the registry holds every actor with their lost key; the vault is empty (ADR-0083). So the
  re-key door works **from the registry when no member row exists**, creating the member row.
- **Who approves when nobody can sign:** *my call* — the first superuser re-keys through an owner-only console verb
  (the authority of `vault-bootstrap-superuser`, which today refuses an existing staff ID), then superusers and
  enrollers re-key everyone else in person (§5.3). Cost: one in-person re-key per person after a disk loss, the
  price the earlier design already accepted.
- Enroller grants and confirmations must survive a restore (keep them out of the excluded `vault_*` tables) —
  otherwise every re-keyed key comes back unconfirmed and no enroller exists. *(Builder: check this against the
  restore path.)*

## 7. Paper-parity benchmark (owed by the plan; targets here)

- **Paper counterpart:** a newcomer signs their own name on their first note — 1 act, ~5 s; no other person
  involved unless policy wants a countersignature.
- **In-person enrolment:** enroller enters 4 fields + signs; newcomer types a passphrase twice. Target **≤ 2 min**
  from "I'm new" to the newcomer's first own signature — against today's 20 min on the phone and an afternoon's
  wait. Step count against paper is necessarily higher (`M > N`): the enrolment is a one-off per person per node,
  not per note; the plan must state this and file it per rule 5 if it judges it an architecture defect.
- **Re-key in person:** **≤ 1 min.** **Change my passphrase:** **≤ 30 s.** Superuser confirm: **≤ 10 s per row.**

## 8. Left to the builder

The grant record's shape; the confirm/repudiate mechanism (§5.5); how the failure count resets on a re-key; where
the expiry setting lives; the window's layout for the four new surfaces (enrol-by-enroller reuses slice 1's "Enrol
staff"); the slice split, if the plan judges it too big for one PR.

## 9. Decided after the builder check (maintainer, 2026-11-01)

- **Telling Dr A on a failed sign:** yes, as §5.3 says; the disclosure is a benefit.
- **Where "unconfirmed" is shown: slice 3,** with the author display and the `Asserted` grade. Slice 2 delivers the
  stored status and its read-side resolver (§5.6), the superuser's worklist (§5.5) and the "replaced" notice
  (§5.3) — nothing per note.
