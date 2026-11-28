# Whole-project review (2026-11-28)

This is the standing gate's second whole-project pass. The first was on 2026-07-15 and covered db/001–034. Since
then the floor has grown to 87 migrations, the whole clinical surface has been built, and so has the #909
terminal stack. On 2026-11-15 a review covered only the seams between GUI slices. This review covers `main` at
`f2f48eef` (spec v0.115, `SCHEMA_GENERATION` 87).

The review made five parallel read-only passes, each tracing one question through every layer:

1. **The in-DB floor, db/035–087,** against every runtime login. What can a raw-SQL client holding each login
   do that an ADR says it cannot?
2. **The #909 terminal stack as one trust chain:** steward key → intake → store → gateway → fetch → root
   installer → swap. It was checked per threat actor: a LAN host, a stolen disk, a local user, a compromised
   node, a revoked terminal.
3. **Sync convergence and DR:** does every stream added since July replicate, converge regardless of arrival
   order, ride the medium, and come back on restore? If it is node-local, does an ADR say so?
4. **The documents against the code:** about 110 checkable claims in ADRs 0083–0113, the ADR index, the
   durable rules' pins, and what CI never runs.
5. **The governing principles across every clinical stream:** identity, append-only, uncertainty, the
   legibility twin, authorship, paper-parity.

Every Critical and Important finding was re-checked against the code before filing, and each was checked
against the issue list for duplicates. The matcher was not re-reviewed: it is advisory, it was reviewed in July,
and it has had one commit since 11-15. Severity follows the 07-15 review:

- **Critical:** a wrong author, a wrong patient, or silent corruption is possible.
- **Important:** a real defect, or a decision is owed.
- **Minor:** a latent problem or an edge case.

## Verdict

The **floor holds where the 07-15 review found it weakest.**

- **All 07-15 floor findings are fixed:** A1–A3 and D1–D3.
- **The definers are sound:** all 103 SECURITY DEFINERs in db/035–087 pin `search_path` with `pg_temp` last and
  revoke EXECUTE from PUBLIC, and their grants match their ADRs.
- **The trusted settings are pinned:** `cairn.remote_apply` and `cairn.reprojecting` are pinned wherever a
  definer writes `event_log`.
- **Each node converges on its own:** within one node, every post-July projection is order-independent.
- **The terminal stack:** no path was found that puts attacker code or keys on a terminal. The installer reads
  staging as hostile and installs the bytes it verified in memory. The verifier counts keys, not signatures,
  and handles forks. The gateway is mutual TLS 1.3 with pinned Ed25519 keys.
- **The ADRs are accurate:** of about 110 checked claims, none was false.

The defects cluster in **four seams:**

- **(A) A repair primitive the spec promises but nothing builds.** Wrong-chart entry has no reattribution, and
  a note cannot be retracted.
- **(B) Privileges inherited, not granted.** The window inherits `cairn_agent`, and through it the matcher's
  write tables. No guard sees write grants.
- **(C) Rules applied to one stream, silent for another.** A repudiated key is handled for notes and
  allergies, not medications. Principle 4's unknown and refused values are missing in allergies. A legibility
  twin is required, never checked.
- **(D) Single-node assumptions meeting multiple nodes and terminals.**
  - A peer's human-signed events are penned, contradicting ADR-0054.
  - A terminal below the floor, or one running a broken release, cannot reach its own fix.
  - A revoke assumes a Postgres that listens only on loopback.

## Critical

| # | Finding | Issue | Fix size |
|---|---|---|---|
| C1 | **Wrong-chart entry has no repair.** Reattribution, the spec's "primary high-frequency use case" (identity §5.5), is unbuilt. ROADMAP's deferral ("waits on a clinical-note surface") went stale at slice 3a and had no issue. A note has one event type and no retraction. A misfiled medication can only be "stopped". Allergies and given doses do have entered-in-error. Paper allows a strike-through; Cairn does not. **Gates any pilot.** | #1226 | large (interim: a note retraction, medium) |
| C2 | **Any window login can forge a matcher identity link, and a false "checked".** `GRANT cairn_agent TO cairn_window` carries the matcher's INSERT/UPDATE on `match_proposal` and its DELETE on `match_pending`. `apply-auto-candidates` trusts the band and version it reads from the table, and enrols any version it has not seen. A terminal's raw-SQL session can therefore link two charts under a freshly minted matcher actor, or erase a chart's pending re-check. This is Critical where auto-apply is run. | #1227 | medium |

## Important

| # | Finding | Issue | Fix size |
|---|---|---|---|
| I1 | An older binary of the same generation replays its old SQL bodies over an in-place floor fix (ADR-0080). The guard refuses only `recorded > embedded`. This reopens D1 for equal generations. | #1228 | small |
| I2 | Doors that lock and then read trust the caller's isolation level. Under REPEATABLE READ an enroller can re-key a person a superuser has just repudiated (ADR-0086). The same pattern appears in db/059 and db/082. This extends #1196. | #1229 | small |
| I3 | A medication sign-off by a since-repudiated key still reads as signed. "Given by" and "checked by" still name the person. ADR-0093 was applied to notes and allergies only. | #1230 | small |
| I4 | The allergy form forces Allergy vs Adverse effect with no "not known", and that choice downgrades the check. It also pre-fills "patient stated". | #1231 | decision + small |
| I5 | Allergies cannot record "unable to obtain" or "declined", so an unconscious patient reads "not yet asked". | #1232 | decision + small |
| I6 | The floor requires a legibility twin but never checks it against the body, although the twin feeds search and RAG (ADR-0012's own falsifier). | #1233 | decision + medium |
| I7 | A revoke does not shut out a stolen terminal if Postgres also listens on the LAN. The password is on the terminal's disk, and `gateway-setup` never checks `listen_addresses`. | #1234 | small |
| I8 | A terminal that launches below the floor, or refused, has no courier. It can never fetch its update, and the kiosk relaunches it into the same refusal every 3 s. | #1235 | medium |
| I9 | A signed release that passes the print-flag check but fails at display start crash-loops every terminal. `.prev` is written but never used. | #1236 | medium |
| I10 | The root-owned installer has no update path: releases carry only `window` artifacts. | #1237 | decision |
| I11 | Through the gateway, vault guessing becomes remote and runs in parallel across all staff. The back-off is per member. Related to #903. | #1238 | decision |
| I12 | A peer's human-signed clinical events are penned, never applied. That contradicts ADR-0054 d5(a)'s "content never waits". At the pen cap the whole peer's watermark freezes, and a sneakernet carrier never forwards a penned event. | #1239 | large (interim: an honest-status line) |
| I13 | A carried newer-profile note, discard or vouch is never floor-checked when this node later learns that profile. Ruling R8 breaks on upgrade. | #1240 | small |

## Minor, process, and what was fixed here

The minor and process findings are gathered in **#1241**. The most consequential is that the window
door-surface guard sees only EXECUTE and SELECT, which is how C2 passed every guard. The others:

- size caps on the window's draft and queue stores;
- the gateway's pending queue can be flooded before proof of possession;
- a compromised node can force endless downloads;
- launch refusals on a kiosk are invisible;
- no two-node test covers any post-July stream, which would have caught I12 at once;
- the release store's restore behaviour is undecided;
- the 500-line rule is written down nowhere.

**#905 is far wider than filed.** About 27 files, roughly 200 window tests, never run in CI. The issue has
been widened and the workflow comment corrected. #963 was widened to cover medication draft lists.

Fixed in this review's PR (#1242), all documentation:

- `security.md` §7.5 said the window holds the node key. ADR-0109 retired that.
- The ADR index lacked six partial supersessions and amendments:
  - 0083 ← 0108/0109
  - 0084 ← 0108
  - 0089 ← 0109
  - 0091 ← 0092 d12
  - 0094 ← 0107/0108
  - 0097 ← 0101 d8
- Two durable rules named pins that no longer exist, and told a developer to undo ADR-0109 (the window commit
  runner) and batch charting slice 1 (the line's Cancel).

## Deliberately not findings

These rules live above the floor by ADR, and were checked as such:

- the warning override and threshold (ADR-0099/0104);
- co-sign judged at read time (ADR-0092/0094);
- a draft part's writer as a claim (ADR-0088);
- the node-signed timer note (ADR-0089);
- registration signed by the node (ADR-0061).

These node-local divergences are each stated in their ADRs:

- the supervision record (ADR-0094);
- the warning threshold (ADR-0104);
- the vault, the drafts, and terminal trust.

The #1192 rotation window is ruled accepted.
