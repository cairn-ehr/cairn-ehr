# Codebase tour

A guided **reading order** through the real source. The goal is not to read everything — it is to
read the *few paths* that teach you the whole shape: a single write, from a Rust builder, through the
validated database door, to a queryable projection; then the same path as a clinical stream signed by
a human; then the window that drives it. Once those make sense, every other slice is a variation.

Have the repo open alongside this page. Each step names exact files and what to look for. Pair it with
[Architecture for developers](architecture-for-developers.md), which explains the *why* behind each
move.

---

## Step 0 — Orient (10 minutes)

Read, in this order:

1. `README.md` (repo root) — the mission and the founding principles.
2. `docs/HANDOVER.md` — **the current state.** What was built last, what's in flight, what's
   deferred. This tells you which slice is the live edge *today*. Then check the open pull requests:
   HANDOVER describes `main`, and the newest work is often on a draft PR.
3. [`docs/ROADMAP.md`](../ROADMAP.md) — the build order and how far each phase has come.
4. [Durable rules](durable-rules.md) — skim the headings now; read the block for any file before you
   change it. Each rule names the test that fails if it is undone, and the tempting wrong fix.

You now know where the project is. The rest of the tour is the code under that.

---

## Step 1 — The wire core: `crates/cairn-event/src/lib.rs`

This is the smallest, most important crate — the **signed event** (layer 1). Read the module-level
doc comment at the top; it states the structural moves the whole architecture rests on:

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
carries a human-readable rendering of itself. Then glance at `signer.rs` (the one `EventSigner` seam
every signing path goes through — a node key, or a clinician key released from the vault) and
`seal.rs` (born-sealed bodies).

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

Every later stream has a sibling module in the same crate (`registration.rs`, `identity.rs`,
`medication/`, `note.rs`, `note_discard.rs`, …) built the same way.

**Takeaway:** the Rust side only *constructs and signs*. It does not decide what is valid or what
"wins" — that is the database's job.

---

## Step 3 — The strict write door: `db/005_submit.sql`

Open `submit_event`. This is the validated path for events **authored on this node** (layer 2). Read
it for:

- It calls the in-DB **`cairn_verify`** (from `cairn_pgx`) to check the signature *inside* the
  database — so even a raw-SQL client cannot insert an unsigned or forged event.
- It dispatches per event type to a structural check and a twin check through **registries**
  (no per-type `IF` ladder in the door), so the validated write surface stays single-source.
- It appends to the immutable `event_log`. There is no `UPDATE`/`DELETE` path.

Then open its sibling, `apply_remote_event` in `db/020_apply_remote_event.sql` — the **remote door**
for events learned from peers — and compare: it admits what the strict door would refuse to author
(an unknown type, a clinical event ahead of its registration) and grades or pens rather than refuses.
[Architecture §5](architecture-for-developers.md#5-two-doors-two-planes) explains why.

These doors, plus the grants each role holds, are *the floor*. Its unbypassability — even against a
hostile agent with direct DB access — was the whole point of
[Spike 0002](../spikes/0002-advisory-actor-write-contract.md).

**Takeaway:** the safety-critical validation lives here, in the database, not in any client.

---

## Step 4 — One slice end to end: `db/010_demographics.sql`

This is demographics **slice 1**, the §4.4 patient-identifier assertion — the canonical first slice.
Read it for the pattern every later slice repeats:

- `cairn_check_identifier_assertion` — the **culture-neutral structural floor**: value/system/
  provenance non-empty, normalized-implies-profile, etc. It performs *no* checksum/format validation
  (that is advisory, and belongs above the floor). It rejects with a clear `RAISE` message.
- The **`patient_identifier` projection** — folds the event into a queryable table as a
  **set-union** (`ON CONFLICT DO NOTHING`), so re-applying or re-learning an event converges.

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

**Takeaway:** "current state" tables are derived projections of the log, and each field's *winner
policy* is a deliberate, ADR-backed clinical decision — not an accident of `ORDER BY`.

---

## Step 5 — The matched pair: hard veto (SQL) vs. advisory scoring (Python)

This is the clearest illustration of the [defect-blast-radius rule](architecture-for-developers.md#6-choosing-a-language-defect-blast-radius).

- `db/016_match_veto.sql` — `cairn_match_veto(patient_a, patient_b)` returns the **closed hard-veto
  set** (same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash). It is
  **in the database** because a wrong auto-merge is catastrophic. It can only ever *withhold* an
  auto-link or *force human review* — never auto-reject.
- `matcher/src/cairn_matcher/` — the **advisory** Python scorer (`scoring.py` is the Fellegi–Sunter
  combiner over the comparators in `comparators.py`). It **only scores**; it owns no
  thresholds-as-truth, no veto, no link decision.
- `matcher/src/cairn_matcher/pipeline/` — connects the two: the runner loads the `patient_*`
  projections, scores via the pure core, **gates on the in-DB veto**, bands the result, and writes an
  *advisory* proposal into `match_proposal` (`db/017`). `sweep.py` and `targeted.py` decide *which*
  pairs to score, so it never does an O(n²) all-pairs comparison.

What a human does with a proposal — compare two charts, link them, review or undo a link — is the
**duplicate repair path**: `crates/cairn-node/src/chart_link*` and `patient/compare.rs`, with linked
charts read as one through `db/054_person_charts.sql`
([ADR-0076](../spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)).

**Takeaway:** same subsystem, split by blast radius — the dangerous decision is in the database, the
iterative heuristic is in Python, and they meet at the database boundary.

---

## Step 6 — A clinical stream signed by a human: the progress note

The note is the newest stream and shows every mechanism the medication stream introduced, in fewer
lines. Read it top-down:

1. `crates/cairn-node/src/note/mod.rs` — the module map: `draft` (the one deliberately *mutable*
   store, never synced or signed), `whos_here` (who a terminal may name as writing — a claim, never a
   sign-in), `sign`, `commit` (the quarantine), `discard`, `read`.
2. `crates/cairn-event/src/note.rs` — the pure body + twin builder, as in Step 2.
3. `crates/cairn-node/src/note/sign.rs` — read the **one-transaction shape** in its header: save the
   text on screen, spend the draft, build one note per writer's part, submit each through the strict
   door — all or nothing. Signing goes through `medication/sealed_submit.rs`'s `seal_sign_submit`,
   the shared path that seals the body ([ADR-0052](../spec/decisions/0052-born-sealed-clinical-bodies.md))
   and attaches `{human, authored}` + `{node, recorded}`
   ([ADR-0053](../spec/decisions/0053-per-write-human-authorship.md)).
4. `crates/cairn-node/src/vault.rs` + `db/061_vault.sql`'s `cairn_vault_release` — where the
   clinician's key comes from for that one act: the passphrase is checked *inside* Postgres, every
   guess counted, the seed returned into a zeroizing buffer
   ([ADR-0083](../spec/decisions/0083-the-human-key-vault.md)).
5. `db/064_clinical_note.sql` (the stream's floor and projection) and `db/065`–`db/069` (drafts,
   parts, the quarantine commit, the discard) — then the matching block in
   [Durable rules](durable-rules.md), which lists the traps in exactly these files.

For the older, larger stream, `crates/cairn-node/src/medication/` + `db/031`–`db/042`, `db/059`,
`db/060` follow the same spine with more verbs (dose change, reconciliation, attestation, coding,
withhold/release, cessation with a reason).

**Takeaway:** a clinical write is a demographic write plus a sealed body, a human signer released
from the vault for one act, and a draft that is not an event until it is signed.

---

## Step 7 — The window: `cairn-gui/cairn-gui-tauri`

Read `cairn-gui/cairn-gui-tauri/README.md` first, then `src/main.rs`'s header. Things to notice:

- **The front door is the search-before-create funnel** (§5.3/§5.8): its rules are pure functions
  in `cairn-gui-funnel`, its database ports in `cairn-gui-live`, and every sentence a clerk reads in
  `src/funnel/view.rs`. A registration carries the search that preceded it
  ([ADR-0061](../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)).
- **The webview decides nothing.** Every clinical display decision is a Rust view model under
  `cargo test` (e.g. `cairn-gui-tab-medications::build_view`, `src/notes_words.rs`); `src-ui/*.js`
  only renders and calls commands, and drift tests check each field it reads.
- **No confirmation dialogs.** Wrong-chart prevention is the persistent identity header
  (possession), the signer strip is typed before the press, and Discard sits beside Sign in the
  same strip — paper-parity, principle 3.
- **`--mock` is a shipped mode**, used for the accessibility pass and for working on the UI without a
  database.

**Takeaway:** the window is one front-end among many possible ones; it has no privilege another UI
could not have, and its own login can only do what a clinician's window should.

---

## Step 8 — The daemons and the console: `crates/cairn-node`, `crates/cairn-sync`

The fastest way to see what the node can do is its **CLI subcommands** in
`crates/cairn-node/src/main.rs` (run `cargo run -p cairn-node -- --help`) — each is a documented
operation. Grouped:

- **Node setup:** `init` (mint a sealed keypair + genesis), `identity`, `status` (honest assembly
  state), `seal-key`, `establish-local-state-key`, `establish-unwrap-key`,
  `provision-runtime-role`, `provision-window-role`, `enroll-device-actor`.
- **Federation:** `pair-offer` / `pair-accept`, `peers`, `unpeer`, `serve`, `run`, `quarantine`,
  `ack-quarantine`.
- **Durability** ([ADR-0026](../spec/decisions/0026-node-durability-and-disaster-recovery.md)):
  `backup`, `verify-backup`, `restore`.
- **Staff and the vault:** `enroll-human`, `vault-bootstrap-superuser`, `vault-rekey-superuser`,
  `vault-repudiate-superuser-key`, `vault-set-terminal-passphrase`, `vault-set-unconfirmed-expiry`,
  `note-set-quarantine`.
- **Patients and identity:** `patient-search`, `patient-register`, `register-john-doe`,
  `identify-patient`, `link-charts`, `unlink-charts`, `assert-identity-evidence`,
  `assert-observed-evidence`, `apply-auto-candidates`, `duplicate-check`.
- **Medication:** `medication-assert`, `-change-dose`, `-correct-dose`, `-cease`, `-withhold`,
  `-release`, `-code`, `-code-correct`, `-reconcile`, `-separate`, `-attest`, `-list`, `-sign-off`.
- **Sensitivity, safety, erasure, maintenance:** `sensitivity-assert`, `sensitivity-withdraw`,
  `patient-sensitivity`, `patient-safety`, `shred`, `reproject`, `event`, `deferred`.

`cairn-sync` (`crates/cairn-sync/src/main.rs`) is the clinical-plane daemon: it pulls pages of
events from a peer or a medium over `cairn-wire` and applies them through the remote door.

The `tests/` directories are the behaviour catalogue — read a test file to see a feature exercised
end to end.

**Takeaway:** the daemons do crypto, transport, and orchestration, then call the database. They are
intentionally thin.

---

## Where to go from here

- To make a change, read **[Contributing workflow](contributing-workflow.md)** — the
  brainstorm→spec→plan→TDD loop the codebase is actually built with.
- For any unfamiliar term, the **[Glossary](glossary.md)**.
- For the deep *why* of any decision, the relevant **[ADR](../spec/decisions/README.md)** — read it
  before reopening a settled question.
