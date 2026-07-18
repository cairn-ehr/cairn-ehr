# HANDOVER — Cairn

## ⇒ NEXT: the 2026-07-15 whole-project review course — P1 + P2 DONE, P3 DONE (#203/#96 + #189/#92); next is #204 [C3] then P4 #188 [D1]

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
enrolled writer; TDD, one branch `feat/floor-hardening-spike0002-p1`, PR #219 incl. its review
round; full detail in the PR + ROADMAP + git): ~~#187~~ · ~~#207~~ · ~~#194~~ · ~~#191~~ ·
~~#192~~ (**resolves #177**) · ~~#190~~ · ~~#193~~ · ~~#195~~. The #187 remote-door policy for
genuinely-broken peers stays the deliberate db/020 clamp-and-admit (documented in `hlc_drift.rs`).
One follow-up filed: [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) — the #190 hard
veto is still evaluated only at link-arrival time (a silent vetoed merge if the clashing
demographics sync in later; needs a re-check hook or background sweep).

**Priority 2 — sync-convergence integrity (the flagship guarantee). ✅ COMPLETE 2026-07-16**
(all five slices landed the same day; full detail in ROADMAP Slices 37–40 + the PRs + git.
B5/#200 — the "refusal + durable re-offer IS the contract" ADR — lives in the Priority 6 design
queue, not here.) ~~#199 (B4)~~ the P2 opener (PR #221 — CI provisions `cairn_test2`/`cairn_test3`
+ exports `CAIRN_TEST_PG2`/`PG3` so no suite self-skips in CI; `medication_remote_apply.rs` +
cairn-sync's `clinical_pull.rs` prove A→B projection equality through the real binary) ·
~~#198 (B3)~~ the cairn-sync SCHEMA subset stands alone (PR #222 — db/027+029 appended + the
standing `schema_subset_tests` drift guard: a wiped DB loaded from the subset ALONE must satisfy
both write doors) · ~~#196 (B1)~~ the clinical seq cursor (PR #223 — `db/036` `event_log.seq` +
per-peer `sync_state.last_seq` + the SEPARATE self-clearing `quarantine_floor_seq` + additive
`EventsAfterSeq`/`seqs[]` wire + a full sweep every 10 cycles) · ~~#197 (B2)~~ `AND NOT acked` on
both clinical quarantine quota subqueries, mirroring the node plane (PR #224) · ~~#202/#201
(B7/B6)~~ the P2 closers (PR #225 — 64 MiB wire-frame caps on BOTH ends, `COLLATE "C"` + `'|'`
field separators on the convergence fingerprints under executed-in-CI drift guards, the byte-tier
`Err` arm logs, and `node.superseded` resolved as **REPLICATE** — the db/007 apply door gained the
supersede arm, trust-bounded like peer/revoke, feeding only the advisory `node_lineage` view;
follow-ups [#227](https://github.com/cairn-ehr/cairn-ehr/issues/227) (HLC-merge helper) +
[#228](https://github.com/cairn-ehr/cairn-ehr/issues/228) (legible malformed-hex refusals) filed).

**Priority 3 — the two closing wire windows (on paper first; cheapest they will ever be).**
- ~~**#203 (C2)** + **#96**~~ — **✅ DONE 2026-07-16** ([ADR-0051](spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md),
  spec v0.52, branch `feat/adr-0051-role-vocabulary-203-96`; full detail in ROADMAP Slice 41): `recorded`
  ratified as the 12th (contributory) member; responsibility = `{held_by, on_behalf_of?}` object
  (proxy wire-expressible; `on_behalf_of` refused at submit until a proxy-grant ADR, admitted at apply);
  future members partition-prefixed (`bearing:x`/`contrib:x`) so old nodes classify them; unknown roles
  read **vouching-unknown**, never un-vouched; `cairn_check_contributors` on BOTH doors — strict submit /
  lenient apply (role membership never rejects at apply — set-union losslessness).
- ~~**#189 (C1)** + **#92**~~ — **✅ DONE 2026-07-17** ([ADR-0052](spec/decisions/0052-born-sealed-clinical-bodies.md),
  spec v0.53, branch `feat/adr-0052-born-sealed-189-92`; full detail in ROADMAP Slice 42 + the PR): born-sealed
  clinical bodies — every clinical JSONB body sealed at write under a per-event DEK held by the node itself
  (**erasability substrate, NOT confidentiality**), so the ADR-0005 erasure ladder stays reachable forever. Both
  doors enforce it (strict submit refuses unsealed `clinical.*` + sealed⇒clinical scope; lenient apply admits
  sealed-without-custody structurally), all 7 medication verbs seal-at-write, custody sidecar on the clinical wire,
  a rung-3 crypto-shred CLI, E2E (sealed sync → shred propagates → restore resurrects nothing) + bench
  (~0.11 ms/event, 37× under Bet-B). Nine follow-ups filed ([#230](https://github.com/cairn-ehr/cairn-ehr/issues/230)–[#238](https://github.com/cairn-ehr/cairn-ehr/issues/238)).
  **Operational caveat:** pre-ADR-0052 plaintext clinical dev/PoC rigs must be **WIPED** (the born-sealed floor
  refuses plaintext clinical at submit; old logs won't cross).
- **#204 (C3)** — **⇐ CONTINUE HERE** (scheduled in Slice 41): the attribution-token / authoring-human slice
  (per-write attribution, `session.user ≠ event.author`, `sign-as`; §3.10/ADR-0008) is the next clinical-plane
  slice before any new clinical stream — `recorded` (ADR-0051) makes the device-only interim honest, #204 ends it.

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

**Session date:** 2026-07-18, latest (the PR #239 post-review fix pass + the GUI/L3 easyGP
editing-area mining session; 2026-07-17 #189+#92 — the P3 closer, ADR-0052 born-sealed clinical bodies;
2026-07-16 had #203+#96 the P3 opener ADR-0051, the full P2 arc #199/#198/#196/#197/#202/#201, the P1
floor-hardening slice + the evening GUI/L3 easyGP-mining thread; review course above; last full
regeneration 2026-07-14) · **Spec/ADRs:** v0.53 (through ADR-0052) · **Phase:** architecture complete
(every original §11 question closed);
**first production clinical surface under construction** on `cairn-node`. Built so far
(full detail in ROADMAP + the ADR log + git):
**demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names ·
administrative-sex/gender-identity · §4.3 address; karyotype resolved as a distinct field,
ADR-0037, no code yet) ·
the **§5.2 advisory Python matcher** (piece A in-DB veto floor · B1 scoring core · B2/B2b
veto-gated pipeline/blocking · the B3 eval harness, compound blocking keys, synthetic volume
generator, supervised Fellegi–Sunter weight-learning · range-DOB/composite-sex evidence scoring) ·
the **§5.7 identity core C1–C5** (linkage · human-accepted apply seam · auto-apply band · dispute ·
identify · repudiate + the known-alias pool — the confirmed/unconfirmed/under-review contract is
COMPLETE; C5+ `reattribute` waits on a clinical-note surface) ·
the **§5.4 John-Doe subsystem** (slices A–D + finishers 1–3 + photo/text evidence + the
`enroll-human` ceremony CLI; still open: the §5.12 push-alert + the search-before-create funnel) ·
the **first clinical-content stream `clinical.medication`, slices 1–5** (assert/cease + the E1
reconciliation flag · bitemporal dose timeline · cross-thread reconciliation links, ADR-0047 ·
the attestation responsibility overlay, ADR-0049 · per-field dose effective/reason correction,
ADR-0050) + the **twin-check registry** (ADR-0048) ·
the **contributor-role vocabulary floor** (ADR-0051 — `recorded` ratified, `{held_by}` responsibility
objects, partition-prefixed future members, strict-submit/lenient-apply) ·
**born-sealed clinical bodies** (ADR-0052 — every clinical JSONB body sealed at write under a per-event
DEK held by the node itself, an erasability substrate not confidentiality; `db/037` custody plane
`event_dek`/`event_clear`/`erasure_shred_log`, both doors enforce sealed⇒clinical scope, all 7
medication verbs seal-at-write, custody sidecar + rung-3 shred CLI; twin registry 18→19) ·
the **L3 reference-UI shell, slice 1** (framework SETTLED — iced FAILS the accessibility bar,
pivot to **Tauri 2**, an L3 choice below the compatibility boundary; PR #174).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node,
Postgres-on-Android).

**Session (2026-07-17) — #189/#92 [C1]: born-sealed clinical bodies, the P3 closer (branch
`feat/adr-0052-born-sealed-189-92`; [ADR-0052](spec/decisions/0052-born-sealed-clinical-bodies.md), spec
§3.5/§3.8/§5.9, v0.53; full detail in ROADMAP Slice 42 + the PR + the ADR).** Posture decision: **every
clinical JSONB body is born sealed** under a per-event DEK wrapped for the node's *own* key — an
**erasability substrate, NOT confidentiality** (the node reads its own data freely; projections/FTS
behave as before; nothing is hidden), so **every ADR-0005 erasure rung stays reachable for every clinical
event forever** (a plaintext default silently forecloses rungs 2–4 — principle 9). Crypto core
(`cairn-event::seal`): per-event DEK XChaCha20-Poly1305, **seal-then-sign** (signature over ciphertext,
survives shred; AAD binds `event_id`), the legibility twin under the same DEK (the sealed row's outer
`plaintext_twin` is a signed stub — no plaintext column to leak from), X25519/HKDF wrap plane derived
from the Ed25519 seed (DB holds only the public half → DEKs unreconstructable from a DB backup; ADR-0026
escrow covers the KEK, now mandatory), signed unwrap-key cert (`CTX_UNWRAP_KEY`). `db/037` custody plane:
`event_dek`/`event_clear`/`erasure_shred_log`, with `event_clear` + `cairn_clear_payload` homed in `db/005`
for `LANGUAGE sql` eager-bind ordering; `erasure.shred.asserted` twin-registered (18→19). Two doors: the
strict submit door **refuses unsealed `clinical.*`** (decrypts in-DB via `cairn_pgx`, runs the full
ADR-0048 twin/floor checks on plaintext, wraps the DEK into `event_dek`), the lenient apply door admits
a sealed event without custody structurally (**can't read → never reject**); the **final review closed the
one gating cross-cutting hole — sealed⇒clinical scope at BOTH doors** (a sealed non-clinical body is
refused). All 7 medication verbs seal-at-write via one `medication::sealed_submit` path (semantics
unchanged, now sealed). Custody sidecar on the clinical wire (unwrap-then-rewrap per peer, **shred-aware
DEK exclusion** — custody never granted to an already-shredded event, arrival-order independent). Shred
CLI (`cairn-node shred`, rung-3 audited crypto-shred + plaintext tombstone; log row never touched). E2E:
sealed sync **with custody** converges → **shred propagates** → **cold-peer restore replays the shred log
before projecting, resurrects nothing**. Bench ~**0.11 ms/event**, ~37× under Bet-B. ADR-0049 §9
false-fresh gate: `reviewed_count` promoted (sealed threads only) to a safe-direction withholding tripwire.
TDD RED-first. Nine follow-ups filed ([#230](https://github.com/cairn-ehr/cairn-ehr/issues/230)–[#238](https://github.com/cairn-ehr/cairn-ehr/issues/238), deferred/hardening).

**Post-review fix pass (2026-07-18, `/review` on PR #239 → `/fixall`).** A subsequent whole-diff code review found the branch's own "READY TO MERGE / none gating / fmt clean" verdict had **missed five issues**, all now fixed on-branch (RED-first where behavioural):
- **[GATING, critical] db/018 sync-wedge** — `patient_link_apply()` cast `(p->>'subject_a')::uuid` in its DECLARE block, which runs *before* the `IF NEW.sealed THEN RETURN NULL` seal guard; a wrongly-sealed `identity.link` with a non-UUID top-level `subject_a` (any enrolled peer can mint one) raised `invalid input syntax for type uuid` at apply, aborting `apply_remote_event` on a verifiable event → **frozen sync watermark, permanent wedge**. Fix: casts moved below the guard.
- **[GATING, high] db/037 incomplete shred** — `cairn_execute_shred` scrubbed only `medication_statement`/`medication_cessation`/`medication_dose_event`, leaving **dose-correction, reconciliation, and attestation plaintext readable after a shred** (4 of 7 verbs) — defeating the ADR-0005 rung-3 / #92(b) guarantee. Fix: scrub all three by `content_address` + recompute `medication_group_member` (derived table) so the erased merge stops grouping threads.
- **[CI-red] rustfmt** — the workspace-**excluded** `cairn_pgx` extension was unformatted (CI's separate `cargo fmt --manifest-path …` gate was red; the "fmt clean" claim only ran `cargo fmt --all`). Fixed.
- **[medium] false erasure** — crypto-shred of a **non-sealed (plaintext) target** reported success while its body stayed in the append-only log (no DEK to destroy). Now refused at both the `cairn-node shred` pre-check and the unbypassable db/005 floor.
- **[low] silent serve-side degradation** — a serve-side per-DEK re-wrap failure (e.g. serve `--key` ≠ registered unwrap key) silently blanked custody; now logged.
- **[#231 reaffirmed]** the unwrap-cert has no trust-set check, so **born-sealed ships _erasability_, not confidentiality**, until cert-kid pinning lands — flagged as the load-bearing gap, not a buried TODO. (No code change; already filed.)

Verified: cairn-node **298/0**, cairn-sync **51/0** (subset + 2-node E2E need cairn_pgx **≥ 0.3.0** on BOTH test DBs), cairn-event **140/0**, +3 new RED-first regression tests; **fmt + clippy clean on both trees** (workspace + `cairn_pgx`). **Operational caveat:** the born-sealed floor refuses plaintext
`clinical.*` at submit, so pre-ADR-0052 plaintext clinical dev/PoC rigs must be **WIPED** — old logs won't
cross (moot pre-production). **Next:** #204 [C3] attribution-token / authoring-human slice, then P4 #188
[D1] schema-version guard.

**Session (2026-07-18, GUI/L3 design thread) — easyGP data-entry mining: the Editing Area grammar
(design-only; no code/ADR/spec change; full detail in
[`scratch/ui-sketches/easygp-editing-area-inventory.md`](../scratch/ui-sketches/easygp-editing-area-inventory.md)).**
The anticipated co-author batch arrived: 18 screen snips (entry *sequences*: allergies, ordering,
prescribing, notes review, draw/webcam, referrals, past-history/care-plan) + the developer-guide "Editing
Area" chapters (source folder git-ignored under `docs/untracked_for_brainstorming/` — real photo /
potentially confidential content; **never commit or publish**; the note records mechanisms only). Headline:
easyGP's **six editing-area invariants ≅ Cairn's event envelope** near line-for-line (invariant 6 —
"key elements imply the totality in a list" — is the legibility twin discovered from the display side):
external validation that the envelope is the right *user-facing* grammar, exposable directly as one entry
grammar for all clinical data. Ten distilled GUI principles queued for shell-spec graduation (one entry
grammar; type-ahead primary; **auto-fill to the fork**; all state ambient never modal — interaction
checking renders *beside* the work; vocabulary never blocks; session folds; documents = previewed
projections; **record-as-book incl. the audit-trail display overlay in the same timeline**; paper's
drawing hand restored; per-user geometry/action-set persistence). Six NEW principle-4 archaeology exhibits
(Nil-Known explicit negative; allergy specificity class/generic/brand; Confirmed Y/N; Uncertain-onset +
Year-or-Age; laterality "None"; Date-of-Reaction vs Date-Entered bitemporal pair). Negative exhibits: the
Accept-or-lose lifecycle (ADR-0020 scratchpad supersedes, keep the visible tri-state), a silently-accepted
11th-century backdate (→ advisory plausibility flag, never block, ADR-0003 posture). **Next:** co-author
questions filed in the note §7 (audit-overlay + draw-editor real usage, Accept-loss anecdotes, one-grammar
counterexamples); results-inbox screenshots still pending (the 2.21 three-zone question rides on them).
**HH clarification (same day):** the easyGP audit trail covered **saved traces only** (never-Accepted data
left no trace anywhere) in **bare wall-clock time** — the abandoned-entry blind spot and the Accept-loss
mode are one defect seen from two sides, and the ADR-0020 durable-scratchpad + quarantine-commit mechanism
retires both (time side: ADR-0027 graded clock confidence vs trusting the box's clock). **Team/scope news:**
the easyGP co-author is interested in returning from retirement to lead **GP-facing GUI design** (ergonomics
of data entry/display is his strength); HH (ED + remote Aboriginal-health practice for ~10 years, out of GP)
will design the **ED & ward GUI/workflow** once core infrastructure is nailed down. The shell design's
role-manifest layer (§7) is the seam that makes this division clean: GP manifest = the co-author's canvas,
ED/ward manifests = HH's, one shell codebase underneath (uniform core, plural edges — ADR-0021 working
as intended).

**Session (2026-07-16, evening, GUI/L3 design thread) — easyGP consult-screen mining → GP-manifest seed
(design-only; no code/ADR/spec change; full detail in
[`scratch/ui-sketches/easygp-consult-screen-inventory.md`](../scratch/ui-sketches/easygp-consult-screen-inventory.md)).**
Mined a 2019 easyGP consult-screen screenshot (co-author-supplied; image deliberately not committed — real
name/photo) panel-by-panel against paper-parity questions, mapped onto the 2026-07-12 shell design. Outputs: a
**GP-manifest seed** (4 pinned safety cards incl. compact ranked recalls; Meds tab as right-pane default;
Condition-dashboard tab bumped to priority 4); billing **resolved** = companion front-desk app + unobtrusive
end-of-consult item/comment widget, consult timer is *advisory only* (multi-room interleaving, principle 4);
toolbar verdict = user-configurable action set (prior art for the shell §7 user preference layer, as is
context-dependent glance frequency); Decision Support/Research captured = **the same fold at condition scope
and practice-population scope** (fractal topology in the UI); results/inbox nutshell recorded incl. the open
**three-zone-layout vs two-pane-shell** question (own session when its screenshots arrive); five principle-4
prior-art exhibits worth citing in spec prose. **Next:** more screenshots incoming from the co-author; the
remaining §4.4 open questions ride on them.

**Session (2026-07-16, latest code thread) — #203 + #96 [C2/B5]: the P3 opener — ADR-0051, the
contributor-role vocabulary floor + responsibility wire shape (branch `feat/adr-0051-role-vocabulary-203-96`;
spec §3.9, v0.52; no new event type, no SCHEMA change; full detail in ROADMAP Slice 41 + the ADR + git).**
`recorded` ratified as the 12th role member (contributory — the recording device: capture fidelity, no
content, no responsibility; 6 bearing + 6 contributory), retroactively legalising every existing mint.
Responsibility is now the spec-§3.9 object `{held_by, on_behalf_of?}` — flat string retired pre-production
(ADR-0040 precedent); `held_by = actor_id = verified attester` extends the #195 binding chain; `on_behalf_of`
wire-expressible day one, refused at submit until a proxy-grant ADR, admitted at apply as a signed
display-gated claim (an apply refusal would re-run the #201 wedge on every future proxy event). #96 resolved:
future members travel partition-prefixed (`bearing:x`/`contrib:x`, permanent part of the signed value) so old
nodes classify them; neither-ratified-nor-prefixed reads **vouching-unknown**, never un-vouched.
`cairn_check_contributors` (db/005) runs at BOTH doors — strict submit / lenient apply (membership never
rejects at apply; refusals only for never-lawful shapes). `contributor_role(role,bears)` table +
`cairn-event::contributor` Rust mirror (`classify_role`) under a standing drift guard. The strict door
immediately caught **three out-of-vocabulary production mint sites beyond the review's list** — cairn-sync's
authoring path minted `role:"author"` with NO actor_id (its events cross db/020 on every pulling peer),
`identity.rs`/`medium.rs` minted `"device"` (an actor kind, not a role) — all now conformant. TDD RED-first
(10 RED + 5 lossless-admission pins); workspace **696/0** + fmt + clippy + docs build clean. **#204 [C3]
scheduled** in Slice 41: the attribution-token/authoring-human slice is committed as the next clinical-plane
slice before any new clinical stream. A **PR #229 review round** then landed on the same branch (4 new
tests, 700/0): `contributor_role` gained the house REVOKE (a stray write MOVES the floor — a hostile
'bearing' row or a flipped `bears`; `floor_enforced.rs` pins INSERT/UPDATE/DELETE denied 42501 for the
runtime role, the event_type_class rationale), `cairn_check_contributors` pins `SET search_path = public`
(the cairn_event_twin discipline), the drift guard sorts `COLLATE "C"` (ADR-0045/#69 — `co-signed`'s
hyphen made it collation-dependent), and 3 more never-lawful apply-door pins. **Operational caveat, by
design:** pre-ADR-0051 event logs (old cairn-sync `role:"author"`-without-actor_id entries, flat-string
responsibility) now REFUSE at db/020 on every sweep — **wipe dev/PoC rigs** (replication-failover demo,
spike rigs), never sync them through; the wedge is pinned deliberately in `contributor_roles.rs`.

**Prior sessions (2026-07-16, same day, condensed — full detail in git + the PRs + ROADMAP Slices 36–40):**
the **P1 floor-hardening slice** (#187/#207/#194/#191/#192[+#177]/#190/#193/#195; PR #219; follow-up
[#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) — the #190 hard veto is still link-arrival-only) ·
the **full P2 arc, five slices, all merged the same day**: #199 [B4] (PR #221 — CI provisions
`cairn_test2`/`cairn_test3`, `medication_remote_apply.rs` + `clinical_pull.rs` E2E) · #198 [B3] (PR #222 —
cairn-sync SCHEMA subset stands alone + drift guard) · #196 [B1] (PR #223 — `db/036` clinical seq cursor +
self-clearing quarantine floor + full sweep every 10 cycles) · #197 [B2] (PR #224 — `AND NOT acked` frees the
quarantine quota) · #202+#201 [B7/B6] (PR #225 — 64 MiB frame caps BOTH ends, `COLLATE "C"` +
`'|'`-separated executed-in-CI convergence fingerprints, byte-tier `Err` logging, `node.superseded` resolved
as REPLICATE with the trust-bounded db/007 apply arm; follow-ups
[#227](https://github.com/cairn-ehr/cairn-ehr/issues/227) HLC-merge helper +
[#228](https://github.com/cairn-ehr/cairn-ehr/issues/228) legible malformed-hex refusals).

**Earlier sessions (2026-07-09 → 07-15), condensed — full detail in git + the PRs + the linked ADRs +
ROADMAP Slices 26–34:** **medication slices 1–5** (assert/cease + E1 flag · bitemporal dose timeline `db/032`
· cross-thread reconciliation ADR-0047 `db/033` · attestation overlay ADR-0049 `db/034` · per-field dose
correction ADR-0050 `db/035`; open [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) db/032
suppression PK-eviction) · the **twin-check registry refactor** (ADR-0048) · the **reference-UI verdict**
(iced FAILS a11y → Tauri 2; PR #174) · the **enroll dual-mapping guard** (ADR-0046, closes #166; open
[#172](https://github.com/cairn-ehr/cairn-ehr/issues/172)) · the **`enroll-human` CLI** + §5.4 finishers 1–3
(open [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168)) · **collation-independent tiebreaks**
(ADR-0045, closes #69) + #159 drift guard · the **HLC-collision advisory log** (`db/029`) + `content_address`
tiebreaker (#115 pt 1) · the **enroll `actor_id` collision floor** (ADR-0044, closes #152) · the
**suppression owner-gate** (ADR-0043; open [#154](https://github.com/cairn-ehr/cairn-ehr/issues/154)).

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

- **First federating node** — built 2026-06-21 ([PR #28](https://github.com/cairn-ehr/cairn-ehr/pull/28)),
  the first implementation of [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md),
  scoped to direct-pairwise trust, no clinical surface: `cairn-node` (Ed25519 keystore,
  `init`/`identity`/pairing/`peers`/`unpeer`, built-in mTLS pinned to the trust set, set-union `node_event`
  sync, honest `status`) + the `db/007` submit/apply doors with a deny-all admission gate. Genesis-stable
  `node_id` = content-address of the genesis enrollment event. **Every honest gap declared at build time is
  CLOSED** (full detail in git + ROADMAP Phases 5/6), including all four
  [ADR-0026](spec/decisions/0026-node-durability-and-disaster-recovery.md) durability slices A–D — only
  optional escrow *rungs* (Shamir M-of-N / QR / TPM) remain, upward options, not blockers. The `localstate`
  DB read/apply **seams** are where the future clinical tier plugs DEKs/drafts/config.
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so plain `cargo test --workspace`
  is reliable.
- **Spike 0002 (advisory-actor write contract)** — ran 2026-06-21, **C1–C5 PASS**
  ([PR #27](https://github.com/cairn-ehr/cairn-ehr/pull/27)) → ADR-0029 + ADR-0030: the in-DB floor held
  against a hostile agent with direct DB access, all rejections legible. Every deferred item since closed
  (the attestation success path E2E, the recall-surface trio, the skeletal twin → ADR-0039).
- **Dual-identifier discipline** — ADR-0031 ([PR #34](https://github.com/cairn-ehr/cairn-ehr/pull/34)):
  the canonical plane (UUIDv7 + multihash) is the *only* identifier on the wire/in signed bodies; the
  projection plane may intern to node-local `bigint` surrogates (`db/008` + the leakage guard). The
  `local_ref` "type barrier" honesty fix merged 2026-06-24
  ([PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43), issue #35 — the domain is an intent-signal +
  one-directional guard; the load-bearing guarantee is the typed signed plane). Final magnitude measured
  on Bet B.
- **Spike 0003 (Postgres on Android)** — ran 2026-06-25, **G0–G3 PASS**
  ([PR #47](https://github.com/cairn-ehr/cairn-ehr/pull/47) + [PR #48](https://github.com/cairn-ehr/cairn-ehr/pull/48)):
  native PG 18.2 + a cross-built pgrx extension on a RedMagic 11 Pro — no Termux userland, no root, no VM
  (fractal topology at the phone tier). Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/).
  Remaining non-load-bearing gaps: from-source PG build, APK/`jniLibs` packaging.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **`clinical.medication` — next slice** (the live clinical build front). Slices 1 (assert/cease) + 2 (dose
  change/correction overlay + bitemporal dose timeline) + 3 (cross-thread reconciliation — ADR-0047, `db/033`;
  PR #178) + 4 (attestation — ADR-0049, `db/034`; PR #182) + 5 (dose effective/reason per-field correction —
  ADR-0050, `db/035`; corrected effective drives winner selection) are DONE. **Next candidates:**
  a **prefer-INN display term** for reconciled groups; **fuzzy/automatic reconciliation** + the Tier-A drug
  dictionary (brand↔generic/DDI) — human-driven resolution exists, automated *detection* is the gap; structured
  sig/frequency (lands with prescriptions); correcting a dose event's *effective date* on the statement-level
  `started` (slice 5 covers the dose-timeline effective; the assert's `started` is a separate concern).
  **Cross-cutting debt:** [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (**cross-thread correction
  *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision**);
  [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory onto the medication/dose/
  reconciliation projections; [#176](https://github.com/cairn-ehr/cairn-ehr/issues/176) (oversize-guard
  remote-apply test); ~~#177 (cross-patient reconciliation)~~ — **RESOLVED 2026-07-16** with the #192
  patient-consistency slice. Spine to reuse: `db/031`–`db/035` + `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine to reuse: `db/010`–`db/030` +
  `cairn-event::demographics`; everything listed in the Phase paragraph above is BUILT — demographics
  slices 1–5, matcher A/B1/B2/B2b/B3, identity C1–C5, the §5.4 John-Doe subsystem).
  **Next (B3 measurement-driven):** a **large hand-crafted gold set** to re-run the learner for
  authoritative magnitudes (slice 24's learner is a PoC on small/synthetic data); locale comparator packs;
  the hub-tier aggressive duplicate sweep; proposal retraction; richer §7.5 matcher-actor determinants
  (served-model digest). **Next identity:** C5+ `reattribute` (§5.5 event-granular strike-through of
  *clinical documentation* — **waits on a clinical-note surface**; note a pending+disputed Doe already reads
  `'under-review'`, severity-max, so the slice-D forcing rule stands down while a dispute is open); the
  §5.12 "prior history now available" push-alert; the §5.3/§5.8 search-before-create funnel.
  Karyotype is resolved as a distinct field ([ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md)) —
  no code yet. Smaller deferred items live in the issues:
  [#79](https://github.com/cairn-ehr/cairn-ehr/issues/79) (B2 minors),
  [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168) (entity→role-actor 1:many); plus (unfiled, in
  code comments): repudiation reversal event + a chart-history VIEW of struck names; fuzzy alias
  recognition + an `alias` blocking pass; fuzzy near-window range softening; volume-generator hard
  negatives / variable cluster size; a veto-aware end-to-end scorer mode; deceased-status veto (stub in
  db/016); a `compare_address` comparator; a CLI sweep entry.
  **Test env:** Rust DB-gated + matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532
  user=hherb dbname=cairn_test"` (PG18+cairn_pgx); the multi-node convergence suites additionally need
  `CAIRN_TEST_PG2`/`PG3` pointing at `cairn_test2`/`cairn_test3` on the same cluster (without them those
  tests self-skip locally — CI sets all three since #199). Matcher integration: `cd matcher &&
  CAIRN_TEST_PG=… uv run --extra pipeline pytest`. The pure matcher suite is dependency-free:
  `cd matcher && uv run pytest` (uv, never venv/pip).
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
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already advanced
  across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#9-bet-b--results-raspberry-pi-5--8-gb-2026-06-25--pass-with-two-honest-caveats)):
  **PASS twice** — 2026-06-25 (caveated: USB-2 dock, PG16) and the clean 2026-07-07 re-run on PG 18.4 + a
  PCIe NVMe HAT, both caveats resolved
  ([§9.5](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#95-clean-re-run-pg-18-nvme-2026-07-07-pass-both-caveats-resolved)):
  B1 p95 **3.99 ms @ 2,004,000 events** (13× under budget), B2 p95 4.5 ms/374-note chart, ~1,515 B/event on
  disk; B4 confirms ADR-0015's BLAKE3 blob-digest default (~4× SHA-256 on Cortex-A76); `cairn_pgx`
  builds+loads on Pi arm64. Artifacts in [`poc/walking-skeleton/results/`](../poc/walking-skeleton/results/).
  **Remaining:** (c) fold the (now un-caveated) B4 number into the ADR-0015 follow-up to drop "provisional"
  from the blob-digest line.
- **easyGP session** — port the [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  deferred items with live easyGP schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon (validates ADR-0001 from production). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **easyGP GUI-mining continuation** — more consult-screen/module screenshots incoming from the co-author;
  they should answer most of the remaining §4.4 open questions in
  `scratch/ui-sketches/easygp-consult-screen-inventory.md` (Todo/BMI strip, pure fossils, Research-module
  ranking logic) and open the **results/inbox design session** (the three-zone-layout vs two-pane-shell
  question is parked there — don't improvise it).
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
| [0051](spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) | Contributor-role vocabulary floor: `recorded` ratified (12th, contributory); responsibility = `{held_by, on_behalf_of?}`; future members partition-prefixed; strict-submit/lenient-apply | §3.9 (refines 0028/0007/0049/0012) |
| [0052](spec/decisions/0052-born-sealed-clinical-bodies.md) | Born-sealed clinical bodies: every clinical JSONB body sealed at write under a per-event DEK held by the node (erasability substrate, not confidentiality); erase ladder always reachable; two doors enforce sealed⇒clinical scope; custody plane + custody sidecar + rung-3 shred | §3.5/§3.8/§5.9 (refines 0005/0006/0026/0048/0051) |

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice, see above); 0002 (advisory-actor —
C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓; PR #47/#48); 0004 (iced reference-UI
viability — FAIL on a11y → Tauri 2).
