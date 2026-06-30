# Codebase tour

A guided **reading order** through the real source. The goal is not to read everything — it is to
read the *one path* that teaches you the whole shape: a single clinical write, from a Rust builder,
through the validated database door, to a queryable projection. Once that path makes sense, every
other slice is a variation on it.

Have the repo open alongside this page. Each step names exact files and what to look for. Pair it with
[Architecture for developers](architecture-for-developers.md), which explains the *why* behind each
move.

---

## Step 0 — Orient (5 minutes)

Read, in this order:

1. `README.md` (repo root) — the mission and the twelve founding principles.
2. `docs/HANDOVER.md` — **the current state.** What was built last, what's in flight, what's
   deferred. This tells you which slice is the live edge *today*.
3. [`docs/ROADMAP.md`](../ROADMAP.md) — the build order and how far each phase has come.

You now know where the project is. The rest of the tour is the code under that.

---

## Step 1 — The wire core: `crates/cairn-event/src/lib.rs`

This is the smallest, most important crate — the **signed event** (layer 1). Read the module-level
doc comment at the top; it states the three structural moves the whole architecture rests on:

- **Sign the bytes; never re-serialize.** `sign` produces a COSE_Sign1 blob over the canonical-CBOR
  body; that blob is stored *verbatim* and `verify_with` checks the signature over those exact bytes.
  Nothing round-trips the structure back to bytes for verification — that is what makes the signature
  trustworthy across schema versions.
- **Self-describing, algorithm-tagged addressing.** `event_address` / `blob_address` are multihashes
  (sha2-256, BLAKE3), so the digest algorithm travels with the digest and can be migrated.
- **Re-attestation is overlay.** A future event can re-sign an old one under a stronger primitive as
  an ordinary overlay event.

Look at the `EventBody` type and note the `plaintext_twin` field — the additive, signed
[legibility twin](glossary.md#legibility-twin). This is principle 11 made concrete: every event
carries a human-readable rendering of itself.

**Takeaway:** events are immutable signed bytes, content-addressed, with a built-in human-readable
twin. This is the contract every node on the wire shares.

---

## Step 2 — The event builder: `crates/cairn-event/src/demographics.rs`

These are **pure functions** that build a demographic event body and its twin — e.g.
`identifier_assertion_body` + `render_identifier_twin`, and the equivalents for DOB, sex-at-birth,
names, and address. Notice:

- They are pure (explicit inputs → an `EventBody`), per the house rule *prefer pure, reusable
  functions* — easy to unit-test, no hidden state.
- The builder produces the **authored twin** alongside the body, so the twin is part of the signed
  bytes from the moment of authoring.
- They parse **no locale date strings** and hardcode **no one culture's** name/address model — that
  is deliberate anti-cultural-capture (principle 4 + [ADR-0014](../spec/decisions/0014-locale-pluggable-matcher-comparators.md)).

**Takeaway:** the Rust side only *constructs and signs*. It does not decide what is valid or what
"wins" — that is the database's job.

---

## Step 3 — The single write door: `db/005_submit.sql`

Open `submit_event`. This is the **only** validated path into the event log (layer 2). Read it for:

- It calls the in-DB **`cairn_verify`** (from `cairn_pgx`) to check the signature *inside* the
  database — so even a raw-SQL client cannot insert an unsigned or forged event.
- It dispatches per event type to a structural-check hook and a twin hook (`cairn_event_twin`),
  *without re-declaring the door* — the validated write surface stays single-source.
- It appends to the immutable `event_log`. There is no `UPDATE`/`DELETE` path.

This door, plus row-level security and the unprivileged runtime role, is *the floor*. Its
unbypassability — even against a hostile agent with direct DB access — was the whole point of
[Spike 0002](../spikes/0002-advisory-actor-write-contract.md).

**Takeaway:** the safety-critical validation lives here, in the database, not in any client.

---

## Step 4 — One slice end to end: `db/010_demographics.sql`

This is demographics **slice 1**, the §4.4 patient-identifier assertion — the canonical first slice.
Read it for the pattern every later slice repeats:

- `cairn_check_identifier_assertion` — the **culture-neutral structural floor**: value/system/
  provenance non-empty, normalized-implies-profile, etc. It performs *no* checksum/format validation
  (that is advisory, and belongs above the floor). It rejects with a clear `RAISE` message.
- The `cairn_event_twin` hook — carries the **authored** twin for demographic events (legacy types
  fall back to a derived skeleton).
- The **`patient_identifier` projection** trigger — folds the event into a queryable table as a
  **set-union** (`ON CONFLICT DO NOTHING`), so re-applying or re-learning an event converges. The PK
  is `(patient_id, system, coalesce(normalized, value))`.

Then skim the later slices to see the *same spine with a different winner policy* per field:

- `db/011_demographics_fields.sql` — DOB + sex-at-birth via **provenance-precedence** (a verified
  value *locks* against lower-provenance ones); introduces the open-floor / gated-projection rule (an
  unknown field is stored and stays legible via its twin, but is not projected — required for
  forward-compatible federation, [ADR-0012](../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)).
- `db/012_demographics_names.sql` — names via a **retained-set** projection + a recency-first display
  VIEW (deliberately *diverges* from DOB's provenance-lock; names legitimately change —
  [ADR-0036](../spec/decisions/0036-demographic-name-display-recency-first.md)).
- `db/013_demographics_sex_gender.sql` — a per-field winner-policy classifier (administrative-sex
  provenance-first; gender-identity recency-first — the inverse).
- `db/014_demographics_address.sql` — address via a per-`use` recency-first winner.
- `db/015_globalise_twin.sql` — globalises the authored twin to every event type.

**Takeaway:** "current state" tables are derived projections of the log, and each field's *winner
policy* is a deliberate, ADR-backed clinical decision — not an accident of `ORDER BY`.

---

## Step 5 — The matched pair: hard veto (SQL) vs. advisory scoring (Python)

This is the clearest illustration of the [defect-blast-radius rule](architecture-for-developers.md#5-choosing-a-language-defect-blast-radius).

- `db/016_match_veto.sql` — `cairn_match_veto(patient_a, patient_b)` returns the **closed hard-veto
  set** (same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash). It is
  **in the database** because a wrong auto-merge is catastrophic. It can only ever *withhold* an
  auto-link or *force human review* — never auto-reject.
- `matcher/src/cairn_matcher/` — the **advisory** Python scorer (`scoring.py` is the Fellegi–Sunter
  combiner over the comparators in `comparators.py`). It is in Python because a scoring bug is caught
  and advisory. It **only scores**; it owns no thresholds-as-truth, no veto, no link decision.
- `matcher/src/cairn_matcher/pipeline/` — connects the two: `runner.propose()` loads the `patient_*`
  projections, scores via the pure core, **gates on the in-DB veto**, bands the result, and writes an
  *advisory* proposal into `match_proposal` (`db/017`). `sweep.py` decides *which* pairs to score
  (blocking) so it never does an O(n²) all-pairs comparison.

**Takeaway:** same subsystem, split by blast radius — the dangerous decision is in the database, the
iterative heuristic is in Python, and they meet at the database boundary.

---

## Step 6 — The daemon and its CLI: `crates/cairn-node`

The thin daemon. The fastest way to understand it is its **CLI subcommands** in `src/main.rs` — each
is a documented operation:

- `init` (mint a sealed keypair + genesis), `identity`, `status` (honest assembly state),
  `provision-runtime-role` (create the unprivileged role the floor binds).
- Federation: `pair-offer` / `pair-accept`, `peers`, `unpeer`, `serve`, `run`.
- Durability ([ADR-0026](../spec/decisions/0026-node-durability-and-disaster-recovery.md)):
  `backup`, `verify-backup`, `restore`, `seal-key`, `establish-local-state-key`.

Then read the supporting modules as needed: `keystore.rs` + `seal.rs` (sealed-at-rest signing key),
`pairing.rs` + `transport.rs` (mTLS pinned to the trust set), `sync.rs` (set-union `node_event`
sync), `backup.rs`/`restore.rs`/`localstate.rs`/`medium.rs` (durability), `db.rs` (the DB seam).

The `tests/` directory is the behaviour catalogue — read a test file to see a feature exercised end
to end.

**Takeaway:** the daemon does crypto, transport, and orchestration, then calls the database. It is
intentionally thin.

---

## Where to go from here

- To make a change, read **[Contributing workflow](contributing-workflow.md)** — the
  brainstorm→spec→plan→TDD loop the codebase is actually built with.
- For any unfamiliar term, the **[Glossary](glossary.md)**.
- For the deep *why* of any decision, the relevant **[ADR](../spec/decisions/README.md)** — read it
  before reopening a settled question.
