# GUI milestone, part 1 — who is at the terminal, who signs the act (design)

**Date:** 2026-10-31 · **Status:** approved in conversation with the maintainer, awaiting a plan ·
**Builds:** [ADR-0008](../../spec/decisions/0008-point-of-care-identity-possession-and-salvage.md) (identity
§5.11) and the deferred half of [ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) ·
**Needs new ADRs:** the human key vault (slice 1) and the emergency key (slice 4) — see *ADRs owed*.

## 1. Why

The GUI milestone (HANDOVER, 2026-10-31) is: enrol a clinician, sign in, find / register a patient, edit
demographics, write a note, record an allergy, chart a medication, sign everything off, sign out. Every one of
those writes needs a human author, and today the only way a window gets one is a key **file path given at
launch** (`cairn-gui-tauri --attester-key <path>`, unsealed by `unlock`, held 15 min — `state.rs`). That works
for one clinician on their own machine. It fails the places Cairn has to work:

- **A consulting room:** one clinician, one machine. Today's model is fine.
- **A ward:** 3–4 clinicians share a machine.
- **An ED:** ~10 nurses share ONE terminal. Any scheme where the next nurse must log the previous one out and
  herself in first means **the documentation does not get done** — the maintainer has watched this happen in most
  systems. The one deployed system that gets this right leaves the record open to every staff member and asks for
  a personal password only to sign an entry. (Name withheld: product-neutrality rule.)

That is the paper model — the chart lies open at the bedside, anyone on staff reads it, every entry carries its
writer's own signature — and it is exactly ADR-0008's split: **gatekeeping** (may this person touch the system?
rare, coarse) is separated from **attribution** (who wrote *this*? per-write, pen-cheap). This design makes that
split concrete for the first runnable window.

## 2. Who is involved

- **The installer** — one designated person: IT, an administrator, or a clinician with IT skills (often the
  clinic owner). Installation makes them the **first superuser**.
- **A superuser** enrols and revokes staff and gets a locked-out clinician working again. In smaller hospitals
  IT is often **centralised and remote**, so a superuser may never be on site. A superuser is frequently **not a
  clinician**.
- **Clinicians** — doctors, nurses, allied health. They sign clinical acts. A clinician who is also a superuser is
  **two role-actors** of one person (the existing `(entity, role)` model, #168).

## 3. Goals, non-goals, anti-goals

**Goals**
1. A shared terminal stays **open for reading**; the next person steps up and writes with **no log-out / log-in**.
2. Every clinical write is signed by its human author with **personal ID + passphrase**, at a cost close to a pen
   signature.
3. A clinician's private signing key **never leaves the facility's node** and is **never in a backup**.
4. A forgotten passphrase is fixed **on the spot**, by remote IT if need be, without anyone but the clinician
   ever holding a key that can sign as them.
5. A partition never blocks a write (availability over consistency): the write lands with an honestly lower
   authorship grade and is attested later by overlay (principle 4). *Interim exception (slices 1–4):* a window
   on the facility node cannot write while its connection is down — today's #807 behaviour; slice 5 lifts it.

**Non-goals (this milestone)**
- A clinician's key following them **across facilities** (roaming between hospitals). Later, its own design.
- Badge / NFC / proximity hardware. ADR-0008 keeps them optional; the plain local credential comes first.
- Printed or electronic prescriptions ("prescribe" here means *charted*, HANDOVER decision 4).
- Multiple warm hidden `(clinician, patient, draft)` contexts on one station (ADR-0008's contention answer).
  Not foreclosed; not built.

**Anti-goals — a build that does any of these has failed even if it works**
- **A superuser can sign as a clinician**, by any route: escrow, a reset that keeps the old key, a key the
  superuser generated or saw.
- **The previous signer's key is still live when the next person writes** on a shared terminal.
- **A confirmation dialog as the safety mechanism** (principle 3).
- **A write refused because authentication is unavailable** — a cut-off terminal, a vault the terminal cannot
  reach. Grade it down; never block it. (The interim exception in goal 5 is the only one, and it is named.)
- **A field satisfiable only by fabrication** — "who's writing" must allow *unknown*, which is ADR-0008's
  `unattributed` (authored at station X, identity unknown — never a guess).

## 4. Constraints (from the canon — do not reopen)

- **ADR-0008 / §5.11:** gatekeeping ≠ attribution; `session.user ≠ event.author`; the grades **attested /
  asserted / unattributed**; the recovery ladder *password → self-recovery (SMS / recovery codes) → audited
  break-glass*, every rung self-service where the deployment allows; `sign-as` salvage; policy chooses which rungs
  exist (principle 9).
- **ADR-0053:** the human SIGNS, the node SEALS and holds the DEK. The strict door's authorship binding
  (`cairn_authorship_bound`, db/005 step 4b) refuses a responsibility-bearing human contributor who is neither
  signer nor verified attester. **Do not weaken it**; every design below fits through it.
- **ADR-0011 §5:** signing publics are immortal; `rotate-key` overlays a new public and keeps the old one for
  verification; the algebra is `enroll / supersede / revoke / suspend / rotate-key`. **db/004 has a door only for
  `enroll`** (`supersede`/`revoke` are admitted by the CHECK, nothing writes them); `rotate-key` and `suspend` are
  specified, unbuilt.
- **The actor registry is NODE-LOCAL.** `actor_event` lives outside `event_log`; there is no actor-sync or
  actor-apply door (db/004's "SINGLE DOOR" note; db/020 "local registry, by design for now"). Both submit doors
  refuse a signer the local registry does not know. Anything that needs one node to recognise a clinician enrolled
  on another needs actor-event sync first.
- **Contributors name a signing KEY id, not an `actor_id`** (#247, `contributor.rs`). So "the same clinician
  across a re-key" is a registry mapping, true only where that registry is.
- **ADR-0026:** the node's private signing key is never backed up, because a stolen backup must not resurrect an
  identity. The same argument applies to a human's key; the vault ADR extends it.
- **ADR-0021:** soft policy lives in the UI; hard policy is DB-anchored. The reference window uses only the
  public API.
- **Product neutrality:** no record-system product is named in tracked docs.

## 5. The design, as decided

### 5.1 The superuser role

- Created by the installer at installation; further superusers are enrolled by a superuser.
- Confers **only** enrol, revoke, issue a re-key token, confirm / repudiate an emergency key's work. It confers
  **no** clinical authorship. A superuser who is also a clinician signs clinical acts as their clinician
  role-actor, never as superuser.
- **Hard policy:** "only a superuser may enrol" is enforced at the floor, not just hidden in the UI — a client
  talking raw SQL must not be able to enrol. *My call:* the floor checks **a superuser actor's signature over the
  enrol request**, verified in the database. A Postgres login role alone would not do: the window's own connection
  would carry it, so anyone with the window's credentials could enrol. Superusers are actors like any other, so
  their keys live in the vault too.
- **Lock-out of the last superuser** is covered by the sealed tokens (§5.6), printed at installation. One
  superuser is therefore acceptable; two is site policy, not a requirement. *(Maintainer accepted this call.)*

### 5.2 Enrolment

1. A superuser enrols a clinician from the window: name, staff ID, role, and **a phone number and/or email**
   (needed by §5.5).
2. The clinician — at a terminal, in person — sets their own passphrase and an optional hint. The facility node
   generates the key pair and stores the private key wrapped under that passphrase. **The superuser sees neither the
   passphrase nor the key.**
3. Enrolment is local to the facility node. ADR-0011 wants it to sync upstream like any other event; that needs
   actor-event sync, which does not exist yet (§4) and is slice 5.

### 5.3 The vault lives on the facility node (option B)

- Clinicians' wrapped private keys are held **only on the facility's node**. They are not replicated to
  workstation nodes, not copied to peers, **not written to any backup**. *(Maintainer chose B over replicating
  wrapped keys to every node: each replicated copy is an offline-crackable target, and clinicians' passphrases are
  weak.)*
- A terminal asks the facility node to sign. The private key is unwrapped there, used, and wiped.
- **Interim (slices 1–4): a terminal is a WINDOW on the facility node's database**, as today's window holds one
  connection. One registry, one DEK custodian, one door — none of the multi-node questions arise. A terminal that
  loses that connection is today's lost-connection case (#807/#875: it says so and reopens); it cannot write while
  cut off. *(Maintainer accepted this as the interim, 2026-10-31.)*
- **Eventually (slice 5): terminals are fully autonomous nodes that sync credentials with SPECIFIED nodes**
  (maintainer, 2026-10-31). That slice needs actor-event sync, decides which node seals and holds custody for a
  human-signed event written at a terminal node, and brings the cut-off path: a terminal that cannot reach the
  facility node still writes, signed by the terminal's node, naming the clinician in a **contributory** role,
  graded **asserted** (§5.7), attested later by overlay. Wrapped keys still stay on the facility node — "sync
  credentials" means the registry (who is enrolled, which public key), never the private keys, unless a later ADR
  decides otherwise against ADR-0026's argument.
- **After disaster recovery** the restored node has no keys: every clinician re-keys by token (§5.5). Their actor
  identity survives (`rotate-key` keeps `actor_id`); only the key is new. The cost is one token per clinician after
  a disk loss.
- In a single-machine clinic the facility node IS the terminal, so all of this is local and costs nothing extra.
- **Online brute force:** the vault limits passphrase attempts per actor (at a shared terminal anyone can try).
  *My call:* it is a growing **back-off**, never a hard lock-out — a lock-out with no remedy fails the anti-goals,
  and the remedy (§5.5) is not built until slice 2. The limit and the back-off curve are the builder's.

### 5.4 The terminal and the act of signing

- **Terminal unlock:** a shared staff passphrase, screensaver-style. After it: patient search, select, read —
  open to any staff member. Reads are attributed to the **terminal** (plus whoever claimed the chart, slice 3).
- **Signing an act:** the clinician types their staff ID (or picks from this terminal's recent signers) and their
  personal passphrase. ID + passphrase together means passphrases never need to be unique, so nothing can leak
  "that password is in use". The facility node signs; the event is **attested**.
- **How long a key stays live after signing is a per-terminal setting** ("key hold"):
  - **shared terminal → 0:** the key is wiped after every act. The next person cannot write as the last one.
  - **personal workstation → up to today's 15 min idle** (`IDLE_TIMEOUT`, `state.rs`), preserving #288's
    whole-list sign-off as one act on a machine only one person uses.
  - *My call:* **the default is shared (0)**. A wrongly-shared personal machine costs extra passphrase entries;
    a wrongly-personal shared terminal costs a mis-attributed signature. Whole-list sign-off stays ONE gesture
    either way — one Sign press signs the whole list — it just carries the credential.
- **Passphrase hints** — *my call:* optional, written by the clinician, shown only after their ID is entered, with
  a warning at setting time that colleagues at a shared terminal can read it.

### 5.5 A forgotten passphrase — re-key by token (normal path)

1. The clinician rings the number on screen / in the manual; IT (a superuser, possibly remote) confirms who they
   are talking to.
2. IT issues a **one-time token bound to that one named clinician**, short-lived, single-use, with very few
   attempts. It is **sent by SMS or email** to the contact recorded at enrolment, so only someone holding that
   clinician's phone or inbox can use it.
3. At any terminal the clinician picks their name → "I have a reset token" → enters it → sets a new passphrase.
4. The facility node generates the new key itself (`rotate-key`). The old key can author **nothing new** (#664);
   what it already signed stays valid and resolves to the same clinician **through the facility node's registry**
   (#247). Beyond that node, continuity needs actor-event sync (slice 5).
5. The ledger records two acts: IT authorised the re-key; the clinician completed it. The clinician sees "your
   key was replaced on … by …" at their next sign, so a re-key they did not ask for is noticed.
- **SMS / email gateways are pluggable and optional** (no mandatory cloud). A site without one falls back to IT
  reading the token out — weaker against phone social engineering, which is why it is the fallback.
- The authorisation must reach the facility node before the token works. When it cannot (partition), §5.6.

### 5.6 Sealed emergency tokens and the emergency key (partition path)

- A superuser generates *n* one-time tokens in advance; the node stores only their hashes, so it can check one with
  no link to anyone. They are printed and kept **under seal** (an envelope in the drug safe). Installation prints
  the first sheet.
- A sealed token cannot be bound to a name in advance, so whoever opens the envelope could claim to be anyone. To
  contain that, a sealed token issues a **temporary extra key**, never a replacement:
  - it **expires** at a fixed point (e.g. 24 h / end of shift — exact value in the ADR);
  - everything it signs is graded **"emergency key, identity not yet confirmed"**;
  - the clinician's real key is **untouched**, so the token cannot lock the real clinician out.
- Using one is **break-glass**: audited and loud afterwards. When the link is back, the clinician re-keys properly
  (§5.5) and a superuser **confirms or repudiates** the emergency key's work. If it was an impostor, every event it
  signed is already flagged and is repaired by overlay.
- A sealed token also recovers a locked-out superuser.

### 5.7 The `asserted` grade

- **No wire change is needed.** `drafted` is already a contributory role (db/005 `contributor_role`). An event
  signed by the node with contributors `[{clinician, "drafted"}, {node, "recorded"}]` passes the strict door's
  authorship binding today.
- **The gap is on the read side:** `classify_authorship_confidence` (`cairn-event/src/contributor.rs`) grades that
  event `Device`, losing the name. It gains an **`Asserted`** variant: no responsibility-bearing contributor, but a
  named **human** in a contributory role. It is Rust read-side only: no SQL function grades `contributors` this way
  (`cairn_claim_authority` reads disjoint inputs), so there is no SQL twin to keep in lockstep.
- *My call:* "human" is a registry fact, not a role convention — `drafted` may also name an AI scribe. To stay a
  pure function, the classifier takes the set of key ids the caller has resolved to `kind = 'human'` actors as an
  input. The contributor names the clinician by key id, as every contributor does today (#247).
- **First produced by** the abandoned draft (slice 3); later by a cut-off terminal node (slice 5). So it is built
  in slice 3, **together with a display read path** — a grade no clinician can see is not an honest lower grade
  (#245's display half is still open).
- *My call:* **an attestation act is never `asserted`.** A whole-list sign-off is the act of vouching; without a
  key it is withheld, never recorded as an unsigned sign-off. Statements (assert, cease, dose change, a note) may
  be `asserted`.
- `info_source` is untouched — it is free text at the floor and records where the information came from, not who
  charted it; "charted by" is authorship, already on the event (HANDOVER sub-question (f): display only).

### 5.8 Drafts and chart opens (slice 3, recorded here because it was decided)

- Starting a draft asks **"who's writing?"** — a quick pick, **no passphrase** — with *unknown* allowed.
- A draft abandoned past the quarantine timer commits as **recorded, not attested, author asserted** (HANDOVER
  decision 3) — or `unattributed` if the writer was left unknown. Attesting it later is an overlay.
- Opening a chart asks the same passphrase-free **"who's opening?"**, which gives the #797 rule ("tell the
  clinician only what changed since they took the chart") a person to measure against. **Site policy** may require
  ID + passphrase to open a chart; *my call:* the default is no — opening charts is ED's most frequent act.
- Break-glass on a sealed sensitive episode (§5.9's C1, when it lands) needs personal ID + passphrase, like a
  write: reads are terminal-attributed, break-glass must name a human.

## 6. Slices, in order

| # | Slice | Contents |
|---|---|---|
| 1 | **Sign-in** | terminals are windows on the facility node; superuser role (signature-gated enrol at the floor); installation creates superuser #1 and prints sealed tokens; window enrolment; vault on the facility node; shared-staff terminal unlock; per-act ID + passphrase signing for the existing medication writes; key-hold setting; attempt back-off. Replaces `--attester-key`. |
| 2 | Re-key | `rotate-key` (and `suspend`) in the actor algebra, fixing **#247** and **#664** first; token re-key; SMS/email gateway seam. |
| 3 | Who's writing / opening | passphrase-free claim; durable draft store; auto-commit after the quarantine timer; the `Asserted` grade + its display; chart-open claim + site policy. |
| 4 | Emergency key | its own ADR; sealed-token use; time-bounded key; confirm / repudiate. |
| 5 | Autonomous terminal nodes | actor-event sync with specified nodes (registry, never private keys); custody of a terminal-written event; the cut-off path graded `asserted`. Needs a brainstorm of its own. |
| — | Then the rest of the milestone | wire `cairn-gui-shell`'s tabs → demographics edit → allergy stream → add a medication from the window → thin encounter + real note stream → one encounter sign-off. Each brainstormed on its own. |

*My call on the order:* re-key (2) comes before drafts (3) because slice 1 alone has no remedy for a forgotten
passphrase or a disk loss except a new actor, which breaks attribution continuity. **No real clinical use before
slice 2** (the project is pre-clinical, so this costs nothing today).

Slice 1 ships sealed tokens **printed and stored** but not yet **usable** — using one is slice 4. Until then a
locked-out last superuser is recovered at the node's CLI (a superuser-enrol verb beside today's `enroll-human`;
an operator at the node's console is already trusted with its database). Say so on the sheet.

## 7. Paper-parity benchmark (§1.2)

- **Paper counterpart:** the bedside chart — open to any staff member, each entry signed by its writer.
- **Steps:** paper N = 1 to sign an entry (a signature). Architecture M = 1 (the Sign gesture carrying ID +
  passphrase). Switching to the next nurse: paper 0, architecture 0 (nothing is logged in to switch away from).
  Terminal unlock is a gatekeeping act with no per-write cost.
- **Time + cognitive load:** sign ≤ 3 s from pressing Sign to signed (ID + passphrase), against a pen signature of
  ~2 s; next-person write needs 0 extra acts. A remote re-key ≤ 2 min from the call to signing again. Measurement
  is owed by slice 1 (the first runnable surface), into the RUNBOOK.

## 8. ADRs owed

1. **The human key vault** (with slice 1): keys only on the facility node, never backed up, re-key after DR —
   the human counterpart to ADR-0026; the superuser role and what it may not do; the key-hold setting. It is the
   natural home for the human key-loss ceremony security §7.5 names as an honest gap.
2. **The emergency key** (with slice 4): sealed tokens, time-bounded extra key, its grade, confirm / repudiate.

## 9. Left to the builder

- **Where signing physically runs on the facility node** (in-DB via `cairn_pgx`, or the node's Rust daemon).
  Constraints: the private key never leaves the
  facility node; **the passphrase is never logged** (Postgres logs bind parameters on error unless told not to);
  the floor still verifies every signature it is handed.
- The key-wrapping construction (reuse `cairn_node::keystore`'s sealing rather than a second one).
- The attempt limit and back-off; token length, alphabet and lifetime (short enough to read aloud, few attempts).
- Screen layouts — the existing window's visual language.
- Whether the shared staff passphrase is one per site or per terminal.
