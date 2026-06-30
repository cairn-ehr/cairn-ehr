# HANDOVER — Cairn

**Session date:** 2026-07-01 · **Spec/ADRs:** v0.40 · **Phase:** architecture complete; **first production clinical
surface under construction** — demographics on `cairn-node` (slices 1–5 done) + the §5.2 matcher (piece A in-DB veto
floor · B1 advisory scoring core · B2 veto-gated pairwise pipeline + proposal worklist · B2b blocking / candidate-pair
generation + batch sweep · B3 eval harness — scorer + blocking-recall measurement · **B3 compound blocking key
(name-token+birth-year) — done this session**; remaining B3 weight-learning / locale packs + piece C link-seam next).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node, Postgres-on-Android).

**This session (2026-07-01):** built the **§5.2 matcher B3 compound blocking key — name-token + birth-year**
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
**Deferred (recorded):** the **synthetic corruption / volume generator** (quantitative before/after at volume); **further
compound keys** (`dob+first-initial`, `name+sex`); **weight-learning**. **The §5.2 compound-blocking-keys item is now BUILT.**

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

**Prior session (2026-06-28):** built demographics **slice 5 = §4.3 address three-facet value** (per-use recency-first winner).
`cairn-event::demographics` address builders: `AddressAssertion`/`Geo`/`StructuredAddress` + `address_assertion_body` +
`render_address_twin`. `db/014_demographics_address.sql`: address branch in the shared floor (structured⇒profile, parts-text,
geo-shape); retained-set `patient_address` (keyed `(patient, use, display)`) + per-use recency-first `patient_address_current`
VIEW (one current address per `use`). Display-winner: **recency-first** within a `use` — addresses are volatile (people move);
a fresh patient-stated "I moved last month" must displace a stale document-verified address. Identical logic to names
([ADR-0036](spec/decisions/0036-demographic-name-display-recency-first.md)); deliberate inverse of DOB's provenance-lock.
Provenance/origin are tiebreaks, not the winner gate. All addresses retained as evidence; provenance feeds the §5.2 matcher.
No new event type, no `submit_event` change; SCHEMA array 12→13. New **[ADR-0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md)**
(per-use recency-first winner; fixes the stale §4.3 table line); spec **0.38 → 0.39**. **cairn-event** 4 new unit tests (33/33
suite green); **cairn-node** 9 new integration tests (`demographics_address`); slices 1–4 regress green; clippy clean.
**Demographics slices 1–5 are now done.** §4.3 address is complete.

**Prior session (2026-06-28):** built demographics **slice 4 = the two remaining §4.2 sex/gender fields** on the existing
`demographic.field.asserted` spine (`db/011`). **Administrative-sex** → provenance-first (document-anchored marker; recency
wins among equal provenance — the same ordering as DOB/sex-at-birth). **Gender-identity** → recency-first regardless of
provenance (the inverse: the patient's current stated identity always wins; provenance still feeds the §5.2 matcher).
Mechanism: one IMMUTABLE `cairn_demographic_field_policy(field)` classifier (`db/013_demographics_sex_gender.sql`) drives
BOTH the projection gate and the winner ordering; supersedes db/011's `patient_demographic_apply` via CREATE OR REPLACE;
db/012 untouched; SCHEMA array 11→12. db/013 also adds an idempotent `cairn_demographic_backfill()` (called once at load)
that re-folds events ALREADY in `event_log` for a now-projectable field — the ADR-0012 "carry now, project once the node
understands the field" catch-up; without it a federated node that carried gender-identity/administrative-sex under db/011
would never surface them on upgrade until a fresh assertion arrived (review fix on PR #73). **Karyotype resolved** (the
slice-2 deferred decision): a distinct field, never
displaces sex-at-birth (= assigned at birth); a `fact-proven` karyotype auto-displacing a `document-verified` assigned-sex
is a field-semantics error — they record different facts. Spec/ADR only, no karyotype code. New
**[ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md)**; spec 0.37 → 0.38. Additive-only: no new
event type, no floor change, no `patient_demographic` schema change. **cairn-event** 3 new unit tests (29/29 suite green);
**cairn-node** 6 new integration tests (`demographics_sex_gender` — incl. apply-order convergence + the backfill catch-up),
slices 1–3 regress green; clippy clean. **Merged to `main` as [PR #73](https://github.com/cairn-ehr/cairn-ehr/pull/73) (2026-06-28); slice 4 complete on `main`.**

**Prior session (2026-06-28):** built demographics **slice 3 = the §4.2 names field** — a retained-set
`patient_name` projection (every asserted name kept as evidence) plus a `patient_name_current` display-winner VIEW.
Display-winner rule: **recency-first within the legal-use tier** (HLC wall-clock wins; provenance/origin break ties);
falls back to the most-recent name of any `use` when no legal name exists (the alias/transliteration-only case).
This deliberately **diverges from DOB's provenance-lock** — names are a volatile, legitimately-changing field
(marriage, gender transition); provenance-first pins a stale married name or a deadname over the current
patient-stated legal name (a dignity and safety failure, paper-parity violation). All names are retained as
evidence regardless; provenance still feeds the §5.2 matcher. The displayed name is the legal-preferred reference
point; surfacing a preferred/chosen "a.k.a." name is **UI soft-policy above the floor** (principle 12), reading
the same retained set. New **[ADR-0036](spec/decisions/0036-demographic-name-display-recency-first.md)**; spec
0.36 → 0.37. **Implementation landed in this PR** (#71, brainstorm→spec→plan→subagent-SDD): `db/012_demographics_names.sql`
(the `patient_name` retained-set trigger + `patient_name_current` display VIEW), the `cairn-event::demographics`
`name_assertion_body`/`render_name_twin` builders, and 8 integration + 3 unit tests (all green; slices 1–2 regress
green; clippy clean). The display tier folds `use_key` to lower-case (deterministic `COLLATE "C"`) so the open `use`
vocabulary's legal token is recognised case-insensitively ("Legal"/"LEGAL") and convergently across nodes, with
`use_raw` preserving the authored casing. **Note:** #71 merged early (before the review fix landed), so that
case-insensitivity fix shipped in a small **follow-up [PR #72](https://github.com/cairn-ehr/cairn-ehr/pull/72)** off
`demographics-names` (merged 2026-06-28). **Both #71 and #72 are now merged; slice 3 is complete on `main`.**

**Prior session (2026-06-27):** built demographics **slice 2 = the §4.2 DOB + sex-at-birth provenance-locked fields**
(brainstorm→spec→plan→subagent-SDD, 5 TDD tasks; spec+plan under `docs/superpowers/`). Introduces the **new mechanic
slice 1 deliberately avoided**: *provenance-precedence projection*. A generic **`demographic.field.asserted`** event (a
`field` discriminator) flows through the **reused** `submit_event` door (never re-declared); **`db/011_demographics_fields.sql`**
adds **`cairn_provenance_rank`** — the §4.1 ladder as a total order, with a **new `fact-proven` top tier (70)** above
`document-verified` (laboratory/scientifically-established truth overriding mere attestation; unrecognized→0 so it can never
displace a known value) — the culture-neutral floor **`cairn_check_demographic_field`** (generic checks + a dob-only
`facets.precision` requirement, principle 4; never parses a date/sex), and the **winner-by-`(rank, HLC, origin)`
`patient_demographic` projection** (PK `(patient_id, field)`; "verified value **locks** vs. lower provenance" falls out;
recency breaks equal-provenance ties). **The load-bearing design call: the floor stays OPEN, the projection is GATED** — an
unknown `field` (a newer node's `gender-identity`) **passes the floor, is stored + legible via its twin, but is not projected**
(required for set-union federation, ADR-0012; never reject a peer's field). Full assertion history stays in `event_log` as the
matching evidence (winner-only projection, no retained-set table). Pure **`cairn-event::demographics`** builders
(`dob_assertion_body`/`sex_at_birth_assertion_body` + twins). Canonical **spec §4.1 ladder** prose extended to name
`fact-proven`. TDD on **PG18+cairn_pgx**: 7 integration tests (happy path · provenance-beats-recency + verified-locks ·
recency-among-equals · 6 floor rejections · unknown-field carried-not-projected · fact-proven displacement · slice-1/legacy
regression), all green; full workspace suite green, clippy clean. Final opus whole-branch review: **ready to merge, no
Critical/Important**. Filed **[issue #69](https://github.com/cairn-ehr/cairn-ehr/issues/69)** (codebase-wide: projection
winner tiebreak compares `node_origin` as collation-sensitive text — negligible blast radius, left consistent with
`patient_chart`/`patient_identifier`). **Noted clinical caveat** carried into the design for the later sex-expansion slice:
`fact-proven` karyotype auto-displacing a `document-verified` assigned-sex is a *field-semantics* question (sex-**at-birth** =
assigned vs. karyotype = a different fact), deliberately deferred; `event_log` retains both regardless. **Explicit deferrals:**
names (multi-valued + display-winner), administrative-sex, gender-identity (recency-wins), the §5.2 matcher/veto, globalising
the authored twin, a CLI verb.

**Earlier today (2026-06-27):** built demographics **slice 1 = the §4.4 patient-identifier assertion** end-to-end
(brainstorm→spec→plan→subagent-SDD, 4 TDD tasks; spec+plan under `docs/superpowers/`): an additive
**`EventBody.plaintext_twin`** field carrying the §4.5 authored twin in the signed body (additive-only — a `None` twin
omits from the wire, content-addresses unchanged; two CBOR tests pin it); pure **`cairn-event::demographics`** builders
(`identifier_assertion_body` + `render_identifier_twin`); **`db/010_demographics.sql`** — the culture-neutral §4.4 floor
helper `cairn_check_identifier_assertion` (distinct structural RAISEs: value/system/provenance non-empty,
**normalized non-empty string when present**, **normalized⇒profile**; never a checksum/format/profile-hold), a per-type
**`cairn_event_twin` hook** (called by the **unchanged** db/005 `submit_event` — *not* a re-declaration of the door)
that runs the §4.4 floor and carries the **authored** twin for demographic events, with legacy types falling back to the
derived skeleton twin (`cairn_twin_skeleton`), and the **set-union `patient_identifier` projection** (PK `(patient_id,
system, coalesce(normalized, value))`, `ON CONFLICT DO NOTHING`, trigger scoped to the demographic type). Integration
tests on **PG18+cairn_pgx**: happy-path (proves authored-twin passthrough), set-union dedup, honest degradation
(profile-less accepted), all **floor rejections** (each isolated, triple-gated: error + empty `event_log` + empty
projection), legacy regression. All green (cairn-event 19, cairn-node 130+), clippy clean. **Post-review fixes (this
session):** (1) the floor now also rejects a whitespace-only `normalized` (it would otherwise project a whitespace
`match_key`, silently conflating distinct identifiers); (2) the demographic twin/floor was lifted out of a 140-line
byte-faithful `submit_event` copy into the `cairn_event_twin` hook above, so the validated write door stays
single-source in db/005 and cannot drift. **Explicit deferrals:** matching/veto (§5.2, the advisory matcher), a CLI
authoring verb, and globalising the authored twin to all event types. Filed **[issue #67](https://github.com/cairn-ehr/cairn-ehr/issues/67)**
(pre-existing: `db/008` surrogate-projection migration is absent from the `cairn-node` SCHEMA array).

**Earlier today (2026-06-27):** closed demographics **gap B** — the provider-number person×org relational model (the last
deferred piece of ADR-0033). New **[ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md)** +
**demographics §4.6**; spec 0.35→0.36. Model: an abstract identity-bearing **entity** (open `kind`: person/org/location/…)
carrying §4.4-verbatim identifier sets; **reified relationships** between entities carrying their own identifier sets
(where the AU Medicare provider number lives, tying a person to an org at a location); **subject-kind partitioning**
(`{patient, entity, relationship}`) that makes ADR-0033's non-conflation **structural** — a billing number can never be a
patient match key or a signing credential; a **one-way non-authorizing `actor_ref`** keeping signing distinct from billing;
and **position-not-value** (the same AHPRA string may validly appear both as a §7.5 actor licensure credential and a §4.6
billing identifier — the WorkCover case). Entity/relationship *data* is fit-for-purpose; the partition tag + `actor_ref` +
floor invariants are safety-critical. Cross-refs added in identity §5.2 and security §7.5/§7.7. No new founding principle.
Design/spec work only — no code. **Demographics gaps A (§4.2), B (§4.4/§4.6), and C (§4.5) are all now closed.**

**Earlier today (2026-06-27):** closed demographics representation gaps B+C — **[ADR-0032](spec/decisions/0032-culture-neutral-address-representation.md)** (culture-neutral address: three-facet display/geo/structured+profile, §4.3), **[ADR-0033](spec/decisions/0033-patient-identifier-representation.md)** (identifier: namespace/profile split + normalized form survives profile-less nodes, §4.4), **[ADR-0034](spec/decisions/0034-demographic-legibility-twin.md)** (demographic legibility twin bound to principle 11/§4.5 — floor enforces non-empty twin, `twin==render` advisory), **[ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md)** (entity/relationship + provider-number person×org, §4.6; subject-kind partitioning; design/spec only — no code). Spec 0.32→0.35.

**Prior sessions (2026-06-25/26) — ADR-0026 node durability + Spike 0003 (condensed; full detail in git + the ADR log):**
**slice C** restore-apply + new-identity `supersede` ([PR #52](https://github.com/cairn-ehr/cairn-ehr/pull/52); `db/009`
+ `node_lineage`); **slice B** backup-as-cold-peer export/verify/health ([PR #51](https://github.com/cairn-ehr/cairn-ehr/pull/51));
**slice D** sealed local-state export (`localstate.rs`, LSK dual-wrap, `CAIRNL1`/`CAIRNX1`, empty content / can't-retrofit
shape; **A–D all done**); [#53](https://github.com/cairn-ehr/cairn-ehr/issues/53) cold-medium self-identification (the
`medium.rs` `CAIRNB2` container self-marker — signed+event-set-bound or unsigned; forgery-proof always, converged-peer
splice is a confirm-on-restore residual); [#54](https://github.com/cairn-ehr/cairn-ehr/issues/54) uniform key zeroization
(`Zeroizing` across `seal.rs`+`localstate.rs`). **All ADR-0026 follow-ons closed; only optional escrow rungs (Shamir/QR/TPM)
remain.** Also **Spike 0003 (Postgres on Android) G0–G3 PASS** ([PR #47](https://github.com/cairn-ehr/cairn-ehr/pull/47)/[#48](https://github.com/cairn-ehr/cairn-ehr/pull/48)).

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
coverage that was missing. **Smaller deferred items remain open** (commented in code):
`events_by_actor_epoch` resolves against `actor_current` not historical `actor_event` rows;
`actor_current` wall-clock ordering needs a monotonic tiebreaker before production; no FK on
`recall_overlay.target_event_id`; plaintext twin is skeletal.

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
  `cairn_matcher/pipeline/{db,sweep}`), **and the B3 eval harness** (`cairn_matcher/eval/` — scorer metrics +
  DB-gated blocking-recall measurement + culture-plural `gold_v1.json` + `python -m cairn_matcher.eval` CLI) and the
  **B3 compound blocking key** (`name+year` additive pass in `pipeline/db.py`) are now BUILT. **Next (B3
  measurement-driven):** **weight-learning** (sweep `evaluate_scorer`'s `weights`/`thresholds` against the gold set) +
  **further compound keys** (`dob+first-initial`, `name+sex`) + locale comparator packs / hub-tier aggressive
  duplicate-sweep + proposal retraction / full §7.5 matcher actor registration; **piece C** — the §5.7 identity-event
  link-apply seam (the destination for match proposals; needs the `link`/`unlink`/… algebra, unbuilt). Deferred: a
  **synthetic corruption / volume generator** (volume + recall curves, same dataset format; would unblock quantitative
  before/after for compound keys) + a **veto-aware / end-to-end scorer mode**; deceased-status veto (stub in db/016); a
  `compare_address` comparator; a **CLI** sweep entry; the matcher test-leak + harness `KeyError`
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

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B prepared); 0002 (advisory-actor — ran, C1–C5 ✓
→ ADR-0029/0030); 0003 (Postgres on Android — **ran 2026-06-25, G0–G3 ✓**; PR #47/#48).
