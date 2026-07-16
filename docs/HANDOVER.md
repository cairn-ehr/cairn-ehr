# HANDOVER — Cairn

## ⇒ NEXT: the 2026-07-15 whole-project review course — P1 + P2 DONE, start at P3

A five-pass whole-project review ran 2026-07-15 (in-DB floor, Rust workspace, spec/ADR corpus,
matcher, cross-cutting seams). Full report: [`docs/code_reviews/2026-07-15-whole-project-architecture-review.md`](code_reviews/2026-07-15-whole-project-architecture-review.md);
every finding is filed as a GitHub issue (#187–#217) with a finding→issue map at the foot of the
report. **Fix in this order** — the ordering is deliberate: items 1–4 get *more* expensive with
every clinical slice stacked on top of them; the matcher/medication feature work is safe to resume
any time and is explicitly deprioritized behind them.

**Standing gate:** whole-project review cycles like this one repeat periodically, and there will be
**no release for clinical use before repeated review cycles pass cleanly.** The review findings
(and the exploit detail now public in #187–#217) make closing P1 and P4 a hard precondition for any
pilot carrying real data.

**Priority 1 — ✅ DONE 2026-07-16** (the floor-hardening slice against the Spike-0002 hostile
enrolled writer; TDD, one branch `feat/floor-hardening-spike0002-p1`, all eight in dependency
order, one commit each; full workspace 640/0 failed; detail in the session block below):
~~#187~~ (local-door drift-ceiling reject) · ~~#207~~ (five additive ALTERs + the pc.* view
upgrade-heal + `migration_replay_widening.rs` guard) · ~~#194~~ (`content_address` tiebreak on
the two set-union demographic projections) · ~~#191~~ (suppression target gate fails closed at
both doors + registry rows) · ~~#192~~ (medication thread patient-consistency, **resolves #177**)
· ~~#190~~ (un-attested `identity.link` faces the db/016 veto at the door; sync path flags +
`under-review`) · ~~#193~~ (drift ceiling at the restore door) · ~~#195~~ (responsibility
contributor bound to the verified attester). The #187 remote-door policy for genuinely-broken
peers stays the deliberate db/020 clamp-and-admit (documented in `hlc_drift.rs`).
**PR #219 code-review round (2026-07-16, same branch):** fixed two review findings — the #190
`link_veto_flag` lifecycle now derives from the STANDING overlay winner, not bare event arrival
(closes a backdated-unlink silent-merge + a stale-link phantom flag; +2 RED tests), and
`medication_group_cross_patient` now derives a member's patient from `cairn_medication_thread_patient`
so a cessation-only thread is not hidden (+1 RED test). Workspace **643/0**. One follow-up filed:
[#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) — the #190 hard veto is still evaluated only
at link-arrival time (a silent vetoed merge if the clashing demographics sync in later; needs a
re-check hook or background sweep — out of scope for the floor-hardening slice).

**Priority 2 — sync-convergence integrity (the flagship guarantee). ✅ COMPLETE 2026-07-16**
(B5/#200 — the "refusal + durable re-offer IS the contract" ADR — lives in the Priority 6 design queue, not here.)
- ~~**#199 (B4)**~~ — ✅ **DONE 2026-07-16** (the P2 opener, PR #221; detail in the session block below):
  CI provisions `cairn_test2`/`cairn_test3` + exports `CAIRN_TEST_PG2`/`PG3` (federation/watermark
  suites no longer self-skip anywhere), `medication_remote_apply.rs` drives every medication verb +
  the attestation-token round trip through db/020 (incl. the #176 oversize clamp-and-flag remote
  branch), and cairn-sync's `clinical_pull.rs` proves A→B projection equality through the real
  binary. The safety net for the rest of P2 is in place.
- ~~**#198 (B3)**~~ — ✅ **DONE 2026-07-16** (branch `fix/sync-schema-subset-198`; detail in the session
  block below + ROADMAP Slice 37): db/027+029 added to cairn-sync's SCHEMA subset, plus the standing
  `schema_subset_tests` drift guard — a wiped `cairn_test2` loaded from the subset ALONE must satisfy
  both write doors (attachment-ref learning, the db/029 collision predicate AND recorder via a real
  Byzantine pair). A future door→function edge into an unlisted migration fails the test with the
  production error instead of shipping a first-write outage.
- ~~**#196 (B1)**~~ — ✅ **DONE 2026-07-16** (branch `fix/clinical-sync-seq-cursor-196`; detail in the
  session block below + ROADMAP Slice 38): ported the #38 seq-cursor + periodic-sweep treatment.
  `db/036` adds `event_log.seq` (node-local insertion order — a low-HLC multi-hop arrival still sorts
  above the cursor), `sync_state.last_seq` (per-peer cursor), `sync_state.quarantine_floor_seq` (the
  re-offer floor, kept a SEPARATE persisted column so it self-clears on a clean cycle — a
  derive-from-rows floor would re-ship forever after a transient corruption heals), and
  `sync_quarantine.refused_seq` (forensics). `do_pull` cursors on seq; `cmd_run` sweeps every 10 cycles
  (`cmd_pull --full`); wire gains additive `EventsAfterSeq`/`seqs[]`. Headline regression guard: a
  low-HLC-below-cursor event still converges A→B.
- ~~**#197 (B2)**~~ — ✅ **DONE 2026-07-16** (branch `fix/quarantine-quota-acked-197`; detail in the session
  block below): `AND NOT acked` copied into both clinical quarantine quota subqueries (rows + bytes),
  mirroring the node plane — acking, the error message's own documented remedy, now actually frees the pen.
- ~~**#202/#201 (B7/B6)**~~ — ✅ **DONE 2026-07-16** (the P2 closers, branch
  `fix/sync-hygiene-202-node-supersede-201`; detail in the session block below + ROADMAP Slice 40):
  cairn-sync `read_frame` 64 MiB cap (refuse-before-allocate, batch-scale because the events response
  is #101-unpaginated), `COLLATE "C"` on BOTH fingerprint orderings (`node_origin` + the
  same-failure-mode `patient_id::text`) under a standing drift guard, the byte-tier `Err` arm logs;
  `node.superseded` resolved as **REPLICATE** — the db/007 apply door gained the supersede arm
  (trust-bounded like peer/revoke; feeds only the advisory `node_lineage` view).

**Priority 3 — the two closing wire windows (on paper first; cheapest they will ever be). ⇐ CONTINUE HERE**
- **#203 (C2)** + **#96** — one small ADR: ratify-or-rename `role:"recorded"` against the ADR-0028
  enum, decide the on-wire responsibility shape (`{held_by,on_behalf_of}` vs the shipped flat
  string), add the enum-membership check to the floor.
- **#189 (C1)** + **#92** — decide the seal-by-default posture, then a walking-skeleton seal slice
  (seal-at-write → twin-under-seal → safety-projection sibling → crypto-shred → restore-replays-shred)
  that surfaces the twin-location / twin-floor / descriptor collisions while they are still prose.
- **#204 (C3)** — schedule the attribution-token / authoring-human slice (pairs with C2).

**Priority 4 — the schema-version guard (an afternoon; retires a Critical latent hazard).**
- **#188 (D1)** — `node_schema(version, loaded_at, loader_build)` + a loader refusal when the
  recorded version exceeds the binary's embedded version + one "old binary, new DB" test. First
  brick of the ADR-0012 code plane; goes live at the first pilot upgrade.

**Priority 5 — one process-mechanization session (attacks the mechanism that produced #182).**
- **#212 (F)** — decide the `db/tests/*.sql` question (wire into CI or delete); add drift guards
  for the three unguarded Rust↔SQL pairs; factor the six-fold verb-then-vouch copy **before**
  medication slice 5.
- **#214** — fix the §3.15/§3.16→§3.3 medication mislabel once, across the three-place registry
  lockstep (registry rows + Rust mirror + SQL mirror together).
- **#215 (G)** — spec prose honesty batch (index.md/CLAUDE.md staleness, the duplicate Slice 30,
  quarantine-floor + ADR-0045 caveats).
- **#213** — Rust hygiene batch (keystore zeroize edges, house-rule-6 bench literals, auto_apply
  lock leak, recovery-code mapping, gui merge fallback).

**Priority 6 — design sessions (no rush, but settle before the dependent feature work).**
- **#205 (C4)** — actor-registry sync-apply merge/quarantine/adjudication semantics (#172/#154);
  settle before clinical federation, not after.
- **#206 (C5)** — distribution/policy-plane trust-root governance (threshold steward signing / key
  transparency); no ADR owns it.
- **#200 (B5)** — an ADR stating "refusal + durable re-offer *is* the sync contract for unknown
  types"; correct the `sync.md` §6.5 over-promise. Pairs with the code-plane work (#188).
- **#208 (D3)** — a generic reprojection mechanism + the written "a projection fix ships with its
  backfill" rule + one measured full-replay number at Bet-B volume.
- **#216** — decide the `t_effective` ceiling semantics against ADR-0027's graded interval
  (write-door bound + remote-door quarantine-vs-reject).
- **#217** — make the §1.2 paper-parity benchmark a required section of every clinical-surface
  slice plan, starting with the Tauri client.

**Explicitly deprioritized (safe to resume any time, behind 1–6):** matcher #209/#210/#211,
further medication slices (5+), matcher B3 measurement work. These are additive, advisory-tier or
well-drilled; nothing above is blocked on them and they get no more expensive by waiting.

---

**Session date:** 2026-07-16, latest (#202+#201 — the P2 closers: cairn-sync wire hygiene + the
`node.superseded` apply arm, **P2 now complete**; earlier the same day #197 quarantine quota, #196 the
clinical seq cursor, #198 the SCHEMA subset, #199 the P2 opener, and the P1 floor-hardening slice;
review course above; last full regeneration 2026-07-14) ·
**Spec/ADRs:** v0.51 · **Phase:** architecture complete; **first
production clinical surface under construction** — demographics on `cairn-node` (slices 1–5 done) + the §5.2 matcher
(advisory Python: piece A in-DB veto floor + B1 scoring core + B2/B2b veto-gated pipeline/blocking + B3 eval
harness/compound-keys/volume-generator/weight-learning + clinician-observed range-DOB evidence + composite-sex
scoring & the unconfirmed-chart REVIEW rule; full detail in ROADMAP slices 6–25) +
the **§5.7 identity core: C1 linkage · C2 human-accepted apply seam · C2b auto-apply of the `auto_candidate` band · C3
`dispute` + the chart trust-state projection · C4 `identify` + the *unconfirmed* trust state · C5 `repudiate` + the
known-alias pool** (the §5.7 confirmed/unconfirmed/under-review contract is COMPLETE) + the
**§5.4 John-Doe registration front door, slices A–D all BUILT** (callsign minting + matcher placeholder exclusion ·
clinician-observed evidence · the birth-year-range blocking pass · administrative-sex scoring + the unconfirmed-chart
forcing rule, [#130](https://github.com/cairn-ehr/cairn-ehr/issues/130) closed);
remaining **B3 measurement-driven follow-ons** (learn against a large hand-crafted gold set; locale comparator packs;
hub-tier aggressive duplicate sweep)
+ the **§5.4 photo evidence slice** (the first content-addressed **attachment** on a clinical surface; ADR-0042 froze
the §3.14 day-one attachment-reference shape)
+ the **§5.4 marks/belongings/EMS-context text-evidence slice** (three text `kind` values on the same
`identity.evidence.asserted` event type — **done this session**)
+ the **§5.4 finishers PR#1** (a node-local "this node's John Doe #N" display ordinal + an `--observed-year`
evidence override)
+ the **§5.4 finisher 3** (`identify`→optional link — the John-Doe *resolution* front door: a device-additive
`identify` flips the chart *confirmed*, plus an OPTIONAL human-attested link to a prior chart, atomic —
**done this session**; the structural finishers 1–3 are now all built)
+ the **§5.4 `enroll-human` ceremony CLI** (enrol a clinician's key as a `kind='human'` actor — the
`identify --link` prerequisite; **done this session**)
+ identity **C5+** (`reattribute` — waits on a clinical-note surface) + the **rest of the §5.4 subsystem**
(the "prior history now available" push-alert; the search-before-create funnel).
+ the **first clinical-content event stream, `clinical.medication` slice 1 — BUILT** (assert/cease verbs,
db/031 floor, `medication_statement`/`medication_cessation` projections, `patient_medication{,_current,_past}`
views + the E1 reconciliation flag, orchestrators + CLI; distinct from the identity/demographics surfaces
above — the first stream carrying actual clinical content; PR #171, on main)
+ the **medication dose overlay, slice 2 — BUILT this session** (`clinical.medication-dose-change`/`-dose-correction`
verbs, `db/032` floor + a bitemporal dose timeline [point-0 seed + change + HLC-wins correction overlay] +
`patient_medication_dose_history`, current/past reworked to the timeline dose, `change_dose`/`correct_dose`
orchestrators + CLI; db/031 untouched).
+ the **medication cross-thread reconciliation resolution, slice 3 — BUILT this session** (ADR-0047;
`clinical.medication-reconciliation`/`-separation` verbs, `db/033` — a symmetric reversible LINK between two
`medication_id` threads [never a false cessation] with min-UUID connected-component collapse to one current-list
row [mirrors identity `patient_link`/`person_member`], latest-effective group status, flag clears without a
cessation; PR #178).
+ the **twin-check registry refactor** ([#173](https://github.com/cairn-ehr/cairn-ehr/issues/173); ADR-0048) —
one stable `cairn_event_twin` dispatcher over a locked registry table, killing the copy-the-whole-IF/ELSIF-chain
floor-regression hazard; a new event type now registers ONE additive row.
+ the **medication attestation slice, slice 4 — BUILT** (ADR-0049; `clinical.medication-attestation.asserted`,
`db/034` — a separable per-thread human-attested responsibility overlay through the existing db/005 gate;
staleness by convergent set-commitment compare, not a head-position pin [closes the lower-HLC late-arrival gap];
supersede-never-retract; `--attest-as` on all six verbs + `medication-attest` CLI; PR #182, on main).
+ the **medication-attestation hardening + coverage — done this session** (closes
[#181](https://github.com/cairn-ehr/cairn-ehr/issues/181)): one real floor improvement (M1 — a
responsibility-less attestation body now gets a legible floor rejection, not a cryptic NOT-NULL) + five
coverage tests + a fixed stale SQL-mirror row count (15→16); no ADR/spec/SCHEMA/event-type change.
+ the **medication dose effective-date/reason correction, slice 5 — BUILT this session** (ADR-0050; `db/035`
— the `-dose-correction` verb becomes a **per-field patch** [dose/effective/reason each set|strike|keep], the
corrected effective date drives current-dose winner selection [bitemporal repair], `schema_version` /1→/2; no
new event type; PR #186, on main).
+ the **L3 clinician reference-UI shell, slice 1 — BUILT** (a standalone `cairn-gui/` workspace with a
framework-agnostic contract/port/manifest/routing core; **Spike 0004 resolved — iced FAILS the accessibility bar**,
so the reference desktop UI **pivots to Tauri 2**, an L3 framework choice *below* the compatibility boundary — no
ADR/spec/wire change; PR #174, on main).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node,
Postgres-on-Android).

**This session (2026-07-16, latest) — #202 + #201 [B7/B6]: the P2 closers (branch
`fix/sync-hygiene-202-node-supersede-201`; no ADR/spec/SCHEMA/event-type change; full detail in ROADMAP
Slice 40 + git).** **#202** (cairn-sync hygiene triple; the node plane already had each hardened version):
`read_frame` now refuses a length prefix over the new `MAX_FRAME_BYTES` (64 MiB) BEFORE allocating — a
hostile/corrupt u32 prefix could previously demand a 4 GiB allocation on both wire ends (server: any client
reaching the port; puller: the peer's response). The cap is batch-scale, NOT the node plane's per-event 8 MiB,
because the events response is #101-unpaginated (a full sweep ships the whole log suffix in one frame) — a log
outgrowing it fails the sweep loudly with the cap named; pagination (#101) stays the real fix.
`do_fingerprint` pins BOTH TEXT sort keys with `COLLATE "C"` (the ADR-0045/#69 discipline — the review named
`node_origin` in event_hash; `patient_id::text` in projection_hash had the identical failure mode): two honest
nodes with different cluster collations no longer raise a false divergence alarm from the very tool meant to
prove convergence; the SQL is extracted to consts under a standing drift guard (the #159 pattern), validated
on PG18. The byte-tier thread's silent `Err(_) => 0` arm now logs a unit-tested line — a permanently failing
blobd pass (bad conn string after a DB restart, schema skew) was indistinguishable from "no blobs to fetch"
for the life of the process. **#201** resolved as **REPLICATE**, not lineage-stays-local: db/007's
`apply_remote_node_event` gained the missing `node.superseded` arm — submit (db/007) and restore (db/009) both
emit/apply it, so the omission left a peer pulling a restored node's history refusing the lineage event on
EVERY full sweep forever (busy-loop noise + a permanent set-union exclusion on the node plane). Admission is
trust-bounded exactly like peer/revoke (the author must resolve to an active peer) and the claim feeds ONLY
the advisory `node_lineage` view — `node_current` resolves keys from `enroll` rows alone, `trust_peer` reads
only `peer`/`revoke` — so a false supersede hijacks nothing (principle 2: an attributable, signed claim). A
stays-local comment could not have fixed the wedge anyway (the serve stream ships the whole `node_event` set),
and ADR-0026's cold-peer durability model wants peers holding the COMPLETE set. TDD RED-first throughout (the
frame test failed UnexpectedEof-not-InvalidData on the doomed-allocation path; the admission test failed with
the production "unknown node event_type node.superseded" verbatim); the admission test covers admit + lineage
row + set-union idempotent re-apply + deny-all stranger + legible malformed refusal. Workspace **674/0
failed** + fmt + clippy `-D warnings` clean.

**Prior sessions (2026-07-16, same day, condensed — full detail in git + the PRs + ROADMAP Slices 36–39):**
the **P1 floor-hardening slice** (#187/#207/#194/#191/#192[+#177]/#190/#193/#195; branch
`feat/floor-hardening-spike0002-p1`, PR #219 incl. its review round; follow-up
[#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) filed — the #190 hard veto is still evaluated only
at link-arrival time) · the **P2 opener #199 [B4]** (PR #221 — CI provisions `cairn_test2`/`cairn_test3` +
exports `CAIRN_TEST_PG2`/`PG3` so no Rust test self-skips in CI anymore; `medication_remote_apply.rs` drives
every medication verb + the attestation-token round trip through db/020; `clinical_pull.rs` proves A→B
projection equality through the real binary) · **#198 [B3]** (PR #222 — db/027+029 appended to cairn-sync's
SCHEMA subset + the standing `schema_subset_tests` drift guard: a wiped DB loaded from the subset ALONE must
satisfy both write doors) · **#196 [B1]** (PR #223 — `db/036` clinical seq cursor: `event_log.seq`, per-peer
`sync_state.last_seq`, the SEPARATE self-clearing `quarantine_floor_seq` [a derive-from-rows floor would
re-ship forever after a transient corruption heals], additive `EventsAfterSeq`/`seqs[]` wire, `cmd_run` full
sweep every 10 cycles) · **#197 [B2]** (PR #224 — `AND NOT acked` on both clinical quarantine quota
subqueries, mirroring the node plane: acking, the quota error's own documented remedy, now actually frees the
pen).

**Prior session (2026-07-15) — medication dose effective-date/reason correction, slice 5 (ADR-0050, spec
v0.50→v0.51; branch `feat/medication-dose-effective-correction`; merged PR #186; full detail in git + the ADR
+ ROADMAP Slice 34).** Closes slice 2's honest gap: the `-dose-correction` verb fixed the dose *value* only, so
a mis-keyed effective date (which drives current-dose winner selection) and clinical reason were uncorrectable.
The correction is now a **per-field patch** of a targeted dose point — three groups `dose`/`effective`/`reason`,
each **set** (a value) / **struck** (named in a `strike` array → set-unknown) / **kept** (omitted); the
brainstormed decision was patch-not-restatement so fixing one field never wipes the rest (principle 4), with an
explicit `strike` sentinel keeping set-to-unknown first-class. **The corrected effective date drives current-dose
winner selection** (bitemporal repair, not a display label). New `db/035` (db/031–034 untouched):
`ALTER`-extends the db/032 overlay (+`effective_value`/`_precision`/`note` + three touched-flags), idempotent
backfill, the correction floor (strike/conflict/no-op + **non-string reason/note/info_source** guards hardened beyond the plan),
the apply trigger, and **five** reworked views — the two db/032 dose views **and** db/033's three group-rollup
views (a mid-build discovery: `patient_medication_current`/`_past` route through the group rollup, so the 2-view
plan would have shipped an invisible headline; all five kept column-identical, replay-safe). `reason` repurposed
to the point's clinical reason; the correction rationale is a separate `note` (CLI `--correction-note`, renamed
to avoid the flattened attest `--note` clash). `schema_version` /1→/2. Reuses the existing verb: no new event
type, no floor bypass, twin-registry unchanged. Convergence stays **one row per point, HLC-wins wholesale** — a
later correction of the same point supersedes an earlier one (not field-merged; field-merge deferred, needs
per-field HLC). Full workspace green (fmt + clippy `-D warnings`; `cargo test --workspace` **0 failed**;
medication_dose 25, reconciliation 15, attestation 27). Subagent-driven build (6 tasks) + opus whole-branch review
= **READY TO MERGE, 0 Critical/Important-in-scope**. A post-review `/fixall` pass then extended the floor's
type-guards from `reason` to `note`/`info_source` (principle 12 — uniform annotation guard, closes the same
`->>`-stringifies-a-non-scalar gap) and added the previously-deferred `medication_group_status`
cross-cessation-boundary test (a corrected dose-point effective flips the group's active/ceased classification);
+3 tests, all green. **Filed [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185)
(OPEN):** a **pre-existing** (db/032) cross-thread correction **suppression** vector — the overlay's single-column
PK lets an authenticated hostile node evict a legit correction via `ON CONFLICT` (bounded: reverts to original,
event auditable); needs a PK/design decision, not a regression here.

**Prior session (2026-07-14, later) — medication-attestation hardening + coverage (condensed; PR #183, closes
[#181](https://github.com/cairn-ehr/cairn-ehr/issues/181); no ADR/spec/SCHEMA/event-type change — an in-place
db/034 floor edit; full detail in git + ROADMAP Slice 33 follow-up).** Paid down the slice-4 review follow-ups:
M1 = a responsibility-less attestation body now gets a **legible** floor rejection (mirrors the db/005
`e ? 'responsibility'` predicate, fail-closed; well-formed events unaffected) + 6 coverage tests (second-subject
rollback, group-rollup unattested-member/singleton, equal-HLC `content_address` tiebreak, builder permutations,
the direct floor-fn non-array-`contributors` branch) + a fixed stale SQL-mirror row count (15→16). Left open
(both unreachable by well-formed clients): cosmetic `reviewed_count` `u32`→`int4` (#181) + the door-level
non-array-`contributors` legibility gap ([#184](https://github.com/cairn-ehr/cairn-ehr/issues/184)).

**Last session (2026-07-14) — medication attestation slice 4 (condensed; ADR-0049, spec v0.49→v0.50; merged
PR #182, on main; full detail in git + the ADR + ROADMAP Slice 33).** One new **additive** event type
`clinical.medication-attestation.asserted` — a **separable per-thread responsibility overlay** (principle 10)
over an existing `medication_id` thread that trips the **existing** db/005 attestation gate (3-arg door, enrolled
`kind='human'`); device-signed medication events unchanged, a *different* human may vouch later. `db/034`
(db/031–033 untouched): structural floor + `cairn_medication_thread_commitment` (one SQL fn, author *and* read
time — no drift) + append-only projection/rollup. **Staleness is a convergent set-commitment compare, not a
head-position pin** (`stale = current_commitment IS DISTINCT FROM reviewed_commitment`) — closes the
**lower-HLC-late-arrival** gap a position pin would silently absorb. **Supersede, never retract** (no
de-attestation event; a corrective content event flips the prior vouch stale). `--attest-as`/`--basis`/`--note`
thread through all six verbs (atomic verb-then-vouch, one txn; `reconcile`/`separate` attest both) + a
`medication-attest` post-hoc CLI. Post-review (PR #182 `/review`→`/fixall`): a partial functional index on
`event_log ((body->>'medication_id')::uuid)` bounds the commitment read; the device-key-cannot-vouch guarantee
promoted to an automated test. The optional slice-4 follow-ups were tracked on
[#181](https://github.com/cairn-ehr/cairn-ehr/issues/181) → **done this session** (see above).

**Prior session (2026-07-13, later) — the `cairn_event_twin` twin-check registry refactor (condensed; [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173); **ADR-0048**, spec v0.48→v0.49; PR #179; **ZERO behaviour change** — pure de-risking of the safety floor; full detail in git + the ADR).** Killed the verbatim-copy hazard: `cairn_event_twin` was re-declared in 11 migrations, each copying the whole growing IF/ELSIF chain — a stale copy could silently DROP a floor check with no error. Replaced with a locked registry table `cairn_event_twin_check(event_type, check_fn, twin_required_msg)` + a fail-closed load-time validation trigger + ONE stable dispatcher (db/005 only, dynamic `EXECUTE %I`); all check fns unified to `(p_type text, b jsonb) RETURNS void`. **Invariants binding all future slices:** dispatcher declared exactly once (guarded by `twin_dispatch_single_source.rs`); a new event type registers ONE additive row and never touches the dispatcher; missing/mis-signed check fn fails closed at load. Post-review hardened: `search_path` pinned on the hook; the registry-contract test asserts the full 15-row mapping byte-for-byte. Whole-branch review: Ready to merge, 0 Critical/Important.

**Prior session (2026-07-13, clinical) — `clinical.medication` slice 3: cross-thread reconciliation resolution (condensed; PR #178; **ADR-0047**, spec v0.47→v0.48; full detail in git + the ADR + ROADMAP Slice 32).** Removed the slice-1 wart: clearing a duplicate reconciliation flag no longer requires a false cessation. Two additive verbs over a `(low,high)` `medication_id` thread pair — `clinical.medication-reconciliation.asserted`/`-separation.asserted` (never-erase reversal); cross-author reconciliation allowed (ADR-0043 owner-gate N/A). `db/033` (db/031+032 untouched): structural floor + a connected-component `medication_group_member` projection (min-UUID canonical, mirrors db/018 `patient_link`) + collapsed group views (`patient_medication_current`/`_past`, latest-EFFECTIVE-wins group status, replay-safe). `reconcile_medications`/`separate_medications` orchestrators + CLI. 560/0 failed; whole-branch review Ready to merge, 0 Critical/Important. **Filed (still open):** [#176](https://github.com/cairn-ehr/cairn-ehr/issues/176) (oversize remote clamp-and-flag test), [#177](https://github.com/cairn-ehr/cairn-ehr/issues/177) (cross-patient reconciliation guard — needs a design decision). **Deferred (now built):** human-attested responsibility → slice 4, ADR-0049, this session.

**Prior session (2026-07-12) — medication dose overlay, slice 2 (condensed; PR #175, no ADR/spec/SCHEMA change).** Two additive dose verbs (`-dose-change`/`-dose-correction`, offline-first `corrects` not `target_event_id`), `db/032` (db/031 untouched): a bitemporal dose timeline (point-0 seed + change + HLC-wins correction overlay), `medication_current_dose` = latest-EFFECTIVE point, `patient_medication_dose_history`; correction join is thread-scoped (fail-safe no-op on mistarget). Whole-branch review clean.

**Session (2026-07-12, GUI/L3 thread) — reference-UI framework SETTLED: pivot to Tauri 2 (condensed; PR #174; no ADR/spec/wire change — an L3 choice below the compatibility boundary; full detail in [eco-eval 0004](ecosystem/0004-reference-ui-framework-iced-vs-tauri.md) + [Spike 0004](spikes/0004-iced-reference-ui-viability.md)).** First L3 reference-UI work (`cairn-gui/`, framework-agnostic contract/port/manifest/routing core). **Verdict: iced FAILS the accessibility bar** (no AccessKit/a11y tree, empirically confirmed); reference desktop UI adopts Tauri 2 (a11y inherited from the browser); the slice-1 core is reusable behind it. **Next:** the Tauri reference client.

**Prior session (2026-07-12, clinical) — `clinical.medication` slice 1: the first clinical-content event stream (condensed; branch `feat/medication-recording-slice-1`; no ADR/spec/SCHEMA change; full detail in ROADMAP Slice 30 + git).** Distinct from every prior (administrative/identity) slice — the first stream of actual clinical content. Two append-only device-additive verbs over an immortal `medication_id` thread — `clinical.medication.asserted` + `-cessation.asserted` (principle-4 honest-unknown substance/dose fields); `db/031` floor + `medication_statement`/`medication_cessation` projections kept **separate** so an orphan cessation is arrival-order-independent + `patient_medication{,_current,_past}` views + the **E1 advisory reconciliation flag** (`coalesce(inn_code, normalized term)`); `medication-assert`/`medication-cease` CLI; 9/9 DB-gated tests. Post-review fix: `asserted_at` derives from the event's HLC, not local clock. **Deferred (now built by later slices):** dose overlay (slice 2), reconciliation resolution (slice 3), human-attested responsibility (slice 4, ADR-0049, this session).

**Prior session (2026-07-12) — the enroll dual-mapping floor guard: B-direction complement of ADR-0044 (condensed; [#166](https://github.com/cairn-ehr/cairn-ehr/issues/166) CLOSED; ADR-0046; spec v0.46→0.47; PR #170; full detail in git + ROADMAP Phase 2).** #152/ADR-0044 guarded only the A-direction (one `actor_id` ← two keys); the B-direction (one key → two `actor_id`s) was unguarded and would silently NULL that key's authorship node-wide via `submit_event`. Fix: `cairn_key_actor_id_conflict` whole-history predicate + a per-key advisory lock (deadlock-free ordering) in `enroll_actor` (`db/004`); scope = whole-history anti-key-reuse (symmetric with #152, even after revoke). 3 new DB-gated tests incl. a concurrent-race regression guard. Whole-branch review: Ready to merge, 0 Critical/Important. Follow-ups filed: [#169](https://github.com/cairn-ehr/cairn-ehr/issues/169) (test-isolation gap), [#172](https://github.com/cairn-ehr/cairn-ehr/issues/172) (future rotate-key/sync-apply doors must mirror both A+B checks).

**Prior session (2026-07-11, latest) — §5.4 `enroll-human` ceremony CLI (condensed; no ADR/spec/SCHEMA change; full detail in git + ROADMAP Slice 29).** Enrols a clinician's signing key as a `kind='human'` actor carrying an ADR-0044 person-distinguishing determinant — the prerequisite for `identify --link`. `cairn-node::enroll`: pure `build_human_pinned` (honest reject if no determinant) + async `enroll_human_actor` (dual-mapping guard + ADR-0044 collision pre-check over the `enroll_actor` floor); `enroll-human` CLI (mint-if-absent sealed key + shown-once recovery code, pre-mint collision check). 6 pure + 7 DB-gated tests; whole-branch review Ready to merge, 0 Critical/Important. Filed [#166](https://github.com/cairn-ehr/cairn-ehr/issues/166) (now CLOSED, ADR-0046, see top block) and [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168) (entity→role-actor 1:many, open). **Remaining §5.4:** the "prior history now available" push-alert (§5.12); the search-before-create funnel (§5.3/§5.8).

**Prior session (2026-07-11, later) — §5.4 finisher 3: `identify`→optional link (condensed; PR #165; no ADR/spec change).** The last structural finisher of the §5.4 John-Doe subsystem: `cairn-node::identify` — device-additive `build_identify_body` (flips chart unconfirmed→confirmed) + `identify_patient` orchestrator, optionally authoring a human-attested `identity.link.asserted` in the SAME transaction (a link rejection rolls the identify back); `identify-patient` CLI. 5 DB-gated + 2 pure tests. **Structural finishers 1–3 all done.**

**Prior session (2026-07-11, earlier) — §5.4 finishers PR#1: node-local John-Doe ordinal + `--observed-year` (condensed; no ADR/spec change).** Finisher 1 — read-only VIEW `db/030` deriving "this node's John Doe #N" (node-local by construction, never on-wire). Finisher 2 — pure `resolve_observed_year` bounds `--observed-year` to `1900..=current` (principle 4 honest reject).

**Prior session (2026-07-10, later) — the `patient_name_current` ORDER BY drift guard (condensed; [#159](https://github.com/cairn-ehr/cairn-ehr/issues/159) CLOSED; no ADR/spec change).** The #69 follow-up: db/012 and db/025's winner `ORDER BY` clauses could drift apart silently. Fix: a no-DB source-level guard (`crates/cairn-node/tests/name_winner_order_drift.rs`) asserting the two `COLLATE "C"` clauses stay byte-identical on every `cargo test`/CI pass.

**Prior session (2026-07-10) — codebase-wide collation-independent projection winner tiebreaks (condensed; [#69](https://github.com/cairn-ehr/cairn-ehr/issues/69) CLOSED; **[ADR-0045](spec/decisions/0045-collation-independent-projection-tiebreaks.md)**; spec v0.45→0.46; full detail in git + ROADMAP Phase 2).** Every projection winner tiebreak on a TEXT key was comparing under the node-local default collation, so two nodes could pick different display winners for an identical tie (a silent set-union convergence violation). Fix: every such tiebreak now compares under **`COLLATE "C"`** (byte order) — one shared predicate fix (`cairn_hlc_overlay_wins`, db/002) covers the five standing-state overlays; inline `COLLATE "C"` on the demographic projections/display VIEWs (identifier, demographic, name, address). ADR-0045 binds the invariant on all future projection slices. Projection-read-side only; full workspace green.

**Prior session (2026-07-10, earlier) — the Byzantine HLC-collision advisory signal (condensed; [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) done, PR #158; no ADR/spec change).** #115's tiebreaker resolved a Byzantine/broken-signer HLC-triple collision silently; `db/029_hlc_collision_log.sql` now also **surfaces** it — a structurally non-gating recorder (can never raise) logs each collision before the unchanged upsert. Advisory/observability only; the §5.13-sweep consumer is a documented future seam.

**Prior session (2026-07-09, evening) — deterministic HLC-overlay tiebreaker (condensed; [#115](https://github.com/cairn-ehr/cairn-ehr/issues/115) part 1 done; no ADR/spec change).** Shared pure `cairn_hlc_overlay_wins()` (`db/002`) appends the event `content_address` as the deterministic final tiebreaker for the five standing-state overlays. Remaining #115 part 2 (twin-ladder registry) is an independent refactor; the #69/#157 follow-ons are done (see above).

**Prior session (2026-07-09, later) — the actor `actor_id` collision floor: enroll fails closed (condensed; [#152](https://github.com/cairn-ehr/cairn-ehr/issues/152) CLOSED; ADR-0044; spec v0.44→0.45; full detail in git + ROADMAP Phase 2).** `enroll_actor` derived `actor_id` from the pinned set only, so two different humans with an identical pinned set collided into one `actor_id` and `actor_current` silently dropped the earlier key — a silent identity-merge on the trust anchor. Fix: `cairn_actor_id_key_conflict` whole-history predicate refuses a distinct-key collision (immortal even after revoke); idempotent same-key re-enroll still passes. Human determinant is guidance only (ADR-0011 keeps pinned-set contents as policy). 5 DB-gated tests + SQL mirror; full workspace green. Post-review: a txn-scoped advisory lock closed the check-then-insert TOCTOU race.

**Prior session (2026-07-09) — the suppression owner-gate: self-only, disagreement is additive (condensed; ADR-0043; spec v0.43→0.44; closes the last open sub-item of [#99](https://github.com/cairn-ehr/cairn-ehr/issues/99); full detail in git + ROADMAP Phase 2).** The FIRST in-DB floor authorization change since the demographics build began. A suppressing overlay that forecloses on a human author's event is now refused unless the suppressor is that human — disagreement is expressed additively, never by touching another author's content; agent-authored/un-owned advisories stay dismissable by any enrolled human. One shared `cairn_suppression_author_ok` helper enforced identically at both write doors (`db/005` + `db/020`). Scope carve-outs: §5.9 sensitivity-sealing and `repudiate` untouched. 9 DB-gated tests; full workspace green. Filed [#152](https://github.com/cairn-ehr/cairn-ehr/issues/152) (now fixed, ADR-0044 above) and **[#154](https://github.com/cairn-ehr/cairn-ehr/issues/154) (OPEN)** — the apply-door gate inherits a node-local-registry limitation for plain-signed human notes; closes with registry federation.

**Merged 2026-07-08 (condensed — full detail in git + the PRs + ROADMAP Phase 1).** §5.4 marks/belongings/EMS-context text identity evidence (PR #142, three text `kind` values on the existing `identity.evidence.asserted` type, no floor/SCHEMA/ADR/spec change) + a CI/tooling catch-up day (PRs #143/#147/#149/#150/#151: fmt gate, cargo-deny, `matcher.yml`, toolchain pin, PG16→18 CI, CodeQL crypto FP fix → house rule 6, matcher test-leak/retraction fixes). Closed [#144]/[#145]/[#146]/[#117]/[#135]/[#84 pt1].

**Earlier sessions (2026-06-25 → 07-08), condensed** — demographics slices 1–5 + gaps A/B/C (§4.2–4.6, ADR-0032→0038); the §5.2 matcher pieces A/B1 + the B2→B3 pipeline; the globalised author twin (ADR-0039); the identity C1/C2 apply doors + the quarantine/legibility trilogy (ADR-0040); §5.4 John-Doe slice A + photo evidence (ADR-0042); ADR-0026 node durability B/C/D + Spike 0003 (Postgres-on-Android). **Full detail: ROADMAP + the ADR log + git.**

**Status of this file:** Disposable working scaffolding, **not** a source of truth. Regenerate at the end
of each session. If it ever disagrees with the canonical docs, **the canonical docs win.** The *why* lives
in the immutable ADR log; the *what* lives in the spec; this file only carries what lives *between* them —
current build state, open threads, and time-sensitive items.

---

## Read these first (the durable state)

- **`docs/spec/index.md`** — canonical architecture spec (mission prose + document map + spec version).
  One file per aspect; cross-refs like *§5.7* stay valid inside the aspect file.
- **`docs/spec/decisions/`** — the **ADR log** (the *why*). Numbered, dated, **immutable** (a reversal is a
  new superseding ADR). **Read the relevant ADR before reopening a settled question.** Index below.
- **`docs/ROADMAP.md`** — the foundation build order (wire core → in-DB floor → sync → identity →
  security → federation → blobs → native API), *below* the policy/GUI line. Disposable scaffolding like
  this file; the spec/ADRs win on any disagreement.
- **`docs/spikes/`** — build-prep records (*what we tried, on what, what we learned*). Not spec, not ADR.
- **`docs/principles/`** — mission/governance; **`GOVERNANCE.md`** + `STEWARDSHIP-OF-THE-NAME.md`.
- Root **`README.md`** — mission + founding principles (same prose as `index.md`).
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`), `/extensions` (`cairn_pgx`), `/db`.
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

### First federating node — built 2026-06-21, [PR #28](https://github.com/cairn-ehr/cairn-ehr/pull/28)
First *implementation* of [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
(federation admission), scoped to **direct-pairwise trust, no clinical surface** — only the federation
machinery flows, exercising the one safety-critical seam (*verified credential → admitted peer*) E2E. **No
spec/ADR change.** Built: `cairn-node` (Ed25519 keystore, `init`/`identity`/pairing/`peers`/`unpeer`, built-in
mTLS pinned to the trust set, set-union `node_event` sync, honest `status`); `db/007` append-only `node_event`
+ `submit_node_event` door + `apply_remote_node_event` deny-all admission gate (reuses `cairn_verify` pgrx — no
new crypto). Genesis-stable `node_id` = content-address of the genesis enrollment event. Two-node E2E green on
local PG16 + `cairn_pgx`.

**Honest gaps / follow-ons declared in the node — ALL CLOSED** (full detail in git + ROADMAP Phase 5/6):
status-before-init crash; runtime-login-role / floor-ENFORCED proof; key-at-rest seal + dual-wrap recovery escrow
(ADR-0026 slice A); incremental sync watermark + genesis HLC (#38/#42); all four ADR-0026 durability slices A–D
(cold-peer backup+health, restore + `supersede`, sealed local-state export); atomic key-file write (#45) + passphrase
`zeroize`-on-drop (#46). Only optional escrow *rungs* (Shamir M-of-N / QR / TPM) remain — upward options, not blockers.
The `localstate` DB read/apply **seams** are where the future clinical tier plugs DEKs/drafts/config.
- Test rig: DB-gated tests need local PG + `cairn_pgx` (`cargo pgrx install` against PG16); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so plain `cargo test --workspace` is reliable.

### Spike 0002 (advisory-actor write contract) — ran 2026-06-21, C1–C5 PASS, [PR #27](https://github.com/cairn-ehr/cairn-ehr/pull/27) → ADR-0029 + ADR-0030
An external advisory agent authored an additive, un-attested, recallable advisory through the validated in-DB
door, **and the floor rejected all five hostile-agent attacks** with legible reasons. PR #27 review (the user)
caught two real floor holes the spike's own review missed — forged authorship (unbound `signer_key_id`) and a
`PUBLIC`-executable `SECURITY DEFINER` door — both fixed before merge (recorded in ADR-0030).

**Honest gap (closed 2026-06-22):** the attestation **success** path (a *valid*, correctly-bound
token accepted) was never exercised E2E — now closed by `cairn-sync attest-stdin` (the token minter),
`crates/cairn-node/tests/attestation.rs` (accept for responsibility-bearing + suppressing events; reject for
wrong-address, tampered, and non-human-attester), and `spike_0002.py` selftest (external-actor accept +
wrong-address/tamper). No `submit_event` logic changed — the accept branch already existed; this is the
coverage that was missing. ~~**Smaller deferred items remain open** (commented in code):
`events_by_actor_epoch` resolves against `actor_current` not historical `actor_event` rows;
`actor_current` wall-clock ordering needs a monotonic tiebreaker before production; no FK on
`recall_overlay.target_event_id`; plaintext twin is skeletal.~~ **All four closed:** the three recall-surface
items 2026-07-02 (issue #99 session, see top); the skeletal twin by ADR-0039 (2026-06-28).

### Dual-identifier discipline — ADR-0031, merged 2026-06-22 ([PR #34](https://github.com/cairn-ehr/cairn-ehr/pull/34); `local_ref` honesty fix merged 2026-06-24 [PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43))
New **[ADR-0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)** (canonical
identifiers + node-local surrogate keys): canonical plane (UUIDv7 + multihash) is unchanged and is the *only*
identifier on the wire/in signed bodies; the **projection plane** may intern canonical IDs to dense node-local
`bigint` surrogates as physical join keys. Leakage of a surrogate into a signed body = silent cross-node
corruption, so it is made *hard* (distinct domain type, mapping confined to floor functions, API egress always
the global ID). Landed with `db/008_surrogate_projection.sql` + the Bet B5 leakage guard. Final magnitude is
**measured on Bet B** (Pi), exactly as ADR-0001's compute bet — a "no measurable win" result narrows scope, not
fails the discipline.

**Honest gap (fixed 2026-06-24, [issue #35](https://github.com/cairn-ehr/cairn-ehr/issues/35)):** the prose
called the `local_ref` domain a "real two-way type barrier," but a PG domain over `bigint` is *not* — a
surrogate flows into any plain `bigint` with no cast/error (empirically confirmed). Corrected the wording in
`db/008`, spike 0001 §6.2, PI-RUNBOOK §6.1, and the walking-skeleton README to name the *actual* load-bearing
guarantee (signed plane typed `uuid` + `bigint ≠ uuid` + the G2 assertion) and to frame the domain honestly as
an intent-signal + one-directional guard. Rewrote **G4** in `db/tests/008_surrogate_test.sql`: it now asserts
the functions exist first (no more vacuous pass via `undefined_function`, now dropped), proves the genuine
guard (G4a `uuid`↛`local_ref`; G4b `bigint`↛`uuid` signed plane), and **characterizes the honest limit**
(G4c: `bigint` flows into `local_ref` silently). The spec body (§3.18) and immutable ADR-0031 were already
accurate (one-directional framing), so neither was touched. All G1–G6 green on PG16. **Merged 2026-06-24 ([PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43)).**

---

### Spike 0003 (Postgres on Android) — ran 2026-06-25, G0–G3 PASS, merged ([PR #47](https://github.com/cairn-ehr/cairn-ehr/pull/47) + [PR #48](https://github.com/cairn-ehr/cairn-ehr/pull/48))
Validated the **fractal-topology** invariant at the phone tier (RedMagic 11 Pro). Native PG 18.2 execs, `initdb`s,
serves SQL over TCP, and a cross-built pgrx extension loads + runs (incl. SPI) — no Termux userland, no root, no VM.
The one real blocker was `libandroid-shmem` (compile-baked Termux prefix + dead `/dev/ashmem`), fixed by a
self-contained, pinned-upstream patch. Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/) + a
Medium-style write-up. **Remaining non-load-bearing gaps:** from-source PG build and APK/`jniLibs` packaging
(not blocking — the bet is proven). No spec/ADR change.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **`clinical.medication` — next slice** (the live clinical build front). Slices 1 (assert/cease) + 2 (dose
  change/correction overlay + bitemporal dose timeline) + 3 (cross-thread reconciliation — ADR-0047, `db/033`;
  PR #178) + 4 (attestation — ADR-0049, `db/034`; PR #182) + **5 (dose effective/reason per-field correction —
  ADR-0050, `db/035`; corrected effective drives winner selection; this session)** are DONE. **Next candidates:**
  a **prefer-INN display term** for reconciled groups; **fuzzy/automatic reconciliation** + the Tier-A drug
  dictionary (brand↔generic/DDI) — human-driven resolution exists, automated *detection* is the gap; structured
  sig/frequency (lands with prescriptions); correcting a dose event's *effective date* on the statement-level
  `started` (slice 5 covers the dose-timeline effective; the assert's `started` is a separate concern).
  **Cross-cutting debt:** [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (**cross-thread correction
  *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision**);
  [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory onto the medication/dose/
  reconciliation projections; [#176](https://github.com/cairn-ehr/cairn-ehr/issues/176) (oversize-guard
  remote-apply test); ~~#177 (cross-patient reconciliation)~~ — **RESOLVED 2026-07-16** with the #192
  patient-consistency slice (local refuse when both patients known + the `medication_group_cross_patient`
  read-time surface). Spine to reuse: `db/031`–`db/035` + `cairn-event::medication`.
- **Demographics build — next slices** (reuse the spine in `db/010`/`db/011`/`db/013`/`db/014` +
  `cairn-event::demographics`). Slices 1–5 are done (§4.4 identifiers, §4.2 DOB + sex-at-birth, §4.2 names,
  §4.2 administrative-sex + gender-identity, §4.3 address). **Karyotype** is resolved as a distinct field ([ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md)) — no code yet.
  **§5.2 matcher:** piece A (in-DB hard-veto floor, `db/016`), B1 (advisory **Python** scoring core), B2 (veto-gated
  **pairwise** pipeline + `db/017` proposal worklist), B2b (blocking / candidate-pair generation + `sweep()` driver,
  `cairn_matcher/pipeline/{db,sweep}`), **the B3 eval harness** (`cairn_matcher/eval/` — scorer metrics +
  DB-gated blocking-recall measurement + culture-plural `gold_v1.json` + `python -m cairn_matcher.eval` CLI), the
  **B3 compound blocking key** (`name+year` additive pass in `pipeline/db.py`), and the **B3 synthetic volume
  generator** (`eval/generator.py` pure + `eval/generate.py` CLI — seed+corrupted-clone entity clusters, recoverable
  by construction), and the **A/B pass-toggle** (`enabled_passes` on `generate_candidate_pairs`; unknown pass name
  raises), the **B3 eval mirror** (slice 23: generator range-DOB emission + `DatasetRecord.administrative_sex` +
  range-aware `shares_blocking_key`/`_birth_window`), **B3 weight-learning (slice 24): the
  supervised Fellegi–Sunter learner** (`eval/learner.py` `estimate_weights`/`derive_thresholds`/`learn_model` +
  `eval/crossval.py` entity-cluster held-out lift + `eval/model_io.py` + the `python -m cairn_matcher.eval.learn`
  CLI), **and B3 further compound blocking keys (slice 25, this session): `dob+first-initial`/`name+sex`**
  (`pipeline/db.py`/`pipeline/blocking.py` — a first-initial relaxation of the name requirement + the
  oversized-name-block per-sex rescue) are now BUILT. **Next (B3 measurement-driven):** a **large hand-crafted gold
  set** to re-run the learner for
  authoritative magnitudes (slice 24's learner is a PoC on small/synthetic data) + locale comparator packs /
  hub-tier aggressive duplicate-sweep + proposal retraction / **richer §7.5 matcher-actor determinants**
  (served-model digest; C2b registered the matcher as a per-epoch `agent` actor keyed on `matcher_version`). **Identity: pieces C1** (§5.1/§5.7 linkage core — `db/018`),
  **C2** (`match_proposal`→apply seam — `db/019`, `apply_proposal.rs`; human-accepted → human-attested link), **and
  C2b** (auto-apply of the `auto_candidate` band — `matcher_actor.rs` + `auto_apply.rs`; matcher-authored, un-attested,
  recallable link, apply-time veto re-check), **C3** (`dispute` + the chart trust-state projection — `db/023`;
  the §5.7 projection-side contract, driven by the patient-initiated dispute front door), **and C4** (`identify` +
  the *unconfirmed* trust state — `db/024`; the §5.4 John-Doe identity-pending front door + the `identify` resolver,
  composing the third trust state into a severity-max `chart_trust`) **are now BUILT — the §5.7
  confirmed/unconfirmed/under-review contract is COMPLETE**, **and C5** (`repudiate` + the known-alias pool — `db/025`;
  the first *suppressing* identity event, a value-grained `name_repudiation` overlay striking a known-false name from
  `patient_name_current` and surfacing it via `patient_alias_pool`, `mode='suppressing'` forcing the human-attestation
  floor) **is now BUILT**. **Next identity slice: C5+** — `reattribute` (§5.5 event-granular strike-through of *clinical
  documentation* + tiered adjudication — **waits on a clinical-note surface** that does not yet exist; premature to
  build against demographics) + the rest of the §5.4 John-Doe registration subsystem. **§5.4 slice A (callsign minting +
  matcher placeholder exclusion)** — `cairn-event::john_doe::callsign` + `cairn-node::john_doe::register_john_doe` + a
  `register-john-doe` CLI + the advisory matcher exclusion (`use_key <> ALL(%s)`) — **and slice B (clinician-observed
  evidence: estimated-age range + observed sex)** — `cairn-event::evidence` (`birth_year_range_from_age` +
  `estimated_dob_body` value `"<min>/<max>"`/precision `"year-range"` + `observed_sex_body` on `administrative-sex`) +
  `cairn-node::evidence::assert_observed_evidence` + an `assert-observed-evidence` CLI + range-aware **positive-only**
  `compare_dob` in the matcher — **and slice C (the birth-year-range blocking pass: anchored `dob-range` +
  `dob-range+sex` passes in `pipeline/db.py` + pure `pipeline/blocking.py`)** — **and slice D (administrative-sex
  scoring via the composite `sex` field + the unconfirmed-chart REVIEW forcing rule + `chart_trust` plumbing, this
  session — closes [#130](https://github.com/cairn-ehr/cairn-ehr/issues/130), see top)** — **are now
  BUILT; NO new event type / migration / floor / SCHEMA / ADR / spec change.**
  **Remaining §5.4:** ~~photo evidence~~ (DONE — `identity.evidence.asserted` + the ADR-0042 attachment tier);
  ~~marks/belongings/EMS-context evidence~~ (DONE this session — three text `kind` values on the same event type,
  `cairn-node::identity_evidence` + `assert-identity-evidence` CLI, zero wire change), the
  "prior history now available" push-alert on link (§5.12, no notification tier yet), the search-before-create
  registration-class funnel (§5.3/§5.8, UI/API tier); ~~a readable callsign suffix~~ (DONE this session as a
  node-local **display ordinal** — `db/030_john_doe_local_ordinal` VIEW, "this node's John Doe #N"; the callsign
  identity string stays UUID-suffixed/partition-safe, a per-day counter deliberately NOT used) and
  ~~a `--observed-year` CLI override~~ (DONE this session — pure `resolve_observed_year`, bounded
  `1900..=current`); ~~and **`identify`→optional-link** wired into one resolution flow~~ (finisher 3 — DONE
  this session: `cairn-node::identify` + `identify-patient` CLI; device-additive identify + optional
  human-attested link, atomic; the advisory `actor_current` human-ness pre-check); ~~an `enroll-human`
  ceremony CLI~~ (DONE — `cairn-node::enroll` + `enroll-human` CLI; enrols a `kind='human'` actor with an
  ADR-0044 determinant; identify.rs tests now use the real ceremony not raw SQL; follow-up #166). Still open: the
  "prior history now available" push-alert on link (§5.12); the search-before-create funnel (§5.3/§5.8).
  Reattribute composes one more *under-review*
  source into the `chart_trust` VIEW when it lands (note: a pending+disputed Doe already reads `'under-review'` —
  severity-max — so the slice-D forcing rule deliberately stands down while a dispute is open). Deferred (repudiate): a **reversal / de-repudiation** event (overlay HLC-versioned, composes without rewrite);
  a **chart-history VIEW** rendering struck names; fuzzy alias recognition + a dedicated `alias` blocking pass.
  Deferred (range blocking): ~~generator range-DOB emission + range-aware eval mirror~~ (done — slice 23); fuzzy
  near-window softening; hub-tier range sweep. Deferred
  (earlier): variable cluster size / an unrecoverable
  fraction / hard negatives in the volume generator; a **veto-aware / end-to-end scorer mode**; deceased-status veto
  (stub in db/016); a `compare_address` comparator; a **CLI** sweep entry; ~~the matcher conftest test-leak
  ([#84](https://github.com/cairn-ehr/cairn-ehr/issues/84))~~ — **pt1 committed-row leak fixed** (PR #150, sixth
  session; pt2 `KeyError` was fixed in PR #131); ~~stale forced-REVIEW proposal retraction
  ([#135](https://github.com/cairn-ehr/cairn-ehr/issues/135))~~ — **fixed** (PR #151, sixth session);
  B2 follow-up Minors (Thresholds `review<auto` guard,
  `band` CHECK, `updated_at` trigger, conftest env read-at-import) → [issue #79](https://github.com/cairn-ehr/cairn-ehr/issues/79).
  Rust DB-gated tests + the matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb
  dbname=cairn_test"` (PG18+cairn_pgx); the multi-node convergence suites additionally need `CAIRN_TEST_PG2`/`PG3`
  pointing at `cairn_test2`/`cairn_test3` on the same cluster (created + extensioned 2026-07-16; without them those
  tests self-skip locally — CI sets all three since #199). Matcher integration: `cd matcher && CAIRN_TEST_PG=… uv
  run --extra pipeline pytest`. The pure matcher suite is dependency-free: `cd matcher && uv run pytest` (uv, never
  venv/pip).
- **Clinical case-mining** — historically the highest-signal generative mode; the event-overlay + key-custody +
  actor primitives have absorbed every case so far without new architecture. Bring a real ED/hospital failure mode.
  The record now lives in [`docs/case-studies/`](case-studies/README.md). First entry
  ([Case 0001](case-studies/0001-improving-practice-software-column.md), 2026-07-11): 16 Australian GP-software
  failure modes from Dr Oliver Frank's magazine column — all absorbed, **0 new architecture**, but three action
  items surfaced: **① re-affirmation-without-change currency** (two timestamps on one fact —
  `asserted-since` vs `confirmed-current-as-of`) — **checked against code → [issue #163](https://github.com/cairn-ehr/cairn-ehr/issues/163)**:
  the envelope already records a re-affirmation (append-only, distinct `content_address`), so no can't-retrofit
  gap; the gap is that every `patient_*` projection (`db/010`–`db/014`) collapses both timestamps into one
  overwrite-on-reaffirm winner-HLC triple, and `first_seen`/`updated_at` are local non-convergent
  `clock_timestamp()` stamps; **② open-loop/obligation** (order/recall/referral with no closing ack) may warrant a named
  projection, and must be surfaced by salience not a modal (paper-parity); **③ impossible-vs-uncertain** constraint
  rule for the in-DB floor (reject only the physically/type-impossible, advisorily flag the merely improbable).
- **Dedupe transitive RustCrypto dep versions** in `Cargo.lock` ([issue #11](https://github.com/cairn-ehr/cairn-ehr/issues/11)) — supply-chain
  hygiene. **Re-verified 2026-06-25: still blocked on upstream** — the `postgres` stack pulls `digest 0.11`/`sha2 0.11`/`chacha20 0.10`
  while `chacha20poly1305 0.10.1` still depends on `chacha20 0.9` and `ed25519-dalek` on `digest 0.10`. Not fixable from our `Cargo.toml`; revisit when the ecosystem converges.
- **Harden the first federating node** — status-before-init crash, runtime-login-role/floor-ENFORCED proof,
  incremental sync watermark + genesis HLC ([#38](https://github.com/cairn-ehr/cairn-ehr/issues/38), PR #42),
  and **all four ADR-0026 durability slices** — A (at-rest seal + recovery escrow, PR #44), B (cold-peer export+health,
  PR #51), C (restore + `supersede`, PR #52), **D (sealed local-state export, this session)** — are all **closed**
  (see node gaps above). **ADR-0026 is fully implemented at the node tier.** No remaining required node-hardening thread;
  ~~[#54](https://github.com/cairn-ehr/cairn-ehr/issues/54) (uniform key zeroization)~~ and ~~[#53](https://github.com/cairn-ehr/cairn-ehr/issues/53)
  (federated-restore self-identification)~~ both **closed 2026-06-26**; only optional escrow rungs (Shamir/QR/TPM) remain.
  The `localstate` DB read/apply **seams** are where the future clinical tier plugs DEKs/drafts/config.
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already advanced
  across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#9-bet-b--results-raspberry-pi-5--8-gb-2026-06-25--pass-with-two-honest-caveats)):
  **RAN 2026-06-25 on a Pi 5 / 8 GB → PASS** (all §6 gates green, large headroom; B4 **confirms** ADR-0015's
  BLAKE3 blob-digest default — BLAKE3 ~4× SHA-256 on Cortex-A76). Artifacts in
  [`poc/walking-skeleton/results/`](../poc/walking-skeleton/results/). **Two caveats** (precision, not verdict):
  storage ran on a **USB-2-limited dock** (power-offload workaround after a Pi 5 brown-out saga — see the §9.2
  *deployment-BOM finding*: PSU + storage-attachment path are part of the validated BOM), and on **PG 16**
  because **`cairn_pgx` is pgrx-0.12.9 / `pg16`-pinned and won't build on PG 18** (§9.3). Bonus: `cairn_pgx`
  builds+loads on Pi arm64 (in-DB Rust surface confirmed on ARM). **Follow-ups:** ~~(a) port `cairn_pgx` to a
  PG-18-capable pgrx~~ **done 2026-06-25 (PR #56: pgrx 0.12.9 → 0.18.1)**; ~~(b) clean re-run on PG 18 + fast
  storage + official PSU~~ **DONE 2026-07-07 → PASS, both caveats resolved
  ([§9.5](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#95-clean-re-run-pg-18-nvme-2026-07-07-pass-both-caveats-resolved))**:
  same 8 GB board, now on **PostgreSQL 18.4 + a PCIe NVMe HAT** (better than the USB-3 SSD the follow-up asked
  for). Headline: B1 p95 **3.99 ms @ 2,004,000 events** (13× under budget), *faster than* the old USB-2 number
  **at 10× the log size**, flat ×2.50 over a ×37 growth jump; B2 p95 4.5 ms/374-note chart (222×); B5 FK-index
  shrink ×1.40 @ 2 M rows (G1–G6 pass); crypto reproduced within noise; measured **~1,515 B/event** on disk.
  Artifacts `poc/walking-skeleton/results/betb-pi5-nvme-pg18-*`. **Remaining:** (c) fold the (now un-caveated)
  B4 number into the ADR-0015 follow-up to drop "provisional" from the blob-digest line.
- **easyGP session** — port the [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  deferred items with live easyGP schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon (validates ADR-0001 from production). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection per
  slice (the production object-store tier). The §8.2 availability + windowing/resume work already shipped.

---

## Parked (don't re-litigate without new reason)

- **Stewarding legal entity & jurisdiction** (German Stiftung/Verein, US 501(c)(3), or an umbrella) — deferred
  until momentum/funding geography is clearer.
- **Formal trademark / wordmark registration** — principle recorded (stewardship doc); legal instrument deferred.

---

## Working context (most also in CLAUDE.md)

- The user is a senior **EM physician**, GNUmed founder (early FOSS Postgres EHR), codes mostly in Python, brings
  real ED/hospital failure modes from multiple health systems. **The mission (anti-capture / anti-vendor-lock-in)
  is the tie-breaker.** Criticism is strongly encouraged — surface flaws/risks immediately.
- **Twelve founding principles** run through everything ([index.md](spec/index.md)); the first four are the lens
  for every design choice: (1) append-only + causal ordering; (2) identity is a claim — never merge/erase, always
  link/overlay; (3) paper-parity (no confirmation dialogs); (4) acknowledged uncertainty. See CLAUDE.md for the
  full set (5–12) and the §9 defect-blast-radius language-selection rule.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0 inbound=outbound,
  DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr` org; `cairn-ehr.org`+`.com`;
  PyPI/crates.io/npm `@cairn-ehr` placeholders).

---

## Decision trail — the ADR index (the *why* is in each linked ADR; do not restate it here)

**Every original §11 open architecture question is closed.** Compact index of the settled decisions; read the
ADR before reopening any of these.

| ADR | Decision (one line) | Spec home / principle |
|---|---|---|
| [0000](spec/decisions/0000-pre-adr-changelog-v0.1-v0.6.md) | Pre-ADR changelog v0.1→v0.6 | — |
| [0001](spec/decisions/0001-fat-postgres-thin-daemon.md) | Fat Postgres, thin Rust daemon | §2/§3.5/§6.1/§9.4 |
| [0002](spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md) | In-DB Rust (pgrx) escape hatch | §9.4 |
| [0003](spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md) | Bitemporal time (`t_recorded` vs `t_effective`) | §3.6/§3.7 · **principle 4** |
| [0004](spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md) | Sync scope = prefetch hint, not authority | §6.4 |
| [0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md) | Erasure = key-custody redistribution / crypto-shred | §3.8/§7.1 · **principle 9** |
| [0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md) | Replication ≠ confidentiality; the safety projection | §5.9 |
| [0007](spec/decisions/0007-authorship-and-accountability.md) | Authorship compositional, accountability separable | §3.9/§7.2 · **principle 10** |
| [0008](spec/decisions/0008-point-of-care-identity-possession-and-salvage.md) | Point-of-care identity, possession, `sign-as` salvage | §5.11/§3.10 |
| [0009](spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md) | Notification economy, salience routing, ack floor | §5.12/§3.11 |
| [0010](spec/decisions/0010-additive-vs-suppressing-classification.md) | Additive-vs-suppressing (derived, not declared) | §3.9 |
| [0011](spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) | Actor registry, version-pinning, key custody | §7.5/§3.12 |
| [0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md) | Schema evolution, two planes, legibility twin | §3.13/§6.5/§7.6 · **principle 11** |
| [0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md) | Attachments = content-addressed lazy blob tier | §3.14/§6.6 |
| [0014](spec/decisions/0014-locale-pluggable-matcher-comparators.md) | Locale-pluggable matcher comparators | §5.13/§4.1 |
| [0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md) | COSE_Sign1 + Ed25519 + SHA-256; BLAKE3 blobs (*provisional*) | §3.5/§3.14 |
| [0016](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md) | Record discovery + replicated essential tier | §6.7/§5.2 |
| [0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md) | Federation admission, sovereignty, trust anchors | §7.7 |
| [0018](spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md) | Federation revocation cascade; anchor-as-power | §7.7 |
| [0019](spec/decisions/0019-author-scoped-record-export-the-medico-legal-copy.md) | Author-scoped export (the medico-legal copy) | §7.8 |
| [0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) | Active-write, thin encounters, delete-vs-erase | §3.15 · vision §1.2 |
| [0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) | Four-layer model; node API; UI pluralism | §9.5 · **principle 12** |
| [0022](spec/decisions/0022-validated-submit-surface-the-write-path.md) | Validated `submit_event` surface (the write path) | §9.6 |
| [0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md) | Native API contract: capability + conformance | §9.7 |
| [0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md) | Hard policy = signed policy-assertion stream | §7.9 |
| [0025](spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) | ICD-11 canonical interlingua + local-terminology overlay | (terminology) |
| [0026](spec/decisions/0026-node-durability-and-disaster-recovery.md) | Node durability & disaster recovery (cold-peer backup) | §7.10 |
| [0027](spec/decisions/0027-trusted-time-anchoring.md) | Trusted-time anchoring (graded-interval `t_recorded`) | §3.17/§7.11/§6.8 |
| [0028](spec/decisions/0028-finalized-closed-contributor-role-enum.md) | Finalized closed contributor-role enum | §3.9 |
| [0029](spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md) | Skill-epoch + served-model digest as pinned actor determinants | §7.5 |
| [0030](spec/decisions/0030-advisory-actor-integration-contract.md) | Advisory-actor integration contract | §9.8 |
| [0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md) | Canonical IDs + node-local `bigint` surrogate keys (dual-identifier discipline) | §3.1/§3.2 |
| [0032](spec/decisions/0032-culture-neutral-address-representation.md) | Culture-neutral address: three-facet value (display twin + geo + culture-tagged parts) | §4.3 (refines 0014) |
| [0033](spec/decisions/0033-patient-identifier-representation.md) | Patient-identifier representation: namespace/profile split + matching-survivable normalized form | §4.4 (refines 0014) |
| [0034](spec/decisions/0034-demographic-legibility-twin.md) | The demographic legibility twin: every demographic assertion legible without its profile | §4.5 (refines 0012) |
| [0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md) | The entity/relationship model + provider-number person×org (subject-kind partitioning) | §4.6 (refines 0033) |
| [0036](spec/decisions/0036-demographic-name-display-recency-first.md) | Demographic name display: recency-first within the legal tier (diverges from DOB's provenance-lock by design) | §4.2 (refines 0014) |
| [0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md) | Sex/gender/karyotype field semantics: per-field winner policy; karyotype is a distinct field, never displaces assigned sex-at-birth | §4.2 (refines 0011/0014) |
| [0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md) | Demographic address display: per-use recency-first (volatile field; follows ADR-0036) | §4.3 (refines 0032, follows 0036) |
| [0039](spec/decisions/0039-globalise-authored-legibility-twin.md) | Globalise the author-materialised legibility twin to every event type; honest-degradation fallback for non-demographic types | §3.13/§4.5 (refines 0012/0034) |
| [0040](spec/decisions/0040-signing-context-domain-separation.md) | Signing-context domain separation (content-type + `external_aad`); one signature per event, co-signing by overlay | §3.5 (refines 0015/0007/0030) |
| [0041](spec/decisions/0041-progress-note-narrative-format.md) | Progress-note format: one signed event, markdown narrative + manifest-keyed media anchors | §3.19 (refines 0012/0013/0020/0039) |
| [0042](spec/decisions/0042-concrete-attachment-reference-shape.md) | Concrete attachment-reference shape (Attachment/Rendition/SealRef; frozen field order) | §3.14 (refines 0013, reconciles 0041) |
| [0043](spec/decisions/0043-suppression-self-only-disagreement-is-additive.md) | Suppression is self-only (human-authored content); disagreement is additive; agent advisories dismissable | §9.6/§3.9 (refines 0010/0022) |
| [0044](spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md) | Enroll fails closed on `actor_id` collision with a distinct key; humans carry a person-distinguishing determinant | §7.5 (refines 0011/0029) |
| [0045](spec/decisions/0045-collation-independent-projection-tiebreaks.md) | Collation-independent projection winner tiebreaks (`COLLATE "C"`) | §5.7/§4 (refines principle 1) |
| [0046](spec/decisions/0046-enroll-fail-closed-on-key-actor-dual-mapping.md) | Enroll fails closed on key→actor dual mapping (B-direction whole-history guard) | §7.5 (refines 0044/0011) |
| [0047](spec/decisions/0047-medication-reconciliation-resolution.md) | Medication reconciliation is a link, not a cessation; symmetric min-UUID collapse; latest-effective group status | §3.15/§3.16 (principle 2; reuses identity linkage) |
| [0048](spec/decisions/0048-twin-check-registry-dispatch.md) | The per-type twin/floor-check registry: one stable dispatcher, register-by-row, unified check-fn signature | §9.6 (refines 0022/0039) |
| [0049](spec/decisions/0049-commitment-based-sign-off-currency.md) | Commitment-based sign-off currency: separable per-thread attestation overlay; staleness by set-commitment compare, not a position pin; supersede, never retract | §3.15/§3.16 (refines 0007, principle 10) |
| [0050](spec/decisions/0050-dose-correction-per-field-patch.md) | Dose correction is a per-field patch: explicit strike sentinel; corrected effective drives current-dose winner selection; correction-note separate from clinical reason | §3.3/§3.6 (refines principle 4) |

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B prepared); 0002 (advisory-actor — ran, C1–C5 ✓
→ ADR-0029/0030); 0003 (Postgres on Android — **ran 2026-06-25, G0–G3 ✓**; PR #47/#48).
