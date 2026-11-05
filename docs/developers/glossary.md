# Glossary

Cairn-specific terms a newcomer meets in the code and docs, in plain language. For the authoritative
treatment of any concept, follow the link into the [spec](../spec/index.md) or the
[ADR log](../spec/decisions/README.md).

---

### Append-only event log
The core store. Clinical content is recorded as **immutable, signed events** that are only ever
*added*, never updated or deleted in place. Corrections and identity repairs are *new* events
referencing the originals. See [principle 1](../spec/index.md) and the [data model](../spec/data-model.md).

### Projection
A queryable "current state" table (e.g. `patient_identifier`, `patient_name`, the medication tables,
`clinical_note`) that is **derived** from the event log by each event type's registered applier.
Projections are caches — rebuildable from the log at any time (`cairn-node reproject`). You never
write a projection directly; you submit an event and the projection updates.

### The floor (in-DB enforcement floor)
The safety-critical validation that lives **inside PostgreSQL** — the validated `submit_event` door,
row-level security, constraints, and projection triggers (`db/*.sql`, layer 2). It is **unbypassable**:
even a client connecting with raw SQL cannot break the record. Proven against a hostile agent in
[Spike 0002](../spikes/0002-advisory-actor-write-contract.md).
See [Architecture for developers](architecture-for-developers.md#3-the-four-layer-model).

### `submit_event` — the strict door
The validated write door for events **authored on this node** (`db/005_submit.sql`). It verifies the
signature (via the in-DB `cairn_verify`), runs the per-type structural floor checks through a
registry, and appends to `event_log`. It **fails closed**: an unknown type, or a chart whose first
event is not its registration, is refused. See
[ADR-0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md).

### `apply_remote_event` — the remote door
The door for events **learned from a peer or a backup medium** (`db/020`). It admits what an honest
peer could legitimately send even when this node cannot interpret it — an unknown type is stored
uninterpreted ([ADR-0056](../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md)), a
clinical event may precede its registration — and **grades** claims it cannot check rather than
refusing them. Only the node's own role may call it. The two doors are the *only* ways into
`event_log`. See [Architecture §5](architecture-for-developers.md#5-two-doors-two-planes).

### Quarantine (sync) / re-adjudication
An event the remote door cannot yet verify — typically because the key that signed it has not
arrived — is held in a durable quarantine (`db/021`, `db/022`) and **re-adjudicated** when what it
was waiting for lands (`db/043`; `cairn-sync`'s `requeue`). Nothing valid is silently dropped
([ADR-0070](../spec/decisions/0070-a-late-key-reaches-the-chart.md),
[ADR-0074](../spec/decisions/0074-a-deterministic-door-failure-is-a-refusal-not-a-fault.md)). Not
to be confused with the note **quarantine commit** below.

### Fat Postgres, thin daemon
The architecture's central engineering choice: safety-critical logic lives **in the database**, and
the Rust daemon stays thin (crypto, transport, orchestration). The integration boundary *is* the
PostgreSQL boundary. See [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md).

### Wire core
Layer 1 — the signed event itself (serialization, signature, content-addressing, the identity/actor
algebras). **Nothing above it sits on the inter-node path**, so any node interoperates with any other.
`crates/cairn-event`. See [ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md).

### HLC (Hybrid Logical Clock)
The ordering mechanism for events. It combines wall-clock time with a logical counter so **causal
order survives clock skew** across offline nodes. Events are ordered by HLC, not by raw wall-clock.

### `t_recorded` vs. `t_effective`
The bitemporal time model. `t_recorded` is the objective record time (HLC-bounded — the ceiling);
`t_effective` is the *asserted*, freely-backdatable clinical time (the displayed claim). Clashes are
**flagged, never auto-resolved.** See [ADR-0003](../spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)
and [principle 4](../spec/index.md).

### Legibility twin
A mandatory, signed, mechanically-derived **plaintext human-readable rendering** carried inside every
event's body (`EventBody.plaintext_twin`) — informally, "the twin". It keeps an event readable forever — even on a node whose
schema has moved on or never understood the event's type — and is the full-text/RAG substrate. An
**authored** twin (built by the event builder) is preferred; non-demographic types **degrade
honestly** to a flagged derived skeleton when one is absent. Principle 11; see
[ADR-0012](../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md),
[ADR-0034](../spec/decisions/0034-demographic-legibility-twin.md), and
[ADR-0039](../spec/decisions/0039-globalise-authored-legibility-twin.md).

### Set-union sync
Synchronizing two nodes is a **set union** of their immutable, content-addressed events plus a small,
explicitly enumerated set of merge policies — never a dangerous field-level merge. Applying the same
event twice, or learning it from two peers, converges. See the [sync spec](../spec/sync.md) and
[ADR-0004](../spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md).

### Identity is a claim, never a fact / *never merge, always link*
Patient identity is an append-only stream of link/unlink/reattribute/repudiate/identify/dispute
events (the closed **identity event algebra**, [§5.7](../spec/identity.md)). Patient UUIDs are
**immortal**; identity errors are repaired by an auditable event with no data loss. You never merge
two patient records or erase one — you *link* or *overlay*. [Principle 2](../spec/index.md).

### Slice
A vertical, end-to-end increment of the clinical product (e.g. "demographics slice 1 = patient
identifiers"). Each slice goes brainstorm → spec → plan → TDD and reuses the event/floor/projection
spine. The current slice is tracked in `docs/HANDOVER.md`.

### Actor
A registered, version-pinned identity that can author or attest events — a human, a device, or an
**advisory agent**. Signing proves origin/integrity; **attestation** confers responsibility, and the
two are separable. See [ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md),
[ADR-0007](../spec/decisions/0007-authorship-and-accountability.md), and (for AI agents)
[ADR-0029](../spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md) /
[ADR-0030](../spec/decisions/0030-advisory-actor-integration-contract.md).

### Veto (match veto)
The **in-DB, safety-critical** check that can stop a patient match: `cairn_match_veto`
(`db/016_match_veto.sql`) returns the closed hard-veto set (same-system identifier mismatch,
verified-DOB clash, verified-sex-at-birth clash). A veto can only ever **withhold an auto-link or
force human review — never auto-reject.** Distinct from the advisory *scorer*. See
[ADR-0014](../spec/decisions/0014-locale-pluggable-matcher-comparators.md).

### Matcher (advisory) / scoring / banding / proposal
The **advisory** patient-matcher (`matcher/`, Python). The pure **scorer** (Fellegi–Sunter) produces
an explainable `MatchScore`; **banding** turns a score + veto findings into `auto_candidate` /
`review` / nothing; a **proposal** is an advisory row in `match_proposal` (`db/017`) for a human to
act on. The matcher never links or rejects on its own — it only proposes.

### Provenance
Where an assertion came from on a graded ladder (e.g. patient-stated → document-verified →
fact-proven). Several demographic fields resolve their display "winner" by provenance (DOB, sex-at-birth,
administrative-sex), while others resolve by recency (names, gender-identity, address). The choice per
field is a deliberate ADR-backed clinical decision. See [demographics](../spec/demographics.md).

### Erasure = key-custody redistribution (crypto-shred)
In an append-only system, "deletion" is implemented as **crypto-shredding** — redistributing or
destroying key custody for an encryption-capable body slot — exposed as a policy-neutral severity
ladder. Deletion is best-effort and declared, never guaranteed. See
[ADR-0005](../spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md), principle 9.

### Fractal topology
One codebase runs at every tier — workstation → department → facility → region → nation. A node's
role is **configuration, not a different product.** Validated down to the phone tier in
[Spike 0003](../spikes/0003-postgres-on-android-bionic-node.md).

### Node / federation / peering
A **node** is one Cairn instance (its own PostgreSQL + the `cairn-node` daemon). Nodes **federate** by
**peering** — exchanging signed pairing offers, pinning each other in an mTLS trust set, and syncing
`node_event` logs by set-union. See [ADR-0017](../spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md).

### Spike
A time-boxed proof-of-concept that answers a viability question before committing to a build (the
walking skeleton, the advisory-actor write contract, Postgres-on-Android). Recorded under
`docs/spikes/`. Proven primitives graduate into `crates/` and `db/`; the spike code in `poc/` is then
frozen reference.

### ADR (Architecture Decision Record)
A numbered, dated, **immutable** record of a load-bearing decision (context → decision →
consequences), under `docs/spec/decisions/`. A reversal is a *new superseding ADR*, never an edit.
**Read the relevant ADR before reopening a settled question.**

### pgrx / `cairn_pgx`
[pgrx](https://github.com/pgcentralfoundation/pgrx) is the framework for writing PostgreSQL extensions
in Rust. `cairn_pgx` (`extensions/`) is Cairn's in-database Rust surface — currently the
`cairn_verify` signature gate the floor calls. Built with `cargo pgrx`, not plain `cargo`.

### Roles: `cairn_node`, `cairn_agent`, `cairn_window` / `db_floor ENFORCED`
Every component connects under a role that holds only the grants it needs, so the in-DB floor
genuinely binds it — a raw `INSERT` is denied (SQLSTATE 42501) while the validated doors still work.
`cairn_node` (provisioned by `cairn-node provision-runtime-role`) holds the peer-admission doors;
`cairn_agent` (`db/004`) is the advisory-actor role — the submit doors and projection reads;
`cairn_window` (`db/062`, provisioned by `cairn-node provision-window-role`) is the reference
window's group role: `cairn_agent` plus a short named list, **never** `cairn_node`
([ADR-0084](../spec/decisions/0084-the-window-never-holds-the-owners-keys.md)). The database owner
loads the schema; nothing else does. `cairn-node status` reports `db_floor ENFORCED` vs.
`BYPASSABLE` so you can tell which path you're on.

### Two planes (node plane / clinical plane)
The **node plane** (`node_event`) records who the nodes are, who is peered and which keys are
trusted; `cairn-node` syncs it. The **clinical plane** (`event_log`) is the record; `cairn-sync`
syncs it over `cairn-wire`. Code and schema travel neither: they move on a separate signed
distribution plane, and a node's schema generation is a local property
([ADR-0012](../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)).

### Schema generation / schema stamp
`SCHEMA_GENERATION` (`crates/cairn-event/src/schema_generation.rs`) numbers the migration set a
build carries. On connect, `cairn-node` replays `db/*.sql` only when the database's `node_schema`
stamp says something changed, and refuses a database stamped by a **newer** build (a replay by an
older binary would silently roll the floor back)
([ADR-0080](../spec/decisions/0080-a-connect-replays-the-schema-only-when-it-changed.md)). The
reference window refuses a database prepared by a different build.

### Born-sealed body / DEK / crypto-shred
Every clinical event body is **encrypted at write** under its own data-encryption key (DEK),
wrapped for the node ([ADR-0052](../spec/decisions/0052-born-sealed-clinical-bodies.md)). This is
*not* confidentiality — the node reads its own data freely — it is what keeps erasure possible for
every event, forever: destroying a DEK's custody (`cairn-node shred`) makes that body unreadable
without touching the append-only log. See *Erasure* above.

### Contributor set / authored / recorded / attested
Who had a hand in an event, as a closed set of `{kind, role}` pairs carried in the signed body. A
clinical write carries `{human, authored}` + `{node, recorded}`: the human signed it, the node
sealed and stored it ([ADR-0053](../spec/decisions/0053-per-write-human-authorship.md)). *Signing*
proves origin; *attestation* (`attested`, `co-signed`) confers responsibility, and may be absent or
come later ([ADR-0051](../spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)).

### Authorship grade / `Asserted`
How confidently the record can say who is responsible for an event, read back from its contributor
set and shown to the reader. `Asserted` is the grade of an event a named human wrote but nobody
signed — e.g. a draft that went in by quarantine commit — and reads "went in unsigned"
([ADR-0089](../spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md)).

### Key vault / release / enroller / superuser
Clinicians hold no key files. Each member's signing key is stored in the node's database sealed
under their own passphrase (`db/061`), readable by no role; the **release** door checks the
passphrase inside Postgres, counts every guess, and hands the key back for **one act**
([ADR-0083](../spec/decisions/0083-the-human-key-vault.md)). A **superuser** (the first made at the
console with `vault-bootstrap-superuser`) or an **enroller** enrols staff in person; an enrolment
by an enroller stays *unconfirmed* — never blocking signing — until a superuser confirms it
([ADR-0085](../spec/decisions/0085-in-person-enrolment-and-re-key.md)).

### Signer strip / terminal lock / key hold
The reference window's sign-in affordances. The **terminal lock** gatekeeps the terminal; the
**signer strip** is where the staff ID and passphrase for *this* act are typed, before the press —
no dialog; the **key hold** (`--key-hold-seconds`, 0 on a shared terminal) lets a personal
workstation keep the last signer's key for their next acts.

### Who's here
A claim, never a sign-in: the terminal names the person currently writing, which labels a draft's
parts. Naming someone releases no key and signs nothing; an unverified claim displays as
`?Name` ([ADR-0088](../spec/decisions/0088-whos-here-and-a-drafts-parts.md)).

### Draft / parts / Sign / quarantine commit / discard
A progress note being typed is a **draft**: node-local, autosaved, deliberately mutable, never
synced or backed up — **not an event** ([ADR-0087](../spec/decisions/0087-the-first-note-stream-and-its-draft.md)).
It holds one **part** per writer. **Sign** turns it into one sealed, human-signed
`clinical.note.asserted` per non-blank part, in one transaction. A draft untouched for the site's
quarantine (`cairn-node note-set-quarantine`) goes in by **quarantine commit**, unsigned and graded
`Asserted`. **Discard** throws away only your own writing and leaves an audited, signed
`clinical.note-discard.asserted` marker on the chart
([ADR-0090](../spec/decisions/0090-the-audited-discard.md)).

### Registration act / search-before-create funnel
A new chart begins with `identity.registration.asserted`, and a standard registration **carries the
search that preceded it** — its attestation names the candidates that were displayed
([ADR-0061](../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)). The strict
door refuses a chart whose first event is not its registration. The window's front door is the
**funnel** that enforces "search, then create only if nothing fits" (spec §5.3/§5.8).

### John Doe (unidentified registration)
A chart for a patient who cannot be identified yet (unconscious, unknown), registered with no search
and a node-local ordinal for a human-usable label; identified later by an `identify` event, never by
editing the chart ([identity §5.4](../spec/identity.md)).

### Linked charts / person / duplicate repair
Two charts judged to be the same person are **linked**, never merged. A linked set opens as **one
combined record** with every row naming its source chart (`cairn_person_charts`, `db/054`), like two
paper folders clipped together; a human judgement outranks a machine's, and a link can be undone by
an unlink event ([ADR-0076](../spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)).
The **duplicate check** (`cairn-matcher watch`) proposes candidates; see
[Running the duplicate check](running-the-duplicate-check.md).

### Sensitivity stream / safety projection
Sensitivity is an append-only stream of graded assertions about an event, a thread or a whole
patient; the effective grade is the strictest standing one
([ADR-0062](../spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md)). The
**safety projection** lets decision support warn about a sealed, sensitive item at a coarsened
level ("confidential medication, severity X") without disclosing it
([ADR-0063](../spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md)).

### Partial validity
A defect on one line of a composite clinical object (one drug on a medication list, one part of a
note) never invalidates the others; validity, signature currency and actionability belong to the
individual line
([ADR-0060](../spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)).

### Backup medium / restore
Off-node durability: `cairn-node backup` writes both planes to a self-verifying medium
(`cairn-medium`), and `restore` rebuilds a node from it into a fresh database, needing only the
medium and one recovery secret, nothing about the dead node
([ADR-0026](../spec/decisions/0026-node-durability-and-disaster-recovery.md),
[ADR-0067](../spec/decisions/0067-a-restore-reads-the-clinical-plane.md)).

### Paper-parity benchmark (N → M → K)
The section every clinical-surface slice plan carries (house rule 7): the paper counterpart's step
count *N*, the steps the architecture forces *M*, and the steps the UI targets *K*. `M > N` is an
architecture defect. The time budget is measured by a human, following the slice's `RUNBOOK.md`
section. See [Contributing workflow](contributing-workflow.md#the-coding-house-rules).

### Durable rule / pin
A rule a later change could undo in good faith, recorded in [Durable rules](durable-rules.md) with
its **pin** — the test that fails if it is undone — and the tempting wrong fix. Read the block for a
file before you change it.

### Reference window
The steward's own UI (`cairn-gui/cairn-gui-tauri`, Tauri 2): the funnel, the chart, duplicate
review, notes and sign-in. It has no privilege another UI could not have, runs on its own
low-privilege login, and makes every clinical display decision in tested Rust; `--mock` runs it on
fixtures with no database.
