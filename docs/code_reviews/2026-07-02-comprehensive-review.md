# Comprehensive code + ADR review — 2026-07-02

**Scope:** the full repository as of `dddd494` — the SQL enforcement floor (`db/*.sql`),
the Rust crates (`cairn-node`, `cairn-event`, `cairn-sync`, `cairn_pgx`), the Python
matcher, and ADRs 0001–0039 — reviewed adversarially for logic flaws and design
footguns (poor early decisions that get expensive later).

**Method:** seven parallel review passes (SQL floor; cairn-node; event-format core;
ADRs 0001–0018; ADRs 0019–0039; spec-vs-implementation audit; matcher), findings
cross-checked and the headline claims re-verified directly against the code before
inclusion. Conflicting findings between passes were adjudicated by reading the code
(noted inline where it happened).

**Overall verdict:** the foundations are unusually strong — the crypto engineering,
the grant-floor discipline, the sign-the-bytes-verbatim canonicalization, and the ADR
log's honesty about its own limits all hold up under adversarial reading. The trouble
concentrates in three places: **(1)** the clinical sync path is a second write door
that bypasses the floor and can silently lose events, **(2)** a cluster of settled-ADR
promises the code doesn't yet keep, with no deferral note anywhere, and **(3)** a set
of wire/design decisions that are one-line fixes today and unretrofittable in a year.

Disposition of every finding (fixed in this branch vs. filed as a GitHub issue) is in the
**Disposition** table at the end of this document.

---

## A. Verified code bugs

### A1 — CRITICAL: `cairn-sync` can silently and permanently lose clinical events

`crates/cairn-sync/src/main.rs:742-766`. Every `apply_signed` error — including
transient DB errors and deterministic insert failures on *validly signed* events — is
counted as a "verify failure" and skipped, while the watermark advances to the max HLC
of the events that *did* apply. Batch [E1@10 fails transiently, E2@20 succeeds] →
watermark 20 → E1 is never offered on this link again (server ships
`WHERE hlc >= watermark`). The only trace is a counter in a log line. This breaks
set-union, the invariant the whole architecture rests on. The node-event plane in
`cairn-node/src/sync.rs` got this exactly right (advance only on clean EOF), so the
correct pattern already exists in the repo.

**Fix direction:** advance only past the contiguous applied prefix; treat only
`BadSignature`-class errors as skippable, and quarantine those durably.

### A2 — HIGH: the sync apply path is an unguarded second write door

`crates/cairn-sync/src/main.rs:167-226` verifies the signature in Rust, then
raw-INSERTs with owner privileges. Skipped for every replicated event: actor
enrollment, fail-closed `event_type_class` classification, **the attestation gate on
suppressing events**, the demographic hard-twin rule, and — because the
`ON CONFLICT DO NOTHING` at line 185 has no conflict target — the **event-id
substitution check** that `submit_event` explicitly raises on (`db/005:160`). A
hostile peer can sync in an un-attested `visibility.suppress` signed by an unenrolled
key; two nodes holding different bytes under one `event_id` diverge forever with no
alarm. ADR-0021 puts the floor *below* the inter-node path; today it only guards
local authors. The node-event plane already has its in-DB gate
(`apply_remote_node_event`) — the clinical plane needs its sibling.

Related sub-findings folded into this item:
- **Twin asymmetry (M8):** three divergent twin-fallback renderers (Rust
  `plaintext_twin()`, `resolve_twin` in cairn-sync, SQL `cairn_twin_skeleton`); a
  twin-less demographic event is rejected by `submit_event` but accepted (with a
  derived twin) by `apply_signed`.
- **A5b:** projection triggers can RAISE at apply time (see A5) — apply-path policy
  must clamp+flag, never veto a validly-signed event.

### A3 — HIGH: `t_effective ≤ t_recorded` is enforced nowhere

ADR-0003 calls this "an envelope invariant… rejected/flagged at write"; ADR-0022 step
5.3 lists it in the door. There is no CHECK, no submit validation, nothing in Rust,
and no deferral note. A signed event claiming `t_effective = 2031-01-01` is accepted
silently. This is the project's own falsification guard, and the one settled-ADR
promise flatly contradicted by the code.

Related: `t_effective` is a free-form string cast per-node under local `TimeZone`
GUCs (`cairn-sync main.rs:183`, `db/005:155`) — an offset-less timestamp is a
*different UTC instant on different nodes*, and a malformed one is a poison event
that (combined with A1) vanishes silently.

### A4 — HIGH: `patient_identifier` is the one non-convergent projection

`db/010_demographics.sql:97` uses `ON CONFLICT DO NOTHING` — first-*applied* wins,
and "first applied" is node-local, not a function of the event set. Two assertions
normalizing to the same `match_key` but differing in `value`/`provenance` leave
different surviving rows on different nodes, and the match-veto (`db/016`) reads
exactly those columns — so two honest nodes can compute **different hard-veto
verdicts**. Every other projection (011–014, 018) uses the order-independent
`DO UPDATE … WHERE tuple >` pattern; this one should too.

### A5 — HIGH: identity-linkage projection — race + config-dependent RAISE

`db/018_identity_linkage.sql:154-232`. (a) `cairn_recompute_component` is a
non-serializable read-modify-write under READ COMMITTED — concurrent `link(A,B)` and
`link(B,C)` can leave `person_member` missing the union. (b) The oversize guard
RAISEs based on a **node-local GUC** (`cairn.max_component_size`), so a node
configured with a smaller cap can permanently refuse a validly-signed event its peers
accepted. Projection maintenance must never veto event application.

**Fix direction:** transaction-scoped advisory lock keyed on the component before
recompute; clamp-and-flag instead of RAISE on the apply path (the RAISE redesign is
folded into the A2 issue).

### A6 — MAJOR: actor enrollment rests on implicit default privileges

`db/004_actors.sql:50-59`. No explicit `REVOKE` exists on `actor_event` /
`enroll_actor` — safe today only because `enroll_actor` is invoker-rights and no
grant exists. One future `GRANT`, or copy-pasting the SECURITY DEFINER pattern from
the other doors, silently collapses the admission gate (self-enroll any pubkey →
"legitimately signed" events). No negative test exists, unlike the raw-INSERT floor
tests.

### A7 — MAJOR (cairn-node): write-side limits missing (three findings, one theme)

- **(a)** No event-size ceiling at any admit door vs hard 8 MiB caps on every read
  path (`sync.rs:84` read-only; `medium.rs:197` `debug_assert!` only; no
  `octet_length` check in db/005/007/009). One oversized event wedges sync at that
  seq forever, and `backup_to` **overwrites the previous good medium** before the
  read-back check fails.
- **(b)** `pull_into` has no I/O timeout (`sync.rs:322-334`) — a stalled pinned peer
  freezes the run loop **including trust refresh, so revocations stop taking effect**
  on the running daemon. Same for `serve` (no handshake timeout / connection cap).
- **(c)** `LocalState` silently drops unknown CBOR fields on old readers
  (`localstate.rs:57-96`) — a future restore quietly discards content (e.g. episode
  DEKs) the module's own comments promise to fail loud on, in the format declared
  can't-retrofit. `from_cbor` never checks `version`.

### A8 — MAJOR: the cairn_pgx `#[pg_test]` fixtures don't compile

`extensions/cairn_pgx/src/lib.rs:60,105` — both `EventBody` literals lack the
`plaintext_twin` field added to the struct (E0063 under the pg_test feature). The
in-DB verification safety net hasn't been runnable since that field landed.

### A9 — Matcher findings (advisory tier, but they shape the human worklist)

- **(a) HIGH:** name token-count mismatch grades DISAGREE
  (`comparators.py:184-185`) — "Mary Smith" vs "Mary Jane Smith" takes a −2.0 clash
  penalty when every token matches; systematically punishes multi-token naming
  cultures, the population ADR-0014 exists to protect. Subset-match should grade
  PARTIAL.
- **(b) HIGH:** a hard-vetoed pair whose veto-subject fields drag the total below the
  review threshold is persisted nowhere (`banding.py:64-68`) — an effective
  auto-reject, which ADR-0014 §6 forbids. Force REVIEW when a veto coexists with
  strong positive evidence.
- **(c) MED:** no Unicode NFC normalization anywhere (adapter `lower().split()`,
  SQL `lower(value)`) — NFD vs NFC of the same name grades DISAGREE and breaks
  blocking. NFC + `casefold()` is culture-neutral canonicalization.
- **(d) MED:** `matcher_version` pins only the weights (`banding.py:71-84`) —
  thresholds and comparator config are invisible to the ADR-0011/0029 recall handle.
- **(e) MED:** `parse_dob` accepts any integer year (`adapter.py:33-47`) — a
  two-digit-year import fabricates the largest DISAGREE penalty in the table.
- **(f) MED-LOW:** `"unknown"` sex sentinel fabricates evidence in both directions
  (EXACT on unknown-vs-unknown, DISAGREE on unknown-vs-male).
- **(g) LOW (batched issue):** stale proposals never withdrawn on re-score below
  review; name provenance rank is max-across-history (inflates evidence toward
  false-merge); blocking eval KeyError on non-empty DBs; the synthetic eval's
  `_repair` makes `pair_completeness` structurally blind to the blocking blind spot.

### A10 — Other verified floor/daemon items (smaller)

- **Recall epoch bug:** `db/006_recall.sql:26` joins `actor_current`, so a
  contamination cascade querying a superseded epoch under-selects (documented in
  code; issue-worthy per house rule 5).
- **Suppression owner-gate open:** `db/005:114-130` — any enrolled human can suppress
  any author's event (`DEFERRED` comment; ADR-0022 step 5.5).
- **TRUNCATE** not covered by append-only triggers (defense-in-depth gap).
- **`blob_store` comment overclaims** (`db/003:26-30`): the CHECK verifies length
  only, not BLAKE3-vs-address (pgcrypto has no BLAKE3); verification is L2-side.
- **`patient_link`-BFS missing index** on the `high` side (`db/018`).
- **Attestation (M7):** `verify_attestation` never binds `attester_key_id` to the
  verifying key (the event-side `SignerKeyMismatch` gate has no attestation
  counterpart); the token is verified then discarded — the responsibility proof
  cannot be re-verified later.
- **Winner-tiebreak residual:** adjudicated — `asserted_origin` is the HLC node id,
  so honest nodes can't tie; but nothing enforces per-node HLC-tuple uniqueness
  against a buggy client. Appending a total-order key (the `value` PK member) to the
  winner `ORDER BY`s makes convergence unconditional.
- **cairn-node minors:** signing-key seed written with default file perms (not
  0600); SPKI algorithm OID unchecked in `peer_pubkey_hex`; `restore` can exit
  non-zero after fully succeeding (tty-less passphrase prompt); single-entry
  passphrase at `init`/`seal-key`; `db.rs` hardcodes `NoTls`; pairing offers never
  expire; HLC merge has no drift clamp; `hlc_counter` is 32-bit; sync-cursor ratchet
  has no sanity clamp; transient-vs-gate errors conflated in `pull_into` skip logic.
- **cairn-sync (M9/M10):** unbounded `EventsAfter` batch + 30 s timeout → permanent
  no-progress loop on large backlogs (paginate); first-writer-wins `byte_len` on
  `blob_note_reference` lets one bad reference permanently wedge a blob fetch (the
  server ships the true total in every slice frame; client ignores it).

---

## B. ADR/design footguns — cheap now, expensive later

### B1 — The erasure ladder doesn't compose with three neighboring ADRs

ADR-0005's crypto-shred meets three plaintext leaks nobody owns: the **safety
projection** (ADR-0006) replicates in the clear and no ADR says what happens to it on
shred (shred it → the Rh-warning use case dies; keep it → rungs 2–4 are incomplete);
the **legibility twin** (ADR-0012) — no ADR states whether it sits inside the DEK
boundary, and outside it the shred is theater (twin + FTS + RAG embeddings survive);
**attachments** (ADR-0013) are content-addressed by *plaintext* hash, so a blob
written unencrypted can never be retro-sealed — the signed event immortalizes the
plaintext digest. Rung 2's "no discoverable institutional record" is unachievable as
written: the envelope is plaintext by construction and its rows can't be removed
(the rescue path — pseudonymous registration from episode start — is a day-one
precondition the ADR doesn't wire in). Since seal-at-write is the only window
forever, the **encryption-default decision** is the classic cheap-now/impossible-later
call. Needs one composing ADR that owns the intersection.

### B2 — The revocation cascade trusts the attacker's own clock

ADR-0018's compromise boundary keys on `t_recorded` — stamped by the authoring node.
A revoked actor backdates fabricated events to before T and launders them through any
peer that hasn't heard the revocation; they arrive everywhere marked "authored while
credentialed", permanently. Fix is a day-one shape: record
**first-seen-by-honest-node** at ingestion and flag
`max(t_recorded, first_receipt) > T` as a clash (the ADR-0003 pattern).
Unretrofittable onto years of already-ingested events.

### B3 — Human key custody is unspecified, and the default is node custody

ADR-0011 node-binds *agent* keys explicitly; for humans nothing is said, and
ADR-0008's badge-tap flow implies the node signs on the clinician's behalf — meaning
the node can mint "Dr X attested this" for anyone enrolled, and the likeliest
authorship dispute is clinician-vs-institution. "Signature proves origin" quietly
degrades to "the node asserts origin." Decide now (it constrains the ADR-0008
hardware story).

### B4 — Two wire-freeze items in ADR-0015's orbit

- **(a)** COSE_Sign1 is single-signer, yet "native multi-signer support" is cited as
  a reason for choosing COSE — either ratify COSE_Sign or amend to
  one-signature-per-event with co-signing via overlay; the rationale as written is
  wrong either way.
- **(b)** **No domain separation** between the three signing contexts (event /
  attestation / pairing bundle) — same key type, empty `external_aad`, no context
  marker; cross-context replay fails today only by structural luck, and additive
  evolution erodes that. Both invalidate existing signatures if fixed after freeze.

### B5 — ADR-0028's closed role enum has no unknown-member story

A 2031 node adds a bearing role; a 2026 node can't classify it into the
bearing/contributory partition everything safety-relevant branches on — a
properly-vouched note projects as *un-vouched AI content* (a precise untruth).
Encode the partition in the role value (`bearing:…`/`contrib:…` or a signed
per-contributor bit) and define an explicit "vouching-unknown" degrade. An
enum-spelling decision, free today.

### B6 — Demographic recency is keyed to the wrong time axis

The winner views order by `last_hlc_wall` — when the assertion was *typed*
(`t_recorded`), not when it was true (`t_effective`). A clerk transcribing a 2015
passport today displaces last month's patient-stated post-divorce name; a
legacy-import 2019 address outranks the current one — ADR-0038's own headline use
case (post-discharge letters, ambulance dispatch) goes to the wrong place. The
project built the bitemporal model for exactly this distinction and didn't consult it
here; neither ADR discusses which axis recency means.

### B7 — ADR-0016's essential tier contradicts ADR-0001 and skips the threat model

If the tier is a synced *projection*, the multi-master merge problem ADR-0001
designed away is back; if it's an event subset (as the boundary rule implies),
"current state, not history" is false as a storage claim, sizing is off by up to an
order of magnitude for elderly polypharmacy patients, and there's no
bootstrap/compaction story. Meanwhile the population's problem lists replicate to the
*weakest node in the federation* with only legal (not mechanical) custody
enforcement.

### B8 — Smaller design items (one sentence each in their ADR/spec homes)

- Nothing forbids CDS from reading *administrative* sex (trans-pregnancy/teratogen
  failure) — one normative sentence, enforceable via the ADR-0023 conformance suite.
- Addresses have no temporary/validity semantics ("staying at my daughter's" wins
  recency forever — ADR-0038).
- Mid-episode name change desynchronizes from printed wristbands/labels — the
  wrong-chart failure ADR-0008 exists to prevent, unnamed in ADR-0036.
- The medico-legal export (ADR-0019) doesn't bundle the ADR-0027 time-anchor proofs,
  so it can't prove the record wasn't authored last week — the accusation it exists
  to defeat.
- ADR-0003's hard reject-at-write meets the RTC-less Pi (cold boot with stale clock
  flags every honest morning note as falsification); ADR-0027's graded interval is
  the fix but ADR-0003's normative wording stands unamended.
- ADR-0021/`language-substrate.md` name RLS as part of the present-tense floor; zero
  RLS exists (ROADMAP correctly lists it as Phase-2).
- ADR-0005 rung-2 as written promises what the plaintext envelope cannot deliver
  (see B1).
- Notification ladder (ADR-0009) "bottoms out in a reachable human" overstates —
  routing guarantees an inbox, not a human; the out-of-band seam (SMS/phone) is the
  actual paper-parity counterpart of the telephone callback.
- Matcher normalizer version skew across nodes fires spurious vetoes (ADR-0033/0035)
  — safe but silent workload; require output-stable normalizer evolution.
- Projection rebuild cost over decades is unmeasured (add to benchmark matrix).
- Attachment descriptor metadata replicates in clear (apply the ADR-0006 coarsening
  ladder to it).
- Attachment-reference field names disagree between `db/001` docs (`digest`) and
  `db/005` code (`digest_hex`) — the one shape declared can't-retrofit.

---

## C. What held up under adversarial review

The seal/keystore work (Argon2id + XChaCha20, compile-pinned zeroization discipline,
tamper tests on both recipients), `fsio::atomic_write`, the backup
verify-before-write ordering, the mutual-pinning transport with resumption disabled,
the sign-the-bytes-verbatim canonicalization (which structurally kills the class of
cross-node digest-divergence bugs), the `signer_key_id`↔verifying-key binding, the
grant floor with pinned `search_path` on every SECURITY DEFINER door, the append-only
triggers, the node-event admission gate and advance-only cursor, ADR-0031's surrogate
confinement (verified: nothing leaks), the ADR-0036/0037/0038 winner SQL matching the
ADRs exactly, the blob tier's per-slice + whole-blob verification, and the matcher's
core no-data-is-never-disagreement scoring. ADRs 0002, 0004, 0007, 0010, 0014, 0017,
0020–0023, 0026, 0029, 0030, 0032, 0034, 0035 read as internally sound.

---

## Disposition

Fixed in this branch (with tests where a harness exists), or filed as a GitHub issue.

| # | Finding | Disposition |
|---|---------|-------------|
| A1 | cairn-sync silently loses events (watermark past failures) | **Fixed** — contiguous-prefix watermark + BadSignature-vs-transient classification (`cairn-sync/src/main.rs`) |
| A3 | `t_effective ≤ t_recorded` ceiling unenforced | **Fixed** — ceiling check in `submit_event` + test (`db/005`, `demographics.rs`) |
| A4 | `patient_identifier` non-convergent (first-apply-wins) | **Fixed** — HLC-deterministic `DO UPDATE` + convergence test (`db/010`, `demographics.rs`) |
| A5a | Linkage recompute race | **Fixed** — transaction advisory lock (`db/018`) |
| A6 | Actor-enrollment gate rests on implicit privileges | **Fixed** — explicit REVOKEs + negative test (`db/004`, `floor_enforced.rs`) |
| A7a | No write-side event-size ceiling | **Fixed** — `cairn_max_event_bytes()` at all admission doors + serve-side skip (`db/001,005,007,009`, `sync.rs`) |
| A7b | `pull_into`/`serve` no timeout; no session cap | **Fixed** — `with_io_timeout` on every net step + serve semaphore (`sync.rs`) |
| A7c | `LocalState` silently drops unknown fields | **Fixed** — `deny_unknown_fields` + version gate + tests (`localstate.rs`) |
| A8 | cairn_pgx test fixtures don't compile | **Fixed** — added `plaintext_twin`; `cargo check --features pg_test` now clean (`cairn_pgx`) |
| A9a | Name subset graded DISAGREE | **Fixed** — subset→PARTIAL + tests (`comparators.py`) |
| A9b | Veto buries its own strong-evidence pair | **Fixed** — veto+identifier-EXACT forces REVIEW + tests (`banding.py`) |
| A9c | No Unicode NFC normalization | **Fixed** — NFC+casefold in adapter + SQL tokenizer + generator mirror + tests |
| A9e | `parse_dob` accepts 2-digit year | **Fixed** — 4-digit-year gate + tests (`adapter.py`) |
| A9f | `unknown` sex sentinel fabricates evidence | **Fixed** — sentinel→absence + test (`adapter.py`) |
| A10 | Winner tiebreak not total | **Fixed** — appended `value`/`display`/`use_key`; collation caveat → [#69](https://github.com/cairn-ehr/cairn-ehr/issues/69) |
| A10 | `patient_link` high-side index | **Fixed** — partial index (`db/018`) |
| A10 | `blob_store` comment overclaim | **Fixed** — comment + constraint renamed (`db/003`) |
| M7 | Attestation `attester_key_id` unbound | **Fixed** — key binding + test (`cairn-event/src/lib.rs`) |
| H3 | Sync silently absorbs event_id substitution | **Fixed** — substitution guard in `apply_signed` (`cairn-sync`) |
| L12 | Signing-key file world-readable | **Fixed** — 0600 perms (`cairn-sync`) |
| — | SPKI algorithm OID unchecked | **Fixed** — Ed25519 OID pin (`transport.rs`) |
| A2 / A5b / M8 / H4 | Clinical sync bypasses the floor; apply-path RAISE; twin triple-impl; t_effective TZ | Issue [#91](https://github.com/cairn-ehr/cairn-ehr/issues/91) |
| B1 | Erasure ladder composition | Issue [#92](https://github.com/cairn-ehr/cairn-ehr/issues/92) |
| B2 | Revocation cascade trusts author clock | Issue [#93](https://github.com/cairn-ehr/cairn-ehr/issues/93) |
| B3 | Human key custody unspecified | Issue [#94](https://github.com/cairn-ehr/cairn-ehr/issues/94) |
| B4 | COSE single-signer + domain separation | Issue [#95](https://github.com/cairn-ehr/cairn-ehr/issues/95) |
| B5 | Closed role enum unknown-member encoding | Issue [#96](https://github.com/cairn-ehr/cairn-ehr/issues/96) |
| B6 / M4 / M5 | Demographic recency time-axis, temp address, name display | Issue [#97](https://github.com/cairn-ehr/cairn-ehr/issues/97) |
| B7 | Essential tier vs ADR-0001 + threat model | Issue [#98](https://github.com/cairn-ehr/cairn-ehr/issues/98) |
| A10 | Suppression owner-gate + recall epoch | Issue [#99](https://github.com/cairn-ehr/cairn-ehr/issues/99) |
| A9d | `matcher_version` recall-key completeness | Issue [#100](https://github.com/cairn-ehr/cairn-ehr/issues/100) |
| M9 / M10 / blob | Sync pagination, blob byte_len wedge, in-DB BLAKE3 | Issue [#101](https://github.com/cairn-ehr/cairn-ehr/issues/101) |
| A10 misc | cairn-node/cairn-sync operational minors | Issue [#102](https://github.com/cairn-ehr/cairn-ehr/issues/102) |
| B8 | Clinical-safety prose (CDS sex, export anchors, RLS prose, …) | Issue [#103](https://github.com/cairn-ehr/cairn-ehr/issues/103) |
| A9g | Matcher stale proposals, provenance-rank, eval blind spot | Pre-existing [#79](https://github.com/cairn-ehr/cairn-ehr/issues/79), [#84](https://github.com/cairn-ehr/cairn-ehr/issues/84) |
| — | TRUNCATE guard on append-only tables | Intentionally not fixed — owner-gated by design (would break the owner reset path); noted in [#102](https://github.com/cairn-ehr/cairn-ehr/issues/102) |
