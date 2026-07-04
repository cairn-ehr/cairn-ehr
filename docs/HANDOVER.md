# HANDOVER — Cairn

**Session date:** 2026-07-04 · **Spec/ADRs:** v0.41 · **Phase:** architecture complete; **first production clinical
surface under construction** — demographics on `cairn-node` (slices 1–5 done) + the §5.2 matcher (piece A in-DB veto
floor · B1 advisory scoring core · B2 veto-gated pairwise pipeline + proposal worklist · B2b blocking / candidate-pair
generation + batch sweep · B3 eval harness · B3 compound blocking key · B3 synthetic volume generator · consumes
`patient_alias_pool` known-alias evidence · **now range-aware, positive-only `compare_dob` for clinician-observed
estimated ages — done this session**) + the
**§5.7 identity core: C1 linkage · C2 human-accepted apply seam · C2b auto-apply of the `auto_candidate` band · C3
`dispute` + the chart trust-state projection · C4 `identify` + the *unconfirmed* trust state · C5 `repudiate` + the
known-alias pool** (the §5.7 confirmed/unconfirmed/under-review contract is COMPLETE) + the
**§5.4 John-Doe registration front door — slice A: callsign minting + matcher placeholder exclusion · slice B:
clinician-observed evidence (estimated-age range + observed sex) — done this session**; remaining B3 weight-learning /
locale packs / A-B pass-toggle + identity **C5+** (`reattribute` — waits on a clinical-note surface) + the **rest of
the §5.4 subsystem** (photo/marks/belongings/EMS evidence, the "prior history now available" push-alert, the
search-before-create funnel, a birth-year-*range* blocking pass) next. Viability proven by spikes (walking skeleton,
advisory-actor contract, a first federating node, Postgres-on-Android).

**This session (2026-07-04) — §5.4 John-Doe slice B: clinician-observed evidence (estimated-age range + observed sex),
full loop** (brainstorm→spec→plan→subagent-SDD, 7 TDD tasks; spec+plan under
`docs/superpowers/{specs,plans}/2026-07-04-john-doe-clinician-observed-evidence*`). A clinician registering an
unidentified patient records honest observed evidence, and the estimate becomes a positive matcher signal. **The
load-bearing finding: the existing demographic spine already carries it** — db/011's `dob` field already requires
`facets.precision` + accepts optional `facets.basis`, and `clinician-observed` is already provenance rank 30 (correctly
displaceable by documents) — so **NO db/ floor change, NO schema/SCHEMA bump, NO new event type, NO ADR/spec edit.**
Three layers: (1) **pure `cairn-event::evidence`** — `birth_year_range_from_age` (principle 4: store the *time-invariant
birth-year window*, never raw age which drifts; an explicit **range**, never a false-precise midpoint) → `estimated_dob_body`
(value `"<min>/<max>"` e.g. `"1981/1991"`, precision `"year-range"`, reusing `dob_assertion_body`/`render_dob_twin`) +
`observed_sex_body` (on **`administrative-sex`**, apparent/phenotypic — NOT the `sex-at-birth` birth fact a clinician
can't know, which also keeps it out of db/016's *verified*-sex veto); (2) **`cairn-node::evidence::assert_observed_evidence`**
+ a standalone `assert-observed-evidence <patient-uuid>` CLI (authors the dob + administrative-sex `demographic.field.asserted`
events on an EXISTING chart in one transaction, provenance `clinician-observed`; evidence accrues over time — `register-john-doe`
unchanged; promoted `db::next_hlc` to `pub`); (3) **advisory matcher range-awareness** — `DateValue` gains an optional
birth-year interval, `parse_dob` reads `"year-range"` (`"<yyyy>/<yyyy>"`, malformed→None safe-degrade), and `compare_dob`
is **range-aware + POSITIVE-ONLY** (interval overlap→PARTIAL, no-overlap→INSUFFICIENT_DATA, **never DISAGREE** — a soft
visual estimate may *support* but never *suppress* a match, mirroring `compare_identifier_sets` + §5.4 recognition/principle 4).
TDD: 6 pure `cairn-event` + 2 pure builder + 4 DB-gated `cairn-node` integration (`observed_evidence.rs`: range dob lands
clinician-observed · document-verified dob displaces it · observed sex on administrative-sex with sex-at-birth untouched ·
age+sex atomic) + `DateValue`/`parse_dob`/`compare_dob` pure tests (incl. inclusive touching-boundary + symmetry) + a
DB-gated matcher e2e (`test_observed_age_pipeline.py`: candidate DOB inside the range→PARTIAL, outside→INSUFFICIENT_DATA,
via the real `load_candidate`→`field_comparisons` path). Full `cargo test --workspace` 0 failed + workspace clippy clean;
matcher 226 passed (pure + DB) + ruff clean, on the PG18 + cairn_pgx 0.2.0 rig (:5532). **End-to-end CLI smoke on a
provisioned node PASSED:** `register-john-doe` → `assert-observed-evidence --age 40 --tol 5 --age-basis … --sex male …`
→ `patient_demographic` shows `dob=1981/1991 (year-range, clinician-observed)` + `administrative-sex=male
(clinician-observed)`, `chart_trust=unconfirmed`. **7-task subagent-SDD each reviewed clean; final whole-branch review
(opus): READY TO MERGE** — all invariants hold, the `year-range`/`/`/`clinician-observed`/`administrative-sex` tokens are
byte-identical across the Rust producer, the DB, and the Python consumer, positive-only compare provably cannot emit
DISAGREE; all Minors deferred. **Honest scope limit (recorded):** a range does NOT generate a blocking key
(`"1981/1991"`→first-4-digit `1981` only blocks a 1981-born candidate; a John Doe's only name is an excluded callsign),
so pure-age evidence becomes a positive **scoring** signal once a pair is blocked by another key (a shared identifier off
belongings, a refined name, the hub sweep) — not a blocking signal; a birth-year-range blocking pass is a clean future
slice. **Deferred (recorded):** photo/marks/belongings/EMS-context evidence (new field home + attachment tier); the
"prior history now available" push-alert (§5.12, no notification tier); the search-before-create funnel (UI/API tier);
fuzzy age-range comparison; a `--observed-year` CLI override; the birth-year-range blocking pass. **§5.4 John-Doe slice B
is now BUILT.**

**Prior session (2026-07-03) — §5.4 John-Doe registration, slice A: callsign minting + matcher placeholder exclusion**
(brainstorm→spec→plan→inline-TDD; spec+plan under `docs/superpowers/{specs,plans}/2026-07-03-john-doe-callsign-registration*`).
The §5.4 **registration front door** C4 explicitly deferred: what a clinician invokes when an unconscious/unknown patient
arrives. C4 (db/024) had already built the *unconfirmed* trust state + `identity.pending.asserted`; this slice supplies
the **UUID + system-generated callsign** and the matcher exclusion that makes the callsign safe, **composing built
primitives — NO new event types, NO `db/` migration, NO floor change, NO SCHEMA/ADR/spec bump.** Three parts:
(1) **pure callsign generator** in `cairn-event` (`john_doe::callsign` → `Unknown-<class>-<site>-<date>-<suffix>`; a
culture-neutral, deterministic, obviously-not-a-real-name string; `sanitize_part` is **Unicode-aware** — a non-Latin
site label is preserved, not dropped, per the anti-cultural-capture mission); (2) **`register_john_doe`** in `cairn-node`
(`john_doe.rs` + a `register-john-doe` CLI subcommand) — mints a UUID, then authors a **callsign name assertion**
(`demographic.field.asserted`, `facets.use="callsign"`, reusing `name_assertion_body`/`render_name_twin`) **and** the C4
`identity.pending.asserted` in **one transaction**, so the chart is never half-registered and renders *unconfirmed*;
(3) **matcher placeholder exclusion** (advisory, `matcher/pipeline/db.py`) — both the blocking `name_tokens` CTE and the
`load_candidate` scoring query exclude `use_key ∈ {callsign}` (`use_key <> ALL(%s)`), so two John Does registered at the
same site on the same day **never false-match on shared callsign tokens** ("unknown", the site, the date). **The
load-bearing calls:** the callsign is a **real, displayed name** in `patient_name` (db/012's unidentified-patient
fallback makes it the header winner) — §5.4's "exclude from feature space" is therefore a **query-time exclusion in the
advisory matcher**, the correct layer (§5.2/§5.13, principle 12), never a floor rule or a decision to withhold the name;
the suffix is **UUID-derived** (last **8 hex = 32 bits**, `SUFFIX_HEX_LEN`), **partition-safe with zero coordination** (a
readable per-site-per-day A/B/C counter would race under partition — §5.4 is local-by-construction); a duplicate callsign
string is never a false-merge issue (the UUID is identity, the callsign is excluded from matching) but is **not merely
cosmetic** — two identical bedside/worklist headers are a wrong-chart hazard (principle 3), so 32 bits sizes the collision
to ~1-in-4.3-billion-per-pair rather than tolerating it; registration-class = C4's pending marker (no new "unidentified"
flag). The **load-bearing** exclusion is the **scoring** one (`load_candidate`): a callsign is a single whitespace-free
token, so distinct callsigns never share a blocking token anyway — the blocking exclusion is cheap defense-in-depth for
the rare identical-callsign collision on the C2b auto-link path, not what keeps two ordinary John Does apart. TDD: 8 pure callsign tests
(`cairn-event`) + 4 pure builder + 3 DB-gated integration tests (`cairn-node/tests/john_doe.rs`: unconfirmed chart ·
callsign is a placeholder-use display-winner · two John Does coexist distinct) + 4 DB-gated matcher tests
(`test_john_doe_exclusion.py`: two IDENTICAL callsigns → no pair · callsign excluded from scoring · a real name on the
SAME chart still blocks/scores · a real name equal to a callsign token does not pair — the blocking tests force the
shared-token condition so they actually exercise the exclusion, not pass vacuously); `conftest.seed_patient` gained a
`callsign_names` param. Full `cargo test --workspace` **379 passed / 0 failed** + workspace clippy clean; matcher **207 passed** (DB) /
164 passed (pure); ruff clean — all on a **PG16 + cairn_pgx 0.2.0** rig stood up from scratch in-container this session
(pgrx 0.18.1, `--features pg16`, `postgresql-server-dev-16` headers, local-TCP `trust`). **End-to-end CLI smoke:**
`register-john-doe` on a provisioned node minted `Unknown-ed-bay3-2026-07-03-dc88`, chart *unconfirmed*, callsign stored
`use_key='callsign'`; a second run produced a distinct callsign and reused the actor (idempotent `device`-actor enroll).
**Review hardening** (2-agent adversarial pass — correctness agent found **0 bugs**): (a) extracted the triplicated
`next_hlc` HLC-tick helper into a shared `db::next_hlc` (`auto_apply` + `john_doe` now both call it — house-rule #4);
(b) dropped a dead `.to_ascii_uppercase()` in `suffix_from_uuid` (the sanitizer lower-cases it anyway — doc/output now
agree); (c) made `sanitize_part` Unicode-aware (was ASCII-only, folding non-Latin labels to "unknown" — a cultural-
capture smell; now preserves any script). **Review round 2** (PR #123 code review — 5 findings addressed): (1) widened
the callsign suffix from **4 hex → 8 hex (16→32 bits)** — 4 hex collided ~1/65 536, a real coexistence-test flake AND a
bedside wrong-chart hazard (identical worklist headers, principle 3), not "cosmetic"; (2) made `ensure_registration_actor`'s
enrolment guard **kind-AGNOSTIC** (`WHERE signing_key_id=$1`, was `AND kind='device'`) — the narrow guard could add a
2nd actor to a key already enrolled under another kind, and db/005 NULLs `actor_id` for EVERY event a dual-mapped key
authors node-wide (silent, irreversible attribution loss); (3) rewrote the two **blocking** matcher tests to force the
shared-token condition (identical callsign; a real name equal to a callsign token) — they were vacuous because a callsign
is a single whitespace-free token, so distinct callsigns never share a blocking token (the load-bearing exclusion is the
**scoring** one), and corrected the false "share the tokens unknown/ed/site1/date" rationale in `db.py` + the design doc;
(4) corrected the **inverted drift note** — a Python placeholder-set *omission* UNDER-excludes → false-**merge** (dangerous
direction), NOT "reduced recall"; (5) **RESOLVED the sync issue (issue #124, now closed)** — hoisted the placeholder-use
set into a pure, psycopg-free `cairn_matcher.placeholder_uses` module (single source of truth, importable by both the
psycopg-bound `pipeline/db.py` AND the pure `eval/generator.py` mirror), made the eval `name_tokens` mirror
placeholder-aware, and added `tests/test_placeholder_uses_sync.py` — a **cross-language guard** that reads the Rust
`CALLSIGN_USE` literal from source and fails CI if it drifts out of the Python set (proven to bite: a Rust-side rename that
forgets Python fails 2 tests with a FALSE-MERGE message). **All suites re-run green on
a PG18.1 + cairn_pgx 0.2.0 rig (:5532):** `cargo test --workspace` all-pass (incl. the 3 DB-gated `john_doe.rs`) + workspace
clippy clean; matcher **207 passed** (DB) / 167 (pure) + ruff clean — and the two rewritten blocking tests were proven
non-vacuous (an identical-callsign pair blocks WITHOUT the exclusion, drops WITH it). **Deferred (recorded):** the
**clinician-observed evidence assertions**
(estimated age with basis → dob, observed sex → sex reuse existing fields; photo/marks/belongings/EMS context need a new
field home — larger, separate slice); the **"prior history now available — N allergies, M meds" push-alert** on link
(§5.12, no notification tier yet); the **search-before-create registration-class funnel** (§5.3/§5.8, UI/API tier); a
**readable sequential callsign suffix** (`-A`/`-B`; needs a partition-safe per-day count); wiring `identify`→optional-link
into one resolution flow. (The cross-language `CALLSIGN_USE`↔placeholder-set guard, previously listed here, is now BUILT —
see review-round-2 item (5) / issue #124.) **§5.4 John-Doe slice A (callsign + matcher exclusion) is now BUILT.**

**Prior sessions (2026-07-03) — the §5.7 identity algebra to C5 + the matcher's alias consumption (condensed; full detail in git + ROADMAP slices 15–19).** All merged on `main`, all additive (no floor/SCHEMA/ADR/spec bump except where noted): **C2b** auto-apply of the `auto_candidate` band (matcher-authored, un-attested, recallable link; apply-time veto re-check; per-`matcher_version` `agent` actor); **C3** `dispute` + the `chart_trust` projection (`db/023`; the *under-review* state); **C4** `identify` + the *unconfirmed* state (`db/024`; the §5.4 identity-pending front door; the §5.7 confirmed/unconfirmed/under-review contract is COMPLETE via a severity-max `chart_trust`); **C5** `repudiate` + the known-alias pool (`db/025`; the first *suppressing* identity event — a value-grained `name_repudiation` overlay strikes a known-false name from `patient_name_current`, `mode='suppressing'` forces the human-attestation floor, `patient_alias_pool` surfaces struck names to the matcher); and the **matcher consuming `patient_alias_pool`** (advisory Python — a `known_alias` evidence entry on the proposal, flag-never-suppress, `band()` forces REVIEW; the confidentiality-split view is reason-free per ADR-0006). **Deferred (recorded):** `reattribute` (waits on a clinical-note surface); fuzzy alias recognition; a per-slice identity-floor helper refactor + a deterministic `content_address` tiebreaker on the HLC overlay ([#115](https://github.com/cairn-ehr/cairn-ehr/issues/115)).

**Merged 2026-07-02 (9 PRs; full detail in git + ROADMAP slices 6–14).** A dense build+review day, all on `main`: the **quarantine/legibility trilogy** (durable pull-plane quarantine + re-offer floor on the clinical `db/021` and node-event `db/022` planes + ADR-0040 legibility/skew primitives wired into every signature door, `cairn_pgx`≥0.2.0 startup floor); **ADR-0040 signing-context domain separation** (spec v0.40→v0.41; the day's only spec bump); the **in-DB clinical apply door** `db/020_apply_remote_event.sql` (a replicated event faces the same floor as a local one) + the contamination-cascade recall-key fix (#99); a **7-agent adversarial review** (`docs/code_reviews/2026-07-02-*`) → in-branch fixes + filed issues #91–#103; and **identity C1** (`db/018` §5.1/§5.7 linkage core) + **C2** (`db/019` human-accepted→attested link).

**Prior sessions (2026-06-29/30/07-01) — the §5.2 advisory matcher pipeline B2→B3 (condensed; full detail in git + ROADMAP slices 8–12).** Advisory Python, no `db/` floor except B2's `db/017_match_proposal.sql` worklist (SCHEMA 15→16); no ADR/spec bump. **B2** veto-gated pairwise pipeline + proposal worklist (`cairn_matcher/pipeline/`); **B2b** blocking / candidate-pair generation (3-pass disjunction, oversized-block guard) + a `sweep()` batch driver; **B3 eval harness** (`cairn_matcher/eval/` — scorer metrics + DB-gated blocking-recall measurement + culture-plural `gold_v1.json` + a `python -m cairn_matcher.eval` CLI, real-path reuse/no-drift); **B3 compound blocking key** (additive `name+birth-year` `UNION ALL` pass in `pipeline/db.py`; recall non-decreasing; honest culture-neutral year degrade via the first 4-digit run); **B3 synthetic volume generator** (`eval/generator.py` pure + `eval/generate.py` CLI — seed+corrupted-clone entity clusters recoverable by construction, drift-canary-pinned to the base blocking passes). **Deferred:** an A/B pass-toggle in `generate_candidate_pairs` (quantitative before/after); weight-learning; further compound keys; a veto-aware/e2e scorer mode; the matcher test-leak + harness `KeyError` ([#84](https://github.com/cairn-ehr/cairn-ehr/issues/84)).

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
  proposal retraction / **richer §7.5 matcher-actor determinants** (served-model digest; C2b registered the matcher as
  a per-epoch `agent` actor keyed on `matcher_version`). **Identity: pieces C1** (§5.1/§5.7 linkage core — `db/018`),
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
  `compare_dob` in the matcher — **are now BUILT; NO new event type / migration / floor / SCHEMA / ADR / spec change.**
  **Remaining §5.4:** photo/marks/belongings/EMS-context evidence (new field home + attachment tier — separate slice), the
  "prior history now available" push-alert on link (§5.12, no notification tier yet), the search-before-create
  registration-class funnel (§5.3/§5.8, UI/API tier), a readable sequential callsign suffix (partition-safe per-day
  count), a **birth-year-*range* blocking pass** (slice B's range is a *scoring* signal, not a blocking key — see the
  2026-07-04 session note), a `--observed-year` CLI override, and `identify`→optional-link wired into one resolution flow. Reattribute composes one more *under-review*
  source into the `chart_trust` VIEW when it lands. Deferred (repudiate): a **reversal / de-repudiation** event (overlay HLC-versioned, composes without rewrite);
  a **chart-history VIEW** rendering struck names; ~~**matcher wiring** consuming `patient_alias_pool`~~ **(DONE this
  session — known-alias evidence; flag-never-suppress; fuzzy recognition + a dedicated `alias` blocking pass deferred)**. Deferred: an **A/B pass-toggle**
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
