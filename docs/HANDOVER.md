# HANDOVER — Cairn

**Session date:** 2026-07-02 · **Spec/ADRs:** v0.41 · **Phase:** architecture complete; **first production clinical
surface under construction** — demographics on `cairn-node` (slices 1–5 done) + the §5.2 matcher (piece A in-DB veto
floor · B1 advisory scoring core · B2 veto-gated pairwise pipeline + proposal worklist · B2b blocking / candidate-pair
generation + batch sweep · B3 eval harness — scorer + blocking-recall measurement · B3 compound blocking key
(name-token+birth-year) · B3 synthetic volume generator) + the **§5.1/§5.7 identity linkage core (piece C1)** + the **§5.2/§5.7
match_proposal→apply seam (piece C2) — done this session**; remaining B3 weight-learning / locale packs / A-B
pass-toggle + identity pieces C2b (auto-apply of the `auto_candidate` band) + C3+ (rest of the §5.7 algebra) next.
Viability proven by spikes (walking skeleton, advisory-actor contract,
a first federating node, Postgres-on-Android).

**This session (2026-07-02, latest) — issue #109: wire the ADR-0040 legibility/skew primitives into the doors +
daemon.** PR #107 added the primitives (`cairn_verify_error(bytea)`, `cairn_pgx_version()`, extension bumped to
0.2.0) but left the consumers unwired; this closes all three parts. **(1) Legible rejection reason at every
signature door** — `submit_event` (db/005), `apply_remote_event` (db/020), `submit_node_event` +
`apply_remote_node_event` (db/007), `restore_node_event` (db/009) keep the boolean `cairn_verify` gate but now
attach `USING DETAIL = coalesce(cairn_verify_error(p_signed),'unknown')`, so a wire-format skew / pre-ADR-0040
context mismatch is distinguishable from tampering at the SQL boundary (during a skew-induced write outage the
generic message misdirected the operator badly). **(2) Daemon startup skew check** — a pure
`parse_pgx_version`/`pgx_version_ok` pair (fail-closed on unparseable) + `assert_pgx_floor` gate cairn-sync on
`cairn_pgx_version() >= 0.2.0` (const `REQUIRED_PGX_FLOOR`) at BOTH startup hooks the issue names: `cmd_init`
(right after `CREATE EXTENSION`, which won't upgrade a stale lib) and `connect_checked` (covers
pull/run/quarantine/requeue); a pre-0.2.0 lib lacks the function entirely → that `UNDEFINED_FUNCTION` error is
translated to the same actionable "rebuild + `cargo pgrx install`" message instead of leaking a raw
"function does not exist". **(3) db/015 `event_twin_provenance`** gains a `verifiable` column
(`cairn_verify(signed_bytes)`, appended last so CREATE OR REPLACE VIEW stays additive): a row whose bytes no
longer verify was silently reported `twin_authored=false` ("author omitted the twin"), so a re-authoring
worklist could clobber genuinely-authored twins — now `verifiable=false` surfaces it as "no longer verifies".
TDD: `crates/cairn-node/tests/verify_error_detail.rs` (2 DB-gated — every door surfaces a non-empty DETAIL;
the view distinguishes an unverifiable row) + 3 pure version-compare unit tests + 1 DB-gated
`assert_pgx_floor_passes_on_the_current_rig`. Full cairn-sync (18) + cairn-node suites + the touched
`db/tests/{005,007,009,020,021}` SQL self-checks + workspace clippy all green on the Mac PG18 / cairn_pgx 0.2.0
rig. Additive only — no SCHEMA-floor bump, no spec/ADR change (implements settled ADR-0040). Closes #109.
Sibling follow-on #111 (node-event-plane quarantine) is the next slice.

**Earlier this session (2026-07-02) — issue #108 + PR #110 review: durable quarantine, re-offer floor, loud
integrity failures in cairn-sync:** closed the clinical-plane pull loop's silent skip-and-advance, then hardened it
against its own review (8 confirmed findings, all fixed in-PR). Before: an unverifiable pulled event left only a
transient stderr line while later verifiable events advanced the watermark past it (silent permanent set-union
exclusion, the A1 class), and a pre-ADR-0040 peer livelocked silently. Now: **(a)** **`db/021_sync_quarantine.sql`**
(explicit REVOKE floor, full lifecycle granted to `cairn_node`) — every refused event is persisted VERBATIM (signed
bytes + travelling attestation pair, enriched by COALESCE if a later re-offer carries a token an earlier one lacked +
legible `EventError` reason) with dedupe-on-re-offer and a **per-peer quota** (10k rows / 64 MiB; at quota the
watermark freezes rather than the pen growing — remote bytes can never fill the clinical node's disk). **(b)** the
**re-offer floor** (`sync_state.quarantine_floor_*`): a durable trace alone is NOT a license to advance past an event
(the review's mixed-batch finding: a re-signed history below a moved watermark would never be refetched) — refusing
an event pins the fetch point below its slot, so every cycle re-offers it (dedupes in the pen) while the watermark
still advances for valid events; a clean cycle from the floor auto-clears it (a repaired/re-signed peer is picked up
with NO operator surgery), and a human can license a permanent exclusion via **`acked = TRUE`** (recorded decision,
releases the floor). **(c)** **ANY unacked refusal fails the pull loudly, every cycle** — not just the all-unverifiable
batch (the old heuristic structurally missed mixed legacy+new batches and already-synced links, exactly where the
livelock lived) — via a typed `PullIntegrityError` that still carries the cycle metrics; `run` logs `integrity: true`
(distinct from `partition`, which bet_a counts as downtime; bet_a `analyze` now reports integrity cycles separately).
Non-hex wire entries are penned like any other garbage (never a whole-pull abort). **(d)** additive
`EventsResponse.signing_context` declared by serve; a DIFFERENT declared context refuses the batch up front naming
both strings (per-event verify binds CTX_EVENT cryptographically anyway — the gate only adds legibility).
**(e)** CLI: `quarantine` (JSON lines incl. `last_requeue_error`/`acked`) + `requeue` (through the REAL
`apply_remote_event` door, streamed row-by-row; the door's refusal goes to `last_requeue_error`, NEVER overwriting the
verify-time `reason` forensics); `pull`/`run`/`quarantine`/`requeue` fail fast with a legible "run `cairn-sync init`"
if the DB predates db/021 (only init applies migrations). TDD: 11 DB-gated integration tests in
`cairn-sync::quarantine_tests` (mixed batch pens+pins floor+recovers on repaired peer · acked releases floor ·
all-unverifiable loud · synced-link unverifiable tail loud · non-hex penned · row quota freezes · byte quota refuses
an overshooting frame · floor SURVIVES a pen-failure re-offer cycle (fresh-eyes catch: clearing it would permanently
release the slot) · skew refused · requeue preserves reason · connect_checked legible) + wire-additivity unit +
`db/tests/021_sync_quarantine_test.sql`. Full
workspace (34 suites) + clippy green on Mac PG18 rig (cairn_pgx rebuilt post-ADR-0040 — stale-dylib gotcha again);
live two-DB smoke green. Closes #108. Split out: #101 (EventsAfter pagination — floor re-ships the suffix wholesale
until then), #111 (node-event plane still lacks a quarantine; db.rs comment points at it).

**Earlier this session (2026-07-02) — issue #95: signing-context domain separation (ADR-0040, spec v0.40→v0.41):**
closed the review-B4 wire-freeze pair before more signatures accumulate. **(a)** The Sign1-vs-multi-signer
ambiguity in ADR-0015's rationale is resolved: an event carries **exactly one envelope signature** (the
authoring actor's); plurality is contributor-set-in-body + attestation tokens + overlay co-signing (ADR-0007 §6)
— COSE_Sign dismissed. **(b)** Every signature is now **domain-separated by a registered signing context**,
bound twice: the context string as COSE protected-header content type (self-describing wire, legible
`ContextMismatch` rejection — principle 11) AND as `external_aad` in the Sig_structure (cross-context
verification fails in the signature math, not just a policy check). Closed additive-only registry:
`application/cairn-event+cbor` (clinical AND node planes — both sign an `EventBody`; plane separation stays
`event_type` + fail-closed classification inside the payload), `application/cairn-attestation+cbor`,
`application/cairn-pairing+cbor`. **Fail closed, no grandfathering:** pre-ADR-0040 uncontextualized blobs are
rejected everywhere (no production data; dev federations/rigs re-init + re-sign — deliberate, recorded in the
ADR). Implementation: one generic `cose_sign1_in_context`/`cose_verify1_in_context` pair in `cairn-event` that
every public sign/verify function delegates to (the triplicated COSE code collapsed; forgetting the context is
now impossible by construction); `cairn_pgx`/DB doors inherit unchanged (they already delegate). TDD: 6 new
red-first tests incl. a demonstration that the pre-fix code DID verify a legacy-signed attestation-shaped
payload as an attestation (the live vulnerability), aad-is-load-bearing (lying header rejected), legacy
fail-closed, wire self-description, and legible cross-context `ContextMismatch`. cairn-event 48/48; workspace +
clippy green. **Operational note:** the local rig's `cairn_pgx` must be rebuilt (`cargo pgrx install`) and any
dev federation re-initialized — old signatures now fail closed (intended).
**PR-review hardening (same PR, post-review):** the verify core now also (a) rejects a protected `alg` header
missing or ≠ EdDSA (`AlgMismatch` — the "header lies about its algorithm" twin of the context check: a genuine
signer could otherwise freeze bytes misdescribing their own primitive, and the re-attestation ladder keys off
this header); (b) rejects any content type in the UNPROTECTED bucket (outside Sig_structure ⇒ an
attacker-appendable second self-description); (c) parses each blob once (`cose_verify1_parsed` /
`cose_verify1_self_described`; the `key_id()`-then-reparse stitch is gone); (d) pins the three wire-frozen
`CTX_*` literals + pairwise distinctness in tests (expectations re-typed, not derived from the consts). Honest
degradation at the seams: restore now distinguishes `NoVerifiableGenesis {unverifiable, total}` (corrupt or
pre-ADR-0040 medium) from a genuinely missing genesis (`medium::scan_enrolls` counts verify failures instead
of silently dropping them); `cairn_pgx` bumped to **0.2.0** (the wire-format break is version-visible) with
new `cairn_verify_error(bytea)` (the legible rejection reason at the SQL boundary) and `cairn_pgx_version()`
(detect a stale .so at daemon startup / mid-outage). cairn-event 52/52, cairn-node 82/82, clippy clean.

**Earlier this session (2026-07-02) — issue #99 (part): the contamination-cascade recall key (review A10):**
fixed the recall-epoch resolution bug + the related deferred floor items on the recall surface, TDD against a
local PG16+cairn_pgx rig. **The bug:** `events_by_actor_epoch` joined `actor_current`, so the moment a
supersede/re-enroll bumped a key's `skill_epoch`, a recall of the OLD epoch silently returned nothing — a
production ADR-0011/0029/0030 contamination cascade would under-select (the dangerous direction). **The fix,
in three layers:** (1) an additive node-local `event_log.actor_id` attribution stamp (db/001) written by BOTH
doors — `submit_event` stamps the unique current key→actor resolution (NULL when the key is concurrently
registered to several actors: honest unknown, principle 4); `apply_remote_event` resolves the stamp against the
key's ENTIRE local registry history (unique only if the key has only ever meant one actor on this node) because
a replicated event's authoring-time epoch is unknowable from the wire (only `signer_key_id` is signed — the
ADR-0029 refinement that would carry actor_id in the signed bytes stays future work); (2) `events_by_actor_epoch`
(db/006) now resolves (key, epoch) against historical `actor_event` rows and returns an `attribution` column —
`'pinned'` (exact stamp match) or `'unattributed'` (NULL-stamped rows, over-selected into EVERY epoch the key
ever registered: a recall over-selects, never silently misses; an unregistered epoch selects nothing); (3) the
deferred neighbours — `actor_event.seq` identity column + deterministic `(recorded_at, seq)` tiebreak in
`actor_current` (same-microsecond registry rows no longer nondeterministic); an FK on
`recall_overlay.target_event_id` (a fat-fingered recall now fails loud instead of "succeeding" on nothing;
recall_overlay is node-local so no out-of-order-sync concern today); explicit REVOKEs on
`recall_overlay`/`recall_event` (the A6 explicit-floor pattern). **PR #106 review hardening (applied
in-branch):** the review caught a residual under-selection in the same family — an origin-side epoch bump
replicating in BEFORE this node registers the new epoch gets confidently mis-stamped to the old (locally
unique) actor, and a later recall of the new epoch would exclude it. Fixed (review-pushed guard commit,
adopted + verified green on the rig) with a third attribution rung `'pre-registration'`: a pinned stamp
excludes an event from a queried epoch ONLY if that epoch was already locally registered when the event was
admitted; older events over-select (noise bounded to events predating the epoch's first local registration —
exactly the set the node cannot attribute; the ADR-0029 refinement carrying actor identity in the signed
bytes retires the rung). Also `actor_event.seq` hardened `BY DEFAULT`→`GENERATED ALWAYS` (+ idempotent
`SET GENERATED ALWAYS` self-heal), so forging the trust-anchor tiebreak needs a loud
`OVERRIDING SYSTEM VALUE`; and the supersede-row integrity caveat (no supersede door yet ⇒
`actor_id = cairn_actor_id(pinned)` unenforced for hand-inserted supersede rows; safe direction) is
documented in db/006. Tests: `crates/cairn-node/tests/recall_epoch.rs`
(6 — superseded-epoch exact recall through both doors · ambiguous-key NULL stamp over-selects · late-arriving
remote event never misattributed after a local epoch bump · registry-lag/late-registered epoch never silently
buried · FK fail-loud + legitimate recall lands) + extended
`db/tests/004`/`006` SQL floor tests (tiebreak, history resolution, registry-lag rung, FK). All additive DDL, no SCHEMA-array change,
no spec/ADR bump (implements settled ADR-0011/0029/0030). **Still open in #99 (deliberately untouched):** the
suppression owner-gate (db/005 step 5 `DEFERRED`) — *who* may suppress *whose* event is an ADR-level decision.

**Earlier this session (2026-07-02) — issue #91: the clinical-plane in-DB apply door (review A2/A5b/M8/H4):**
built **`db/020_apply_remote_event.sql`** — `apply_remote_event`, the sibling of `apply_remote_node_event`, closing
the review's highest-priority structural finding: the cairn-sync apply path no longer raw-INSERTs with owner
privileges; a replicated clinical event now faces the SAME in-DB floor as a locally-authored one (signature,
enrollment, fail-closed classification, the attestation gate on suppressing events, the demographic/identity
hard-twin rule, the t_effective ceiling, size ceiling, substitution guard — shared helpers, so the doors cannot
drift). Replication-appropriate deltas, each reasoned in the file header: idempotent set-union no-op; in-door HLC
merge; **attestation tokens now STORED** (`db/001` additive `attestation`/`attester_key` columns; also closes the
M7 residual "verified then discarded") **and shipped on the sync wire** (additive parallel arrays in
`EventsResponse`) so the suppress gate is re-runnable at every hop; **t_effective wire-pinned** to an explicit UTC
offset via the single `cairn_t_effective` validator (db/001, both doors + author-side CLI check — H4); **node-local
projection guards clamp-and-flag at apply instead of vetoing** (db/018 component cap → `identity_projection_flag`
worklist under the transaction-local `cairn.remote_apply` marker; local authoring keeps its fail-loud veto — A5b);
the M8 twin triple-implementation collapses (apply-path Rust fallback deleted; one in-DB skeleton renderer at both
doors; Rust `plaintext_twin` remains only as the authoring renderer). `cairn-sync` gained an **`enroll`** subcommand
(operator ceremony) and the bet_a harness enrolls every authoring key on every node — **known residual:** the actor
registry does not replicate yet (ADR-0011 future work), so an event from a not-yet-enrolled signer freezes the
puller's watermark (A1 discipline: delayed, never lost) until enrollment. Tests: 15 new DB-gated integration tests
(`crates/cairn-node/tests/apply_remote_event.rs`), `db/tests/020` grant-floor SQL tests, wire-compat + offset-pin
unit tests; end-to-end two-DB converge + unenrolled-freeze + enroll-heal exercised live. cairn-node SCHEMA array
18→19 entries (020 added); no spec/ADR bump (implements settled ADR-0021/0022/0030). The t_effective
explicit-offset wire pin is recorded in spec §3.6 prose (one line on the `t_effective` bullet, refining
ADR-0015 — the ADR itself stays untouched per immutability).

**Earlier this session (2026-07-02) — comprehensive review + hardening pass:** an adversarial full-repo review (7 parallel
agents over the SQL floor, the Rust crates, the Python matcher, and all 39 ADRs; findings cross-checked and re-verified
against the code). Report + full disposition table: **`docs/code_reviews/2026-07-02-comprehensive-review.md`**. The
foundations held up well (crypto, grant floor, sign-the-bytes canonicalization, ADR honesty); trouble clustered in the
clinical sync door, a few settled-ADR promises the code didn't keep, and cheap-now/expensive-later wire decisions.
**Fixed this session (all with tests where a harness exists; full workspace + 186 matcher tests + clippy green):**
*floor* — `t_effective ≤ t_recorded` ceiling now enforced in `submit_event` (A3); `patient_identifier` made
HLC-convergent (was first-apply-wins → cross-node divergence feeding the veto, A4); explicit REVOKEs on
`actor_event`/`enroll_actor` + negative test (A6); linkage-recompute advisory lock (A5a); a shared
`cairn_max_event_bytes()` ceiling at every admission door (A7a); total-order winner tiebreaks on
names/address/demographic (A10; text-collation caveat tracked in #69); attestation `attester_key_id` binding (M7).
*daemon* — cairn-sync watermark now advances only over the contiguous applied prefix so a transient failure can no
longer silently drop a clinical event (A1); event_id-substitution guard on the sync apply path (H3); pull/serve I/O
timeouts + serve session cap so a stalled peer can't freeze trust refresh (A7b); `LocalState` `deny_unknown_fields` +
version gate so a newer bundle fails loud instead of dropping content (A7c); SPKI Ed25519 OID pin; 0600 signing-key
perms (L12). *matcher* — subset-name → PARTIAL (the ADR-0014 cultural-bias footgun, A9a); veto+shared-identifier now
forces REVIEW instead of silent burial (A9b); NFC normalization (A9c); 4-digit-year DOB gate (A9e); `unknown`
sentinel → absence (A9f). *tests* — cairn_pgx `#[pg_test]` fixtures compile again (A8; `cargo check --features pg_test`
clean). **Filed as issues (design / large-refactor), #91–#103:** clinical sync in-DB apply door (#91), erasure-ladder
composition (#92), revocation-clock backdating (#93), human key custody (#94), COSE domain separation (#95), closed
role-enum wire encoding (#96), demographic recency time-axis (#97), essential-tier vs ADR-0001 (#98), suppression
owner-gate + recall epoch (#99), matcher recall-key (#100), sync pagination + blob wedge (#101), operational-hardening
batch (#102), clinical-safety prose batch (#103). Matcher minors already tracked by #79/#84. No spec/ADR bump, no
SCHEMA-floor version bump (all additive DDL + additive Rust).

**Earlier the same day (2026-07-02):** built matcher/identity piece **C2 — the §5.2/§5.7 match_proposal→apply seam**
(brainstorm→spec→plan→subagent-SDD, 5 tasks; spec+plan under `docs/superpowers/`). A **human-accepted** advisory
`match_proposal` (B2 output, `db/017`) becomes a real **human-attested** `identity.link.asserted` event through the
C1 door, projecting into `patient_link`/`person_member`. **Human-accepted only** (auto-apply of the `auto_candidate`
band deferred to C2b); the accepting reviewer is a **responsibility-bearing (attested) contributor**. **Key property:
no floor change** — the link is additive, but placing a responsibility-bearing contributor trips the **existing**
db/005 attestation gate (valid human token, bound to the event, enrolled-human attester), so C2 composes settled §5.7
(C1) + ADR-0030 (attestation) + ADR-0014 **verbatim**; `submit_event` untouched, no new event type, no spec/ADR bump.
**`db/019_apply_proposal.sql`** (additive `applied_event_id UUID` column on `match_proposal`; wired into `db.rs`
SCHEMA array 17→18, no floor version bump). **`crates/cairn-node/src/apply_proposal.rs`**: pure `compose_provenance`
+ `build_attested_link_body` (event_id caller-supplied ⇒ deterministic/testable; responsibility-bearing contributor;
authored twin) + IO `apply_accepted_proposal` — read accepted proposal → `sign`+`sign_attestation` with the human key
→ 3-arg `submit_event` → mark `status='applied'`+`applied_event_id`, all in **one `client.transaction()`** so
**atomicity is the idempotency guarantee** (any rejection rolls back ⇒ no event, proposal stays `'accepted'` for
retry). Provenance = `matcher:{version} accepted-by:{kid}`; confidence = `{score:.3}`. **Tests:**
`crates/cairn-node/tests/apply_proposal.rs` — 6 (3 pure unit + DB-gated happy-path [link event appended · edge ·
both patients project to min-UUID person · proposal applied→event_id] + idempotency [one event on re-apply] +
**non-human-attester refused** [floor rejects `not an enrolled human actor`, nothing leaks, stays accepted] +
pending-not-applied + reverse-order-pair-still-applies); full cairn-node suite green; clippy `--tests` clean.
**PR-review hardening (applied in-branch):** the accepted-proposal read is now `SELECT … FOR UPDATE` so concurrent
applies of the same pair serialize (the loser sees `'applied'` and bails, instead of both appending a link event
under READ COMMITTED); and `apply_accepted_proposal` canonicalizes its `(low, high)` args to `(least, greatest)`, so
a caller supplying the pair reversed still finds the accepted proposal rather than silently missing it. **Deferred
(recorded, next): C2b** = auto-apply of the `auto_candidate` band; the **matcher as a compositional contributor**
(principle 10 — needs the §7.5 matcher-actor registration; lives in the provenance string for now); a **CLI
subcommand** + production human-key custody (ADR-0011); **C3+** = the rest of the §5.7 algebra
(identify/repudiate/dispute/reattribute). Test command: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb
dbname=cairn_test" cargo test --test apply_proposal` (PG18 + `cairn_pgx`). **The §5.2/§5.7 C2 apply seam is now BUILT.**

**Earlier this session (2026-07-02):** built matcher piece **C1 — the §5.1/§5.7 identity linkage core** (the C2 seam's
destination; full detail in **ROADMAP slice 13** + git). Pure `cairn-event::identity` `LinkAssertion` builder; additive
**`db/018_identity_linkage.sql`** (SCHEMA 16→17, no floor bump): two additive `identity.link/unlink.asserted` types
through the **reused** `submit_event` door, `cairn_check_link_assertion` culture-neutral structural floor + HARD
authored-twin, `patient_link` HLC-overlay edge table (latest-HLC-wins, out-of-order convergent), `person_member`
golden-identity projection (`person_id` = min-UUID of the connected component via `cairn_recompute_component` — correct
on merge **and** unmerge/split, fail-loud oversize guard), `person_chart` thin union VIEW. 15 DB-gated tests; suite +
clippy green. **Principle 2 made real:** never merge/always link; unmerge is always clean. No ADR/spec bump (settled
§5.1/§5.7/ADR-0014). Deferred (Minor): an accept-at-cap boundary test for the oversize guard.
**The §5.1/§5.7 identity linkage core (piece C1) is now BUILT.**

**Prior session (2026-07-01):** built the **§5.2 matcher B3 synthetic blocking-eval volume generator**
(brainstorm→spec→plan→subagent-SDD, 6 TDD tasks; spec+plan under `docs/superpowers/`) in `matcher/`. Pure,
stdlib-only **`eval/generator.py`** (no psycopg, 274 lines): `shares_blocking_key` mirrors the three base blocking
passes; four pure corruption operators (`corrupt_dob_format`, `corrupt_dob_typo`, `corrupt_name`,
`corrupt_identifier`); culture-plural curated name pools + `synth_seed`; `GenSpec` + `generate_dataset(spec)` builds
seed+one-corrupted-clone entity clusters (cluster size fixed at 2 — one true pair per entity) with a `_repair` step
that **guarantees** every seed↔clone pair stays recoverable by ≥1 base blocking key (appends the seed's primary name
if corruptions destroyed all keys). Deterministic (seeded PRNG). **`eval/generate.py`** is the disk/CLI edge:
`write_dataset` + `python -m cairn_matcher.eval.generate --entities N --seed S [--out path]`, byte-deterministic JSON
(`sort_keys=True`), feeding the existing `python -m cairn_matcher.eval <file>` unchanged. **Advisory tooling — no
`db/` floor, no SCHEMA bump, no spec/ADR change** (implements settled §5.2/§5.13/ADR-0014); no new dep. Tests: pure
suite **147 passed / 29 skipped** (`uv run pytest`); DB suite **173 passed**. A **drift canary**
(`test_eval_generator_sync.py`) pins `shares_blocking_key`'s mirrored assumptions to `pipeline/db.py`'s `_GROUPS_SQL`,
so narrowing a base blocking pass trips the fast suite instead of silently voiding the recoverability guarantee
(review-fix on PR #88). New DB-gated volume test: on a generated
200-entity set at `max_block_size=10_000`, `pair_completeness==1.0`, 0 dropped true matches, `reduction_ratio≈0.919`
(6,467/79,800 pairs) — the recoverability invariant confirmed end-to-end through the real blocking SQL. This is a
regression/volume instrument, not a statistical accuracy claim: the generated set is recoverable **by construction**,
not by resemblance to real-world data. **Deferred (recorded, not lost):** variable cluster size (>2 records/entity);
a deliberately unrecoverable fraction (models the hub-sweep floor); hard negatives / scorer-precision curves; an
A/B pass-toggle in `generate_candidate_pairs` for one-command before/after (today it's git-revert) — this last one
is what would unblock a *quantitative* compound-key before/after using this generator; still deferred.
**The §5.2 matcher B3 synthetic volume generator is now BUILT.**

**Prior session (2026-07-01):** built the **§5.2 matcher B3 compound blocking key — name-token + birth-year**
(brainstorm→spec→plan→subagent-SDD; spec+plan under `docs/superpowers/`). One **additive** `UNION ALL` branch in
`pipeline/db.py`'s `_GROUPS_SQL` (a `birth_year` CTE + a `name+year` pass): it partitions an over-broad single-name-token
block by birth-year so the sub-blocks survive the oversized-block cap, recovering true-match pairs the cap would drop
wholesale. Additive ⇒ **recall non-decreasing** (pairs deduped by canonical uuid pair across passes); also rescues
**precision-mismatched** DOBs (`"1990"` vs `"1990-05-12"` — same first 4-digit run groups them, exact-DOB never does).
Birth-year is an **honest, culture-neutral degrade** (principle 4): the **first 4-consecutive-digit run**
(`substring(value FROM '[0-9]{4}')`, guarded by `value ~ '[0-9]{4}'`) — no date parsing, no calendar; an ISO value and a
day-first import (`"12/05/1990"`) of the same person both yield `1990` and group, while a DOB with no 4-digit run stays
covered by the single-token pass. (Originally a leading-only `left(value,4)`/`^[0-9]{4}` guard; widened to the 4-digit-run
form 2026-07-01 so cross-format imports group — advisory, so a mis-extracted year only ever feeds the scorer extra pairs it
rejects, never a false link.) **Advisory — no `db/` floor, no SCHEMA bump, no spec/ADR change**
(implements settled §5.2/§5.13/ADR-0014); no new dep; `db.py` 166 lines. Tests: 5 new DB-gated integration tests (rescue,
honest-degrade, precision-mismatch, cross-format, cross-pass dedup); full matcher suite **151 with DB / 123 + 28 skipped** without.
Harness sanity check on a clean DB: `pair_completeness=1.000`, `reduction_ratio=0.911`, 0 dropped true matches on `gold_v1`
(additivity confirmed). Per-task opus/sonnet reviews clean (spec ✅). **Known limitation** (user-flagged): year extraction
still degrades on 2-digit years and non-Gregorian calendars — revisit on richer/real data (safe degrade, not a false group).
Discovered + filed **[issue #84](https://github.com/cairn-ehr/cairn-ehr/issues/84)** (pre-existing: integration tests
commit-leak rows via `seed_patient`; `evaluate_blocking` `KeyError`-crashes on a dirty DB — out of this slice's scope).
**The §5.2 compound-blocking-keys item is now BUILT.**

**Prior session (2026-06-30):** built the **§5.2 matcher eval harness (piece B3 keystone)** — a labelled-dataset
measurement substrate to unblock the measurement-driven B3 items (compound blocking keys, weight-learning), via
**brainstorm→spec→plan→subagent-SDD** (8 TDD tasks + final review; spec+plan under `docs/superpowers/`). New
pure-by-default **`matcher/src/cairn_matcher/eval/`** sub-package mirroring the `pipeline/` pure-core + optional-DB split:
**`dataset.py`** (entity-cluster JSON format + loader; `record_to_candidate` *reuses the real `candidate_from_rows`
adapter* — no drift; `truth_pairs`/`all_pairs` ground truth), **`metrics.py`** (confusion + precision/recall/F1 at
strict+lenient operating points + auto-false-link-rate + missed-match-rate + score separation; zero-denominator → 0.0,
never NaN), **`scorer_eval.py`** (`evaluate_scorer` runs the *real* `field_comparisons→score→band` path; `weights`/
`thresholds`/`config` are params — the weight-learning lever; banding with no veto = documented pure-eval simplification),
**`report.py`** (plain-text report incl. the honest "regression/tuning instrument, not a statistical accuracy claim"
caveat), **`__main__.py`** (`python -m cairn_matcher.eval` — scorer always; blocking when `CAIRN_TEST_PG`, psycopg
**lazy-imported** so the pure path never touches it), **`blocking_eval.py`** (DB-gated, `pipeline` extra: seeds `patient_*`
label→uuid5, calls the *real* `generate_candidate_pairs`, `conn.rollback()` xmin-guard, computes pair-completeness /
reduction-ratio / dropped-true-matches / Σ`C(size,2)` dropped-pair estimate), + a hand-authored **culture-plural gold
fixture** (`eval/fixtures/gold_v1.json`: mononym / patronymic+diacritic / multi-token). **Advisory only — no `db/` floor
file, no SCHEMA bump, no spec/ADR change** (implements settled §5.2/§5.13/ADR-0014). **No new dep** (pure core stdlib-only;
blocking under existing `pipeline`/psycopg extra). Tests: **146 with DB** / **123 + 23 skipped** without (`uv run pytest`);
purity probe confirms the pure surface (incl. `__main__`) imports no psycopg. Final **opus whole-branch review:
READY-TO-MERGE, 0 Critical / 0 Important** (real-path reuse/no-drift, purity, advisory-only, metric math all verified;
two cosmetic Minors applied in-branch). **Post-review fixes (in-branch, PR #83):** `blocking_eval.seed_dataset` no longer
`conn.commit()`s — the seed now lives in the read txn that `evaluate_blocking`'s `rollback()` discards, so the DB-gated
eval is **idempotent + leaves no synthetic patients** (a committed seed would re-hit `patient_demographic`'s
`PK(patient_id, field)` on a second run, since `uuid5` labels are deterministic); the dataset loader now validates
name/identifier inner keys → a located `DatasetError` instead of an opaque downstream `KeyError`. **Deferred (recorded, not lost):** the **synthetic corruption generator** (volume +
recall curves, same format); the **compound-blocking-keys** + **weight-learning** slices themselves (this harness *measures*
them); a **veto-aware / end-to-end scorer mode**. **The §5.2 matcher eval harness (B3 keystone) is now BUILT.**

**Prior session (2026-06-30):** built the **§5.2 advisory matcher — piece B2b** (blocking / candidate-pair generation +
batch sweep): read-only `db.generate_candidate_pairs(conn, *, max_block_size=100)` — a 3-pass blocking disjunction
(shared identifier excl. `unknown` · exact-DOB · shared name token), group CTEs deduped to canonical `(low,high)` by uuid
VALUE order, oversized-block guard → `skipped_blocks` (never a silent cap) — + `pipeline/sweep.py` (`sweep()` two-phase
batch driver: generate→`rollback` xmin-guard, then loop `runner.propose()` per pair with skip-and-report errors). Advisory;
no `db/` floor, no SCHEMA bump. 113 with DB / 93+20 skipped without. Opus review READY-TO-MERGE 0C/0I. Merged **[PR #81](https://github.com/cairn-ehr/cairn-ehr/pull/81)**. **B2b BUILT.**

**Prior session (2026-06-29):** built the **§5.2 advisory matcher pipeline — piece B2** (veto-gated **pairwise**
pipeline + advisory proposal worklist) in a new IO-bearing sub-package **`cairn_matcher/pipeline/`** beside B1's pure
core: pure `adapter.py` (`patient_*` rows → `CandidateRecord`; ISO DOB, `sorted()` token-bag names, `match_key`
identifiers, safe-degrade) + pure `banding.py` (`MatchScore`+veto → `auto_candidate` iff `≥T_auto` **and no veto**,
else `review`, else `None`; any veto caps at `review`, never auto-link/auto-reject) + `db.py`/`runner.py` (the only
psycopg modules; `propose` = load→score→veto→band→upsert→commit, commit owned by runner). New **`db/017_match_proposal.sql`**
(SCHEMA 15→16): an **advisory** worklist table (human `status` preserved on re-run) — *not a safety gate*. **psycopg**
optional (`pipeline` extra). 92 with DB / 87+5 skipped without. Opus review MERGE-READY 0C/0I. Non-blocking Minors →
**[issue #79](https://github.com/cairn-ehr/cairn-ehr/issues/79)**. **B2 BUILT.**

**Prior sessions (2026-06-28/29) — §5.2 matcher pieces A + B1 (condensed; full detail in ROADMAP slices 6–7 + git):**
**piece A** = the **§4.4/§5.2 in-DB hard-veto floor** (`db/016_match_veto.sql`, SCHEMA 14→15; `cairn_match_veto` returns
the closed hard-veto set — same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash; two
verdicts `hard_veto`/`degrade_hold`; precision-gated, parses no dates; `system:unknown` never vetoes; forces a human
decision, never auto-link/auto-reject; 12 integration tests; deceased-status veto deferred, stub in db/016). **piece B1**
= the **§5.2/§5.13 advisory scoring core** (new `matcher/` uv project, `cairn-matcher`, AGPL-3.0, **zero runtime deps,
pure functions only** — the fit-for-purpose §9 tier): the `Comparator`/ordinal `AgreementLevel` contract (`PHONETIC`/`NICKNAME`
reserved but never emitted by core — anti-cultural-capture), in-house **Jaro–Winkler** + 4 culture-neutral comparators
(`compare_exact`/`compare_edit_distance`/`compare_dob` [parses no date strings]/`compare_name_set`) + positive-only
`compare_identifier_sets` (never DISAGREE) + the field→comparator registry + the **Fellegi–Sunter** combiner producing an
explainable `MatchScore`; 55 pure tests; final review caught + fixed one Critical (`score(a,b)≠score(b,a)` from greedy
name-pairing → now `max(greedy(a,b),greedy(b,a))`, symmetric). No new ADR, no spec bump (both implement settled
§5.2/§5.13/§4.4; refine ADR-0014/0033).

**Prior session (2026-06-28):** **globalised the §3.13/§4.5 author-materialised legibility twin to every event type**
(ADR-0039; spec v0.39 → v0.40), via brainstorm→spec→plan→subagent-SDD (5 tasks, spec+plan under `docs/superpowers/`).
The in-DB floor (`db/015_globalise_twin.sql`, SCHEMA 13→14) now PREFERS the authored twin for every type; non-demographic
types **degrade honestly** to a flagged, payload-rendering derived skeleton when absent (older/non-conformant peer) —
set-union convergence preserved; the two demographic types KEEP ADR-0034's HARD authored-twin requirement. Authored-vs-derived
is **NOT stored** — it is derivable from the immutable signed body via `cairn_twin_is_authored(bytea)` + the
`event_twin_provenance` view; **no new column, `submit_event` NOT re-declared** (only the `cairn_event_twin` hook changed).
Improved `cairn_twin_skeleton` now renders the payload — **closes the `db/005:29` TODO**. `cairn-event` gained pure
`resolve_twin` + `materialise_generic_twin` (the single rule both cairn-sync and the SQL floor follow); `cairn-sync` now
carries the authored twin on apply and materialises it on authoring. Tests: cairn-event 3 unit (36/36 suite green); cairn-node
4 integration (`twin_globalise` — authored verbatim+flag; twin-less degrade+flag+payload; twin-less demographic hard-reject
triple-gated; + a whitespace-twin demographic hard-reject); demographics + attestation regress green; clippy clean. A
**floor bug** surfaced by the whitespace hardening test was fixed in the same branch: PG `trim()` strips only ASCII space
(not `\n`/`\t`), so the blank-test used `length(regexp_replace(x,'\s+','','g'))>0` in **both** the write gate (`v_authored`)
and read predicate (`cairn_twin_is_authored`), realigning them with Rust `str::trim()`. Residual Unicode-whitespace
asymmetry (PG `\s` ⊂ Rust `char::is_whitespace`; degrades safe) tracked as [issue #75](https://github.com/cairn-ehr/cairn-ehr/issues/75).
**The "globalise the authored twin" deferral is now CLOSED.**

**Prior sessions (2026-06-27/28) — demographics slices 1–5, condensed (full detail in ROADMAP slices 1–5 + git):**
**slice 1** = §4.4 patient-identifier assertion end-to-end (`db/010`, `EventBody.plaintext_twin`, `cairn_event_twin`
hook, set-union `patient_identifier` projection; [issue #67](https://github.com/cairn-ehr/cairn-ehr/issues/67));
**slice 2** = §4.2 DOB + sex-at-birth provenance-locked fields (`db/011`, generic `demographic.field.asserted` +
`cairn_provenance_rank` ladder incl. new `fact-proven` top tier; floor open / projection gated — the ADR-0012
federation-forward call; [issue #69](https://github.com/cairn-ehr/cairn-ehr/issues/69)); **slice 3** = §4.2 names
(`db/012`, `patient_name` retained-set + `patient_name_current` recency-first-within-legal-tier display VIEW,
[ADR-0036](spec/decisions/0036-demographic-name-display-recency-first.md); PR #71+#72); **slice 4** = administrative-sex
+ gender-identity (`db/013`, one `cairn_demographic_field_policy(field)` classifier driving both projection gate and
winner ordering — sex provenance-first, gender-identity recency-first; karyotype resolved as a distinct field,
[ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md); PR #73); **slice 5** =
§4.3 address (`db/014`, per-use recency-first `patient_address_current` VIEW, same logic as names,
[ADR-0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md)). Also closed demographics **gap B**
(provider-number person×org relational model, [ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md),
§4.6: entity/relationship + subject-kind partitioning, design/spec only) and representation gaps B+C
([ADR-0032](spec/decisions/0032-culture-neutral-address-representation.md) address,
[ADR-0033](spec/decisions/0033-patient-identifier-representation.md) identifier namespace/profile split,
[ADR-0034](spec/decisions/0034-demographic-legibility-twin.md) legibility twin). Spec 0.32→0.39 across this run.
**Demographics slices 1–5 + gaps A/B/C all done; §4.2/§4.3/§4.4/§4.5/§4.6 complete.**

**Prior sessions (2026-06-25/26)** — ADR-0026 node durability slices B/C/D closed (backup-as-cold-peer, restore +
`supersede`, sealed local-state export) + issues #53/#54 (cold-medium self-identification, uniform key zeroization)
+ **Spike 0003 (Postgres on Android) G0–G3 PASS**. Full detail: ROADMAP Phase 5/6 + git + the ADR-0026 log.

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

**Honest gaps / follow-ons declared in the node (candidate "harden the node" work):**
- ~~`status` **crashes if run before `init`**~~ **closed 2026-06-23** — `load_local_opt` (`query_opt`) +
  an `initialized` flag; `status` degrades honestly with a "run `cairn-node init`" hint
  (`tests/status.rs::status_before_init_degrades_gracefully`).
- **In-DB floor caveat** — ~~runtime should connect as a login role granted `cairn_node` (NOLOGIN)~~
  **closed 2026-06-23**: `db::provision_runtime_role` (charset-guarded against DDL injection) + a
  `provision-runtime-role` CLI subcommand create that role, and `tests/floor_enforced.rs` now **proves the
  ENFORCED path** — over a `cairn_node`-granted login role a raw `INSERT` into `node_event` is denied
  (SQLSTATE 42501), `status` reports `db_floor ENFORCED`, yet `submit_node_event` still works.
- ~~**Key-at-rest plaintext-0600**; **DR/recovery escrow a named stub** (`dr_escrow: STUBBED`)~~ **closed
  2026-06-24** (ADR-0026 **slice A**, [PR #44](https://github.com/cairn-ehr/cairn-ehr/pull/44)): the signing key is now **sealed at rest** — a random DEK seals the
  seed (XChaCha20-Poly1305), DEK **dual-wrapped** under Argon2id KEKs from an operational passphrase
  **and** a one-time **recovery code** (paper escrow, shown once at `init`). New pure `seal.rs`
  (seal/unseal/CBOR + base32 recovery code); `keystore` gained `generate_sealed`/`generate_plaintext`/
  `seal_existing` + auto-detect `load` + `key_at_rest_state`; CLI seals by default (`--insecure-plaintext`
  escape hatch) and added `seal-key` migration; daemon unseals via `CAIRN_KEY_PASSPHRASE`. `status` now
  reports `key_at_rest SEALED` + `dr_escrow recovery code set` + `recovery_escrow`. **Honest ceiling
  (documented, not engineered away): lose both the passphrase AND the recovery code → node loss.**
- ~~Genesis **HLC 0/0 placeholder**; **full-pull, no incremental watermark**~~ **closed 2026-06-23**
  ([issue #38](https://github.com/cairn-ehr/cairn-ehr/issues/38), **merged [PR #42](https://github.com/cairn-ehr/cairn-ehr/pull/42)**):
  incremental pull keyed on a monotonic local-insertion `node_event.seq` (a node always inserts newly-learned
  events with a fresh high `seq`, so the watermark is **structurally** skip-proof — decoupling it from the HLC,
  which dissolved the stated coupling), per-peer `sync_cursor` written only through an advance-only
  `checkpoint_sync_cursor` `SECURITY DEFINER` door (the runtime role keeps **zero raw DML**), with an explicit
  periodic + trust-change-triggered **full-sweep** as the correctness floor for the residual commit-order /
  rejected-then-trusted / address-remap hazards. The `0/0` HLC is now a real local clock (`hlc_state` +
  `node_hlc_tick()` + merge-forward on apply, mirroring `cairn-sync`). Acceptance test
  `sync_watermark::out_of_order_skip_is_reconciled_by_full_sweep` proves a jammed-cursor skip is reconciled by
  the sweep; the seq prefix is transport-only (signed core byte-identical, principle 12). Full node suite green
  on PG16 + `cairn_pgx`, clippy clean.
- ~~**backup-as-cold-peer** + backup-health (slice B)~~ **export half closed this session**: `backup`/`verify-backup`
  CLI + `last_backup` status line; signed-event medium, self-verifying via the existing signature invariant (tamper
  → non-zero exit); fail-safe node-local health sidecar; **verify-before-write** (the image self-verifies *before* the
  atomic rename, so a bad set never overwrites the previous good medium) plus a read-after-write tripwire gate the
  health update so it never over-claims. New `backup.rs` (pure medium format + verify + health) + shared `fsio`
  atomic-write.
- ~~**Restore (apply) + new-identity `supersede`** (slice C, [#50](https://github.com/cairn-ehr/cairn-ehr/issues/50))~~
  **closed**: `cairn-node restore` + self-trusting `restore_node_event` door (empty-genesis fenced), `supersede`(dead→new),
  fresh-key mint, `status` `supersedes` line. `db/009` + a `supersede` branch in `submit_node_event` (db/007). Residual
  footgun ~~[#53](https://github.com/cairn-ehr/cairn-ehr/issues/53) (a federated medium's `--superseded-node` could name a
  peer)~~ **closed this session** via the container-level self-marker (`medium.rs`, `CAIRNB2`; signed+medium-bound or
  unsigned) — see top.
- ~~**Sealed local-state export** (slice D, ADR-0026 point 3)~~ **closed this session**: `localstate.rs` (LSK dual-wrap,
  `CAIRNL1`/`CAIRNX1` containers, additive `LocalState` with empty slots, DB seams); `.lsk` at provisioning;
  `establish-local-state-key`; `backup` writes / `restore` consumes the export; `status` `local_state` line. **All ADR-0026
  slices (A–D) now done.** Remaining escrow *rungs* (Shamir M-of-N, QR, TPM/keyring) are optional upward options, not blockers.
- ~~atomic key-file write ([issue #45](https://github.com/cairn-ehr/cairn-ehr/issues/45)); passphrase
  `zeroize`-on-drop ([issue #46](https://github.com/cairn-ehr/cairn-ehr/issues/46))~~ **closed 2026-06-25**:
  `write_key_file` is now atomic (temp sibling → fsync → `rename` → **parent-dir fsync**, 0600 forced
  explicitly), so an interrupted `init`/`seal-key` can never leave a half-written key that boots `Corrupt`,
  the rename itself survives a power loss (not just the bytes), and a stale wide-perm `<key>.tmp` can no longer
  leak its mode onto the key; the operational passphrase and recovery code are held as `Zeroizing<String>`
  from `resolve_passphrase`/prompt through to the Argon2 call, wiped on drop (`zeroize` was already a transitive
  dep — no new crate). TDD: red-first tests for the new `tmp_sibling` helper, no-temp-litter, stale-temp clobber,
  0600 perms, stale-wide-perm-temp non-leak, and the `Zeroizing` return type. (PR #49 review: + dir fsync,
  explicit 0600, non-unix fsync.)
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
  by construction) are now BUILT. It unblocks measuring blocking recall/reduction at volume (confirmed:
  `pair_completeness==1.0` on a generated 200-entity set) — **but not yet** a quantitative before/after across a
  compound-key change, which needs the still-deferred A/B pass-toggle below. **Next (B3 measurement-driven):**
  **weight-learning** (sweep `evaluate_scorer`'s `weights`/`thresholds` against the gold set) + **further compound
  keys** (`dob+first-initial`, `name+sex`) + locale comparator packs / hub-tier aggressive duplicate-sweep +
  proposal retraction / full §7.5 matcher actor registration. **Identity: pieces C1** (§5.1/§5.7 linkage core —
  `db/018`, `cairn-event::identity`, `patient_link`/`person_member`/`person_chart`) **and C2** (the
  `match_proposal`→apply seam — `db/019`, `cairn-node::apply_proposal`; human-accepted proposal → human-attested link
  event) **are now BUILT** (this session). **Next identity slices: C2b** — auto-apply of the `auto_candidate` band
  (matcher-authored, un-attested, recallable link) — and **C3+** — the rest of the §5.7 algebra
  (identify/repudiate/dispute/reattribute). Deferred: an **A/B pass-toggle**
  in `generate_candidate_pairs` (one command instead of git-revert for compound-key before/after — the piece that
  would make the volume generator's numbers a quantitative comparison); variable cluster size / an unrecoverable
  fraction / hard negatives in the volume generator; a **veto-aware / end-to-end scorer mode**; deceased-status veto
  (stub in db/016); a `compare_address` comparator; a **CLI** sweep entry; the matcher test-leak + harness `KeyError`
  ([issue #84](https://github.com/cairn-ehr/cairn-ehr/issues/84)); B2 follow-up Minors (Thresholds `review<auto` guard,
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
  builds+loads on Pi arm64 (in-DB Rust surface confirmed on ARM). **Open follow-ups:** ~~(a) port `cairn_pgx` to a
  PG-18-capable pgrx~~ **done 2026-06-25 ([PR #56](https://github.com/cairn-ehr/cairn-ehr/pull/56): pgrx 0.12.9 → 0.18.1,
  default feature `pg16`→`pg18`)**; (b) clean re-run on **PG 18 + USB-3 SSD + official 27 W PSU** for authoritative
  precision numbers; (c) fold the B4 number into the ADR-0015 follow-up to drop "provisional" from the blob-digest line.
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

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B prepared); 0002 (advisory-actor — ran, C1–C5 ✓
→ ADR-0029/0030); 0003 (Postgres on Android — **ran 2026-06-25, G0–G3 ✓**; PR #47/#48).
