# HANDOVER — Cairn

**Session date:** 2026-07-11 · **Spec/ADRs:** v0.46 · **Phase:** architecture complete; **first
production clinical surface under construction** — demographics on `cairn-node` (slices 1–5 done) + the §5.2 matcher
(piece A in-DB veto floor · B1 advisory scoring core · B2 veto-gated pairwise pipeline + proposal worklist · B2b
blocking / candidate-pair generation + batch sweep · B3 eval harness · B3 compound blocking key (`name+year`) · B3
synthetic volume generator · B3 eval mirror (range-DOB + administrative-sex) · B3 weight-learning: supervised
Fellegi–Sunter estimation · **B3 further compound blocking keys `dob+first-initial`/`name+sex` — done this session**
· consumes `patient_alias_pool` known-alias evidence · range-aware, positive-only
`compare_dob` for clinician-observed estimated ages · anchored birth-year-range blocking passes (`dob-range` /
`dob-range+sex`) + the A/B pass-toggle · composite `sex` scoring + the unconfirmed-chart REVIEW rule) +
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
evidence override — **done this session**)
+ identity **C5+** (`reattribute` — waits on a clinical-note surface) + the **rest of the §5.4 subsystem**
(finisher 3 `identify`→optional-link — its own spec/PR next; the "prior history now available" push-alert;
the search-before-create funnel).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node,
Postgres-on-Android).

**This session (2026-07-11) — §5.4 finishers PR#1: the node-local John-Doe ordinal + `--observed-year`
(no issue; branch `feat/john-doe-ordinal-and-observed-year`; no ADR/spec/SCHEMA/floor/wire change).** Two
small self-contained §5.4 finishers, brainstorm→spec→plan→subagent-driven TDD (design+plan under
`docs/superpowers/{specs,plans}/2026-07-11-john-doe-ordinal-and-observed-year*`). **Finisher 1 — a
node-local friendly John-Doe ordinal.** The callsign identity string stays UUID-suffixed (partition-safe;
the code deliberately rejected a partition-racing per-day counter), so instead a new **read-only VIEW**
`db/030_john_doe_local_ordinal.sql` derives "this node's John Doe #N" from `event_log`: `row_number()`
**PARTITION BY `node_origin`** over the callsign registrations each node first recorded (exact predicate:
`demographic.field.asserted` + `field=name` + `facets.use=callsign` + `provenance=system:john-doe-registration`),
ordered by the collation-free `(hlc_wall,hlc_counter,content_address)` spine (#115/#69). Node-local by
construction (a replicated foreign registration lands in its own partition, never shifts this node's
sequence); never signed, never on the wire, never an identity/merge key. `register_john_doe` now returns
`(Uuid,String,i64)` and the CLI prints `local ref: John Doe #N (this node)`. All-time (no daily reset → no
TZ semantics). **Finisher 2 — `--observed-year` override** for `assert-observed-evidence`: a pure
`resolve_observed_year(Option<i32>, current_year) -> Result<i32>` bounds a supplied year to
`1900..=current_year` (reject future / absurdly-historical; principle 4 — honest reject, never a garbage
range), defaulting to today; parameterizes the computed DOB range only, **not** `t_effective` (deliberate
scope boundary; the library fn was already `observed_year`-parameterized). TDD (new DB-gated per-node
partition/callsign-only test + 5 pure `resolve_observed_year` tests); full workspace green (cairn-node
DB-gated all pass · cairn-event · cairn-sync); fmt + clippy clean. **Finisher 3 (`identify`→optional-link)
deliberately deferred** to its own spec/PR — `identify` has no authoring surface/CLI yet and the optional
link needs attestation-from-CLI (new ground).

**Prior session (2026-07-10, later) — the `patient_name_current` ORDER BY drift guard
([#159](https://github.com/cairn-ehr/cairn-ehr/issues/159) CLOSED; no ADR/spec change).** The #69
follow-up: the winner `ORDER BY` of `patient_name_current` is written TWICE — db/012 and db/025's
repudiation anti-join re-definition (which, loading last, is the **live** one) — with nothing keeping
the two `COLLATE "C"` tiebreak clauses in lockstep, so a future edit to one that missed the other would
silently re-open the #69 cross-node display-winner divergence. A true SQL single-source is infeasible
(`DISTINCT ON` forces each view to carry its own ORDER BY, and db/025 must anti-join struck names
*before* the winner is picked, so the ordering can't be factored into a shared base view/window). Fix:
a **no-DB source-level drift guard** — the migration SQL is `include_str!`-embedded, so
**`crates/cairn-node/tests/name_winner_order_drift.rs`** reads both clauses and asserts they are
byte-identical (whitespace-normalized, via a pure `winner_order_by` extractor), catching drift in EITHER
direction (incl. db/012's otherwise-inert copy) in every `cargo test`/CI pass — no cluster needed. Plus
cross-reference `DRIFT` comments on both migrations naming the guard. TDD (extractor unit test RED→GREEN;
guard RED-confirmed by a temporary db/025 COLLATE-drop, reverted). Full workspace green; fmt + clippy clean.
**Post-review polish (this PR):** the extractor now strips `--` comments (string-literal-aware) before
scanning, so a future `-- …order by…` comment between the view header and the real clause can't be
mis-read as the winner ordering (two added regression tests); and the case-handling docstring is
corrected — keyword *location* is case-insensitive but the compared slice preserves case on purpose
(`COLLATE "C"` is a case-sensitive quoted identifier), so the guard errs strict by design.

**This session (2026-07-10) — codebase-wide collation-independent projection winner tiebreaks
([#69](https://github.com/cairn-ehr/cairn-ehr/issues/69) CLOSED; ADR-0045; spec v0.45→0.46).** The residual #115
explicitly deferred: every trigger-maintained projection broke a `(rank, hlc_wall, hlc_counter)` winner tie on
TEXT keys (`node_origin`/`asserted_origin` and the final `value`/`display`/`use_key`) compared under the
**node-local default collation** — so two nodes with different default collations could replay the identical
event set and pick **different display winners** for a tie (a silent set-union convergence violation, principle 1;
not data loss — full history stays in `event_log`). The **cross-origin** `(wall,counter)` tie needs no misbehavior,
just two honest nodes coinciding on wall+counter, and was decided *before* #115's collation-free `content_address`
was reached. Fix: every projection winner tiebreak over a TEXT key now compares under **`COLLATE "C"`** (byte order
of the identical-on-every-node UTF-8 bytes) — **[ADR-0045](spec/decisions/0045-collation-independent-projection-tiebreaks.md)**
records the invariant (binds all future projection slices). One shared predicate fix `cairn_hlc_overlay_wins`
(`db/002`) covers the **five standing-state overlays**; inline `COLLATE "C"` on the demographic projections +
display VIEWs: `patient_identifier` (db/010), `patient_demographic` (db/011 superseded + **db/013 both CASE
branches + `cairn_demographic_backfill`**), `patient_name` (db/012 trigger + `patient_name_current` VIEW, **plus its
db/025 re-definition** — the identity-repudiate migration `CREATE OR REPLACE`s the VIEW *after* db/012, so the
db/012-only fix was inert until db/025's copy was fixed too), `patient_address` (db/014 trigger +
`patient_address_current` VIEW). **Scope audited codebase-wide:** the db/018 person-canonical `min UUID` uses the
uuid `<` operator (not text — already collation-free); the identity aggregation VIEWs have no text winner tiebreak.
**Projection-read-side only** — no wire/event-format/floor-gate/SCHEMA change. TDD, **new
`projection_collation_convergence.rs`** (7 tests) + a collation case in `overlay_tiebreaker.rs`: each proves the
winner follows `"C"` byte order in **both arrival orders** via the `'B'`/`'a'` locale-flip pair (a locale collation
orders them oppositely), covering both the trigger ON-CONFLICT origin path and the VIEW/DISTINCT-ON display path
per projection; the backfill fix RED-confirmed via temporary revert. Full workspace green (cairn-node all DB-gated
pass · cairn-event 86 · cairn-sync 18); fmt + clippy + mkdocs clean. brainstorm→design→plan→**subagent-driven TDD**
(per-task spec+quality review; caught + fixed: db/025 VIEW shadow, the backfill 2nd-copy, an fmt-gate slip, two
untested-trigger-path gaps). **Follow-up ([#159](https://github.com/cairn-ehr/cairn-ehr/issues/159)) — now CLOSED (see top block):** the
`patient_name_current` ORDER BY duplicated across db/012 and db/025 with nothing enforcing lockstep — a drift
trap; guarded by `name_winner_order_drift.rs`. Design+plan under `docs/superpowers/{specs,plans}/2026-07-10-collation-independent-projection-tiebreaks*`.

**Earlier this session (2026-07-10) — the Byzantine HLC-collision advisory signal
([#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) done, PR #158; no ADR/spec change; full detail in git +
ROADMAP Phase 2).** #115's tiebreaker resolved a Byzantine/broken-signer HLC-triple collision (two **distinct**
`content_address`es sharing one `(hlc_wall,hlc_counter,origin)` triple) *silently*; now it is also **surfaced**.
New `db/029_hlc_collision_log.sql`: a shared pure `cairn_hlc_triple_collision()` predicate (null-safe) + a
**convergent** append-only `hlc_collision_log` (canonical unordered `content_address` pair as PK → one row per 2-way
collision per node) + a **structurally** non-gating `cairn_record_hlc_collision()` recorder (`INSERT ... SELECT` with
a null-guard `WHERE` + `ON CONFLICT DO NOTHING` → can never raise → cannot gate the apply path by construction). Each
of the five overlay triggers records the signal before its unchanged upsert (#115 resolution untouched; AFTER-INSERT
trigger → door-agnostic). Projection-read-side only. Accepted limits: a concurrent apply may miss the signal; a
≥3-way collision records a non-convergent pairwise chain — §5.13 sweep is the backstop, the resolution stays correct.
The Python §5.13-sweep / human-worklist consumer is a documented future seam. TDD/subagent-driven; design+plan under
`docs/superpowers/{specs,plans}/2026-07-09-hlc-collision-advisory-signal*`.

**Prior session (2026-07-09, evening) — deterministic HLC-overlay tiebreaker
([#115](https://github.com/cairn-ehr/cairn-ehr/issues/115) part 1 done; no ADR/spec change; detail in git + ROADMAP
Phase 2).** A shared pure `cairn_hlc_overlay_wins()` predicate (`db/002`) appends the event `content_address` (BYTEA,
collation-free) as the deterministic final tiebreaker for the five standing-state overlays, fixing an arrival-order
divergence when two distinct events shared one `(wall,counter,origin)` triple. **Remaining #115:** part 2's twin-ladder
registry + `cairn_require_uuid` helper (independent refactors). Its #69 residual (cross-origin/`value`/`display`
TEXT-collation) and its #157 collision-signal follow-on are both **done** — see the top blocks.

**Prior session (2026-07-09, later) — the actor `actor_id` collision floor: enroll fails closed
([#152](https://github.com/cairn-ehr/cairn-ehr/issues/152) CLOSED; ADR-0044; spec v0.44→0.45).** The SECOND
in-DB **floor authorization** change of the demographics build (after #99). `enroll_actor` derives
`actor_id = cairn_actor_id(pinned)` — the content-address of the **pinned set only**, never the signing key
(the key must stay mutable across the future `rotate-key`, ADR-0011 §5). So two **different** humans enrolled
with an **identical** pinned set (the minimal `{"role":"clinician"}`) collided into one `actor_id`, and
`actor_current`'s `DISTINCT ON (actor_id)` silently dropped the earlier key — a **silent identity-merge** on
the trust anchor (principle 2), surfaced by the #99 owner-gate tests. Fix: a new pure `STABLE`
`cairn_actor_id_key_conflict(actor_id, key)` predicate + an `enroll_actor` guard that **refuses** a
distinct-key collision across the **whole `actor_event` history** (immortal even after `revoke` — no
post-revoke reuse); **idempotent same-key re-enroll still passes** (re-runnable provisioning; the matcher
per-epoch re-enroll). The key is deliberately **not** hashed into `actor_id` — this is an enforcement gate,
not a derivation change. **Human determinant = guidance only** (ADR + `db/004` comment: a human's pinned set
should carry a person-distinguishing handle/registration id; the floor makes a forgotten one **loud** on the
second enroll; the actual field is left to the future enrollment surface — ADR-0011 keeps pinned-set contents
as policy). **Single door** — no remote-apply door for actor enrollment exists yet (`INSERT INTO actor_event`
lives only in `enroll_actor`); forward caveat recorded (mirror the check when actor-event sync lands, ADR-0011
§4 — analogue of #154). `db/004` edited **in place** (pre-clinical posture, the #99 pattern). **Notable
side-fix (house rule 5):** the #99 `cross_human_suppress_refused_after_author_key_rotation` test **staged a
key rotation by re-enrolling the identical pinned set with a new key** — i.e. it leaned on the very
silent-merge bug #152 fixes; it now stages the rotation end-state with a raw `actor_event` insert (what a
future `rotate-key` door produces internally), making the #99 test more honest. TDD, **5 DB-gated Rust tests**
(`actor_enroll_collision.rs`: collision refused via distinct key; idempotent same-key allowed; distinct pinned
sets don't collide; immortality after revoke; same-key re-enroll after revoke refused) + a SQL mirror in
`db/tests/004_actors_test.sql`. Full cairn-node DB-gated workspace green; fmt + clippy clean; mkdocs builds.
Design+plan under `docs/superpowers/{specs,plans}/2026-07-09-actor-id-collision-floor*`. Post-review polish
(detail in git/ADR-0044): a txn-scoped `pg_advisory_xact_lock` closes the check-then-insert TOCTOU race; the
predicate deliberately also refuses a same-key re-enroll onto a **revoked** `actor_id` (anti-resurrection),
documented + test-pinned.

**This session (2026-07-09) — the suppression owner-gate: self-only, disagreement is additive (ADR-0043; spec
v0.43→0.44; closes the last open sub-item of [#99](https://github.com/cairn-ehr/cairn-ehr/issues/99)).** The
FIRST in-DB **floor authorization** change since the demographics build began. Design+plan under
`docs/superpowers/{specs,plans}/2026-07-09-suppression-self-only-owner-gate*`. A suppressing overlay
(`salience.downgrade` / `visibility.suppress`) that forecloses on a **human author's** event is now refused unless
the suppressor is that human — disagreement is expressed **additively** (a note referencing the target), never by
touching another author's content (principle 1/2 + paper-parity, read correctly: you cannot un-write a colleague's
ink). **Agent-authored / un-owned advisories stay dismissable** by any enrolled human (principle 10 —
clinician-overrides-the-machine). One shared STABLE helper `cairn_suppression_author_ok(target, attester_key)`
(human-authors = `{signer_key_id if kind='human'} ∪ {hex(attester_key) if present}`; empty ⇒ dismissable, non-empty
⇒ self-only; safe-refuse on ambiguity) enforced identically at **both** write doors — `submit_event` (`db/005`) and
`apply_remote_event` (`db/020`) — so a replicated cross-human suppress faces the same refusal (principle 12). Both
migration files edited **in place** (the #99 hardening pattern; pre-clinical posture). **Scope carve-outs:** §5.9
sensitivity-sealing (separate authorized path, its own safety projection) and `repudiate` (`targets_other=FALSE`,
value-grained) are untouched. **Deliberate divergence from ADR-0010 §2** (cross-author *demotion* is gated too, not
just hiding — the maintainer's clinical call). **The other three #99 sub-items were already fixed 2026-07-02**
(recall-epoch join, `recall_overlay` FK, `actor_current` tiebreak). Signer human-ness is resolved from the
**append-only `actor_event` history**, not `actor_current`, so a departed/rotated author's notes stay protected
(the over-permission hole caught in whole-branch review; guarded by a rotation regression test). TDD, **9 DB-gated
tests** (self via signer + attester paths; self-hide via `visibility.suppress`; agent-dismissable; cross-human
downgrade + hide refused; cross-human refused at the apply door; cross-human refused after author-key rotation;
cross-human suppress of a human-*attested* advisory refused — the attester-branch refusal); full workspace green
(cairn-node DB-gated all pass · cairn-event 86 · cairn-sync 18); clippy + fmt clean; mkdocs builds. **Follow-ups
filed (house rule 5):** (1) [#152](https://github.com/cairn-ehr/cairn-ehr/issues/152) — `enroll_actor`/`actor_current`
collide two humans with identical pinned JSON into one `actor_id` (`cairn_actor_id` hashes the pinned set only, not
the signing key), a latent identity-merge footgun on the `db/004` actor floor (now fixed — #152/ADR-0044 above); (2)
**[#154](https://github.com/cairn-ehr/cairn-ehr/issues/154) (OPEN)** — the apply-door gate inherits the
node-local-registry limitation: a **plain-signed** human note (no stored `attester_key`) is protected at a remote
node only once that node has learned the author's `kind='human'` enrolment (attested targets are registry-independent);
the origin always refuses; closes with registry federation.

**Merged 2026-07-08 (condensed — full detail in git + the PRs + ROADMAP Phase 1).** A dense build+hardening day,
all on `main`:
- **§5.4 marks/belongings/EMS-context text identity evidence** (PR #142) — three text `kind` values (`mark`,
  `belongings`, `ems-context`) on the **existing** `identity.evidence.asserted` type (photo's non-attachment
  sibling); no migration/floor/SCHEMA/ADR/spec change, `attachments` stays `vec![]`. Pure
  `cairn-event::identity_evidence` (`TEXT_EVIDENCE_KINDS` closed set + typo-drift guard + `text_evidence_body`/twin)
  + a `cairn-node` author path (`validate_description` honest-content floor in the library) + one folded
  `assert-identity-evidence --kind photo|mark|belongings|ems-context` CLI gated by a pure `route_identity_evidence`.
- **CI + tooling catch-up** (PRs #143/#147/#149/#150/#151) — rustfmt-defaults `fmt` gate + cargo-deny (`deny.toml`,
  pinned 0.19.9) + `matcher.yml` (ruff+pytest); `rust-toolchain.toml` pins `1.96.0` + `[workspace.lints]`; PG16→18 CI
  bump + PG-version-independent floor job name; matcher DB-suite now runs in CI ([#145]); CodeQL crypto FP fixed at
  source → **CLAUDE.md house rule 6** ([#146]); required-checks tabled in `CONTRIBUTING.md` ([#117]); stricter ruff
  ruleset. Closed [#144]/[#145]/[#146]/[#117]. Matcher debt: stale forced-REVIEW proposal retraction ([#135], PR #151,
  append-only `retract_pending_proposal`) + integration-test committed-row leak ([#84] pt1, PR #150).
**Prior session (2026-07-08, first) — §5.4 photo evidence + the day-one §3.14 attachment-reference shape (ADR-0042; spec
v0.42→v0.43; design+plan under `docs/superpowers/{specs,plans}/2026-07-08-attachment-shape-and-photo-evidence*`).**
The FIRST content-addressed **attachment** on a clinical surface, which forced finalizing the ONE can't-retrofit
piece of ADR-0013. **Two phases, 9-task subagent-SDD, final whole-branch review "ready to merge" (0 Critical/
Important).** (1) **The shape** — replaced the walking-skeleton `AttachmentRef` stub with
`Attachment{descriptor, renditions:[Rendition{role,alg,digest_hex,media_type,byte_len,inline?,seal?}]}` +
`SealRef{alg,dek_wrap}` in `cairn-event/src/attachment.rs` (all five §3.14 reserves: content digest, descriptor
metadata, **rendition set** [structurally can't-retrofit], **seal indicator** + **inline-vs-reference** [reserved,
None]); field order frozen by **ADR-0042** (refines 0013, reconciles with **ADR-0041**'s note `payload.media`
manifest — one shared primitive, two carriers: `EventBody.attachments` for non-narrative events vs the note payload).
Empty-vec byte-identity proven, so every past zero-attachment event keeps its content-address. (2) **The floor** —
`db/027` `cairn_learn_attachment_refs` walks `attachments[*].renditions[*]` (skips inline); db/005 **and** db/020
call the one shared helper (no drift). (3) **Photo author path** — new non-demographic `identity.evidence.asserted`
(payload `{kind:"photo",provenance:"clinician-observed",basis?}`, photo in `EventBody.attachments`, twin from
descriptor **never pixels**; `db/028` registers the type — fail-closed floor); `cairn-node/photo_evidence.rs`
(pure `prepare_local_blob` + atomic `assert_photo_evidence` storing the blob present through the db/026 verify
trigger + authoring the event in ONE txn, `ON CONFLICT DO UPDATE` to fill a pre-existing reference-only placeholder)
+ an `assert-photo-evidence` CLI. Suites **workspace 418 passed / 0 failed; clippy clean**. **Honest limits:** ships
**plaintext** (seal reserved), a **single `original` rendition** (no preview — needs an image lib), **bytes stay
local** (cross-node byte fetch deferred), and the frozen POC harness diverges from the new shape. **Review fixes
applied post-build:** the honest-descriptor rule now lives in the library (`photo_evidence::validate_photo_descriptor`,
not only the CLI, so a future UI backend inherits it); a **direct db/020 apply-door attachment test** now exercises the
remote-apply call site of `cairn_learn_attachment_refs` (both doors directly covered); the local-blob **size-guard** gap
(no ceiling on `blob_store.content`, whole file read into memory) is lodged as **[#141](https://github.com/cairn-ehr/cairn-ehr/issues/141)**
for the §6.6 byte-tier slice. Residual accepted: DO-UPDATE overwrites caller-supplied `media_type` on an already-present
row (benign). Env: `cairn_pgx` upgraded to **0.3.0** on the Mac :5532 cluster this session (was 0.2.0 — db/026 requires ≥0.3.0).

**Earlier 2026-07 slices (condensed — full detail in ROADMAP slices 20–25 + git + the linked PRs).** All merged on
`main`, all advisory-tier / additive unless noted:
- **07-07** — B3 compound blocking keys `dob+first-initial` + `name+sex` (slice 25, PR #138; registry 6→8, shared CTE
  fragments); B3 weight-learning: a supervised Fellegi–Sunter learner + entity-cluster held-out lift (slice 24) —
  ships the *mechanism*, not new shipped weights; safety-first thresholds (`auto = max(non-match)+margin`).
- **07-06** — B3 eval mirror: generator range-DOB emission + `DatasetRecord.administrative_sex` + range-aware
  `shares_blocking_key` (slice 23, PR #136) — unblocked weight-learning.
- **07-05** — §5.4 slice D: composite `sex` scoring + the unconfirmed-chart REVIEW forcing rule (slice 22, PR #134,
  closes #130); the **blob self-verification in-DB floor** (`db/026`, `cairn_pgx` 0.3.0, PR #132 — hostile-client
  proof); **clock-drift admission ceiling** on both remote-apply doors + the first `rust.yml` CI gate (PR #133,
  closes the #102 ratchet).
- **07-04** — §5.4 slice C: anchored birth-year-range blocking passes + A/B pass-toggle (slice 21, PR #131); §5.4
  slice B: clinician-observed evidence (estimated-age range + observed sex) + range-aware positive-only `compare_dob`
  (slice 20).

**Prior session (2026-07-03) — §5.4 John-Doe slice A: callsign minting + matcher placeholder exclusion (condensed;
full detail in git + ROADMAP slice 20 + PRs #123/#125).** No new event type / migration / SCHEMA / ADR / spec bump.
Pure `cairn-event::john_doe::callsign` (`Unknown-<class>-<site>-<date>-<suffix>`, Unicode-aware sanitizer, 32-bit
UUID-derived suffix — partition-safe, bedside-collision ~1-in-4.3-billion-per-pair); `register_john_doe` + CLI
(callsign assertion + C4 `identity.pending.asserted` in ONE txn → *unconfirmed* chart); matcher excludes placeholder
`use_key` from blocking AND scoring (the scoring exclusion is load-bearing). Review hardening: kind-AGNOSTIC
actor-enrolment guard; the placeholder-use set hoisted into pure `cairn_matcher.placeholder_uses` + a cross-language
Rust↔Python drift guard (#124 closed). **§5.4 slice A is BUILT.**

**Prior sessions (2026-07-03) — the §5.7 identity algebra to C5 + the matcher's alias consumption (condensed; full detail in git + ROADMAP slices 15–19).** All merged on `main`, all additive (no floor/SCHEMA/ADR/spec bump except where noted): **C2b** auto-apply of the `auto_candidate` band (matcher-authored, un-attested, recallable link; apply-time veto re-check; per-`matcher_version` `agent` actor); **C3** `dispute` + the `chart_trust` projection (`db/023`; the *under-review* state); **C4** `identify` + the *unconfirmed* state (`db/024`; the §5.4 identity-pending front door; the §5.7 confirmed/unconfirmed/under-review contract is COMPLETE via a severity-max `chart_trust`); **C5** `repudiate` + the known-alias pool (`db/025`; the first *suppressing* identity event — a value-grained `name_repudiation` overlay strikes a known-false name from `patient_name_current`, `mode='suppressing'` forces the human-attestation floor, `patient_alias_pool` surfaces struck names to the matcher); and the **matcher consuming `patient_alias_pool`** (advisory Python — a `known_alias` evidence entry on the proposal, flag-never-suppress, `band()` forces REVIEW; the confidentiality-split view is reason-free per ADR-0006). **Deferred (recorded):** `reattribute` (waits on a clinical-note surface); fuzzy alias recognition; a per-slice identity-floor helper refactor + a deterministic `content_address` tiebreaker on the HLC overlay ([#115](https://github.com/cairn-ehr/cairn-ehr/issues/115)).

**Merged 2026-07-02 (9 PRs; full detail in git + ROADMAP slices 6–14).** A dense build+review day, all on `main`: the **quarantine/legibility trilogy** (durable pull-plane quarantine + re-offer floor on the clinical `db/021` and node-event `db/022` planes + ADR-0040 legibility/skew primitives wired into every signature door, `cairn_pgx`≥0.2.0 startup floor); **ADR-0040 signing-context domain separation** (spec v0.40→v0.41; the day's only spec bump); the **in-DB clinical apply door** `db/020_apply_remote_event.sql` (a replicated event faces the same floor as a local one) + the contamination-cascade recall-key fix (#99); a **7-agent adversarial review** (`docs/code_reviews/2026-07-02-*`) → in-branch fixes + filed issues #91–#103; and **identity C1** (`db/018` §5.1/§5.7 linkage core) + **C2** (`db/019` human-accepted→attested link).

**Prior sessions (2026-06-29/30/07-01) — the §5.2 advisory matcher pipeline B2→B3 (condensed; full detail in git + ROADMAP slices 8–12).** Advisory Python, no `db/` floor except B2's `db/017_match_proposal.sql` worklist (SCHEMA 15→16); no ADR/spec bump. **B2** veto-gated pairwise pipeline + proposal worklist (`cairn_matcher/pipeline/`); **B2b** blocking / candidate-pair generation (3-pass disjunction, oversized-block guard) + a `sweep()` batch driver; **B3 eval harness** (`cairn_matcher/eval/` — scorer metrics + DB-gated blocking-recall measurement + culture-plural `gold_v1.json` + a `python -m cairn_matcher.eval` CLI, real-path reuse/no-drift); **B3 compound blocking key** (additive `name+birth-year` `UNION ALL` pass in `pipeline/db.py`; recall non-decreasing; honest culture-neutral year degrade via the first 4-digit run); **B3 synthetic volume generator** (`eval/generator.py` pure + `eval/generate.py` CLI — seed+corrupted-clone entity clusters recoverable by construction, drift-canary-pinned to the base blocking passes). **Deferred:** an A/B pass-toggle in `generate_candidate_pairs` (quantitative before/after); weight-learning; further compound keys; a veto-aware/e2e scorer mode; the matcher test-leak + harness `KeyError` ([#84](https://github.com/cairn-ehr/cairn-ehr/issues/84)).

**Prior sessions (2026-06-28/29) — §5.2 matcher pieces A + B1 (condensed; full detail in ROADMAP slices 6–7 + git):** **piece A** = the **§4.4/§5.2 in-DB hard-veto floor** (`db/016_match_veto.sql`, SCHEMA 14→15; `cairn_match_veto` returns the closed hard-veto set — same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash; two verdicts `hard_veto`/`degrade_hold`; precision-gated, parses no dates; `system:unknown` never vetoes; forces a human decision, never auto-link/auto-reject; 12 integration tests; deceased-status veto deferred, stub in db/016). **piece B1** = the **§5.2/§5.13 advisory scoring core** (new `matcher/` uv project, `cairn-matcher`, AGPL-3.0, **zero runtime deps, pure functions only** — the fit-for-purpose §9 tier): the `Comparator`/ordinal `AgreementLevel` contract (`PHONETIC`/`NICKNAME` reserved but never emitted by core — anti-cultural-capture), in-house **Jaro–Winkler** + 4 culture-neutral comparators (`compare_exact`/`compare_edit_distance`/`compare_dob` [parses no date strings]/`compare_name_set`) + positive-only `compare_identifier_sets` (never DISAGREE) + the field→comparator registry + the **Fellegi–Sunter** combiner producing an explainable `MatchScore`; 55 pure tests; final review caught + fixed one Critical (`score(a,b)≠score(b,a)` from greedy name-pairing → now `max(greedy(a,b),greedy(b,a))`, symmetric). No new ADR, no spec bump (both implement settled §5.2/§5.13/§4.4; refine ADR-0014/0033).

**Prior session (2026-06-28) — globalised the §3.13/§4.5 author-materialised legibility twin to every event type (ADR-0039; spec v0.39 → v0.40; condensed — full detail in git + the ADR).** `db/015` (SCHEMA 13→14): floor PREFERS the authored twin for every type; non-demographic types degrade honestly to a flagged, payload-rendering derived skeleton (closes the `db/005:29` TODO); demographic types keep ADR-0034's HARD requirement; authored-vs-derived derivable, not stored (`cairn_twin_is_authored` + `event_twin_provenance`). Pure `resolve_twin`/`materialise_generic_twin` shared by cairn-sync + the SQL floor. Same-branch floor bug fix: PG `trim()` is ASCII-space-only → blank-tests use `regexp_replace(x,'\s+','','g')` in BOTH write gate and read predicate; residual Unicode-whitespace asymmetry is [issue #75](https://github.com/cairn-ehr/cairn-ehr/issues/75). **The "globalise the authored twin" deferral is CLOSED.**

**Prior sessions (2026-06-27/28) — demographics slices 1–5, condensed (full detail in ROADMAP slices 1–5 + git):** **slice 1** = §4.4 patient-identifier assertion end-to-end (`db/010`, `EventBody.plaintext_twin`, `cairn_event_twin` hook, set-union `patient_identifier` projection; [issue #67](https://github.com/cairn-ehr/cairn-ehr/issues/67)); **slice 2** = §4.2 DOB + sex-at-birth provenance-locked fields (`db/011`, generic `demographic.field.asserted` + `cairn_provenance_rank` ladder incl. new `fact-proven` top tier; floor open / projection gated — the ADR-0012 federation-forward call; [issue #69](https://github.com/cairn-ehr/cairn-ehr/issues/69)); **slice 3** = §4.2 names (`db/012`, `patient_name` retained-set + `patient_name_current` recency-first-within-legal-tier display VIEW, [ADR-0036](spec/decisions/0036-demographic-name-display-recency-first.md); PR #71+#72); **slice 4** = administrative-sex + gender-identity (`db/013`, one `cairn_demographic_field_policy(field)` classifier driving both projection gate and winner ordering — sex provenance-first, gender-identity recency-first; karyotype resolved as a distinct field, [ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md); PR #73); **slice 5** = §4.3 address (`db/014`, per-use recency-first `patient_address_current` VIEW, same logic as names, [ADR-0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md)). Also closed demographics **gap B** (provider-number person×org relational model, [ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md), §4.6: entity/relationship + subject-kind partitioning, design/spec only) and representation gaps B+C ([ADR-0032](spec/decisions/0032-culture-neutral-address-representation.md) address, [ADR-0033](spec/decisions/0033-patient-identifier-representation.md) identifier namespace/profile split, [ADR-0034](spec/decisions/0034-demographic-legibility-twin.md) legibility twin). Spec 0.32→0.39 across this run. **Demographics slices 1–5 + gaps A/B/C all done; §4.2/§4.3/§4.4/§4.5/§4.6 complete.**

**Prior sessions (2026-06-25/26)** — ADR-0026 node durability slices B/C/D closed (backup-as-cold-peer, restore + `supersede`, sealed local-state export) + issues #53/#54 (cold-medium self-identification, uniform key zeroization) + **Spike 0003 (Postgres on Android) G0–G3 PASS**. Full detail: ROADMAP Phase 5/6 + git + the ADR-0026 log.

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
- **Demographics build — next slices** (the live build front; reuse the spine in `db/010`/`db/011`/`db/013`/`db/014` +
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
  `1900..=current`); and **`identify`→optional-link** wired into one resolution flow (finisher 3 — its own
  spec/PR: `identify` needs an authoring surface/CLI built from scratch + attestation-from-CLI). Reattribute composes one more *under-review*
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
  dbname=cairn_test"` (PG18+cairn_pgx); matcher integration: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline
  pytest`. The pure matcher suite is dependency-free: `cd matcher && uv run pytest` (uv, never venv/pip).
- **Clinical case-mining** — historically the highest-signal generative mode; the event-overlay + key-custody +
  actor primitives have absorbed every case so far without new architecture. Bring a real ED/hospital failure mode.
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

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B prepared); 0002 (advisory-actor — ran, C1–C5 ✓
→ ADR-0029/0030); 0003 (Postgres on Android — **ran 2026-06-25, G0–G3 ✓**; PR #47/#48).
