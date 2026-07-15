# Whole-project architecture review — 2026-07-15

Five parallel review passes over the repo at spec v0.50 / 34 migrations / 602 workspace tests:
(1) the in-DB safety floor (db/001–034 + db/tests), (2) the Rust workspace (cairn-event,
cairn-sync, cairn-node, cairn_pgx, cairn-gui), (3) the spec/ADR corpus (index + all aspect files +
ADR-0000–0049), (4) the Python matcher, (5) cross-cutting seams (migration model, sync
completeness, test architecture, L2/L3 boundary, pilot readiness).

Overall verdict: the corpus and code are unusually coherent for their size; the drift-guard
instinct (name_winner_order_drift.rs, twin_dispatch_single_source.rs, the Rust↔Python placeholder
guard, ADR-0048's registry) is the right one and the guarded pairs are in good shape. The findings
below are where the discipline has holes — concentrated in four clusters: (A) the floor does not
yet hold against the Spike-0002 hostile-enrolled-agent on the HLC/time axis, (B) the sync plane's
convergence guarantee is under-enforced and under-tested, (C) two can't-retrofit wire windows
(authorship vocabulary; seal-by-default) are closing as clinical events accumulate, and (D) the
replay-all-migrations model has a silent floor-downgrade failure mode.

Severity legend: **Critical** = silent corruption/unrepairable state possible; **Important** =
real defect or closing window, needs an owned fix; **Minor** = latent/edge; **Process** = will
manufacture future defects.

---

## A. The floor vs the hostile enrolled writer

### A1 (Critical) — A future-dated HLC wall permanently captures every overlay projection, fleet-wide
`db/005_submit.sql:319-331` never bounds the *asserted* `hlc_wall`; `db/020_apply_remote_event.sql:209-266`
admits the event verbatim (the #102 clamp bounds only the `hlc_state` merge, not admission or
projection ranking). Found independently by the DB-floor and Rust passes. A hostile-but-enrolled
agent (exactly the Spike-0002 / ADR-0030 threat actor: `cairn_agent` + `submit_event` EXECUTE)
signs an additive event with `hlc_wall ≈ 2^62`; it is admitted locally, replicates (remote door
clamps only the clock merge), and wins every `ORDER BY hlc_wall DESC` overlay forever —
`patient_name_current`, sex/gender, address, demographic fields, links, disputes,
`medication_thread_attestation`. No honest later event can outrank it; append-only means the only
recovery is operator recall + projection rebuild. The node plane already rejects this class
(`db/007:342-346`); the clinical plane admits it. Half-acknowledged as issue #97, but for a system
whose only repair primitive is a *later* event, an unoutrankable winner is a floor violation, not
a display concern. Cheap, fork-safe fix: mirror the node-plane rejection
(`wall > now + cairn_max_hlc_drift_ms()` ⇒ reject) at the **local** door `submit_event` — nothing
has accepted the event yet, so rejection there cannot fork the fleet. The remote-door policy
(clamp-and-flag vs reject) can then be decided separately for genuinely-broken peers.

### A2 (Important) — The db/016 hard-veto floor is not enforced at any DB door
The only callers of `cairn_match_veto`/`cairn_has_hard_veto` are the Rust auto-apply seam
(`crates/cairn-node/src/auto_apply.rs:131-136`) and tests. `identity.link.asserted` is additive
and unattested by design (`db/018:16-24`), so a hostile agent with raw `submit_event` access can
link two patients carrying a trustworthy-identifier hard-veto clash; `person_member` merges the
charts silently, nothing even flags trust state. The veto is currently L2-convention, not floor —
the exact gap principle 12 says must not exist. Ruling needed: either an agent-signed
(non-human-attested) link faces the veto inside the door, or it lands as `under-review` instead of
silently merging. (A human-attested link *is* the human decision and may pass.)

### A3 (Important) — Suppressing events with a missing/misspelled `target_event_id` fail open
`db/005:290-305` and `db/020:175-188` wrap the target-existence and ADR-0043 owner-gates in
`IF v_targets_other AND (b -> 'payload' ? 'target_event_id')` — key-presence-conditional, so
absence bypasses both gates. `salience.downgrade`/`visibility.suppress` have no
`cairn_event_twin_check` row either. Latent today (nothing in-DB projects suppression yet), but
the first lenient consumer inherits an unowner-gated cross-human suppression path. Contract should
be: `targets_other_author = TRUE` ⇒ `target_event_id` present and valid; the ADR-0048 registry is
the natural home.

### A4 (Important) — Medication threads have no patient-consistency floor (thread re-homing)
`db/031:113-132`: `medication_statement` PK is `medication_id` alone and the overlay updates
`patient_id = EXCLUDED.patient_id`; cessations join by `medication_id` only; `db/033` never checks
that reconciled threads belong to the same patient. A buggy or hostile client re-asserting an
existing `medication_id` under patient B convergently moves the whole thread — including the dose
history — onto B's chart on every node, unflagged. Cross-patient reconciliation produces
mixed-attribution `patient_medication_current` rows. The `chart_dispute` subject-consistency
pattern (`db/023:139-154`; fail loud locally, converge-and-flag on sync) exists for exactly this
rebind hazard and is unused here. Directly feeds the open design decision in issue #177.

### A5 (Important) — `restore_node_event` lacks the clock-drift ceiling: a tampered backup ratchets a fresh node out of the federation
`db/009:39-113` verifies size + signature, then merges `hlc_state` forward unconditionally
(`:105-111`); enrolls need no trust (`:74-80`). One attacker-appended self-signed `node.enrolled`
with `hlc_wall = 2^62` on the sneakernet medium ratchets the restored node's clock permanently
(GREATEST is monotone); every event it then authors is rejected by every peer's drift ceiling.
The #102 fix covered both remote-apply doors and missed this third signed-bytes admission door.

### A6 (Important) — Residual arrival-order divergence in `patient_identifier`/`patient_demographic` tiebreaks feeds the veto floor
`db/010:129-132`, `db/011:171-174`, `db/013:79-90` tiebreak on `value` and not `content_address`:
two Byzantine events sharing (wall, counter, origin, value) but differing in
facets/provenance/profile leave first-applied-wins state, and `cairn_field_clash`
(`db/016:117-147`) reads the winner's precision/provenance — so two honest nodes can compute
**different hard-veto verdicts** from the same event set. The exact class #115 was built to kill;
these two projections also lack the #157 collision recorder. Fix: append `content_address` to the
comparison tuples (needs the additive-ALTER treatment, see D2).

### A7 (Minor) — Attestation gate never binds the body's named `responsibility` contributor to the attester key
`db/005:261-283` / `db/020:146-168`: the token proves *some* enrolled human vouched; the signed
body may claim responsibility for a different human. Projections key on the verified
`attester_key`, but the signed record permanently carries an unverified claim. Needs a cross-check
or a documented ruling.

---

## B. Sync convergence — under-enforced, under-tested

### B1 (Important) — Clinical-plane pull can permanently skip events (HLC watermark, no sweep)
`crates/cairn-sync/src/main.rs:1087-1125` fetches `EventsAfter(min(watermark, quarantine_floor))`
ordered by HLC with no full sweep. Any event landing in a peer's log with an HLC below an
already-advanced watermark (multi-hop arrival; an L2 writer with an older self-stamped wall) is
never fetched — a silent set-union violation. The node plane hit and fixed exactly this (#38:
local-insertion-order `seq` cursor + periodic sweeps, `crates/cairn-node/src/sync.rs:47-89`); the
clinical plane still orders by the one key that can skip. Decide: rebuild clinical sync on the
node-plane seq model (and say so somewhere load-bearing), or port the #38 treatment.

### B2 (Important) — Clinical quarantine quota counts acked rows: the documented remedy cannot unfreeze the watermark
`crates/cairn-sync/src/main.rs:1049-1052` has no `AND NOT acked` in the per-peer quota subqueries;
the node plane deliberately excludes acked rows so "ack to release" works
(`crates/cairn-node/src/sync.rs:420-427`, with the comment explaining why). After a >quota corrupt
burst, acking everything (the error message's own instruction, `main.rs:1076-1082`) leaves the
watermark frozen forever; the only real remedy is an undocumented manual DELETE. Copy the
predicate.

### B3 (Important) — cairn-sync's SCHEMA subset is rotten: its own doors call functions the subset never installs
`crates/cairn-sync/src/main.rs:35-67` loads {001–006, 020, 021, 026} only, but `db/005:345` and
`db/020:241` unconditionally `PERFORM cairn_learn_attachment_refs(b)` (defined only in db/027) and
`db/002:82-85` calls the db/029 collision recorder. PL/pgSQL late binding hides it until the first
write: a standalone `cairn-sync init` database (the documented walking-skeleton flow) gets a total
write outage on the first `submit_event`. CI stays green only because all test binaries share one
database and cairn-node's loader (all 34 files) happens to run first — alphabetical test order is
the only guard. Fix: add 027+029 to the subset AND add a test running both doors against a
database loaded from cairn-sync's subset alone. Two migration lists = a mirrored surface with no
drift guard.

### B4 (Important) — The two-node E2E tests self-skip in CI; the medication stream has zero remote-apply coverage
`federation.rs`/`sync_watermark.rs` require `CAIRN_TEST_PG2`/`PG3`, which `rust.yml` never sets —
they skip on every CI run, so the system's flagship guarantee (author on A, apply on B, identical
projections) is verified only by hand. And `crates/cairn-node/tests/apply_remote_event.rs`
exercises demographics/identity/suppression only: no `clinical.medication.*` event has ever been
driven through db/020 in a test — in particular the attestation-token round trip
(`clinical.medication-attestation.asserted` at the apply door is exactly where the PR #183 M1
class of bug lives). Issue #176 is the same species. Cheap: set `CAIRN_TEST_PG2` in CI (second DB
on the same cluster), un-skip, add medication cases + one A→B projection-equality test.

### B5 (Important) — Unknown event types are refused by peers until they upgrade — and no code plane exists yet
`db/020:143` fails closed on an unregistered `event_type` (correct: can't classify
additive-vs-suppressing). Consequence: every new slice's events are refused by every peer that
hasn't taken a code update, and the ADR-0012 distribution plane that would carry that update has
zero implementation (`packaging/` is placeholders; `docs/spec/deployment.md` is 11 lines).
`docs/spec/sync.md:56` over-promises ("forward-compatibly" holds for unknown *fields*, not unknown
*types*). Likely the design intent is "refusal + durable re-offer *is* the contract" — but that
needs an ADR-level statement, and the re-offer backlog cost grows with fleet version skew.

### B6 (Minor) — `node.superseded` cannot replicate
`db/007:324-328` omits it from the apply-door op map while submit (`db/007:187-192`) and restore
(`db/009:67-69`) emit it — peers skip-and-sweep it forever (permanent node-plane set-union
exclusion + sweep noise). Either a missing arm or an undocumented lineage-stays-local rule.

### B7 (Minor) — Framing and fingerprint hygiene
`read_frame` (`crates/cairn-sync/src/main.rs:268-275`) allocates up to 4 GiB from an untrusted
length prefix on both puller and server (node plane caps at 8 MiB with rationale —
`sync.rs:124-146`); `do_fingerprint` (`main.rs:726-728`) orders by TEXT without `COLLATE "C"`, so
two honest nodes can emit different event-hashes for identical sets — a false divergence alarm in
the tool meant to prove convergence. Byte-tier thread swallows all errors (`main.rs:1895`).

---

## C. Closing can't-retrofit windows (wire + crypto posture)

### C1 (Critical, window closing) — The erasure ladder is unreachable for the default deployment
ADR-0005's crypto-shred rungs only work on bodies sealed under a per-record DEK at write time —
and per-record encryption is off by default; `data-model.md` §3.5 itself concedes the shape cannot
be retrofitted without re-encrypting history. So in a default deployment: (a) retention-ceiling
destruction and no-retention-basis GDPR requests hit permanently un-shreddable plaintext;
(b) the clinically common "sensitivity recognized later" case cannot retro-seal a body already
replicated in plaintext to N nodes — the sensitivity stream can only raise visibility overlays.
This is a major policy-facing limitation hiding behind a "resolved" label, and it argues for
deciding the **seal-by-default posture before the first production clinical event** — a day-one,
can't-retrofit choice. Related unconfronted collisions (all cheap to close in prose now):
the twin's location under seal is implied, never stated (a future implementer materializing
`plaintext_twin` into an FTS column turns every seal into a full-content leak); every ADR-0048
check fn reads the plaintext body, so the seal path either bypasses the mandatory-twin floor or
needs a different door — unspecified; and the mandatory clear-text attachment descriptor survives
figure-granular shred by design ("photo of self-harm scars, left forearm" outlives the pixels).
Recommended: one walking-skeleton **seal slice** (seal-at-write → twin-under-seal →
safety-projection sibling → crypto-shred → restore-replays-shred) to surface all of these while
they are prose problems.

### C2 (Important, window closing) — The authorship wire vocabulary is violated by every production event
Every orchestrator mints contributors with `role: "recorded"` — which is in no ADR and not in the
ADR-0028 **closed** enum ("a new member is an ADR-recorded act"); ADR-0049's own Context quotes it
without noticing. No floor check anywhere validates role membership, so the closed enum is
convention, not floor. Additionally §3.9 specifies responsibility as structured
`{held_by, on_behalf_of}` ("the column exists from day one"); the implementation ships a flat
string `responsibility: "attested"` (`crates/cairn-node/src/medication/attestation.rs:59`) — the
proxy case, load-bearing for principle 10, is not expressible on the current wire. These are
signed immutable artifacts accumulating daily; additive-only evolution means `recorded` can never
be excluded, only ratified retroactively. ADR-0040 set the precedent: pre-production is the
cheapest this will ever be. One small ADR: ratify-or-rename `recorded`, decide the on-wire
responsibility shape, add the enum check to the floor.

### C3 (Important) — The first clinical stream ships without §3.9/§3.10 human authorship, and the gap has no owning slice
As built, the clinician who ordered the medication is absent from authorship (only the node key,
role `recorded`) — visible only if they optionally attest afterwards. By §3.9's own definition,
every un-attested medication row is machine-generated content; the §5.10 informational floor would
render the whole med list "un-vouched machine content." The interim is acknowledged (ADR-0049
Context), but the attribution-token/authoring-human path (`session.user ≠ event.author`, sign-as,
per-write attribution) has no scheduled slice while attestation — the *second* half of the
principle-10 split — shipped first. Each additional device-additive stream deepens the retrofit.

### C4 (Important) — Actor-registry federation semantics are unspecified, and fail-closed enrolment contradicts set-union custody
Two partitioned nodes can each locally-validly enrol the same key under different `actor_id`s (or
the same pinned set under two keys). On reconnect, set-union delivers both signed enrol events; a
sync-apply door that mirrors ADR-0044/0046 must refuse a signed event (violating "never reject"
custody), while accepting both recreates the silent dual-mapping the guards exist to prevent. No
ADR specifies the resolution (quarantine + adjudication? deterministic tiebreak + supersede?).
Meanwhile the attestation gate trusts only the local registry (#154), so a human's vouch is not
portable across nodes. Everything accountability-shaped now routes through `actor_current`; this
is the next place a "resolved" label meets a partition. (#172 already flags the door-mirroring
half.)

### C5 (Important) — The distribution plane's trust root is a single steward key
`security.md` §7.6 / ADR-0012 sign code — the highest-blast-radius artifact (native extensions in
the DB trusted base) — against one steward key with no threshold signing, rotation,
compromise-recovery, or succession story, while mere timestamps (ADR-0027) and federation
credentials (ADR-0017/0018: "never mandate a single anchor — that builds the kill-switch") both
got multi-anchor treatment. A steward-key compromise is signed-RCE into every upgrading node;
steward capture is the mission's own named adversary. No ADR owns this.

---

## D. The migration/replay model

### D1 (Critical, latent) — Replay-everything-on-connect allows a silent safety-floor downgrade
`connect_and_load_schema` (`crates/cairn-node/src/db.rs:269`) replays all embedded `db/*.sql` on
every connect; there is no `schema_version` table, no checksum, no refusal rule. An **older binary
connecting to a newer database `CREATE OR REPLACE`s old function bodies over the newer floor** —
including safety-floor check functions — with no error and no trace. Two binary versions touching
one DB (a pilot mid-upgrade; the future GUI sidecar; any second tool linking the loader) is all it
takes. This is the ADR-0048 incident class in another coat. Cheapest first brick of the ADR-0012
code plane: a `node_schema(version, loaded_at, loader_build)` table + a loader guard that refuses
to replay when the recorded version exceeds the binary's embedded version + one "old binary, new
DB" test. An afternoon of work; the hazard goes live at the first pilot upgrade.

### D2 (Important) — Five overlay tables were widened in place with no idempotent ALTER
`patient_chart.demo_content_address` (`db/002:57`), `patient_link.content_address`
(`db/018:97`), `chart_dispute.content_address` (`db/023:95`), `chart_identity_state.content_address`
(`db/024:97`), `name_repudiation.content_address` (`db/025:120`) were all added by editing the
`CREATE TABLE IF NOT EXISTS` text (#115) — on any database created before that and replayed in
place, the CREATE no-ops, the trigger INSERTs name a missing column, and those event types get a
total write outage at trigger depth. Dev DBs were likely rebuilt (pre-clinical posture), but the
pattern has now recurred five times against the project's own additive rule (`db/001:127-141` and
`db/007` show the correct `ADD COLUMN IF NOT EXISTS` pattern). Mechanical fix + a lint/guard test
for the pattern.

### D3 (Important) — No reprojection story
Projections are trigger-populated; a `CREATE OR REPLACE` of projection logic heals only future
inserts. The only backfill in the tree is the bespoke `cairn_demographic_backfill()` (`db/013`) —
which also runs a full unindexed `event_log` scan on **every connect** (`db/013:180`; no
`event_type` index exists), linear in log size on Pi-class nodes. ADR-0045 shipped
"projection-read-side only" while trigger-materialized winner tables settled under the old
comparison wherever no backfill exists. Needed: a generic `cairn_reproject(...)` (or an ADR-0048
registry `backfill_fn` column) + the written rule "a projection fix ships with its idempotent,
convergent backfill" + one measured full-replay number at Bet-B volume on the Pi.

---

## E. Matcher (advisory tier)

### E1 (Important) — The learned auto threshold silently loses its safety anchor on an empty non-match set
`matcher/src/cairn_matcher/eval/learner.py:144-148`: when the training partition has zero
non-match pairs (single-cluster dataset; 2 clusters with `--folds 2`), `derive_thresholds` falls
back to anchoring `auto` on the *weakest true match* instead of the strongest impostor — and
nothing flags it (no metadata bit, `model_io` serializes happily). The documented invariant
"auto = max(non-match)+margin ⇒ zero false auto-links" is vacated without warning. Fix: raise, or
set a dedicated metadata flag consumers must check.

### E2 (Important) — Stale pending proposals are never retracted once the pair drops out of the blocking universe
Retraction only fires inside `propose()`, and sweep only proposes pairs blocking currently
generates (`runner.py:99-112`, `sweep.py:91-107`). The realistic flow — an unconfirmed Doe is
forced-REVIEW against N window-overlap candidates, then fully identified (real DOB replaces the
range, real name added) — removes the pair from every pass, so the pending REVIEW rows sit on the
worklist forever under a nonexistent Doe. The regression test masks it (its `_identify` leaves the
year-range DOB in place so the pair conveniently still blocks). Needs sweep-level reconciliation
or identification-triggered cleanup. (The #135 hazard, one layer deeper.)

### E3 (Minors) — alias-map lookup skips UUID canonicalization that the trust-map applies three lines later
(`runner.py:75-76` vs `:87-91`); `Thresholds` never validates `review <= auto` at the type
(`banding.py`); blocking `lower()` vs scorer `casefold()` diverge on ß/ligatures so casefold-
divergent true pairs are never generated (recall loss only); the generator's `_repair` leaks a
verbatim EXACT name into the scorer/learner world on exactly the hardest pairs (optimistic eval).

Clean: comparator symmetry, positive-only invariants, F-S combiner, cluster-striped crossval,
parameterized SQL, placeholder-exclusion coverage, both drift canaries.

---

## F. Process / test architecture

- **`db/tests/*.sql` run nowhere** (no CI step, no script) — the SQL mirror tier already drifted
  once (#182, 15 vs 16) and will again. Wire a `psql -f` loop into CI or delete the mirrors and
  declare the Rust suite canonical; the halfway state is worse than either.
- **Unguarded drift pairs** (the guarded ones are exemplary; these copy that template):
  cairn-sync's migration list vs the doors' function dependencies (B3); the medication
  content-type list duplicated in `db/034:122-127` vs `attestation.rs:84-88` (slice 5 adds a verb
  to one and every subsequent attestation carries a permanently wrong signed `reviewed_count` —
  the #182 pattern on the wire); two hand-rolled framing implementations (B7).
- **No upgrade/compat testing by construction** — every run replays full schema onto a warm
  cluster; "DB at N, binary at N±1" is untested anywhere (the D1 hazard is invisible to the suite).
- **No property/fuzz testing of floor fns** — they take attacker-controlled JSONB; exactly the
  fuzzing shape.
- **Six near-verbatim copies** of the verb-then-vouch orchestrator block across
  `medication/*.rs` — the pre-ADR-0048 copy-a-chain shape on the Rust side; one shared
  `submit_verb(...)` helper before slice 5.
- **House-rule-6 violations in non-test code**: `cairn-event/src/lib.rs:464,856,877`,
  `cairn-sync/src/main.rs:892-894` (hard-coded key/nonce literals in bench paths).
- **Keystore zeroization stops one layer above `seal.rs`** (`keystore.rs:64-71,120-124`).
- **Stale prose**: `docs/spec/index.md:8` still says demographics-only; root CLAUDE.md says "only
  the §4.4 identifier slice exists"; ROADMAP has two "Slice 30" entries (lines 416, 437);
  HANDOVER (508 lines) + ROADMAP (719) have outgrown "disposable scaffolding".
- **§3.15/§3.16 medication mislabel is systemic and self-propagating** — baked into db/031–034
  headers, ~20 Rust comments, ROADMAP:439, and the **error strings of the locked ADR-0048 registry
  rows** (asserted byte-for-byte by the Rust mirror AND the SQL mirror — a three-place lockstep
  fix). Every new medication migration has copied the wrong header from the previous one, four
  times. Correct home is data-model.md §3.3.

## G. Spec honesty (Important, prose-level)

- **§6.7/§5.2 "every federated node holds the essential snapshot"** is untrue at the tiers the
  mission headlines: ADR-0016's own sizing makes the essential set ~2.5 TB at 100 M (a full-mirror
  node), vs 0.4–1.6 GB for the summary. On a Pi clinic the offline first-contact story is
  hit-detection, not the safety snapshot. Needs a per-tier scope knob + honest degradation prose,
  or a correction.
- **§3.6 t_effective ceiling vs ADR-0027's graded interval**: the write doors enforce a binary
  `t_effective ≤ hlc.wall` against a clock the ADR declared an interval — a slow local clock
  (grade self-asserted, ADR-0027's motivating case) rejects a clinician's *truthful* time, a
  principle-4 violation manufactured by the mechanism. Decide: ceiling reads against
  `t_recorded.upper` + flag above `.lower`? Also db/020 hard-rejects ceiling violations in signed
  foreign events instead of quarantine-and-flag (contrast the deliberate HLC clamp at
  `db/020:243-255`).
- **Paper-parity is normative and falsifiable — and no built workflow has been benchmarked.**
  ADR-0049's every-member-thread-vouched rule composes whole-list sign-off from N attestations
  where paper is one signature on one form; nothing yet guarantees the UI collapses that to one
  gesture. Make the §1.2 benchmark (time/steps/cognitive load vs the paper form) a required
  section of every clinical-surface slice plan, starting with the Tauri client.
- **Quarantine/re-offer floor semantics live only in migration comments** (db/021/022) — the
  `acked = TRUE` permanent custody exclusion is a real policy exception to "never reject, never
  drop" the spec never names; the unbounded pen growth from a hostile credentialed peer belongs in
  the §6 failure table.
- **ADR-0045 convergence claim** holds only among nodes running the same projection code; under
  no-lockstep-upgrade, projection-*semantics* changes alter displayed clinical state
  fleet-inconsistently — nothing governs projection semantics the way additive-only governs the
  wire. Worth one honest sentence + a convention that winner-rule changes are ADR-gated.
- Smaller: human key-loss recovery ceremony unspecified (§7.5); "SMS" named in the §7.3/§7.5
  recovery ladder; deceased-status veto still a stub behind §5.13's "closed set is live" framing;
  known-alias pool vs rung-2 deniable deletion unaddressed.

---

## What to focus on next (ranked)

1. **A floor-hardening slice against the hostile enrolled writer** — A1 (HLC wall ceiling at
   `submit_event`; decide remote-door clamp-vs-flag), A3 (suppression target fail-closed via a
   registry row), A4 (medication patient-consistency, folds into #177), A2 (veto at the door for
   agent-signed links), A5 (restore-door drift ceiling), A6 (content_address tiebreaks + D2's five
   additive ALTERs). This is one coherent theme — re-run the Spike-0002 threat model against
   today's floor — and A1 is the single worst finding in the review: silent, fleet-wide,
   unrepairable-by-events, and cheap to fix.
2. **Sync-convergence integrity** — B4 (CAIRN_TEST_PG2 in CI + un-skip the two-node tests +
   medication remote-apply coverage incl. the attestation-token round trip + one A→B
   projection-equality test), then B1/B2/B3 (clinical watermark model, acked-quota predicate,
   the cairn-sync schema subset + its guard test). The convergence guarantee is the product; today
   it is enforced by hand and holed in three places.
3. **The two closing wire windows, on paper first** — C2 (one small ADR: ratify/rename `recorded`,
   fix the responsibility shape, enum check at the floor) and C1 (the seal-by-default decision +
   a walking-skeleton seal slice). Both get strictly more expensive with every clinical event
   authored; both are ADR-0040-class "cheapest it will ever be" items.
4. **The D1 schema-version guard** — `node_schema` table + loader refusal + one downgrade test.
   An afternoon; kills the silent floor-downgrade hazard before the first pilot upgrade makes it
   live; first brick of the ADR-0012 code plane.
5. **Process mechanization, one session** — decide the db/tests question (CI or delete); add
   drift guards for the three unguarded pairs (F); fix the §3.3 mislabel once across the
   three-place lockstep; factor the six-fold verb-then-vouch copy before medication slice 5.
6. **Actor-registry federation semantics (C4) as a design session** — the sync-apply merge rule
   for concurrent enrolments (#172) + attestation portability (#154). Everything
   accountability-shaped routes through `actor_current`; settle the partition semantics before
   clinical federation, not after.
7. **Resume feature work through the API seam** — when the Tauri client resumes, make its IPC
   boundary the first thin ADR-0023 native-API slice (capability descriptor + two read endpoints)
   so the GUI becomes the API's first conformance client instead of coupling to projection
   internals; and attach the first paper-parity benchmark note to it (G).

Deprioritized with reasons: further matcher B3 refinements and medication slices 5+ are safe,
additive, well-drilled, and can resume any time — items 1–4 above get more expensive with every
slice stacked on top of them. E1/E2 (matcher) are real but advisory-tier; fold them into the next
matcher session.

---

## Finding → issue map

Every finding was filed on GitHub on 2026-07-15. Pre-existing related issues noted in parentheses.

| Finding | Severity | Issue |
|---|---|---|
| A1 future-HLC wall projection capture | Critical | #187 |
| D1 silent floor-downgrade on replay (schema-version guard) | Critical | #188 |
| C1 seal-by-default decision + seal slice | Critical (window) | #189 (rel. #92) |
| A2 hard-veto floor is L2-only | Important | #190 |
| A3 suppression target fails open | Important | #191 |
| A4 medication thread re-homing | Important | #192 (rel. #177) |
| A5 restore-door drift ceiling | Important | #193 (rel. #102) |
| A6 value-tiebreak divergence feeds veto | Important | #194 (rel. #115) |
| A7 attestation responsibility not bound to attester | Minor | #195 |
| B1 clinical watermark can skip events | Important | #196 (rel. #38) |
| B2 acked rows count against quarantine quota | Important | #197 |
| B3 cairn-sync schema subset missing 027/029 | Important | #198 |
| B4 two-node E2E skipped in CI + no med remote-apply | Important | #199 (rel. #176) |
| B5 unknown event types refused; no code plane | Important | #200 (rel. #98) |
| B6 node.superseded cannot replicate | Minor | #201 |
| B7 framing/fingerprint/byte-tier hygiene | Minor | #202 |
| C2 role:"recorded" not in enum; flat responsibility | Important (window) | #203 (rel. #96) |
| C3 first clinical stream lacks human authorship | Important | #204 |
| C4 actor-registry federation semantics unspecified | Important | #205 (rel. #172/#154/#94) |
| C5 single steward key as distribution trust root | Important | #206 |
| D2 five overlay tables widened without additive ALTER | Important | #207 |
| D3 no reprojection story; backfill scans every connect | Important | #208 |
| E1 learner threshold loses safety anchor | Important | #209 |
| E2 stale pending proposals never retracted | Important | #210 |
| E3 matcher minors (alias/thresholds/casefold/repair) | Minor | #211 |
| F test-infra: db/tests unrun, drift pairs, verb copy | Process | #212 |
| F Rust hygiene batch (zeroize/rule-6/lock/recovery) | Minor | #213 |
| §3.3 medication mislabel (systemic) | Important | #214 |
| G spec prose honesty batch | Minor/Important | #215 |
| t_effective ceiling vs graded interval | Important | #216 (rel. #97) |
| paper-parity benchmark as required slice section | Important (process) | #217 |
