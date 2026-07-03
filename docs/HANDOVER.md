# HANDOVER — Cairn

**Session date:** 2026-07-03 · **Spec/ADRs:** v0.41 · **Phase:** architecture complete; **first production clinical
surface under construction** — demographics on `cairn-node` (slices 1–5 done) + the §5.2 matcher (piece A in-DB veto
floor · B1 advisory scoring core · B2 veto-gated pairwise pipeline + proposal worklist · B2b blocking / candidate-pair
generation + batch sweep · B3 eval harness · B3 compound blocking key · B3 synthetic volume generator) + the
**§5.7 identity core: C1 linkage · C2 human-accepted apply seam · C2b auto-apply of the `auto_candidate` band · C3
`dispute` + the chart trust-state projection · C4 `identify` + the *unconfirmed* trust state · C5 `repudiate` + the
known-alias pool — done this session** (the §5.7 confirmed/unconfirmed/under-review contract is COMPLETE, and C5 adds
the first *suppressing* identity event); remaining B3 weight-learning / locale packs / A-B pass-toggle + identity
**C5+** (`reattribute` — waits on a clinical-note surface — + the full §5.4 John-Doe registration subsystem) next.
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node, Postgres-on-Android).

**This session (2026-07-03) — identity C5, `repudiate` + the known-alias pool** (brainstorm→spec→plan→inline-TDD;
spec+plan under `docs/superpowers/{specs,plans}/2026-07-03-identity-c5-repudiate-alias-pool*`). The **first
*suppressing*** identity event — C1–C4 were all additive/annotative; `repudiate` strikes a known-false name from the
display projection. The §5.5(a) **fabricated-persona** case: a patient presented under a deliberately false name; once
established false, the name leaves the header but **stays in the record** (fact of presentation preserved, principle 1)
and enters a matcher-visible **known-alias pool** (aliases are reused). One event type **`identity.repudiate.asserted`**
through the reused `submit_event` door, registered **`mode='suppressing'`** — so the db/005 attestation gate
**structurally forces a valid human token** (§5.7's "Human" made unbypassable, *no floor special-case*: it reuses the
exact gate that guards `salience.downgrade`). This is the deliberate design contrast with the additive C1/C3/C4 (whose
"human vouches" bit only when a responsibility-bearing contributor was named). **Digital strike-through** (principle
1+2): the assertion event and db/012's `patient_name` retained set are **left untouched**; a **value-grained**
`name_repudiation` overlay (keyed by `(subject, value)` — a false name is false *however* labelled, and value-keying
**avoids replicating db/012's `use`-fold** so there is nothing to drift; HLC-latest-wins so a future reversal composes)
records the struck value, and `patient_name_current` is `CREATE-OR-REPLACE`d to **anti-join** it (column contract
UNCHANGED ⇒ reload-idempotent). New **`patient_alias_pool`** VIEW surfaces struck names to the §5.2 matcher.
`cairn_check_repudiation_assertion` culture-neutral structural floor (valid subject uuid, non-empty value + reason) +
**HARD-required legibility twin**. **Documented design call:** striking a chart's *only* name → `patient_name_current`
has **no** row for it — honest (the name is genuinely unknown-now; displaying the known-false one is a *precise
untruth*, principle 4); "the header always shows something" is satisfied one layer up by the §5.4 callsign /
*unconfirmed* rendering (C4), not by lying. **Honest limit:** the value match is **exact-string** on an opaque value
(the only culture-neutral, deterministic, convergent choice — the suppression floor must be precise or it risks
striking the wrong, possibly true, name); fuzzy recognition of a returning alias is the **advisory matcher's** job over
`patient_alias_pool`, never the floor's. **`db/025_identity_repudiate.sql` (wired into `db.rs`) + a pure `cairn-event`
builder; NO SCHEMA/ADR/spec bump, db/010–024 left UNTOUCHED** (implements settled §5.5/§5.7; CREATE-OR-REPLACEs the
shared twin hook + `patient_name_current`). TDD: 2 pure builder unit tests + 10 DB-gated integration tests
(`crates/cairn-node/tests/identity_repudiate.rs`: struck name leaves winner + surviving name takes over + alias-pool
entry + retained-set evidence preserved · only-name → no winner (honest, never the lie) · idempotent re-assert +
HLC-latest reason · **newer re-assertion does NOT un-strike** (the HLC-blind anti-join pinned) · **un-attested AND
agent-attested repudiation refused** — the suppressing "Human" floor, both branches · four floor rejections [empty
value, empty reason, bad subject, missing twin]). Full `cargo test --workspace` (**364 passed / 0 failed**, incl.
C1/C3/C4 + demographics-names regression green) + workspace clippy clean on a **PG16 + cairn_pgx 0.2.0** rig stood up
from scratch in-container this session (pgrx 0.18.1, `--features pg16`, `postgresql-server-dev-16` headers, local-TCP
`trust` in pg_hba). **Review hardening** (3-agent adversarial pass; SQL-correctness agent found 0 hard bugs):
(a) **confidentiality split** — `patient_alias_pool` is now **reason-free** and the base `name_repudiation` table is
**not** granted to `cairn_agent`, so the free-text forensic `reason` can't leak cross-patient on the name-searchable
matcher view (ADR-0006); (b) `reason` made `NOT NULL` (the floor already guarantees it); (c) the HLC-blind anti-join +
the agent-attested-refused branch got explicit tests; (d) floor-rejection tests now also assert they reject at the
**floor**, not the attestation gate; (e) the `origin`-tiebreak collation note folded into #115. **Deferred (recorded):** a **reversal / de-repudiation** event (the overlay is HLC-versioned so it
composes with no rewrite — the append-only correction path is a separate §5.5 decision); a **chart-history VIEW**
rendering struck names (the retained set + overlay already carry the data); **matcher wiring** that *consumes*
`patient_alias_pool`; `reattribute` (§5.5 event-granular strike-through of *clinical documentation* — waits on a
clinical-note surface that does not yet exist; premature) and the full §5.4 John-Doe registration subsystem.
**Identity C5 is now BUILT — the first suppressing identity event, `repudiate` + the alias pool.**

**Prior session (2026-07-03) — identity C4, `identify` + the *unconfirmed* trust state** (brainstorm→spec→plan→
inline-TDD; spec+plan under `docs/superpowers/{specs,plans}/2026-07-03-identity-c4-identify-unconfirmed*`). The third
and final state of the §5.7 chart trust-state contract C3 opened. Two **additive** event types through the reused
`submit_event` door: **`identity.pending.asserted`** (the §5.4 John-Doe front door — marks a chart identity-pending →
*unconfirmed*) and **`identity.identify.asserted`** (§5.7 "who, method": establishes identity → *confirmed*).
**Keyed by the SUBJECT itself** — a per-chart lifecycle state, unlike a dispute's own id ⇒ **no subject-consistency
guard** is possible or needed (the design contrast with C3). `cairn_check_identity_state_assertion` culture-neutral
structural floor (`basis` for pending / `method` for identify — required non-empty = "method recorded" enforced
structurally) + **HARD-required legibility twin**; `chart_identity_state` HLC-overlay table (latest-HLC wins, full
pending⇄identified lifecycle incl. re-registration re-opening unconfirmed, out-of-order convergent, no BFS/oversize
guard — cheaper than C1). **`chart_trust` reworked into a severity-max UNION** composing **under-review (open dispute,
2) over unconfirmed (pending, 1)** — the §5.9 "highest standing assertion" discipline — column contract UNCHANGED so
`CREATE OR REPLACE VIEW` stays reload-idempotent and C3's `person_chart_trust` is untouched (it surfaces `unconfirmed`
for free). **Precedence** (documented safety call): under-review (attribution actively challenged — data present
possibly wrong-patient) outranks unconfirmed (who-is-this unknown — absent history). **§5.7 "Human" adjudication**:
`method` structurally required; the human-vouches requirement composes via the existing attestation gate when a
responsibility-bearing contributor is named (workflow-tier policy, not a floor special-case — considered/rejected
forcing it at the type level). **`db/024_identity_identify.sql` (wired into `db.rs`) + pure `cairn-event` builders;
NO SCHEMA/ADR/spec bump, and `db/023` left UNTOUCHED** (implements settled §5.4/§5.7; CREATE-OR-REPLACEs the shared
twin hook + `chart_trust`). TDD: 3 pure builder unit tests + 15 DB-gated integration tests
(`crates/cairn-node/tests/identity_identify.rs`: accept · HLC overlay both directions · re-pending-reopens · idempotent
re-assert · pending→unconfirmed on `chart_trust`+`person_chart_trust` · identify→confirmed · pending-before-chart
safety signal · the **C3⊔C4 compose/precedence proof** [dispute outranks pending → resolve→unconfirmed → identify→
confirmed] · five floor rejections). Full `cargo test --workspace` (0 fail incl. C1/C3 regression green) + workspace
clippy clean on a **PG16 + cairn_pgx 0.2.0** rig stood up from scratch in-container (pgrx 0.18.1, `--features pg16`,
`postgresql-server-dev-16` headers). **Deferred (recorded):** the full §5.4 John-Doe registration subsystem
(system-generated callsign, clinician-observed evidence assertions — age/marks/belongings/EMS context, matcher re-run
on new evidence); the "prior history now available" push alert on link (§5.12); registration-class partitioning of the
search-before-create funnel (§5.3/§5.8); `reattribute` (§5.5 strike-through + tiered adjudication) and `repudiate`
(alias pool + suppressing semantics); the §5.2 coherence feedback loop; person-level trust aggregation (read-surface
tier). **Identity C4 is now BUILT — the §5.7 trust-state contract (confirmed/unconfirmed/under-review) is COMPLETE.**

**Prior session (2026-07-03) — issue: identity C3, `dispute` + the chart trust-state projection** (brainstorm→
spec→plan→inline-TDD; spec+plan under `docs/superpowers/2026-07-03-identity-c3-dispute-trust-state*`). The §5.7
patient-initiated **`dispute`** front door (§5.5(b) identity theft) **and** the §5.7 projection-side contract —
the chart **trust state** (*confirmed / under-review*) — the keystone C1 explicitly deferred and that the rest of
the algebra composes into. Two **additive** dispute event types (`identity.dispute.asserted` / `.resolved`) through
the reused `submit_event` door (low-ceremony like the C1 link — a dispute *annotates* trust, never erases/moves/blocks,
so no attestation unless a responsibility-bearing contributor is named); a culture-neutral
`cairn_check_dispute_assertion` structural floor + **HARD-required legibility twin**; a `chart_dispute` standing
overlay keyed by the dispute's own id (HLC-latest-wins, converges out-of-order — the C1 `patient_link` shape, but a
single-row fact ⇒ no BFS/oversize guard); a `chart_trust` effective-state **VIEW** deliberately shaped so
`identify`/`reattribute`/the §5.2 coherence check ADD source branches later (never a rewrite); surfaced as
a composed `person_chart_trust` view. **`db/023_identity_dispute.sql` (wired into `db.rs`) + pure `cairn-event`
builders; NO SCHEMA/ADR/spec bump, and `db/018` left UNTOUCHED** (implements settled §5.7). **Review-driven design
choice** (correctness-agent finding): `person_chart_trust` is a SEPARATE view *composing on top of* C1's
`person_chart` (reusing its `person_member` join) rather than extending `person_chart` in place — the first cut
`DROP+CREATE`-d `person_chart` in both db/018 and db/023 (because `CREATE OR REPLACE VIEW` cannot shrink an
already-extended view across the `connect_and_load_schema` reload), but a bare `DROP` would abort node boot the moment
any dependent view sits on `person_chart` (the API/UI read surface). Composing sidesteps it: `person_chart` stays
droppable-free, and `person_chart_trust` is the view future trust-source slices extend. TDD: 3 pure builder unit tests + 14
DB-gated integration tests (`crates/cairn-node/tests/identity_dispute.rs`: accept · HLC overlay · out-of-order
convergence · multi-dispute resolve-one-stays / resolve-all-confirmed · idempotent re-assert · dispute-before-chart
safety signal · five floor rejections). Full `cargo test --workspace` (38 binaries, 0 fail) + workspace clippy green
on a stood-up **PG16 + cairn_pgx 0.2.0** rig (this session built the rig from scratch in-container: pgrx 0.18.1,
`--features pg16`). **Deferred (recorded):** the *unconfirmed* (identity-pending) state + registration classes / John
Doe (C4/C5 with `identify`); `reattribute` (§5.5 strike-through + tiered adjudication) and `repudiate` (alias pool);
the §5.2 coherence feedback loop; notification/contamination cascade on dispute; person-level trust aggregation
(read-surface tier); a future refactor to collapse the per-slice identity-floor duplication (twin-hook ladder /
uuid-check / HLC-upsert) into shared helpers **+ a deterministic final tiebreaker (content_address) on the HLC
overlay upsert** so a Byzantine same-`(wall,counter,origin)` collision can't diverge honest nodes
([#115](https://github.com/cairn-ehr/cairn-ehr/issues/115), filed from this review). **Identity C3 is now BUILT.**

**Prior session (2026-07-03) — issue: identity C2b, auto-apply of the matcher's `auto_candidate` band** (brainstorm→
spec→plan→inline-TDD; spec+plan under `docs/superpowers/`). A matcher proposal banded `auto_candidate` (score ≥ auto
AND zero vetoes at propose time) becomes a **matcher-authored, un-attested, recallable** `identity.link.asserted`
event — **no human in the loop** — through the *same* `submit_event` door. **Rust-only in `cairn-node`
(`matcher_actor.rs` + `auto_apply.rs` + `apply-auto-candidates` CLI); NO `db/` migration, NO floor change, NO
SCHEMA/ADR/spec bump** — the db/018 floor already made an identity link additive + `targets_other_author=FALSE`, so
an un-attested matcher link needs no attestation. **Realises the deferred §7.5 matcher-actor:** each distinct
`matcher_version` is its OWN `agent` actor with its OWN key (auto-enrolled on first sight, owner ceremony), pinned
under **`skill_epoch`** (= matcher_version) so db/006 `events_by_actor_epoch` recall selects a bad config's auto-links
**precisely** (contamination cascade). Contributor role **`suggested`** (ADR-0028 contributory, no `responsibility`)
⇒ authorship present, accountability absent (principle 10). **Apply-time veto re-check** (the no-human-backstop safety
add): a pair that acquired a veto since propose is kicked to human **`review`**, never auto-linked over. Status
`pending → auto_applied` (distinct from C2's human `applied`) or `→ review`; idempotent (only `pending` picked up),
respects a human `rejected`. Matcher key sealed under the op passphrase (no recovery escrow — regenerable). TDD: 6
pure + 7 DB-gated tests (`crates/cairn-node/tests/auto_apply.rs`: enroll-once/reuse · distinct-epoch actors ·
un-attested link + person projection · veto→review · human-rejected skipped · batch across epochs + idempotent ·
recall precision) + end-to-end CLI smoke (applied 1 → 1 un-attested link + 2-member person + auto_applied; re-run
applied 0). Full cairn-node suite + workspace clippy + `cargo test --workspace` green on the Mac PG18 / cairn_pgx
0.2.0 rig. **Deferred:** no background scheduler (operator CLI only); ADR-0028 role enum still not DB-enforced
([#96](https://github.com/cairn-ehr/cairn-ehr/issues/96)); richer §7.5 determinants (served-model digest). **Identity
C2b is now BUILT.**

**Merged 2026-07-02 (9 PRs; full detail in git + ROADMAP slices 6–14).** A dense build+review day whose work is now
all on `main`:
- **Quarantine/legibility trilogy** — durable quarantine + re-offer floor + loud integrity on the **clinical** pull
  plane (#108, `db/021_sync_quarantine.sql`; PR #110), the **node-event** pull plane (#111, `db/022_node_event_quarantine.sql`;
  PR #113), and wiring the **ADR-0040** legibility/skew primitives into every signature door + a daemon `cairn_pgx ≥ 0.2.0`
  startup floor (#109; PR #112). Each closes a silent skip-and-advance set-union exclusion (the A1 loss class); each
  hardened against its own PR review.
- **ADR-0040 signing-context domain separation** (#95, spec v0.40→v0.41): one envelope signature per event, domain-
  separated by a registered signing context bound twice (COSE content-type + `external_aad`); fail-closed on pre-ADR-0040
  blobs; `cairn_pgx` → 0.2.0. **The only spec/ADR bump of the day.**
- **In-DB clinical apply door** `db/020_apply_remote_event.sql` (#91): a replicated clinical event now faces the same
  in-DB floor as a locally-authored one (attestation stored + shipped on the wire; t_effective wire-pinned; clamp-and-flag
  projection guards). **Contamination-cascade recall key** fixed (#99): `events_by_actor_epoch` resolves against the full
  registry history + an `attribution` column (pinned/unattributed/pre-registration) — a superseded-epoch recall no longer
  silently under-selects.
- **Comprehensive adversarial review** (7 parallel agents over SQL floor / Rust / Python matcher / 39 ADRs;
  `docs/code_reviews/2026-07-02-comprehensive-review.md`) → a batch of in-branch floor/daemon/matcher fixes + filed issues
  **#91–#103** (design / large-refactor backlog).
- **Identity C1** (`db/018_identity_linkage.sql`): the §5.1/§5.7 linkage core — additive link/unlink types through the
  reused `submit_event` door, `patient_link` HLC-overlay edge table, `person_member` connected-component projection with
  clean unmerge (principle 2). **Identity C2** (`db/019` + `cairn-node::apply_proposal`): a human-ACCEPTED proposal becomes
  a human-ATTESTED link event (the accepting human is a responsibility-bearing contributor tripping the db/005 attestation
  gate). Both additive, no floor change. **C2b (this session) is the auto-band sibling of C2** — see top.

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
  build against demographics) + the full §5.4 John-Doe registration subsystem (callsign, clinician-observed evidence
  assertions, matcher re-run); reattribute composes one more *under-review* source into the `chart_trust` VIEW when it
  lands. Deferred (repudiate): a **reversal / de-repudiation** event (overlay HLC-versioned, composes without rewrite);
  a **chart-history VIEW** rendering struck names; **matcher wiring** consuming `patient_alias_pool`. Deferred: an **A/B pass-toggle**
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
