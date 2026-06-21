# HANDOVER — Cairn

**Session date:** 2026-06-21 (built + ran **Spike 0002 — advisory-actor write contract**; C1–C5 PASS; [PR #27](https://github.com/cairn-ehr/cairn-ehr/pull/27); build-prep, spec unchanged at **v0.30**). Prior session 2026-06-20: trusted-time anchoring → ADR-0027 *closes the last open §11 architecture question*; closed-role-enum finalization → ADR-0028; node durability → ADR-0026; units ruling.
**Status of this file:** Working scaffolding, not a source of truth. Disposable — regenerate
at the end of each working session. If this file ever disagrees with the canonical documents,
the canonical documents win.

---

## Built + ran 2026-06-21 — Spike 0002 (advisory-actor write contract): C1–C5 PASS → [PR #27](https://github.com/cairn-ehr/cairn-ehr/pull/27)

Executed the previously-drafted **[Spike 0002](spikes/0002-advisory-actor-write-contract.md)** (the one build-prep thread that did **not** need the Pi — the Bet B compute run stays blocked on hardware). **Build-prep, not architecture: no spec or ADR change.** Design + plan: [`docs/superpowers/specs/2026-06-21-…-design.md`](superpowers/specs/2026-06-21-spike-0002-advisory-actor-write-contract-design.md), [`docs/superpowers/plans/2026-06-21-….md`](superpowers/plans/2026-06-21-spike-0002-advisory-actor-write-contract.md). Built on the Spike 0001 walking skeleton in `poc/walking-skeleton`; executed subagent-driven (10 TDD tasks, spec+quality review gate per task, final whole-branch review = *ready to merge, no Critical/Important*).

- **What it demonstrated — C1–C5 all PASS, exit 0** (`harness/spike_0002.py selftest`): an external advisory agent authors an **additive, un-attested, provenance-anchored, recallable** advisory through a validated in-DB door, **and the floor rejects all five hostile-agent attacks** (unsigned/malformed, raw INSERT as the agent role, forged human author, suppressing-un-attested, cross-author salience downgrade) — each with a **legible** reason, no gate loosened. ADR-0021's *"floor-in-DB, direct DB access safe by construction"* made checkable and **confirmed**.
- **What was built** (faithful to the §9 blast-radius rule): a real **`cairn_pgx` pgrx extension** wrapping `cairn-event` (one verify impl) — `cairn_verify`/`cairn_body`/`cairn_actor_id`/`cairn_attestation_ok`, the **ADR-0002 in-DB verify move, no longer stubbed**; **`db/004`** append-only actor registry where **`actor_id` *is* the content-address of the pinned set** (incl. `skill_epoch` — the parked ADR-0011 refinement, exercised); **`db/005` `submit_event`** (the single `SECURITY DEFINER` write door + classification + the `REVOKE`/`GRANT` grant floor); **`db/006`** recall query + append-only contamination overlay; `cairn-event` additions (`canonical_json_address`, Ed25519 attestation token, typed contributor set — **no `is_ai` flag, un-vouched is emergent**); a Python (`uv`) **agent stand-in** that signs client-side via a new `cairn-sync sign-stdin` helper (Rust owns the canonical COSE encoding — no second crypto impl).
- **Exit criteria met → both ADRs now WRITTEN (spec v0.31, same session):** **[ADR-0029](spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md)** (skill-epoch + served-model digest as pinned determinants of an agent actor's identity; refines ADR-0011; canonical home [security §7.5](spec/security.md)) and **[ADR-0030](spec/decisions/0030-advisory-actor-integration-contract.md)** (the advisory-actor integration contract — L2/L3 attachment + authorship through the in-DB floor; refines ADR-0021/0022; canonical home [language-substrate §9.8](spec/language-substrate.md)), promoting [ecosystem/0001](ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md) from *evaluation* to *decision*. Both **No new founding principle**. Wove into spec (§7.5/§9.8), bumped index to **v0.31**, added the open-questions resolved area + ADR-index rows + this pointer; `mkdocs build --strict` clean.
- **Honest gap to carry into the ADR work (recorded so it isn't lost):** the attestation **success** path (a *valid* token, correctly bound) is **never exercised end-to-end** — every C2/C5 test hits the "no token presented" gate, which is all the spike's bar requires. Exercising it needs an `attest-stdin` CLI + positive/wrong-address token tests; in scope for the integration-contract ADR, not the spike. Smaller deferred items (commented in code): `events_by_actor_epoch` resolves against `actor_current` not historical `actor_event` rows (recall-after-supersede); `actor_current` wall-clock ordering needs a monotonic tiebreaker before production; no FK on `recall_overlay.target_event_id`; the plaintext twin is skeletal.
- **Toolchain note for the next session:** the pgrx extension builds with `cargo pgrx` (separate from `cargo test --workspace`, which stays green without the toolchain — the crate is `exclude`d from the workspace). On macOS/Postgres.app PG16: `cargo pgrx init --pg16 "$(which pg_config)"`, and `cargo pgrx test` needs `BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$(xcrun --show-sdk-path)"` + a short `--pgdata /tmp/…` (the worktree path exceeds the 103-byte unix-socket limit). Documented in the `crates/cairn_pgx` README.
- **Remaining threads:** the **Bet B Pi compute-cost run** (still awaiting the board) and continued **clinical case-mining**. The one open thread *atop* the new contract is the **attestation success path** (ADR-0030 honest ceiling): the spike proved the *rejection* half (un-attested suppress/responsibility refused) but not a *valid* token accepted — exercising it needs an `attest-stdin`-style author flow + positive/wrong-address token tests, the first thing to build on the integration contract.

---

## Resolved 2026-06-20 — finalized the closed contributor-role enum (spec v0.30) → ADR-0028

Closed the **ADR-0007 deferred follow-on** ("closed role-enum membership"). The enum was already populated +
identical in ADR-0007 and [data-model §3.9](spec/data-model.md); the real task was to **ratify it as closed** and
rule on the three parked candidates (`dictated`/`reviewed`/`co-signed`). → **[ADR-0028](spec/decisions/0028-finalized-closed-contributor-role-enum.md)**
(refines 0007), canonical home [data-model §3.9](spec/data-model.md). **No new founding principle** (mechanism of
principle 10); **no schema migration** (the `role` field + descriptor existed day one — this fixes the closed
value set). Build `--strict` clean.

- **The bar I applied (and the user agreed to):** a role earns a closed-enum slot **only if the safety/DB layer or
  hard policy must branch on its responsibility semantics**; otherwise the distinction is a **descriptor** (flavor),
  a **policy gate** (workflow), or an **acknowledgment** ([ADR-0009](spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)).
  The enum is a safety primitive (the ADR-0010 suppressing-op owner-gate + the "AI-generated" structural reading
  both branch on bears-vs-not), so it stays small + closed + additive-only.
- **Final membership — 6 bearing + 5 contributory:** bearing = `authored`, `ordered`, `attested`, **`co-signed`**,
  **`witnessed`**, **`dictated`**; contributory = `drafted`, `transcribed`, `graded`, `triaged`, `suggested`.
- **The user (EM physician) ruled to add all three flagged-as-bearing candidates**, including the two I leaned
  against keeping (co-signed/dictated) — their clinical-workflow judgment: **`co-signed`** (supervisory sign-off,
  gateable "pending until co-signed" — pervasive in EM with registrars/residents/NP/PA); **`witnessed`** (attests
  an event *occurred/was observed* — controlled-drug waste, consent, restraint, verbal-order read-back, death
  verification; *I raised this one* as a stronger claim than two of the flagged three); **`dictated`** (voice
  source of content — bears intent while verbatim text rides a `transcribed` contributor with an ASR-accuracy gap
  until separately attested).
- **`reviewed` rejected** (the user agreed): it collapses to either `attested` (if it confers responsibility) or an
  acknowledgment event (if not) — admitting it would re-fuse the signature≠attestation split ADR-0007 separated.
- **Boundary recorded to forestall the next candidate round:** these roles describe **contribution to the record,
  not performance of the clinical act** → `performed` is out of scope (body content); `ordered` sits on the line by
  design. Remaining ADR-0007 follow-on (`on_behalf_of` proxy/liability) stays out of scope (jurisdictions interpret
  the recorded chain).

---

## Resolved 2026-06-20 — trusted-time anchoring (spec v0.29) → ADR-0027 — **closes the last original §11 open architecture question**

Resolved the final genuinely-open §11 item (**§11.14 trusted-time anchoring**), which the prior session had
logged with a direction + four recorded critique caveats. The user explicitly asked me to **surface modern
solutions beyond their 2001-era `gnotary` (RFC-3161) design** before committing — their knowledge predated 25
years of progress. It **dissolved into existing primitives — no new founding principle** (principle 4 applied to
wall-clock truth). → **[ADR-0027](spec/decisions/0027-trusted-time-anchoring.md)**, canonical home
**[data-model §3.17](spec/data-model.md)** (the grade + interval + envelope field), with the notary/anchor node
role in **[security §7.11](spec/security.md)** and the gossip/offline mechanism in **[sync §6.8](spec/sync.md)**.
One **day-one can't-retrofit** field (the clock-confidence grade + `t_recorded` interval, born on every event).
Build `--strict` clean.

- **The reframe that did the work (offered + accepted):** "trusted time" hides **two distinct problems** the
  RFC-3161 notary conflates — **(A) clock-setting** (trustable *current* time, bounds `t_recorded` from **below**,
  fights the drifting RTC) and **(B) existence-proof** (prove an event existed by T, bounds from **above**, fights
  backdating). gnotary is a (B)-only mechanism. Together they **bracket** `t_recorded` into an interval — and the
  bracket *is* principle 4 (the §3.7 uncertainty-capable time type), not a weaker fallback.
- **`t_recorded` becomes a graded interval, not a point.** Carries a single ordered **clock-confidence grade**
  (`unknown < self-asserted < network-synced < hardware-sourced < externally-anchored < multi-anchor-corroborated`,
  best-corroboration-wins — the severity/legibility/retrievability ladder shape). **Envelope floor + overlay
  refine** (the user agreed with my recommendation): initial grade+interval is a **day-one envelope field**; later
  anchor tokens are **overlays** that upgrade it ([ADR-0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)
  re-attestation-as-overlay). `self-asserted` is the honest default — proves integrity, **not** external time;
  never displayed as trusted.
- **The modern-landscape survey I brought forward** (the heart of the user's ask): the existence-proof world moved
  from "trust the TSA" to **transparency logs** (Certificate Transparency RFC 6962 → Trillian → Sigstore Rekor:
  append-only Merkle log, signed tree-heads, inclusion/consistency proofs, gossip to catch a lying log),
  **blockchain anchoring** (OpenTimestamps — Merkle root → Bitcoin, no trusted party), and **threshold notaries**
  (FROST). Clock-setting moved to **NTS** (authenticated NTP RFC 8915), **Roughtime** (multi-server with
  lying-server proofs), **GNSS/GPS**, **TPM/Secure-Enclave** secure clocks. VDFs noted-and-dismissed as overkill.
  **The convergence I didn't expect until I worked it through:** Cairn's event log is *already* an append-only,
  signed, Merkle-izable, gossip-synced log — so the transparency-log approach is **the same machinery pointed at
  time**, more Cairn-native than the centralized TSA. The user agreed it's "truer to the Cairn principles."
- **Two pluggable planes on the [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
  anchor spectrum — no Cairn-owned root, multi-anchor default.** Self-signed → practice transparency log →
  national notary; RFC-3161 TSA kept as **one supported anchor type** (interop); **threshold via the FROST already
  earmarked in ADR-0015** kills the single-notary trust/availability/capture point.
- **Offline = bracket, not degradation, via the existing gossip plane:** peer cross-attestation gives the **lower**
  bound (a received anchor at T → any later-authored event has `t_recorded.lower ≥ T`, no write-time round-trip);
  deferred **Merkle-root batch** notarization on reconnect gives the **upper** bound *and is the privacy fix* (the
  anchor learns "a batch existed," not per-event clinic-activity metadata). Solo node still gets an honest interval
  (TPM monotonic + RTC, HLC for ordering). Confidence is **graded, never required** — availability floor preserved.
- **Decisions the user ruled on this session:** (1) grade representation = **envelope floor + overlay refine**
  (agreed my rec); (2) primary canonical home = **data-model §3.17** (the load-bearing piece travels in the
  envelope), security/sync as mechanism homes (agreed my rec); (3) **public-chain anchoring = named-but-unshipped**
  (pluggable future, never default/dependency — governance + longevity optics; the user's call).
- **Honest ceilings recorded:** the real hardening is the **anchor's own time provenance + signing-key protection +
  long-term token renewal** (the protocol is the easy part); self-signed proves integrity not time; single anchor =
  capture/privacy point → multi + self-hostable + Merkle-root.
- **Also fixed:** the stale CLAUDE.md pointer ("§11.9 is now the sharpest" — §11.9 was long resolved by ADR-0008).
  **With ADR-0027 every original §11 open architecture question is closed.** Remaining generative threads are
  build-prep (Bet B Pi compute-cost run; [Spike 0002](spikes/0002-advisory-actor-write-contract.md)) and continued
  clinical case-mining.
- *(Private memory note, per product-neutrality: the user's own `gnotary` — submitted to gnu.org CVS in 2001 — is a
  ready candidate implementation of the notary role and may be revived; **never named in public docs.**)*

---

## Resolved 2026-06-20 — node durability & disaster recovery (spec v0.28) → ADR-0026; + units ruling; + 2 open items

Case-mined a **foundational gap not in the original §11 set**: the spec designed *deliberate* key-death
(crypto-shred, [ADR-0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md)) but left
*accidental* key-death and node disaster recovery undesigned. The only DR answer
([sync §6.3](spec/sync.md)) was *"re-provision from parent"* — which assumes a parent and excludes the
off-sync-plane keystore. The forcing case is the **solo node** (the
[ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md) sovereignty
floor): no peer → a dead disk is total loss. → **[ADR-0026](spec/decisions/0026-node-durability-and-disaster-recovery.md)**,
canonical home **[security §7.10](spec/security.md)**, mechanism notes in
[data-model §3.8](spec/data-model.md) + [sync §6.2/§6.3](spec/sync.md), open-questions resolved entry,
index version bump. **No new founding principle** (principles 1/2/3/4 applied to DR); one **day-one
can't-retrofit** requirement (the recovery-secret escrow + sealed local-state export must exist at
provisioning). Build `--strict` clean.

- **Dissolved into existing primitives** (the recurring pattern): **clinical events back up as a cold
  peer** (the sync daemon's peer is a local encrypted volume; restore = set-union apply through the
  existing verify-on-apply path → **self-verifying, no separate integrity check** — a tampered/bit-rotted
  medium fails the same ADR-0015 signature/content-address invariants as a malicious peer); **non-event
  trust material rides a sealed local-state export** (data-at-rest keystore + config + draft store — the
  only private-key-touching surface, the small safety-critical seam).
- **New identity on recovery, `supersede`-linked** (the [§7.5](spec/security.md) actor algebra — `enroll/
  supersede/revoke/suspend/rotate-key`, no new mechanism): the restored node mints a fresh, ideally
  hardware-bound (non-extractable) keypair; the dead node's publics stay verifiable forever. **The private
  signing key never enters a backup** → a stolen backup can't resurrect the node, and the scheme composes
  with TPM/Secure-Enclave keys. Cost: re-peer ([§7.7](spec/security.md)) — a no-op for a solo node.
- **Recovery secret = paper-escrow at the floor, pluggable upward** (printed code/QR sealed in the safe,
  optional Shamir M-of-N; opt up to token / peer-quorum). No mandatory cloud; fractal. The secret's own
  survival is the **named new single point of failure** (loss-model clause 3 — *"if the recovery secret is
  also lost, everything; we never pretend an artifact whose key is gone is recoverable"*).
- **Erasure survives DR**: crypto-shred is an append-only event the restore **replays** before projecting;
  the post-backup-shred window is closed by **shred completion ⊇ backup propagation** (a shred isn't done
  until it has reached attached node-controlled media and re-wrapped their key material). Detached/offline
  media = the declared honest ceiling. **Backup health is a first-class honest-assembly fact**
  (*"last successful backup N h ago"*).
- **Brainstorming-skill design doc:** the design was developed through the brainstorming flow but captured
  **directly as the ADR + spec weave** (Cairn's convention wins over the skill's `docs/superpowers/specs/`
  default — every prior decision landed as an ADR). No separate design doc written.

**Also this session (smaller):**
- **Units ruling → canon, no ADR.** The user ruled: **canonical SI in the event core; UI translates to
  locale via policy** — [principle 12](spec/index.md) (uniform core, plural edges) applied to quantities.
  Recorded as a one-line note in [data-model §3.7](spec/data-model.md) (quantities stored in canonical SI,
  unit intrinsic to the value, encoded against an international unit standard e.g. UCUM). Closes the
  units/value-normalization gap without an ADR-sized fight.
- **Two new OPEN items logged** ([open-questions §11](spec/open-questions.md)): **#13 operational
  observability — RESOLVED as out-of-core** (separable add-on software; Postgres already supplies
  `LISTEN`/`NOTIFY`+logical decoding+`pg_stat_*`; record-level safety facts already first-class; a central
  dashboard is optional add-on a deployment may take or leave — the user's ruling). **#14 trusted-time
  anchoring — OPEN, direction noted:** a pluggable, optional **RFC-3161-style timestamping notary**
  (defaulting to a self-signed timestamp), a node role on the ADR-0017 anchor spectrum; tokens are overlaid
  signed data, re-notarization is ADR-0015 re-attestation-as-overlay. **Critique recorded as the open
  caveats** before it's ADR-ready: (a) bounds `t_recorded` only *from above* and only *online* → offline
  needs deferred Merkle-batch notarization + graded clock-confidence + causal lower-bounding; (b) the
  self-signed default proves integrity, not time (grade it self-asserted); (c) single notary =
  trust/availability/capture/privacy point → multi + self-hostable + Merkle-root submission; (d) the real
  hardening is the notary's own time provenance, key protection, and long-term token renewal. Load-bearing
  core requirement: a **clock-confidence grade travels with every event**. *(Private memory only, per
  product-neutrality: a ready candidate implementation of the notary may exist and be revived — never named
  in public docs.)*
- **Standing instruction memorized:** **always surface flaws/risks/issues freely; criticism is strongly
  encouraged** — a project-wide working principle (applies to my own proposals too).

---

## Evaluated 2026-06-18 — agent & messaging plugins (kastellan, localmail) → new `docs/ecosystem/` area; spec unchanged (v0.26)

Evaluated two of the user's own AGPL projects for suitability as **pluggable Cairn extensions** (the user's framing:
infrastructure to make the EHR *flow* without compromising security needlessly):
[kastellan](https://github.com/hherb/kastellan) (a security-first personal AI agent — Rust, AGPL-3.0) and
[localmail](https://github.com/hherb/localmail) (a read-only IMAP→Postgres mirror — Python, AGPL-3.0). **No spec or
ADR change** — this is an *evaluation*, captured in a **new published doc tier**
**[`docs/ecosystem/`](ecosystem/README.md)** (analyses of external infra against the decided architecture; neither
spec nor ADR; wired into `mkdocs.yml` nav after Spikes). Main artifact:
**[ecosystem/0001](ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md)**.

- **The synthesis — three nested membranes, none on the inter-node path** (so neither can break interop/safety even
  if it fails — principle 12): **localmail** = quarantined intake membrane (read-only content-addressed mirror of an
  insecure channel; governs *what gets in*); **kastellan** = contained actor runtime (sandbox + egress + CASSANDRA
  plan-review; governs *what the agent may do*); **Cairn** = record-integrity core (governs *what becomes truth*).
  Both are **L2/L3**, consume the native API, write only through `submit_event` (ADR-0022), couple via the Postgres
  boundary (§9.3).
- **kastellan fit:** AGPL + Rust + in-Postgres (safety-critical bucket, §9.1); its dispatcher chokepoint rhymes with
  `submit_event`; CASSANDRA (gates *agent actions*) is complementary to Cairn's record-safety model (gates *record
  entry*), reconciled by **additive, un-attested AI authorship** (ADR-0007/0010 — raise salience, never suppress,
  never auto-act on the irreversible; human attests by overlay — which is also how CASSANDRA's "no irreversible act
  without a human" lands *without* tripping the principle-3 confirmation-dialog ban).
- **The scaling reframe (the user's correction, now canon in the eval):** single-occupancy is a **feature**, not a
  ceiling — the heavy resource is the **served models** (which scale horizontally on their own); the orchestrator is
  thin → **N thin single-occupant instances over a shared serving fabric**. A "user" need not be a clinician: a
  **purpose-tuned pathology-import pipeline is one registered actor** (ADR-0011) dropping advisories into Cairn → a
  *fleet of specialist actors*. Matrix = operator surface only; clinical surface = the notification economy
  (ADR-0009). Accountability routes via `on_behalf_of` to the deployer, never to a clinician who only saw an advisory.
- **The one ADR-worthy nugget — PARKED:** **skill-epoch as a pinned determinant of an agent actor's identity**
  (extends ADR-0011). Kastellan crystallises skills only when user-approved + pinned (model/harness fixed → this is
  *toward* determinism, not drift); treating the approved skill-bundle digest as a pinned determinant keeps
  contamination-cascade recall bounded to a skill epoch. Ratify *if/when* kastellan is actually adopted. Sibling
  operational seam noted: the served-model version must be **pinned per-actor** (the advisory records the model digest
  it ran against) so the shared fabric can't silently mutate a pinned identity. *Drift vs staleness* distinction
  recorded: pinning kills drift; staleness is handled one layer up by additive + human-review + re-crystallisation.
- **localmail fit:** license absence was an oversight, **now AGPL-3.0, deps confirmed clean**. Striking primitive
  convergence — its `blobs/<aa>/<bb>/<sha256-hex>` store *is* ADR-0013 content-addressing, so a derived clinical event
  can cite the exact immutable source bytes (provenance, legible across time). It is the **boundary skin** (§3.4):
  mirror read-only → matcher (ADR-0014) proposes patient links → additive events via `submit_event`. Security payoff:
  the agent reads a *quarantined mirror* with no send/delete path, so a mail-borne prompt-injection can't act —
  three nesting containment layers. **Embedding model:** served placeholder is fine (fit-for-purpose, reversible);
  the license bites at the *production fine-tune base* (Gemma ToU field-of-use **propagates to derivatives**) → keep
  the production base Apache/MIT (Arctic 2.0 / Nomic), fine-tune with an MRL-256 objective (Postgres index-load win by
  construction). Gemma beat Arctic on the user's *tax* corpus at matched 256-dim — out-of-domain, so low-signal for
  clinical; plug-and-play, low priority.
- **Next step — DRAFTED as [Spike 0002](spikes/0002-advisory-actor-write-contract.md) (Proposed, not yet run):**
  kastellan registers as a Cairn actor and drops one synthetic triage advisory through `submit_event` as an additive,
  un-attested event, **and a hostile/buggy agent fails to breach the in-DB floor** (the sharpest half — ADR-0021's
  "direct DB access safe by construction" made checkable). Extends the Spike 0001 walking skeleton (adds a real actor
  registry + contributor-set/responsibility + a minimal `submit_event` + a Python agent stand-in). Pass/fail in §5;
  C1–C5 PASS is the trigger to ratify *two* ADRs — the parked ADR-0011 skill-epoch refinement and an advisory-actor
  integration-contract ADR (promotes ecosystem/0001 from evaluation to decision). Still build-prep; spec unchanged.

---

## Prepared 2026-06-18 — Bet B (the Pi compute-cost spike): field runbook + self-describing, floor-finding harness

Build-prep, **not** architecture — spec/ADR log unchanged (**v0.26**). The user has a **Pi 5 / 16 GB + 1 TB
SSD** in hand and wants to run **Bet B** (the [ADR-0001](spec/decisions/0001-fat-postgres-thin-daemon.md)
projection/keystore compute go/no-go, [Spike 0001 §6](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#6-bet-b--projection--keystore-cost-on-the-pi-prepared-awaiting-the-board)).
The harness was already green on x86; this session made the run **reproducible and its numbers
trustworthy on real hardware**, with **zero change to the safety-critical Rust** (the §9 blast-radius
rule — all of it is fit-for-purpose Python in `poc/walking-skeleton/harness/`). `cargo test --release` 6/6.

- **Field runbook — [`poc/walking-skeleton/PI-RUNBOOK.md`](../poc/walking-skeleton/PI-RUNBOOK.md)** (the
  main deliverable). Start-to-finish for when the board comes out of the drawer: **PGDATA on the SSD, never
  the SD card** (the one Pi mistake that silently invalidates B1/B2), **PostgreSQL 18 on arm64** via PGDG,
  the **`performance` governor + active cooling + `vcgencmd get_throttled`** check, **deployment-honest PG
  tuning** (`fsync`/`synchronous_commit` left **on** — a clinic node must survive power loss), the release
  build on the Pi, the prescribed run, and how to read the result.
- **Self-describing harness.** Every `bench_b.py` run now prints (and `--json-out` records) an environment
  header: board, cores/RAM, kernel, PG version, **which block device PGDATA sits on** (shouts if SD card),
  CPU governor, Pi throttle state, build profile — and warns on anything skewing the result. Rationale: Bet B
  is a *hardware-class* bet, so a §6 number is meaningless without its host — concretely, the *same* release
  binary measured SHA-256 at ~1500 MB/s on a SHA-NI host and **~200 MB/s on this container** (validated live).
- **The floor question answerable from one board.** Each gated row prints **headroom** (× under budget / over
  floor). On the x86 validation run B1 landed **18× under budget**, B2 **207× under**, B4 **~11× over floor** —
  so the smallest-headroom row (expected **B4** raw crypto, which tracks clock/core not tuning) sets the floor,
  and the Pi 5 run predicts whether a *smaller* board is viable before it's plugged in. New flags: `--label`
  (tag the board), `--json-out` (durable artifact), `--patients` (realistic B2 chart size = ~count/patients).
- **Floor experiment = Pi 4 / 8 GB** (confirmed with the user — they keep several Pi generations on hand; the
  earlier "8 GB Pi 3" was a slip, a Pi 3 tops out at 1 GB). Older, cheaper hardware — the interesting "does it
  still clear the gates" test; it changes only the `--label`, not the procedure.
- **Docs wired:** Spike 0001 status line + new **§6.1** (preparation/floor methodology); skeleton README Bet B
  section + Next pointer; this entry + the build-prep pointer below. **Validated on the container** (PG 16
  here — harness flags it as < the PG 18 floor): `bench` and full `selftest` both run green end-to-end, env
  capture degrades gracefully off-Pi (no governor file / no `vcgencmd` → "n/a"), `--json-out` writes a clean
  record. The PG-18-on-Pi numbers are the real ones, recorded when the board runs.

---

## Resolved 2026-06-17 — hard policy expression (now spec v0.26) → ADR-0024 (refines 0021); **closes the layering/API arc**

Closed the last ADR-0021 follow-on (the user's pick, "3"): *how hard policy is expressed.* It **dissolved into
Cairn's universal shape applied to policy itself** — **no new founding principle** — and **closes the
layering/API arc (0021 → 0022 → 0023 → 0024).** → [ADR-0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md),
canonical home **[security §7.9](spec/security.md)**, with back-pointers from §9.5/§9.6, identity §5.10, the §7
intro, index principle 9, and the open-questions resolved area.

- **Hard policy is an append-only, signed, scoped policy-assertion stream + an effective-policy projection** —
  the §5.9 sensitivity / §3.11 routing / §5.1 link-graph shape applied to policy; **not a mutable config
  file**. Every policy act is an audited event (who/when/scope/authority/selection), never mutated, always
  overlaid. Makes ADR-0010's "explicit audited configuration act" concrete.
- **Declarative selection over a closed Cairn-shipped mechanism set, NEVER arbitrary code** (the RCE surface
  ADR-0012 forbids on the data plane). The two-plane split applies: the *selection* is data on the event
  plane; the *evaluation code* travels the §7.6 distribution plane.
- **The user's "DB-anchored config vs role-gated L2" fork dissolves:** same expression, the enforcement
  *locus* is a §9.1 blast-radius call — the effective-policy projection is in-DB and the §9.6 submit surface +
  RLS read it (unbypassable by default); richer evaluation runs in role-gated L2. Mirrors §9.4's
  PL/pgSQL-default / pgrx-escape-hatch split.
- **Authority-gated authoring, bootstrapped at provisioning** (the §7.6 root authority, like the steward key /
  §7.7 self-issued practice key); meta-policy ("changing the retention floor needs two-person authority") is
  the same mechanism on itself. **Scoped + floor-composing:** a federation floor ratchets *stricter, never
  weaker* (the safety-floor-never-relaxes pattern; the policy analogue of the trust anchor), local non-floor
  policy is node-autonomous (sovereignty floor). Partition-honest (last-known policy; local reads never fail
  closed).
- **Unifies the scattered "expressible policy rungs"** (§5.10 attestation, ADR-0005 erasure rungs, ADR-0006
  sensitivity, ADR-0009 routing, ADR-0010 suppression-permission, §7.5/7.6/7.7 who-may-X) under one mechanism —
  the consolidation of an existing scatter, not a new subsystem. Closes ADR-0010's conservation-of-responsibility
  loop.
- **Blast-radius (§9):** the effective-policy projection + the §9.6/RLS gates that read it are safety-critical
  (mis-enforcement → compliance breach *or* care blocked); the **policy-authority model + provisioning
  bootstrap** are the sensitive seam (who changes policy is who can weaken a floor → authority-gated + audited +
  floor-protected). Shipped mechanism set stays **closed + additive-only**. Build `--strict` clean.

---

## Resolved 2026-06-17 — the native API contract: capability description + conformance (now spec v0.25) → ADR-0023 (refines 0021)

Pursued the next ADR-0021 API follow-on (the user's pick, "2"): the **native API contract** — the
capability-negotiation format + the conformance suite that turns the principle-12 compatibility guarantee into
something a small UI team can *run*. **Determined by existing canon** (additive evolution + anti-capture) —
**no new founding principle.** → [ADR-0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md),
canonical home **[language-substrate §9.7](spec/language-substrate.md)**, with a §9.5 pointer and the
resolved-area note in [open-questions.md](spec/open-questions.md).

- **Two framing realizations did the work:** (1) **API compatibility = schema evolution** (ADR-0012) —
  permanent offline version skew — so the primitive is **additive capability flags over a mandatory baseline,
  NOT a monotonic version number** (a number linearizes what is really a *set* of independently-present
  capabilities), with the §3.13 `min()` ladder for degradation; (2) **anti-capture forbids a Cairn-owned
  conformance gatekeeper** → conformance must be **self-runnable + self-verifiable** (the ADR-0014
  signed/content-addressed registry pattern), never a steward-issued certificate.
- **Capability descriptor = a served, self-describing projection of local-node-properties** (installed schema
  versions + loaded validators/extensions + config — all already ADR-0012 local-node-properties; not new
  state), additively evolvable, legible across time, **transport-independent** (operations, not REST endpoints
  — ADR-0021's "properties fixed, transport later").
- **Negotiation = stateless description + client-side graceful degradation, NOT a handshake** (no
  partition-fragile round-trip; availability). Degradation may cut *experience*, **never correctness/safety** —
  the mandatory core IS the floor, so the §5.9 safety projection is present on every conformant node.
- **Conformance suite = the executable contract, two faces:** *wire/node* (correct L0 participation — the "any
  node talks to any node" guarantee made checkable; a federation admission **technical** gate, distinct from
  ADR-0017's trust gate) and *API* (L2 honors the contract for advertised capabilities; capability-partitioned;
  additively versioned; **tests never removed**). Self-runnable, signed, content-addressed; **anti-capture
  turned inward a second time** (ADR-0021 denied the steward's UI a private API; this denies a conformance
  chokepoint); doubles as the spec's executable form (principle 11).
- **Blast-radius / the bet:** the load-bearing call is the **mandatory core** — small enough for a Pi to fully
  conform, rich enough that "conformant" means something to a UI (the ADR-0001 cost tension, now for the
  contract). New artifacts (core definition + capability taxonomy + the suite) must be maintained **additively**
  (never drop a test). Build `--strict` clean.
- **The API thread now has one follow-on left:** **(3)** *how* hard policy is expressed (DB-anchored config vs
  role-gated L2 — the §5.10 expressible-policy rung).

---

## Resolved 2026-06-17 — the validated submit surface (now spec v0.24) → ADR-0022 (refines 0021)

Pursued the sharpest ADR-0021 follow-on (the user's pick, "1"): the **validated submit-function surface**
whose *completeness* is the bet ADR-0021's "floor in the DB" rests on. It **dissolved into already-decided
primitives** (append-only + the accumulated write-time seams) — **no new founding principle.**
→ [ADR-0022](spec/decisions/0022-validated-submit-surface-the-write-path.md), canonical home
**[language-substrate §9.6](spec/language-substrate.md)** (the authoring counterpart to the §9.4 apply
boundary), with a back-pointer from data-model §3.1 and the resolved-area note in [open-questions.md](spec/open-questions.md).

- **The completeness/minimality paradox** (smallest audited surface vs every-write-expressible) dissolves
  because the system is **append-only**: *almost every write is the same operation.* So the surface is
  **one generic `submit_event`** (type-validated by **dispatch to additively-registered validators** — a
  new event type adds a validator *behind the same door*, never a new function; ADR-0012 discipline) **+ a
  small closed set of non-append operations** (erasure/key-custody ADR-0005, author-scoped export ADR-0019,
  blob byte-tier put §6.6). Small AND complete by construction. Co-produced events (the ADR-0020 order +
  note-line) commit atomically; drafts (§3.10) are adjacent local mutable state, never on the log.
- **`submit_event` is the in-DB convergence of every write-time seam** the spec had named — authorship
  stamp (ADR-0008), clash detection (ADR-0003), seal-time safety projection (§5.9), suppressing owner-gate
  (ADR-0010), legibility-twin derivation (ADR-0012), canonicalize+sign (ADR-0011/0015) — run atomically in
  one pipeline (attestation-token → authz → envelope+Tier-1 ceiling → body+Tier-2 clash → hard-policy gates
  → canonicalize+twin+sign → idempotent append). **Not a new mechanism — where they all land.**
- **A real consequence of ADR-0021's floor-in-DB, now stated:** signing **must be reachable from the in-DB
  path** (else a direct-DB caller couldn't produce a signed event and the floor would be incomplete) → the
  node's trusted base **includes the database process** (fat Postgres); signing is in-DB (pgrx + keystore)
  by default or delegated to a co-located signer the in-DB submit invokes, **never L2-only**.
- **Authoring ≠ applying:** `submit_event` mints+validates+signs+appends; the §6.1/§9.4 apply path verifies
  peer signatures + idempotent-appends, **never re-signs**. The **author-attestation token** (ADR-0008,
  never the DB session) is what stops a direct-DB client forging authorship.
- **Blast-radius (§9):** that one in-DB function becomes the **most safety-critical code in the system** →
  reviewer-legible, the prime pgrx candidate (ADR-0002); the attestation-token verification + in-DB
  signer/keystore access are the two sub-seams; the validator-dispatch registry must itself be additive-only
  + tamper-evident. **The bet (refining ADR-0021's):** the generic-append + closed-non-append set stays
  complete as the clinical model grows (a future write that is neither would force a new door — itself a
  signal worth scrutinising). Build `--strict` clean.

---

## Resolved 2026-06-17 — layering, the node API & UI pluralism (now spec v0.23) → ADR-0021 + founding principle 12

Case-mined the **application-layer / API architecture** — the user's framing: fat-Postgres core + Rust/PL-pgSQL,
**hard policy** in a thin Rust layer, **soft policy** in the UI, UIs reaching an API *or the DB directly*, a
baseline "best-of-breed" reference UI — all in service of **UI diversity without ever compromising inter-node
compatibility** ("regardless of UI/policy, any Cairn node must talk to any other"). It mostly **dissolved into
already-fixed primitives** + one bypass-tension resolution, and **elevated a new founding principle (12).**
→ [ADR-0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md), canonical home
**[language-substrate §9.5](spec/language-substrate.md)**, principle in **[index.md](spec/index.md)** + `CLAUDE.md`,
new resolved area in [open-questions.md](spec/open-questions.md), with a topology note + back-pointers from §9.3 /
data-model §3.4.

- **The reframe that did the work:** "any node talks to any other regardless of UI/policy" is **already
  guaranteed** — the compatibility contract is the signed **event core** (ADR-0015 format / ADR-0001+§6 sync /
  §5.7+§3.12 algebras / ADR-0012 additive evolution / ADR-0017 federation), all UI/policy-independent by
  construction. So the task was to **name that core as the sole inter-node contract and forbid everything above
  it from the inter-node path**, not to *build* compatibility.
- **Four layers, compatibility boundary below the application layer:** L0 wire/event core (uniform) · L1 node
  enforcement floor (fat Postgres + in-DB/pgrx, uniform, unbypassable) · L2 policy + API (thin Rust, plural) ·
  L3 UI (plural; reference UI is one citizen).
- **The bypass tension resolved — floor in the DB (the user's call, fork 1).** Every safety/compatibility
  invariant is enforced in-DB (validated submit functions + RLS + constraints), so **direct DB access is safe by
  construction**; UIs call submit-functions, never raw `INSERT` (the §9.4 grant model extended to the UI role);
  *"via API vs DB directly" is a privilege gradient, not a contradiction* — L2 is ergonomics + deployment hard
  policy, **never the sole wall**.
- **Hard vs soft policy** = the §9.1 blast-radius rule applied to policy (hard = DB-anchored or role-gated;
  soft = UI, swappable, zero blast radius).
- **Anti-drift guarantee (the core ask):** a UI is a pure producer/consumer over a contract it can't alter (the
  *node* owns serialization/signing), the native API evolves **additively** (principle 11 on the contract), is
  **capability-described** (graceful degradation, the §3.13 `min()` ladder) + **conformance-tested** — so a
  bespoke UI can produce wrong-for-its-clinic content but **never a wire-incompatible event**. **Native API ≠
  FHIR façade** (two surfaces); the reference UI is built only on the public API (anti-capture turned inward).
- **Founding principle 12 — *uniform core, plural edges*** (the user's call, fork 2): compatibility is a property
  of the event core, below UI and policy; many front-ends, one record.
- **Blast-radius (§9):** the validated submit-function surface + RLS + role/grant model are now the trusted base
  for *external* clients too (they **are** the floor for direct-DB callers); completeness of that submit surface
  is the bet (a gap pushes UIs to raw access and re-opens the bypass). Transport (REST/gRPC/…) deliberately left
  open — §9 fixes the rule, not the tech. Fresh user case-mining (not a parked §11 item). Build `--strict` clean.

---

## Resolved 2026-06-17 — the active-write model promoted to canon (now spec v0.22) → ADR-0020

Promoted the **write-model / UI design cluster** — resolved in conversation across PRs #15–#17, captured in
`scratch/ui-sketches/`, but never in canon — into the spec + ADR log. It dissolved into existing primitives
+ one principle-3 reconciliation: **no new envelope field, no new event stream, no new founding principle.**
→ [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md),
canonical home **[data-model §3.15](spec/data-model.md)**, with the forced-rationale gate in
**[vision §1.2](spec/vision.md)** and back-pointers from data-model §3.5/§3.10/§3.13 and identity §5.11.

- **Same one-word-hides-many-dials motif** that resolved "scope"/"signature"/"authentication"/"priority",
  now applied to the write surface — "encounter", "the order's consult", "the note line", "delete".
- **Thin encounter / context-entity.** `encounter` is an opaque grouping id that asserts **nothing** about
  formality (a 5-second results-review with one annotated order is a first-class "virtual encounter"); a small
  first-class header `{HLC time, place, contributor set, ≥1 events}`; author **may be non-human** (a recall
  system spawns it). **Guard the prose against importing FHIR-`Encounter`/billing semantics — grouping id,
  full stop.** It rides on the ADR-0008 armed write-context; events inherit it ambiently like
  facility/department — no new field.
- **Order provenance falls out of the encounter key** — `result → order → order.encounter → fold`; the
  external-results gap is structurally explained + **honestly degraded** (labelled most-recent fallback,
  never silently "the ordering consult"). AI cross-ref only *proposes* a link (overlay discipline).
- **Type-through write model** (`rx!`/`tx!`+tab, non-modal side panel; smart-default-vs-**forced-manual**
  dosing for paediatric/pregnant/breastfeeding/renal/hepatic). The readable note line is a **derived
  projection of the one structured event = the §3.13 legibility twin rendered inline, born at authoring
  time** — so the "two artifacts diverge" worry dissolves at the root (principle 11 at the point of authoring).
- **Delete-vs-erase taxonomy** (conventional EHRs conflate them): **delete** = suppress a *rendering*
  (visibility overlay, reversible, **zero friction**, routine) vs **erase** = crypto-shred the *data*
  (ADR-0005, irreversible, ≈never). *"Delete only ever removes one UI aspect, never the original data"* =
  never-erase-always-overlay (principle 2) applied to the **display layer**. Suppression is itself a recorded
  visibility-overlay event (the *that*; the *why* may stay unstated). Slots under ADR-0006 (confidentiality in
  presentation, never existence — the STI-screen case).
- **Forced-rationale gate ≠ banned confirmation dialog** — a reconciliation of principle 3. Confirmation
  dialogs stay banned (click-through fatigue); but the genuinely irreversible few (erase, repudiation) earn a
  **forced-rationale** gate that demands a substantive recorded reason (can't be click-throughed). Because
  overlay makes almost everything reversible, the modal-worthy set collapses to ~1–2×/yr — rarity preserves
  the signal. Rule: **never block the reversible; for the irreversible few, don't confirm — demand a reason
  and record it.** Canonical home vision §1.2.
- **Blast-radius (§9):** the thin-encounter grouping + type-through state machine + forced-manual rule table
  are fit-for-purpose; the **delete-is-never-erase** boundary and the **suppression-is-always-a-recorded-
  overlay-event** invariant are safety/privacy-critical (trusted apply surface — the recurring seam motif).
- **Still in `scratch/`, gated on next-week easyGP schema access (build-prep, intentionally NOT promoted):**
  the `rx!`/`tx!` parser + type-through state machine port, the formulation/drug data source + forced-manual
  rule table, and the **prefetch/materialization warming daemon** internals (validates ADR-0001 from
  production; splits into *scavengeable mechanism* vs *swappable prediction policy*). See
  `scratch/ui-sketches/easygp-prefetch-notes.md` (banner added pointing at the promoted canon).
- **Why the previous handover missed this:** the whole UI/write-model thread — wireframes under
  `scratch/ui-sketches/`, the `web/` landing-page + chart examples — ran across **PRs #15–#17 *after* the
  2026-06-16 handover was last regenerated**, so it was unreflected until now. Build verified `--strict` clean.

---

## Resolved 2026-06-16 — federation revocation + author-scoped export (now spec v0.21) → ADR-0018, ADR-0019

Pressure-tested ADR-0017 with adversarial cases (the user's bad-actor revocation case + generated follow-ons),
then folded the harvest into two ADRs. Both **dissolved** — **no new founding principle.**

- **[ADR-0018](spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md) (refines 0017),
  canonical [security §7.7](spec/security.md) revocation block.** The struck-off-operator-with-subsidiaries
  case sharpened seven properties: (1) **enforced by counterparties, never the revoked node** (the enforceable
  boundary is the honest set); (2) **forward-distrust, not retroactive erasure** (events authored while
  credentialed stay, marked; later ones refused); (3) **cascade over the issuance/affiliation graph — revoke
  the principal, not the key** (by chain + a new additive **controlling-entity/`on_behalf_of` credential
  attribute** fed to the contamination cascade; issuance checks principal status → no whack-a-mole); (4)
  **anchor revocation (bidirectional, cascades) ≠ voluntary unpeering (unilateral, local)**; (5) **the anchor
  is a position of power** — anti-capture turned inward: minimise blast radius (sovereignty floor, multi-anchor
  default — *never mandate a single anchor*, audited signed revocation, availability floor), but never prevent
  *legitimate* exclusion (the deepest pressure-test finding — the captured-registry kill-switch); (6)
  **partition-honest** with a **local-read-never-fails-closed** freshness knob; (7) **one credential per
  accountable principal** (granularity guidance). **Clawback of already-synced data = authorities' matter, not
  Cairn's** (honest ceiling).
- **[ADR-0019](spec/decisions/0019-author-scoped-record-export-the-medico-legal-copy.md) (refines 0007),
  canonical [security §7.8](spec/security.md).** From the roaming-locum case: a clinician's records are their
  **sole litigation defence decades on**, and per-workplace loss risk **compounds across a portfolio career**.
  So a first-class, **audited** export selected by **contributor identity**, **strictly author-scoped** (the
  user's ruling: progress notes, path/imaging *requests*, referrals — the reasoning + actioning; **not**
  results — results are a separable practice-custodianship duty the clinician enforces by delivery/re-litigation).
  **Self-verifying + legible-across-time** (signed bytes + plaintext twins → court-admissible 20 yrs on,
  tamper-evident, schema-drift-proof). Export is an append-only audit event recording **blast radius**
  (the user's requirement). **Seal mode = policy-neutral key-custody ladder** (author-readable /
  authority-public-key-sealed / both). It is the **general mechanism behind ADR-0005 rung-2's escrowed clinician
  copy**; the erasure interaction is the intended honest ceiling. Pleasing fit: Cairn's append-only signed
  design is almost purpose-built for "my records are my defence."
- **Pressure-test cases run (all dissolved):** whack-a-mole re-enrolment, captured/compromised registry (the
  deep one → principle-level "anchor is power"), anchor-revocation-vs-unpeering, roaming-locum (clinician
  identity is itself a §5 claim; cross-federation linkage is link-events, no auto-propagating revocation),
  shared-node collateral (granularity guidance), mid-sync atomicity, insider-races-revocation (audited, not
  preventable), reinstatement-on-appeal (new overlay), cross-jurisdiction anchors (mutual recognition = policy),
  feed forgery/DoS (signature + availability floor).
- **Blast-radius (§9):** revocation checking + the controlling-entity cascade seam (ADR-0018) and the
  export predicate+seal+audit-emit seam (ADR-0019) are safety/privacy-critical; UI/packaging fit-for-purpose.

---

## Resolved 2026-06-16 — Custodian & Federation Admission (spec v0.20) → ADR-0017

Drafted the spec dependency ADR-0016 surfaced, **same session, while the memory was fresh.** It dissolved into
existing primitives + one operational corollary — **no new founding principle.** → [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md),
canonical home **[security §7.7](spec/security.md)**, with back-pointers from [topology §2](spec/topology.md)
and the §7 intro mTLS bullet; the [open-questions.md](spec/open-questions.md) item is struck resolved.

- **The user's governing principle (verbatim intent):** a single node needs **no permission** as long as it
  doesn't talk to others (works out of the box, zero data); once two nodes want to talk they **negotiate who
  may access what**; a private practice may build **its own node network with no third-party authority** (no
  capture) **but must set its own join rules**; a national system **ideally runs a registry**; the
  infrastructure must serve that **whole spectrum with least friction.**
- **The elegant result — it's mostly the §7.5 actor registry + §7.6 ceremony applied to node-to-node
  relationships, not a new subsystem.** A node is a `device`-kind actor with a self-generated signing identity;
  peering is the closed actor-event algebra (peer/supersede/revoke) appended to a trust set; revocation reuses
  the contamination cascade.
- **Three rulings carry it:** (1) **the sovereignty floor** — permission is a property of inter-node
  *relationships*, never of a node's right to run (corollary of paper-parity + availability + anti-capture;
  default deny-all peering). (2) **Pluggable, self-hostable trust anchors are the spectrum knob** (fractal
  topology applied to trust): no-anchor pairwise → the practice's own issuing key → a national registry **as a
  node role**, one verification mechanism, **no Cairn-owned root**. (3) **Admission gates the outer boundary
  only** — *peered ≠ may-see-everything*; intra-federation confidentiality stays ADR-0006 key-custody +
  visibility (don't re-introduce replication-as-access-control).
- **The custodian contract** = signed, verifiable metadata bound to the credential; Cairn ships
  format/verification/revocation, **legal force is jurisdiction** (the ADR-0007 records-the-chain posture). Solo
  practice self-issues. Verification is **offline-capable**; revocation is an **honestly-stale signed feed**
  (§6.2). Onboarding reuses the §5.11 possession gesture (low-time, high-distinctiveness, no mandatory cloud).
- **Blast-radius (§9):** credential/signature verification + peering gate + anchor evaluation + revocation
  checking are safety-critical (in-DB/Rust, beside the §7.5 registry); issuance UI / contract tooling /
  onboarding wizards are fit-for-purpose; the *verified credential → admitted peer* seam is the one
  safety-critical path (the recurring seam motif).

---

## Resolved 2026-06-16 — national-scale record discovery (spec v0.19) → ADR-0016

Case-mined a **new** problem (not in the original §11 set): at national scale **no node holds the whole
population's records**; a patient new to a region presents at a small under-resourced clinic — how does it
discover a record exists elsewhere, and request it? It dissolved into existing primitives + **one new
replication tier**, **no new founding principle**. → [ADR-0016](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md),
canonical homes **[sync §6.7](spec/sync.md)** (the tier + discovery mechanism) and
**[identity §5.2](spec/identity.md)** (discovery feeds the matcher; national ID as accelerator), with the
confidential-essential composition in **[identity §5.9](spec/identity.md)**.

- **The gap:** §5.2's *"match at the lowest tier that sees both registrations"* breaks when the lowest
  common ancestor is *the nation* — which can be neither a fat index the clinic queries nor one it can hold.
  The conventional fix (a national **Master Patient Index**) is the surveillance/lock-in capture surface
  principle 7 forbids.
- **Two phases of opposite character:** *identity* discovery is irreducibly the matcher's fuzzy problem
  (**you cannot content-address a human** — why ADR-0013's content-addressing doesn't solve it); *part/locator*
  discovery (UUID → which nodes hold its events) **is** content-addressable, the §6.6 swarm-fetch shape.
- **The resolution (the user's keystone intuition, validated by the math):** a **replicated essential-state
  tier** — a tiny, replicate-to-all-*federated* projection of each person's essential safety set (demographics,
  active allergies, active meds, problem list, code-status, care pointer) + a blocking-key summary on every
  node. Discovery becomes a **local matcher query** — offline, partition-proof, *no broadcast of who is being
  sought*. Hit → middle band → human → `link` → §6.4 lazy acquisition of the full record. ADR-0013's
  **reference-eager/byte-lazy** generalized from attachments to **patient existence**.
- **The footgun the research caught + nailed in the spec:** the essential tier carries **current state, not
  transaction history** (~77 % of dispensed items are repeats that don't change the list). Dispensing history,
  observations, labs, notes stay in the scoped/lazy full record. This is the line that keeps it affordable.
- **"Essential" is a graded, multi-source, append-only flag, not a fixed list** (the user's *Sildenafil* case:
  privacy-sensitive, sporadic, undisclosed, but lethal-with-nitrates). Policy default pre-label pack + any
  accountable contributor may tag; *when unsure, err toward essential* (principle 4). **Confidential ∧ essential
  composes with the §5.9 safety projection:** the **de-identified projection** (interaction class + severity,
  naming nothing) replicates broadly and is itself the actionable fact; the **identified body stays sealed**
  behind break-glass — patient kept safe without being outed and without point-of-care disclosure.
- **National/memorable ID (Norway *personnummer*) = deterministic accelerator, never a dependency** (the user:
  patient-carried tokens fail in practice — forgotten cards, failed logins). Patient-carried token optional.
- **Sizing validated by a 5-angle deep-research pass** (PubMed + national stats; full numbers in
  [ADR-0016 §8](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md)): essential set
  **~25 KB/person → ~2.5 TB for 100 M** (range 1.2–5 TB; a commodity 4 TB SSD), discovery summary ~0.4–1.6 GB;
  churn **~5–10 essential events/person/yr → ~75–150 kbit/s per full-mirror node ≈ ~1 % of a mediocre
  Starlink** (1–2 orders of headroom even at a sicker ~25/yr). Anchored on the spike's measured ~494 B/event,
  England NHSBSA (~21 items/yr, ~77 % repeats), Scottish/Swedish polypharmacy registries (~40 % on zero meds),
  Barnett 2012 multimorbidity, Zhou 2016 allergies (n≈1.77M), US MEPS utilisation skew.
- **New open item surfaced (a hard dependency):** **Custodian & Federation Admission** — a separate
  governance/security spec. Replicating a nation's essential set is lawful only because every holding node is a
  **contracted, accountable custodian** (proof of health-system participation + enforceable privacy contract to
  join the mesh; else isolated). This bounds the unavoidable existence-disclosure to vetted custodians at
  **region** granularity. Logged in [open-questions.md](spec/open-questions.md).
- **Blast-radius (§9):** the summary build + local matcher query + ranking are fit-for-purpose (advisory); the
  essential-tier replication predicate + **current-state projection seam** + the essential-flag→safety-projection
  seam + federation-admission credential verification are safety-critical (the recurring seam motif).

---

## Read these first (the durable state)

The real project state lives in these documents. This handover points at them; it does not
restate them. Repository layout:

- **`docs/spec/`** — the canonical architecture spec, **one file per aspect**. Start at
  **`docs/spec/index.md`** (mission prose + document map), then read aspect files / jump via the map.
  - **`docs/spec/decisions/`** — the **ADR log**: the *why* behind settled decisions. Numbered,
    dated, **immutable** (reversal = a new superseding ADR). **Read the relevant ADR before
    reopening a settled question.** Pre-ADR history (v0.1→v0.6 changelogs) preserved in
    `decisions/0000-pre-adr-changelog-v0.1-v0.6.md`.
  - No filename version suffixes / in-file changelogs; git is the line history; spec version in
    `index.md`. HTML is generated, not committed:
    `uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build` (config `mkdocs.yml`).
- **`docs/principles/`** — statements of project principle / governance.
  - `STEWARDSHIP-OF-THE-NAME.md` — the "name belongs to the mission" governance commitment.
- Root **`README.md`** — mission, founding principles, eventual goal, project identity (GitHub
  shopfront; the same mission prose also lives canonically in `docs/spec/index.md`).

Everything below is the stuff that lives *between* those documents and would otherwise be lost.

---

## Drafted 2026-06-16 — first implementation spike (build-prep; spec version unchanged at v0.17)

The architecture backlog being empty, started the **build-prep** thread. New area **`docs/spikes/`**
(build-prep is neither architecture nor an ADR — kept separate so the spec stays a clean *what* and the
ADR log a clean *why*), with the first task **[Spike 0001](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)**
and a [spikes/README.md](spikes/README.md) index. Added to `mkdocs.yml` nav; builds `--strict` clean.

- **Reframed "the Pi-benchmark spike" into two separate bets.** The user's available test rig (MacBook in
  Cape York/Bamaga on Starlink-mini ↔ DGX Spark in Dorrigo NSW on Starlink, over WireGuard) does **not**
  stress the ADR-0001 compute bet — both machines are fast. It stresses a *different, more fundamental*
  bet: **sync convergence + partition behaviour + bandwidth economy over a real adverse WAN** (design-
  validity, hard to retrofit). The Pi compute-cost bet (the documented go/no-go) waits for next week's
  real Pi-class node. Both ride **one shared walking skeleton**, built once.
- **The serialization/signature question the user asked me to weigh.** The load-bearing answer is **three
  structural moves**, not a cleverer primitive: (1) **sign the stored bytes, parse a view, never
  re-serialize** (shrinks the determinism/safety surface — already implied by §3.13/§3.14); (2)
  **algorithm-tagged, self-describing digests + signatures** (the day-one choice is reversible by policy);
  (3) **re-attestation is an overlay** (the append-only model migrates its own crypto — defers PQC safely).
  With those, the **tagged, migratable defaults**: deterministic-CBOR **COSE_Sign1** + **Ed25519**
  (aligns with the WireGuard transport; freed from JSON because principle 11's plaintext twin owns human-
  legibility) + **SHA-256** event digest. **BLAKE3 for blobs** (the user's explicit call) — its Merkle-tree
  structure fits the ADR-0013 chunked/preemptible/resumable/swarm byte tier (independent chunk verification,
  fast on ARM). Honest dismissals recorded (BLS, ML-DSA/SLH-DSA-now, RSA/P-256, Protobuf/Avro-for-signed-form).
- **Not yet ratified.** These are *validate-then-ratify* defaults — the spike is how we learn if they hold.
  The serialization/signature **ADR is written after the spike**, citing its results (Spike 0001 §7 exit
  criteria). Spec version intentionally **not** bumped: no §1–11 aspect or ADR changed yet.

### Built + validated 2026-06-16 — the walking skeleton (first code in the repo)

**[`poc/walking-skeleton/`](../poc/walking-skeleton/)** — Rust + SQL, sibling to the existing Python
`poc/replication-failover`. This is the §3 shared prerequisite for both spike bets, and (Spike 0001 §7)
the **seed of the real implementation**. Faithful to the §9 blast-radius rule: signed envelope +
content-address invariant + trigger-maintained projection in-DB/SQL; canonical-bytes/COSE_Sign1/Ed25519/
multihash/BLAKE3 + the thin set-union ship-apply daemon in Rust (no merge logic).

- **It compiles and runs end-to-end** — proven live on a real PostgreSQL (PG16 here; SQL uses no 18-only
  syntax, UUIDv7 minted in Rust): schema load · the in-DB **content-address CHECK rejects a tampered row** ·
  sign → wire → **verify-on-apply** · bidirectional **set-union convergence to an identical event set + HLC
  order** · idempotent re-pull · **watermark-0 re-pull still converges** (hint, not authority — ADR-0004) ·
  correct projection under **out-of-order** apply · **BLAKE3 lazy blob fetch + verification**. `cargo test`
  green (incl. the Bet-A2 round-trip/tamper test), clippy clean.
- **Two real bugs found and fixed by running it** (the value of building, not just specifying): a
  NULL-safe projection winner-comparison (a node that writes a note *before* the patient arrives), and
  param-type binding (`$n::text::uuid/jsonb`, int4 chunk offsets).
- **Stubs are documented** (README "what it proves / deliberately stubs"): key trust = embedded key (not yet
  the ADR-0011 registry); change-capture = watermark-pull (not yet logical decoding); verify-in-applier (not
  yet the in-DB pgrx gate); inline BYTEA blobs. None change either bet.
- **Bet A measurement harness — built + green** (`poc/walking-skeleton/harness/bet_a.py`, stdlib-only).
  Added `gen` / `fingerprint` / `pull --metrics` to the daemon; the harness emits the §5 PASS/FAIL table
  directly. `selftest` passes all six rows on real PG; A4 reads **563ms base → 551ms during** (fixed-batch
  per-sample work so it's a like-for-like comparison — single-box validates mechanics; the real A4 contention
  is on the link). One subtlety found + fixed: a free-running generator made the A4 baseline a misleading
  backlog drain (23s); switched to drain-then-fixed-batch sampling.
### Run 2026-06-16 — Bet A concluded on the real Cape York ↔ Dorrigo link — **ALL SIX ROWS PASS + a real bug fixed** (spike §8)

The lunchtime run is **done**: ran the §5 table over the actual link — this MacBook (Cape York, WG `10.0.0.2`,
PG16) ↔ the **DGX Spark** (Dorrigo, WG `10.0.0.3`, a *user-owned* **PG18.4** instance on :5444, no sudo) — via
the **canonical field path added by PR #8** (`cairn-sync run` per node → `bet_a.py analyze`/`report`). The
field-readiness session's predicted fix was right: **bind `--listen` to the WireGuard address, not
`127.0.0.1`** (so the peer can reach you); that was the whole "minor issue."

- **The link is a genuine ~710 ms-RTT satellite path** (`ping` 667/710/775 ms, with loss) — real adverse-WAN.
- **A1** both nodes **792 events, event + projection hash identical**; conflicting shared-patient overlay →
  **same winner both sides** (deterministic HLC tie-break). **A2** 0 verify-failures, both directions. **A3**
  HLC merged past every event; max gap reported (35 s/42 s), never auto-resolved. **A5** **494–495 B/event**
  (budget 4096). **A6** referenced-but-unfetched blob shown as not-present. **A4** see the bug below.
- **The link surfaced a real availability-floor bug — now fixed (spike §8.1).** The field `run` loop fetched
  blobs **inline in the clinical pull cycle**; on the 710 ms link a single 2 MB blob's ~32 sequential RTTs
  **head-of-line-blocked Cape York's whole 150 s run** — 1 cycle, **0 clinical pulls**, never converged
  (396 vs 792) while its serve thread still fed Dorrigo. That's exactly the ADR-0013 failure (byte transfer
  starving clinical availability — the Kimberley nightly-imaging case in miniature). **Fix in this change:**
  the lazy byte tier now runs on **its own thread** in `cairn-sync run` (like the serve thread), so it can
  never block clinical sync. Re-run: Cape York did **30 cycles + full convergence** while the blob fetched
  lazily. `cargo test` + `clippy` green.
- **Deferred byte-tier findings (spike §8.2, both ADR-0013-mandated):** `do_blobd` is still (1) **synchronous
  one-RTT-per-64 KiB-chunk** (latency binds: a 64 MB blob ≈ 12 min of pure RTT — the real tier must
  pipeline/window/swarm) and (2) **not resumable across passes** (restarts from offset 0 on any drop — the
  2 MB blob was still `referenced-only` at the 150 s cutoff). The thread move is the *availability* fix;
  pipelining + resumability is the *throughput* fix, left for the production byte tier.
- **Merge note:** my interim SSH-orchestrated driver `bet_a_wan.py` was **dropped** — superseded by PR #8's
  `run`/`analyze` (the blessed field path). The set-union/signature/HLC/bytes verdicts were first seen on
  that driver (442 events) and re-confirmed here on the canonical path.
- **DGX left provisioned** for re-runs / Bet B: rustup + `~/cairn-skeleton` + the :5444 PG instance
  (`/usr/lib/postgresql/18/bin/pg_ctl -D ~/cairn-pg -o "-p 5444 ..." start`). All user-local, nothing
  system-wide.

- **§4 primitives ratified 2026-06-16 → [ADR-0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)**
  (spec v0.18). Fixes the three structural moves (sign-the-stored-bytes / algorithm-tagged digests+sigs /
  re-attestation-as-overlay) + the day-one defaults: **deterministic-CBOR COSE_Sign1 + Ed25519 + SHA-256**
  events, **BLAKE3 blobs (held *provisional* pending Bet B's ARM throughput number)**, FROST earmarked for
  steward keys. Cites the Bet A evidence (A2 zero verify-fails, A5 ~494 B/event, A1 content-address
  convergence). Back-pointers added from data-model §3.5/§3.14; no new founding principle.
- **Bet B harness — BUILT + green 2026-06-16** (`poc/walking-skeleton/harness/bench_b.py`, stdlib-only,
  release-binary required). Daemon grew `bench-insert` (B1 maintained-write latency), `chart` (B2 full chart
  assembly from projection + plaintext twins), `bench` (B3/B4 pure-CPU crypto: Ed25519, SHA-256-vs-BLAKE3,
  DEK-wrap/body-seal — `chacha20poly1305` added). Validated on the container (x86, *not* a Pi): **B1 stays
  flat — p95 ~4 ms, growth ×1.15 across an 8× log-size jump** (the ADR-0001 bet); B2 chart read ~4 ms p95
  over 342 notes; B4 **BLAKE3 6279 vs SHA-256 1489 MB/s → BLAKE3 faster, ADR-0015 provisional blob-digest
  default holds** (x86 with SHA-NI; the *real* check is the Pi/ARM number). Prints the ADR-0002 mitigation
  ladder on a miss. `cargo test`/`clippy` green.
- **Next:** run `bench_b.py selftest` **on a Pi-5-class node** (the only place the numbers are real) — that
  ARM SHA-256-vs-BLAKE3 number is the one input that could revisit ADR-0015's provisional blob-digest line.
  Then the byte-tier **throughput** work (§8.2: pipelined/windowed + resumable/swarm fetch — the availability
  fix shipped in #9; this is the throughput fix).

### Byte-tier throughput (§8.2) — SHIPPED in PR #12 + post-merge review hardening 2026-06-16

**PR #12 closed §8.2** (the throughput half; the availability half was #9): `do_blobd` went from a
synchronous one-RTT-per-64 KiB stub to **windowed** (worker pool, `--window N` ≤16) + **resumable**
(verified slices persist in a new `blob_chunk` set-union table; a restart fetches only the missing indexes)
+ **multi-source swarm** (`--blob-peer` repeatable, per-slice failover) + **per-slice BLAKE3 verified** via
the `bao` crate (`cairn-event::verify_slice` — the single trust seam; a lying source is rejected per-slice
and healed by another). Own thread + clamped window + preemptible budget preserve the #9 availability floor.

**A retrospective review of #12 found no correctness blockers** (re-ran `cargo test` 6/6 incl. the 3
adversarial verify-slice cases, `clippy -D warnings` clean). The actionable findings were addressed
**post-merge on the branch** (this session), all validated end-to-end with `bench_blob.py selftest` on the
local 3-DB rig (T1–T5 PASS over the new protocol):

- **Byte tier now ships raw binary slice frames** (`[found:u8][total_len:u64 BE][slice…]`), not hex. Hex
  doubled every transferred byte — a real artifact for a *throughput* measurement; removing it before the
  WAN run means the recorded MB/s and chosen `SLICE_BYTES`/`--window` are tuned against the real wire, not
  an encoding we'd delete. The clinical plane stays JSON (small, latency-bound).
- **`verify_slice` failures now return a dedicated `EventError::BlobVerify`** instead of reusing
  `BadSignature` (a slice integrity failure isn't a signature failure — clearer logs/metrics).
- **Added a wrong-claimed-length adversarial test** (offset and bytes were already covered; length wasn't).
- **Documented the skeleton artifacts** in the README "deliberately stubs" section: (a) BYTEA storage means
  the **server re-reads the whole blob from PG per slice** — but this is *local* I/O on the serving node,
  **not** over the WAN link being measured (the production object-store tier replaces it with a ranged read);
  (b) the byte tier has **no per-blob authorization** — visibility scope / safety projection / break-glass
  (ADR-0006) are not exercised here, the WireGuard link is the trust boundary; (c) `chunk_index` is i32, so a
  >~549 GB blob would overflow it (far beyond any DICOM study); (d) a single `blobd` call is one pass —
  re-run until `blobs_completed` covers your references, or use `run` (its byte-tier thread loops).

**READY FOR THE SPIKE — the real Cape York ↔ Dorrigo §8.2 throughput run** (user-driven, immediately after
merge). The PR's unchecked box. Field path is the same canonical `cairn-sync run` per node as Bet A
(bind `--listen` to the **WireGuard** address, not `127.0.0.1` — the recurring gotcha). Checklist:
  1. `cargo build --release` on **both** nodes (MacBook Cape York `10.0.0.2`; DGX Spark Dorrigo `10.0.0.3`,
     user PG18 on :5444 — both already provisioned, see the Bet A note).
  2. On a source: `gen-blob --size-mb N` (or `put-blob` a real DICOM); note the printed address + length.
  3. On the fetcher: `blob_note_reference(...)` the address, then `run … --blob-peer <wg-addr> [--window N]
     [--budget-ms 20]` so the byte-tier thread fetches lazily while clinical sync continues.
  4. **Confirm the three §8.2 claims on the real link:** throughput + round-trip reduction vs the old stub
     (T1 reports `~seq RTTs → ~windowed waves`); **resume across a real Starlink drop** (kill mid-fetch,
     restart, only missing indexes refetch); **clinical p95 unaffected** during the windowed fetch (the
     ADR-0013 floor — re-confirm at the *production* `--budget-ms`, not just the selftest's aggressive 2 ms).
  5. **Record the chosen `SLICE_BYTES` / `--window`** from the run and mark §8.2 *delivered* in the spike doc.

### Run 2026-06-16 — §8.2 byte tier DELIVERED on the real Cape York ↔ Dorrigo link — **T1/T2/T5 PASS**

Ran the byte-tier throughput spike over the live ~680–860 ms-RTT WireGuard satellite link (MacBook 10.0.0.2
fetcher ↔ DGX Spark 10.0.0.3 source, user PG18 :5444). New driver **`harness/wan_spike.py`** (runs on the
MacBook, drives the DGX over ssh for *setup only*; all fetch/pull timing crosses the real link). Full results
table written into **[Spike 0001 §8.2](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)**; headline:

- **T1 windowing** — 4 MB in **21.4 s (w8) vs 101 s (w1) = 4.7×**, ~64 sequential RTTs → ~2 windowed waves.
- **T2 resume** — killed mid-fetch → **14/16 verified slices persisted**, resumed from `blob_chunk`, completed.
- **T5 floor** — clinical pull p95 **+28 %** during a concurrent fetch (budget-ms 20); **clinical sync never
  stalled** (ADR-0013 availability preserved).
- **Tuning:** throughput is RTT-bound (~0.2 MB/s/link) — the cost is a **fresh TCP connection + slow-start
  per slice**, so **parallel flows beat bigger slices** (window sweep peaks at **w8**: 0.12/0.19/0.17 MB/s
  for w4/8/16; a *larger* slice was *worse*: 256 KiB 0.19 → 1 MiB 0.16 → 4 MiB 0.11). **Keep
  `SLICE_BYTES = 256 KiB`, default `--window` 4–8.** On a shared-bandwidth link the floor knob is window
  width, not `--budget-ms`. **Next throughput lever (future, not this spike): connection reuse / persistent
  streaming** instead of one TCP connection per slice — the production object-store tier.
- **Cleanup:** SLICE_BYTES sweep edits reverted (const back to 256 KiB); no lingering DGX serve processes.
  `SLICE_BYTES` is only used by the *fetcher* (`do_blobd`); the server serves arbitrary offset/len, so a
  slice-size change rebuilds only the fetcher — handy for future tuning.

### Field-readiness 2026-06-16 (PR #7 merged to main; this work is post-merge on the branch)

User merged PR #7, then **started the real Cape York ↔ Dorrigo run over Starlink/WireGuard at lunch** —
hit a minor issue (fixed locally, **likely the `--listen 127.0.0.1` vs WireGuard-address bind**; that local
fix may be uncommitted on their MacBook — confirm/commit when they're back), but the run didn't conclude
before they had to return to work. So this interim session made the skeleton **runnable unattended**:

- **`cairn-sync run`** — serves + pulls + lazy-fetches blobs on a timer, appends one JSON line/cycle to a
  log, and **survives link drops** (bounded `connect_timeout` + retry/backoff; a sustained outage logs a
  `partition`, never fatal). Start it, walk away, analyse later. Default runs until killed (`--duration-s 0`).
- **`bet_a.py analyze --log run.jsonl`** — turns an unattended run log into the §5 numbers (duration,
  partition cycles, pull p50/p95/max, A2 verify-fails, A5 bytes/event, A3 merge+gap, A6 blob state) and
  writes a `.fingerprint.json` so two nodes' logs compare for A1 via `report`.
- Refactored `pull`/`blobd`/`fingerprint` into reusable cores (`do_*`) the run loop drives. Build/clippy/
  tests green; validated end-to-end on the container PG (two nodes converged to identical 106-event hashes
  under mid-run load; lazy blob fetched; transient + dead-peer partitions logged honestly).
- **Skeleton README** gained an "Unattended field run" section (the exact two-node commands, with the
  bind-to-WireGuard-address warning called out).

---

## Resolved 2026-06-16 — §11.7 locale-pluggable comparators (now spec v0.17) — **§11 is now fully closed**

Case-mined **§11.7** (the matcher comparator extension point). It dissolved — **no new founding principle, no
envelope reserve, one small *additive* data-model field** — and with it **every original §11 open question is
closed.** → [ADR-0014](spec/decisions/0014-locale-pluggable-matcher-comparators.md), canonical home
**[identity §5.13](spec/identity.md)**, with the assertion-level profile tag in **[demographics §4.1](spec/demographics.md)**
and the §5.2 comparator bullet expanded to point at it.

- **Structurally low-stakes because the matcher is advisory** ([§5.2](spec/identity.md)/[§9.4](spec/language-substrate.md)):
  it only *proposes*; proposals become ordinary `assert`/`link` events through the algebra. Blast radius doubly
  contained — additive advisory evidence into a conservative human-backstopped decision, **and** §5.1 clean
  unmerge makes even a wrong auto-link reversible. So unlike §11.6/§11.4 there's nothing irreversible to commit.
- **Principled framing (resonated): hardcoding one culture's name/date/address model is *cultural capture*** —
  anti-capture/vendor-independence (principle 7) applied to the demographic model. Pluggable comparators are
  *paper-parity for the registrar in any culture* (the user works the Australian Top End/Kimberley — Indigenous
  naming + birthdate-uncertainty are the rule there, utterly different from the east/south coasts).
- **The hard problem the user flagged — "comparators must travel with the data" (people relocate; forcing Cape
  York comparators onto Melbourne records on a merge is catastrophic) — without syncing code or a central
  registry.** Resolution (the session's sharpest new design): **split comparator *identity* (data, travels) from
  *code* (distribution plane, resolved locally).** A **content-addressed comparator-profile tag** (`namespace@hash`)
  rides each demographic assertion as declarative provenance — globally meaningful with **no central registry**
  (the ADR-0013 content-addressing payoff), **silently defaulting from the registering node's locale with a
  registrar-visible override** (the user's call — the *tourist injured in Cape York* case: low-risk friction
  reduction). The code travels the §7.6 distribution plane; a node lacking a record's comparator (or matching
  *across* two profiles) **degrades honestly to human review, never forcing the wrong comparator** — the §3.13
  legibility-ladder pattern applied to matching. **Safety-preserving by construction:** uncertainty about *which*
  comparator can only ever *withhold* an auto-link, never manufacture one (the safe side of false-merge ≫ false-split).
- **The miss-rate problem the user explicitly asked me to crack** (confident-rejects are silent false splits the
  live matcher can never measure). Solution: a **periodic, low-priority, *preemptible*, aggressive (low-threshold)
  background re-match sweep at the hub tier** that **never auto-acts** → emits a ranked advisory possible-duplicate
  worklist, can never starve clinical work (the ADR-0013 byte-tier discipline), and whose **yield IS the
  miss-rate/drift metric** (the ADR-0010 atrophy-signal pattern). Completed by two existing legs: **opportunistic
  re-match on every new assertion** (a reject flips as a shared phone/ID/refined-DOB lands — monotonic refinement)
  and a **point-of-care "this might be a duplicate — search & link" affordance** (paper-parity *gain* — the patient
  saying *"I have another file here"* is evidence the matcher never had). The user accepted generous surfacing as
  paper-parity-passing; this closes the one gap it left.
- **Comparator API contract:** field-typed, **agreement-leveled** (exact / nickname- or transliteration-equivalent
  / phonetic / edit-distance / none — Fellegi–Sunter), **uncertainty-aware** (*no-data is never disagreement*,
  principle 4), **provenance-aware**, and **operates over the multi-valued name *history set*** (not the display
  value) with role-tagged, order-tolerant tokens — directly answering the user's daily failure cases (first-name
  order, nicknames, indigenous names, DOB mismatches, hyphenated/maiden/married family-name switches).
- **Safety floor pluggability can't relax:** conservative threshold + wide-middle-band-to-humans + coherence-check
  vetoes hold regardless of plugged comparators; a small closed **hard-veto set** (same-system-ID mismatch;
  *verified* DOB/sex-at-birth clash; deceased-status conflict) **forces a human decision — never auto-link, never
  auto-reject** (user's "err on caution, prompt the user").
- **Reuse:** matcher = **registered actor** (ADR-0011; comparator config = version-pinned standing config; recall
  via §5.5 contamination cascade). **GitHub doubles as a federated, signed, content-addressed registry** (user's
  call — "as long as GitHub exists … primary-developer-vetted comparators"), convenience never a dependency
  (mirrorable, sneakernet-cloneable; trust in signature/hash, not host).
- **Blast-radius (§9):** all comparators + weight-learning + harness + sweep are fit-for-purpose (Python,
  advisory); the conservative threshold + hard-veto set + coherence check + proposal→algebra apply **seam** are
  safety-critical in-DB (the recurring seam motif).

---

## Resolved 2026-06-15 — §11.6 attachment strategy (now spec v0.16)

Case-mined **§11.6** (inline vs. content-addressed blob store with lazy sync). It dissolved into existing
primitives with **no new founding principle** — same trajectory as §11.8/§11.9/§11.10 — forcing only one
day-one envelope reserve and one ladder-axis generalization. → [ADR-0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md),
canonical homes **[data-model §3.14](spec/data-model.md)** (attachment-reference shape + rendition set) and
**[sync §6.6](spec/sync.md)** (the lazy byte tier), with back-pointers from [§3.8](spec/data-model.md)/[§3.13](spec/data-model.md)
and the §6.1 priority bullets.

- **Content-addressed by reference, never inlined — principle 1 applied to large binaries.** The content digest
  is to a blob what the signature is to an event body (same bytes → same address → idempotent set-union, zero
  merge). The event body names each attachment *by digest* and the **event signature covers that digest**, so a
  blob carries no separate signature and **self-verifies against any source**. Tiny blobs may inline below a
  node-tuned threshold; both forms expressible day one.
- **Reference-eager, byte-lazy + the availability floor (the user's load-bearing case).** The reference rides the
  eager event plane; bytes follow on a **separate, resource-isolated byte tier**. The user's Kimberley/*Communicare*
  case — a nightly bulk-imaging sync that ground the whole system to a halt so emergencies could retrieve **no**
  record at all (and recurred even in the degenerate single-server/thin-client topology) — sharpened the ruling:
  **priority ordering is insufficient**; an in-flight gigabyte head-of-line-blocks. So byte transfer is
  **chunked, preemptible, separately budgeted** (the user's "better async"), and the floor is an *availability*
  one: **blob transfer must never reduce clinical-data availability** (availability + paper-parity applied to the
  transport).
- **Byte-replication is opt-in and separately scoped (the user's second requirement).** §6.4's prefetch-hint
  applied to bytes, but the **blob predicate is a separate, much narrower thing** than event-scope: references
  everywhere, **bytes by election**; a resource-starved node is **references-only, fetch-on-demand** (it need not
  store every PACS blob). Content-addressing → **multi-source, self-verifying, resumable swarm fetch** (LAN
  sibling / parent / patient-carried device, zero trust in source — sneakernet generalized to binaries).
- **The rendition set is the binary's legibility twin** (resolves the §3.13 "how do you twin a CT's pixels?"
  tension — *you don't*; the twin is the coded/descriptor fields, the lightweight rendition is the blob's twin).
  Adds a **retrievability** axis to the §3.13 ladder: **effective rendering = `min(retrievable, parseable,
  cleared)`** (present / pending / shredded). *Coarseness varies; existence never disappears* — the floor invariant
  generalized once more.
- **Erasure + lossless passthrough inherit unchanged:** per-blob DEK → crypto-shreddable like the §3.5 body slot
  (GC ≠ erasure; **no convergent encryption** for sealed blobs — confirmation-attack leak); bytes are **never
  transcoded in place** (would break embedded signatures + change the hash) — a preview is a *new* rendition added.
  DICOM/WADO/XDS is a **façade**, never the store (§3.4 FHIR posture).
- **The four forks the user ruled on (all agreed):** (1) no convergent encryption for sealed blobs; (2) yes, a
  small-blob inline path; (3) blob store is the **sync plane's lazy tier, not a third plane** (it's content — no
  code, no RCE); (4) the day-one reference fields locked (self-describing digest w/ algorithm agility · seal/DEK-wrap
  indicator · clear-text descriptor · rendition set · inline distinction).
- **Blast-radius (§9):** digest-binding-in-signed-event + seal/DEK-wrap + crypto-shred + **content-verification on
  fetch** (a wrong-hash blob must never be served as the named one) are safety-critical; store/transfer/dedup/GC +
  all viewers are fit-for-purpose; the **fetch-verify seam** (bytes-in → hash-check → serve) is the one
  safety-critical path (the content-addressing analogue of the §3.13 write-time twin seam).

---

## Resolved 2026-06-15 — §11.4 schema migration + founding principle 11 (now spec v0.15)

Closed **§11.4** (schema migrations across a fleet of offline nodes) and, along the way, added an
**eleventh founding principle: legibility across time.** → [ADR-0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md),
canonical homes **[data-model §3.13](spec/data-model.md)** (event-format invariants),
**[sync §6.5](spec/sync.md)** (two planes + lossless forwarding), **[security §7.6](spec/security.md)**
(distribution plane), with the safety-projection unification in **[identity §5.9](spec/identity.md)** and the
new principle in **[index.md](spec/index.md)** / **[vision §1.9](spec/vision.md)**.

- **Schema evolution = the append-only/overlay + acknowledged-uncertainty principles applied to the schema
  itself.** The user's framing held: this is the *highest-leverage* remaining §11 item because — unlike
  attachments/comparators — it **constrains the event envelope**, which can't be retrofitted onto an
  append-only log (same logic as `t_effective` and the encryption-capable body slot being reserved day one).
- **Two planes that run at different speeds (the central ruling).** **Sync plane:** clinical events, set-union,
  AP, skew-tolerant, **never executable code**; the event *format* evolves forward-compatibly. **Distribution
  plane:** code/DDL/pgrx extensions, per-node, **per-architecture**, signed against a steward key, verified
  before load, **sneakernet-capable**. The decoupling that dissolves "lockstep fleet upgrade": **the
  schema/extension version is a *local node property* — node X's extension only has to match node X's own
  schema, never the version of events arriving from elsewhere.** Syncing a native `.so` over the clinical mesh
  is a hard no (RCE channel; violates principle 8).
- **The user's two sharp inputs, both absorbed:** (1) *pgrx extensions must travel with migrations* → the
  migration unit is one signed atomic bundle `{DDL + per-arch extension binary + projection-rebuild recipe}`;
  difficulty tracks native-code surface, so ADR-0001/0002's "small native surface" earns a *second* payoff
  (minimized migration blast radius). (2) *the stuck-at-V1-forever node that downloads a V9 record* → must not
  just display but **forward and safeguard** it: **lossless passthrough** (store/sync/export the original
  signed bytes untouched; signature covers a canonical byte form, never re-serialized JSONB), local annotations
  are **additive overlays**, and a node renders down a **legibility ladder**.
- **The user's proposal, refined and then elevated.** Their "any post-V1 format ships a to-plaintext function,
  retaining the original" was right; refined to **a mandatory, signed, mechanically-derived plaintext twin on
  *every* event** (the user's call, motivated by full-text indexability + compact RAG context + human audit —
  storage is cheap and compresses). The twin is a *local projection*, never the synced/exported record; carries
  a `rendered-by` stamp; an upgraded node regenerates a richer one.
- **The elegant unification (worth carrying):** the legibility ladder and the §5.9 confidentiality ladder are
  the **same mechanism** — effective rendering = `min(what this node can parse, what it is cleared to see)`. A
  can't-parse-the-format node is in the same position as a can't-decrypt-the-body node; both degrade down one
  ladder (rich → generic-descriptor → plaintext twin → §5.9 safety projection → partition-honest floor).
  *Coarseness varies; existence never disappears.* **Tolerance window = infinite for custody, best-effort for
  understanding.**
- **Four day-one event-format essentials** (can't-retrofit): `schema_version` (also the future schema-descriptor
  registry join key), the mandatory plaintext twin, lossless passthrough, additive-only evolution.
- **Scope call (the user's): design A, let B inform it.** Committed the four day-one essentials + the carried
  twin (Rung 0) now; the **generic descriptor-driven renderer (Rung 1)** is explicitly deferred and asserted to
  need **no envelope change / no migration** to add later (because `schema_version` is forward-designed as its
  join key). No new event stream.
- **Blast-radius (§9):** serialization/signature-canonicalization, lossless passthrough, additive-only
  enforcement, and distribution-plane signature-verification + extension load are safety-critical; all renderers
  + search/RAG are fit-for-purpose; the write-time body→twin seam *is* the §5.9 seal-time seam (one seam now).
- **New founding principle 11 — legibility across time** (the user's call to elevate it from a footnote): an
  event stays human-readable for as long as it exists regardless of schema drift — paper's note-from-decades-ago
  property; *schema is versioned data, not privileged structure*.

---

## Resolved 2026-06-15 — actor registry / AI-agent identity (now spec v0.14)

Closed the next ADR-0007 follow-on: the **AI-agent identity registry** (registration, keying,
version-pinning, key custody). → [ADR-0011](spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md)
(refines 0007), canonical home **[security §7.5](spec/security.md)**, invariants [data-model §3.12](spec/data-model.md),
with a recall-marker note in [identity §5.10](spec/identity.md).

- **General actor registry** (human/device/AI, AI the forcing case) — the user's call, with the foresight
  that "the boundaries will increasingly blur and the type of actor will matter less," so `kind` is a
  **de-emphasizable discriminator**, not a separate subsystem.
- **Immutable, version-pinned identity over a closed actor-event algebra** (`enroll/supersede/revoke/
  suspend/rotate-key`) — the §5.7 patient-identity-algebra shape applied to actors. A version bump = a new
  UUID + `supersede` link; compromise = `revoke` overlay (with compromise-time). *Never merge/erase, always
  link/overlay*, now for non-human actors. Forced by recall-traceability (mutating v2.3→v2.4 in place
  destroys "which events did the defective v2.3 author?").
- **The user's sharp refinement — identity granularity tracks objectively-recordable behavioral
  determinants.** The AI tuple expands beyond model+version+vendor+node to the **declared inference/decoding
  config** (temperature, top-p/k, sampling, system-prompt/template, tool/RAG config) — because under current
  tech these *distinguishably* shape output and consistency. The deep principle (the user's): humans vary too
  (mood, fatigue) but there is **no objective criterion** to split "happy Dr X" from "sleep-deprived Dr X,"
  so they stay one identity — **granularity is bounded by what's objectively recordable** (the same
  epistemics as t_recorded vs t_effective; fabricating a split violates principle 4). Identity-explosion
  avoided by pinning the *standing* config to the identity and stamping *per-invocation* variance on the
  event (objective-vs-asserted split again); both queryable for recall.
- **Enrolment: binding mandatory, output-responsibility policy** (the user's call). An audited, signed
  ceremony (mirrors node provisioning/mTLS) that **must record a named responsible human** — the
  introduction-accountability backstop that **completes ADR-0010's conservation chain** (even a fully
  un-owned AI output traces to a human who decided the agent may write here); ongoing per-output
  responsibility stays separable/policy (ADR-0007).
- **Key custody un-conflated — opposite lifecycles:** **signing publics are immortal** (verify history
  forever; `revoke` = distrust-new-after-T, never can't-verify-old), **DEKs are destroyable** (ADR-0005
  keystore). Private AI signing key node-bound trusted-base; a stolen key forges *origin* not
  *responsibility* (signature ≠ attestation), bounded by un-vouched-by-default + revocation + recall.
- **A model recall reuses the contamination-cascade primitive** (§5.5/§5.12): select by agent-UUID (+ the
  queryable per-event config), re-surface for review, overlay a §5.10 recall trust marker — **never erase.**
  Structurally identical to a misfiled-note cascade.
- **Blast-radius (§9):** registry projection + actor-event algebra + signature verification are
  safety-critical (in-DB, beside the §5.7 identity algebra); the **agent runtime** is fit-for-purpose (output
  additive/advisory by default, ADR-0010); the runtime→signing/registry **seam** is the one safety-critical
  path (the recurring seam motif). **No new founding principle.**

---

## Resolved 2026-06-15 — additive-vs-suppressing classification (now spec v0.13)

Closed the **sharpest ADR-0007/0009 deferred follow-on**: *how* an output's additive-vs-suppressing
nature is classified, validated, and enforced. → [ADR-0010](spec/decisions/0010-additive-vs-suppressing-classification.md)
(refines 0007), canonical home **[data-model §3.9](spec/data-model.md)**, with [identity §5.10](spec/identity.md)
(atrophy detection) and [§5.12](spec/identity.md) (the triage seam).

- **Derived, not declared — additive ≡ overlay, suppressing ≡ foreclosure.** The **append-only principle
  (1) applied to the attention/decision layer.** A self-declared "I'm additive" is the banned flag. Test:
  *could a human still independently see and act on everything they would have without this output?*
- **The user's reframe (load-bearing): suppression is often *desirable*** (drowning in thousands of
  objectively-normal results). Resolved by the §5.12 line: **demotion (priority-lowering) is additive**
  (still reaches the human) and is the primary, safe, un-owned noise tool; **only hide-to-nothing /
  auto-decide is suppressing.** The dangerous tail is a **closed enumerated set** (merge-policy discipline)
  behind a **structural in-DB owner-gate**; additive is the default, curated suppressing-until-proven-additive.
- **Conservation of responsibility:** un-owned suppression is a contradiction — accountability sits at the
  event, or (policy-permitted class) at the explicit audited config act that permitted it. Policy relocates
  the owner, never abolishes it (same shape as ADR-0005 deniable-rung, ADR-0008 sign-as).
- **Declaration is a one-way caution ratchet** (answer to "declared vs derived vs both"): derived sets the
  floor; a responsible human may declare a formally-additive output *more* suppressing, never less — the
  handle for **de-facto suppression** (automation complacency).
- **Triage = a salience-scoring extension point (mechanism, not policy — the user's recurring insistence):**
  trend-aware rule classifier (eGFR 90→70→30 = ALERT; 30→35→38 = TREND IMPROVING — trend beats instantaneous
  value) + optional AI oversight (meds/history/consults for context), wired to the §5.12 salience dial. Its
  output is an additive `{rule-classifier | AI, graded | triaged}` event — the §3.9 contributory roles built
  for exactly this; safe un-owned because additive.
- **Automation-complacency atrophy detection — BUILT NOW (user's call):** an **additive governance meta-signal**
  computed from the audit/ack streams when independent human review of a class collapses to ~0 (humans only
  ack the AI, never assess first) → *"the automated layer for X is now a single point of failure."* Additive
  (safe un-owned, self-consistent), population/governance-facing (mostly-pull), honest only at volume.
- **Blast-radius (§9):** the closed suppressing set + owner-gate + demotion-can't-silently-become-hide floor
  are safety-critical (in-DB/Rust); the salience classifier and atrophy detector are fit-for-purpose; the
  classifier→floor **seam** is the one safety-critical path (the recurring seam motif).

---

## Resolved 2026-06-15 — §11.10 notification economy (now spec v0.12)

Case-mined **§11.10** (notification priority taxonomy). It dissolved into existing primitives with
**no new founding principle and no new event stream** — same trajectory as §5.11. → [ADR-0009](spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md),
canonical home **[identity §5.12](spec/identity.md)**, invariants [data-model §3.11](spec/data-model.md),
with [security §7.4](spec/security.md), [sync §6.2](spec/sync.md), [vision §1.2](spec/vision.md).

- **"Priority" is one word hiding orthogonal dials** (the recurring *scope/signature/authentication*
  motif a 4th time). Dials: **salience × ack-requirement × addressing × modality × escalation.** The
  load-bearing split is **salience ≠ interruptiveness** — a standing fact (allergy) is *ambient*, only an
  urgent *transition* is interruptive (once). Alert fatigue **is** confirmation-dialog click-through
  (§5.11) generalised; the mechanism of fatigue is collapsing the dials into one popup-defaulted scale.
- **A notification is a projection, not a mailbox** — a *delta* over the log against the clinician's own
  audit history (view/act, already recorded). The inbox is a query; **acknowledgment is an append-only
  audit event** (single explicit human confirm; the user's call), **never auto-satisfied** for the
  hard-ack class (auto-ack = silent falsification). No new stream.
- **Noise reduction IS suppression IS accountable** (ties §11.10 straight into ADR-0007 — the bridge to
  the still-open *additive-vs-suppressing* follow-on). **Demotion/coalescing/digest = additive** (free);
  **filtering-out / auto-ack / below-threshold-hiding = suppressing** (owned, audited, policy-gated).
- **The user's routing ruling (load-bearing):** the locum reality is that the orderer has usually *left*
  before the result lands; many sites have no follow-up policy, remote sites run informally. So the
  **co-equal inbox is infrastructure; policy does prioritisation.** Responsibility-to-follow-up is a
  **graded, multi-source, append-only tag overlay** (orderer intrinsic + always telephone-prioritised;
  policy adds fallbacks; **timeout-reassignment** when the present responsible doctor is busy; *multiple*
  holders at once) → effective responsible set is a projection (same shape as §5.9 sensitivity / §5.1
  link graph). **Single co-equal inbox, not a single exclusive owner.**
- **Safety floor — routing is NEVER a visibility gate (the user's clincher case):** the *"orderer must
  release before anyone sees it"* policy has caused missed critical results. It is expressible as **ambient
  state only**; the architecture **refuses to enforce withholding** from a present clinician. Consumer-side
  mirror of ADR-0006's *"replication is never the confidentiality boundary"*: routing decides who *owns
  acting/acknowledging*, never who may *see*. New incoming results are **always** visible to whoever opened
  the patient.
- **Other floors:** escalation ladder never dead-ends (severity-ladder motif a 4th time → bottoms in the
  §5.11 current care-context holder); filtering changes modality, **never extinguishes** a hard-ack class
  (mirror of §5.9 *"blurs, never extinguishes"*); partition-honest inbox (no false *"all caught up"* —
  §6.2 honest-assembly for alerts); **mostly-pull, selectively-push** is the paper-parity default (paper
  = pull + critical-value callback + allergy sticker; everything-push is a parity *regression*).
- **Blast-radius (§9):** floor enforcement (hard-ack un-filterable; present-clinician never blind;
  escalation fires) is safety-critical (in-DB/Rust); advisory salience-ranking + digest UI are
  fit-for-purpose; the filter→floor **seam** is the one safety-critical path (like the §5.9 seal-time and
  §5.11 proximity→stamp seams).

---

## Resolved 2026-06-15 — §11.9 + §11.12 point-of-care identity (now spec v0.11)

Case-mined the two **point-of-care possession/identity** problems — §11.9 (armed write-context) and
§11.12 (authentication vs. paper-parity) — and found they are **one problem**: the binding of *which
patient* and *which clinician* to a write. Dissolved into existing primitives + one new data-model
invariant; **no new founding principle** (the three operational principles below are corollaries of
existing ones). → [ADR-0008](spec/decisions/0008-point-of-care-identity-possession-and-salvage.md),
canonical home **[identity §5.11](spec/identity.md)**, with [security §7.3](spec/security.md),
[data-model §3.10](spec/data-model.md), [vision §1.2](spec/vision.md).

- **The §11.12 "tension" is illusory** (the session's clincher reframe): the deployed-EHR audit-trail
  collapse is *caused by* the parity violation — expensive per-write auth makes shared logins rational,
  and sharing is what destroys attribution. So paper-parity and accountability are achieved **together**.
  Same shape as ADR-0006 ("scope") and ADR-0007 ("signature"): **one word hides separable dials.**
- **Unbundle `authentication` → gatekeeping (rare, coarse) + attribution (per-write, cheap).**
  Load-bearing invariant: **`session.user ≠ event.author`, independently bindable** ([data-model §3.10](spec/data-model.md));
  its absence is exactly why deployed EHRs can't salvage stranded work.
- **Possession binds `(clinician, patient)` in one ambient gesture** — cheap in time, **high in
  distinctiveness** (the antidote to confirmation-dialog click-through), **cold = warm** cost.
- **Three operational principles (corollaries, the user's), not new founding principles:**
  (1) *never make the user wait if engineering can avoid it* (latency limb of paper-parity; cache-and-hide
  not clear; instant re-auth is the **precondition** that makes auto-de-arm parity-legal);
  (2) *always a fallback, no dead-ends, no IT dependency* (badge → password → self-recovery → **audited
  break-glass**; the severity-ladder motif recurring a 3rd time — recovery is break-glass for the auth layer);
  (3) *never make the user redo work already done* (the **`sign-as`** salvage).
- **`sign-as` salvage = identity-repair applied to authorship.** Trichotomy sign-as (default) / switch /
  stay; rescues *your own* stranded work; replaces the three bad real-world hacks (free-text `[Dr X:]`,
  wrong-author save, lost work). **Authorship-confidence is a grade (attested/asserted/unattributed),
  never a gate** — composes into the existing trust projection, no new stream.
- **Settled forks:** authorship is **note-level** — span-granular-within-a-note **rejected** (user's call:
  "hideously complicates" for a rare edge; free-text hatch remains). **Make contention cheap** (multi-warm-
  context shared station) is the software's answer to the 2–5-clinicians-per-workstation reality. Design is
  **rhythm-agnostic** (live / after-each / batch / AI-scribe / forced-retrospective all first-class via
  bitemporal time) and **degrades to no special hardware**.
- **Blast-radius (§9):** the `(clinician, patient)` binding + authorship stamp are safety-critical (trusted
  Rust/in-DB surface); proximity/UI is fit-for-purpose; the proximity-event → authorship-stamp **seam** is
  the one safety-critical path (like the §5.9 seal-time seam).

---

## Resolved 2026-06-15 — authorship & accountability (now spec v0.10)

Reframed "tag AI-generated content" (raised the prior session) into a general model and a **tenth
founding principle**: **authorship is compositional; accountability is separable**
([ADR-0007](spec/decisions/0007-authorship-and-accountability.md)). No new overlay stream — it reuses the
envelope and existing lineage.

- **Contributor set** replaces the single `author` field: `{identity, role, descriptor?, responsibility?}`,
  identity = human / AI agent (model+version+vendor+node) / device. "AI-generated" is the emergent reading
  "non-human author + no responsible human," never a flag. ([data-model §3.9](spec/data-model.md))
- **Responsibility = `{held_by, on_behalf_of}`** — absent / held / proxied; orthogonal to human-vs-machine.
  *"AI is never responsible" is a policy default, not a schema law* → the transition toward AI accountability
  needs no migration.
- **Signature decoupled from attestation** — signed proves origin+integrity, attestation confers
  responsibility; *signed ≠ vouched-for*; AI agents get a registered crypto identity for recall-traceability.
  ([security §7.2](spec/security.md))
- **No responsible party is legitimate** for a *strictly additive* (win-or-no-change) output — the
  pathology-triage case. Additive-vs-suppressing is a recordable property; un-owned *suppressing* output is
  policy-gated (principle 9). Consumer side = three layers on the existing trust projection
  ([identity §5.10](spec/identity.md)).

**Open follow-ons:** exact role-enum membership; AI-agent identity registry + key custody (trusted-base /
blast-radius); additive-vs-suppressing classification (sharpest — author-declared vs derived); proxy/liability
semantics (out of scope — Cairn records the chain). See [open-questions.md](spec/open-questions.md).

---

## Resolved 2026-06-14 — §11.8 visibility-scope ↔ sync-scope (now written into spec v0.9)

Case-mined **§11.8** (does a sequestered episode replicate to a node at all?) plus the **rung-1 metadata
follow-on left open by ADR-0005**. It dissolved into existing primitives + two explicit constructs; no
new architecture, no new founding principle.

- **§11.8 RESOLVED → [ADR-0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md),
  [identity §5.9](spec/identity.md) (canonical home), with pointers from [sync §6.4](spec/sync.md),
  [security §7](spec/security.md), [data-model §3.5](spec/data-model.md), [index principle 9](spec/index.md).**
- **The core ruling (the user's): replication is *never* the confidentiality boundary.** Because there is
  almost always a patient's-best-interest case for the treating clinician to break glass with consent
  (the clincher: a sealed pregnancy termination still implies **Rh-sensitization** a future antenatal
  clinician must act on), a safety-relevant sensitive episode **replicates unconditionally**.
  Confidentiality lives entirely in **key-custody + body-visibility + envelope-abstraction**, never in
  withholding the row. This *confirms* ADR-0004 from the other side (sync scope was never an access control).
- **The word "scope" was hiding four dials**: replication (always on), decryptability (gated),
  body-visibility (sealed), and a newly-sharp fourth — **envelope-metadata exposure** (the plaintext
  envelope's scope key `department = sexual-health` is itself the disclosure; ADR-0005 only seals the body).
- **Two new explicit constructs:** (1) a **safety projection** — a de-identified, severity-graded signal
  (*"⚠ Grade X interaction with confidential content — break glass"*) **mechanically projected from the
  body's coded fields**, replicated in the clear like an allergy, naming nothing; makes the §5.6 promise
  concrete; partition-safe. (2) **Sensitivity as a graded, multi-source, append-only assertion stream**
  (effective grade = projection). **Safety-floor invariant:** the grade controls the signal's *coarseness,
  never its existence* — secrecy blurs the safety signal, never extinguishes it.
- **Infrastructure, not policy (principle 9):** Cairn ships exactly three pieces — a **category blacklist**
  (coded-category → default grade; whitelist is impossibly wide), the **confidentiality grading system**,
  and **human editability** of tag/grade (patient request / clinician judgment). *Whether a blacklist
  auto-tag applies silently, needs clinician acceptance, or is manual-only is a UI-layer policy decision*
  Cairn makes expressible but never enforces.
- **Two findings worth carrying:** the **semantic scope key is abstractable to an opaque "confidential-
  episode" token** — and doing so *forces* safe behavior (the sync prefetch predicate can no longer
  select, so it falls back to replicate-everything-for-this-patient). And the **policy-neutral
  severity-ladder pattern recurs** (erasure ladder → now a disclosure-coarsening ladder) — a structural
  motif, not yet elevated to anything.
- **Break-glass** is audited key-*use* (distinct from key-*destruction*/erasure), mirroring the ADR-0004
  acquisition trichotomy, partition-honest (*"sealed content exists here; the key is not present"*).

**Open follow-on:** the seal-time projection seam (the one code path that reads the coded body en route to
ciphertext) is safety/confidentiality-critical → a §9 blast-radius concern when implementation begins; and
projection quality tracks coding quality (uncoded body → weaker class, still better than paper's nothing).

---

## Resolved 2026-06-14 — §11.5 erasure/GDPR (now written into spec v0.8)

Case-mined **§11.5 (tombstones / retention / GDPR erasure)** — the sharpest standalone open problem —
from the user's real subpoena experience (an EM physician who contested *every* disclosure subpoena and
had each waived or restricted; most clinicians don't, so records leak). It dissolved, and added a ninth
founding principle along the way.

- **§11.5 RESOLVED → [ADR-0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md),
  [data-model §3.8](spec/data-model.md), [security §7.1](spec/security.md).** **Erasure is the
  redistribution of key-custody, not the deletion of data.** The clinical log is never mutated; the
  deletion primitive is **crypto-shredding** — destroy a body's DEK and the immutable, signature-valid,
  sync-safe row becomes keyless noise (the only deletion model compatible with append-only + WORM;
  mesh-resurrection of an opaque row is harmless). Exposed as a **policy-neutral severity ladder**:
  *hide → sequester → deniable sealed-escrow deletion → audited crypto-shred → best-effort oblivion*.
  Cairn builds the rungs; **which are offered is policy/UI configuration** — it facilitates conflicting
  legal/health-system requirements without taking sides.
- **The clinician-vs-patient conflict became positive-sum.** Clinicians want retention (medico-legal
  cover); patients sometimes want erasure (subpoena fishing-expeditions; stigma). Reframed as *who holds
  a key*, both are satisfied: the deniable rung destroys the institution's discoverable index + node key
  and escrows sealed copies to the patient + chosen clinician(s), so **the institution holds nothing**
  and can honestly answer a subpoena "no record" — the clinician's cover migrates to their own retained
  sealed copy, producible by consent.
- **Per-record encryption with a key-holder hierarchy including the patient** is reserved in the §3.5
  envelope **from day one** (can't retrofit onto an append-only log) but is **off by default** (a
  patient-held key trades availability for confidentiality).
- **Two principle-level additions:** a **9th founding principle — "policy-neutral infrastructure"**
  (Cairn provides mechanism, never policy; written into [index §principles](spec/index.md), [vision
  §1.8](spec/vision.md), `CLAUDE.md`); and a **corollary of the 4th** — *deletion is best-effort and
  declared, never guaranteed*. The honest ceiling, in the user's words: **"to our knowledge, we have
  erased all copies in our existence."**

**GDPR was used only as an illustrative example** (article references Art. 17(1), 17(3)(b)/(c)/(e),
9(2)(h)–(i) were **verified by web search**, June 2026, not asserted from training) — Cairn stays
jurisdiction-agnostic.

**Open follow-ons explicitly deferred:** the concrete *policy-defined* safety-relevant metadata that may
remain in rung-1 sequestration (→ §11.8); key granularity (per-event vs per-episode hierarchy) and
keystore Pi-cost (→ the Pi-benchmark spike); the deniable rung's interaction with mesh reach.

---

## Resolved 2026-06-14 — §11.3 dynamic sync scopes (spec v0.7, here for the trail)

Brainstormed **§11.3** from a real ED→ICU transfer case. It dissolved, and spun off the fourth governing
principle.

- **§11.3 RESOLVED → [ADR-0004](spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md),
  [sync §6.4](spec/sync.md).** **Scope is an administrative *prefetch hint*, not an authority.** Nobody
  owns the record — it's the sum of autonomous signed parts, assembled when it can be. A transfer
  triggers *acquisition* (sibling-on-LAN / carried-with-patient / from-parent-on-reconnect), not
  reassignment; the parent ratifies+audits, never gates. Surviving requirement: **honest assembly-state
  disclosure**. (The surplus-copy GC follow-on it spun off is now absorbed by §11.5 / ADR-0005.)
- **4th governing principle "Acknowledged uncertainty"** + **bitemporal time** →
  [ADR-0003](spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md),
  [data-model §3.6/§3.7](spec/data-model.md). `t_recorded` (HLC, objective, the **ceiling**) vs.
  `t_effective` (author-asserted, freely backdatable). Clash detection **flags, never resolves**.

---

## Resolved 2026-06-13 (spec v0.6 — here for the trail)

The **"Postgres-intelligence" cluster** (§11.1 / §11.2 / §11.11) is **resolved** as **"Fat Postgres,
thin Rust daemon"** — full rationale in **[ADR-0001](spec/decisions/0001-fat-postgres-thin-daemon.md)**
(written into `spec/topology.md` §2, `data-model.md` §3.5, `sync.md` §6.1, `language-substrate.md` §9.4):

- **§11.2 storage (→ §3.5):** hybrid event envelope — typed/normalized columns where invariants,
  identity, sync, and matching bind; **Cairn-native JSONB** for clinical bodies; **FHIR is a façade
  only**, never the storage model. *(As of v0.8, the JSONB body slot is also encryption-capable — §3.8.)*
- **§11.11 merge boundary (→ §9.4):** structural invariants + identity event algebra + **all projections
  in Postgres** (trigger-maintained, `AFTER INSERT`); the Rust daemon ships/applies but **carries no
  merge logic**; the probabilistic matcher stays **Python and advisory**. Per-projection pgrx escape
  hatch on measured Pi-performance need ([ADR-0002](spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)).
- **§11.1 sync backbone (→ §6.1):** **build** a thin custom Rust service on Postgres logical decoding;
  **borrow** pgactive/SymmetricDS patterns, **do not depend** on them.

**The load-bearing bet to validate first:** that trigger-maintained in-DB projections + the identity
algebra stay cheap enough on **Pi-class hardware** to keep chart reads local and fast (the §1.2
paper-parity floor). The designed first spike is a **Raspberry-Pi-5 benchmark harness** (rural-clinic
profile, low concurrency; measure single-op projection-maintenance and chart-read latency; threshold =
beat "grab the paper chart"). Mitigation ladder if slow: PL/pgSQL → **pgrx (in-DB Rust)** → external
Rust. *This spike is the go/no-go on the approach.* **(v0.8 adds a second thing the spike should measure:
keystore cost / key granularity for crypto-shredding — see ADR-0005.)**

---

## Decided in conversation, NOT yet written into the documents

1. **Governance / CONTRIBUTING document — DONE (2026-06-16).** Written as
   **[docs/principles/GOVERNANCE.md](principles/GOVERNANCE.md)** (a principles-level doc) folding in
   Stewardship of the Name by reference, plus a thin root **`CONTRIBUTING.md`** pointer (GitHub
   convention). README Contributing stub and mkdocs nav updated. Notable governance commitments recorded:
   **mission as tie-breaker; AGPL-3.0 inbound=outbound with DCO and *no CLA*** (a CLA would be the capture
   surface the project guards against); name-stewardship binds the steward too; case-mining is a
   first-class contribution. Entity/jurisdiction and formal trademark remain **parked** (carried into the
   doc, not re-litigated).
2. **Status line:** README and spec both say "specification / architecture phase." Still accurate today
   — flip when implementation begins.

*(Reference — already written: name is **Cairn** / repo **cairn-ehr**; domains `cairn-ehr.org` canonical
+ `cairn-ehr.com` defensive redirect, both registered, reflected in STEWARDSHIP-OF-THE-NAME.md.)*

---

## Time-sensitive (do soon, before squatters do)

- **Package-registry namespaces — DONE (2026-06-14).** `cairn-ehr` reserved-name placeholders
  (v0.0.0, AGPL-3.0-only) **published** to PyPI, crates.io, and npm (`@cairn-ehr` scope). The bare name
  `cairn` was already taken on all three, so `cairn-ehr` is the canonical package name. Stub sources in
  `packaging/`. Domains held. **GitHub `cairn-ehr` org created, both repos transferred** in
  (`cairn-ehr/cairn-ehr`, `cairn-ehr/cairn`); personal `hherb/…` URLs redirect. Moving the org under a
  stewarding legal entity is the parked governance question.

---

## Open questions / where we'd pick up

**Every original §11 open question is now resolved** (items 1–12 struck-through), and the ADR-0007 deferred
**additive-vs-suppressing** ([ADR-0010](spec/decisions/0010-additive-vs-suppressing-classification.md)) and
**AI-agent identity registry** ([ADR-0011](spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md))
follow-ons are closed too. The last two — **§11.6** (attachments, [ADR-0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md))
and **§11.7** (locale-pluggable comparators, [ADR-0014](spec/decisions/0014-locale-pluggable-matcher-comparators.md))
— closed this session. The only ADR-0007 follow-ons still open are small (closed role-enum membership
finalisation; proxy/liability semantics, out of scope — Cairn records the chain). This session's record-discovery
case-mining (→ [ADR-0016](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md)) surfaced the
**Custodian & Federation Admission** dependency, which was then **drafted the same session** (→ [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md),
[security §7.7](spec/security.md)). This session (2026-06-17) was busy: promoted the **active-write model**
cluster to canon (→ [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)),
then case-mined the **application-layer / API architecture** (→ [ADR-0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md),
founding principle 12) and **closed its entire follow-on arc** in one session: the **validated submit surface**
(→ [ADR-0022](spec/decisions/0022-validated-submit-surface-the-write-path.md)), the **native API contract**
(→ [ADR-0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md)), and **hard policy
expression** (→ [ADR-0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)); spec
**v0.26**. **The layering/API arc (0021 → 0022 → 0023 → 0024) is now complete** — no open architecture
follow-ons from it. The highest-signal modes are now **fresh clinical case-mining** and the **build-prep
threads** below. Build-prep next steps: the **easyGP next-week session** (port the `rx!`/`tx!` type-through +
the prefetch/materialization warming daemon — the ADR-0020 deferred items), **Bet B on a Pi** (now
**prepared** — runbook + self-describing harness ready, awaiting the board; see the top entry), then the
byte-tier connection-reuse throughput lever.

**The recurring menu** when resuming (pick one):
- More clinical **case-mining** — the most productive mode so far (the event-overlay + key-custody + actor
  primitives have absorbed every case raised without new architecture). The AI-authorship arc (ADR-0007 →
  0009 → 0010 → 0011) is now complete, so fresh clinical cases are the highest-signal next input.
- ~~Write the GOVERNANCE / CONTRIBUTING document~~ **DONE 2026-06-16** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`).
- ~~**Define the Pi-benchmark spike**~~ **DRAFTED 2026-06-16** as **[Spike 0001](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)**,
  reframed into two bets (WAN-sync now / Pi-cost next week) on one shared walking skeleton, with the
  day-one serialization/signature/digest defaults. ~~build the skeleton~~ **DONE**; ~~run Bet A on the
  Cape York ↔ Dorrigo link~~ **DONE 2026-06-16 — all six §5 rows PASS** (see the run note above).
  ~~run Bet A~~ **DONE — §4 primitives ratified as [ADR-0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)**.
  **Now: Bet B on the Pi — PREPARED 2026-06-18** ([`PI-RUNBOOK.md`](../poc/walking-skeleton/PI-RUNBOOK.md) +
  self-describing, floor-finding harness; see the top entry), awaiting the Pi 5 / 16 GB + 1 TB SSD.
- **easyGP next-week session** (the live build-prep thread) — with full easyGP Postgres schema + PL/pgSQL +
  PL/Python access, port the ADR-0020 deferred items: the `rx!`/`tx!` parser + type-through state machine, the
  formulation/drug data source + renal/hepatic/pregnancy forced-manual rule table, and the prefetch/
  materialization warming daemon (mechanism scavenge, validates ADR-0001). Pre-read:
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **Polish a non-developer landing page** for the generated site (frontend-design work; draft plans
  already exist under `docs/superpowers/`). Note: the `web/` landing page + chart examples already advanced
  across PRs #15–#17 (founding-principles cards, two-zone chart UI example).

*(All §11 open architecture questions are now resolved — no remaining items in that backlog.)*

---

## Parked (deliberately not decided yet — don't re-litigate without reason)

- **Stewarding legal entity & jurisdiction.** Options floated: German Stiftung / gemeinnütziger Verein,
  US 501(c)(3), or an umbrella (e.g. Software Freedom Conservancy or a health-specific foundation).
  Deferred until the project has momentum and funding/adoption geography is clearer.
- **Formal trademark / wordmark registration.** Principle recorded now (stewardship doc); the legal
  instrument deferred until there is enough substance to be worth capturing.

---

## Working context for whoever resumes

- The user is a senior physician with an interest in ML / AI / health IT; codes mostly in Python. Brings
  real ED and hospital experience from several nations and health systems — case-mining sessions are
  unusually productive. (Founder of GNUmed, an early FOSS Postgres EHR; instincts are high-signal.)
- The project's founding motivation is explicitly **anti-capture / anti-vendor-lock-in**, rooted in the
  user's experience of government EHR committees being sabotaged by commercial interests. Decisions
  consistently favour the mission over convenience; treat that as the tie-breaker.
- **Twelve founding principles** now run through everything ([index.md](spec/index.md)); the **first four**
  are the lens checked before any new design choice: **(1)** append-only + causal ordering; **(2)**
  identity is a claim, never a fact (never merge/erase, always link/overlay); **(3)** paper-parity;
  **(4)** acknowledged uncertainty (incl. the corollary *deletion is best-effort and declared*). The
  rest: availability-over-consistency, fractal topology, vendor independence, safety-critical-logic-in-
  Rust/DB, **(9) policy-neutral infrastructure** (mechanism, never policy), **(10) authorship is
  compositional, accountability is separable**, **(11) legibility across time** (paper-parity along the
  time/version axis; the mandatory plaintext twin + additive-only schema evolution; *schema is versioned data,
  not privileged structure* — ADR-0012), and **(12) uniform core, plural edges** (compatibility is a property
  of the signed event core, below UI and policy; the floor is enforced unbypassably in the DB so UIs may
  proliferate freely; *many front-ends, one record* — ADR-0021). Note: the §5.11 point-of-care work added **no** new
  founding principle — its three operational principles (never-wait / always-a-fallback / never-redo-work)
  are corollaries of paper-parity, availability, append-only, and identity-repair. The §5.12 notification
  economy likewise added none — its rulings (salience ≠ interruptiveness; notification-as-projection;
  noise-reduction-is-accountable-suppression; routing-is-never-a-visibility-gate) are corollaries of
  paper-parity, acknowledged uncertainty, append-only, accountability, and policy-neutral infrastructure.
  ADR-0010 (additive-vs-suppressing) is a *refinement* of principle 10, not a new principle — its core
  identity (additive ≡ overlay, suppressing ≡ foreclosure) is principle 1 applied to the attention layer.
  ADR-0011 (actor registry) likewise adds none — version-pinned immutable actor identity is principle 2
  (never merge/erase, always link/overlay) applied to non-human actors, and identity-granularity-tracks-
  what's-objectively-recordable is principle 4 applied to the actor model.
