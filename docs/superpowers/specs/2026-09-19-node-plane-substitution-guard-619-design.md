# Design — the node plane refuses a substitution at both of its live doors, and pens it on the pull path (#619)

- **Issue:** [#619](https://github.com/cairn-ehr/cairn-ehr/issues/619) — db/007's two node-plane doors
  have no substitution guard, including the live federation admission gate. Touches
  [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) (the node plane's skip-and-advance
  divergence) by carving **one** class out of it, and nothing else of it.
- **Date:** 2026-09-19. **Branch:** `feat/619-node-plane-substitution-guard`.
- **Records:** a new **ADR-0073**, continuing
  [ADR-0072](../../spec/decisions/0072-a-restore-loses-no-record-silently.md). Spec **v0.74 → v0.75**.
  **No new migration; `SCHEMA_GENERATION` stays 53** (db/007 is edited in place — see §5).

## 0. Maintainer decisions taken in the brainstorm (2026-09-19)

| # | Fork | Ruling |
|---|------|--------|
| 1 | What should this session build — #619, a restore polish batch, or leave DR? | **#619.** |
| 2 | A substituted node event arriving over the network: pen it, refuse-and-skip, or do all of #268? | **Pen it.** Refuse at the door AND pen the rival bytes in `node_event_quarantine` (db/022) — durable evidence, loud until a human acks. The routine deny-all *scoping* refusals keep skip-and-advance; the rest of #268 is untouched. |
| 3 | Guard each of db/007's five `ON CONFLICT` sites inline, or restructure each door to one shared tail (db/009's shape)? | **Single tail per door** — two call sites, not five, and a future arm cannot forget the guard. |

Two further questions were **not** asked, because the codebase already answers them:

- **How does the puller tell a substitution from a scoping refusal?** Not by SQLSTATE: db/001's header
  makes P0001 *a contract* for every floor refusal and forbids `USING ERRCODE`, because both pull
  loops route on it (a distinct code would also turn `cairn-sync`'s clinical pen into a freeze). Not by
  message text either. **By state** — §2.2.
- **Does a RAISE on the pull path wedge the watermark?** #619 feared so (its point 2). It does not: the
  node puller's P0001 arm skips-and-advances today, and the pen arm advances too (it freezes only at
  quota). The refuse-vs-skip question #619 raised is therefore narrower than the issue framed it.

## 1. Why this piece exists

A **substitution** is a second, different event filed under an `event_id` the log already holds. Every
door inserts `ON CONFLICT (…) DO NOTHING` so that an idempotent re-write of the *same* event stays a
silent no-op (set-union, principle 1) — and the identical no-op is what a substitution looks like from
the INSERT's side. Without a comparison, the rival is **discarded in silence** and two nodes hold
different bytes under one id forever.

ADR-0072 gave three doors one shared refusal (`cairn_refuse_substitution`, db/053). Its review then
found the census wrong: the `node_event` table has **three** writers, and only `restore_node_event`
(db/009) was guarded. db/007's `submit_node_event` (two sites) and `apply_remote_node_event` (three
sites) compare nothing.

### 1.1 What the missing guard actually does — stated precisely

#619's failure scenario says a compromised peer B can make node A *"keep trusting C"*. **That cannot
happen**, and the ADR must not repeat it: `trust_peer` reads only `node_event` rows whose
`author_node_id` is **this** node, so no peer's `peer.revoked` ever changes A's trust. What the missing
guard really costs:

- **The remote door (`apply_remote_node_event`) — silent, permanent divergence of the replicated node
  plane.** A holds X; a trusted peer serves a different signed event under X; A's INSERT is a no-op, the
  function returns normally, the puller counts it `admitted` and advances past it, and set-union never
  re-offers it. No alarm. **The sharpest concrete case is a rival genesis:** `node_current` resolves a
  node's key from its `enroll` row, so if a trusted peer's genesis is the dropped rival, that peer's key
  never resolves on A and **every event it authors thereafter is refused** as *"author key maps to no
  known node"* — logged as *"recoverable, non-fatal"*, which it is not.
- **The local door (`submit_node_event`) — #615's shape.** If A's own `peer.revoked(C)` is authored
  under an id A already holds, the revocation is dropped and **A keeps trusting a peer it revoked**, at
  success. Reaching it needs A's signing key (a bug in id minting, or a caller holding the key), so it is
  less reachable than the remote case — but the consequence is the trust set itself.

Both are **pre-existing** (db/007 predates db/053) and both are silent.

## 2. The design

### 2.1 db/007 — one guard per door, after the branch

Each door is restructured to the shape db/009 already has: **every arm that inserts falls through to
one shared tail**, and the tail reads the stored content address **unconditionally** and calls the
helper once.

**`apply_remote_node_event`** (three arms today, each with its own INSERT, `cairn_node_hlc_merge` and
`RETURN`):

```text
IF v_op = 'enroll' THEN
    <genesis trust check>                     -- unchanged
    INSERT … ON CONFLICT (node_event_id) DO NOTHING;
ELSE
    <author resolves; author is an active peer> -- unchanged, hoisted out of the old fall-through
    IF v_op = 'supersede' THEN
        <payload check>; INSERT … ON CONFLICT DO NOTHING;
    ELSE
        <payload check>; INSERT … ON CONFLICT DO NOTHING;
    END IF;
END IF;
-- SUBSTITUTION REFUSAL (#619) — once, AFTER the IF/ELSE, never above it.
SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'apply_remote_node_event');
PERFORM cairn_node_hlc_merge(…);              -- once, instead of three copies
RETURN v_eid;
```

**`submit_node_event`**: the genesis arm is **left as it is** — it has no `ON CONFLICT`, is once-only by
the `local_node` check, and a colliding id would raise `unique_violation` loudly, which is not silence.
The `supersede` and `peer/revoke` arms become `IF supersede … ELSE … END IF`, then the same two-line
guard naming `'submit_node_event'`, then `RETURN`.

**Why these choices, each of which a later session might "tidy":**

- **After the `IF/ELSE`, never before it.** Above the branch the row does not exist yet, `v_found` is
  NULL, and IS DISTINCT FROM refuses — a *clean* apply would be refused (ADR-0072's mutation M7 is
  the same trap).
- **Unconditional read, no `ROW_COUNT`.** The node plane carries tens of events, not 100k; a
  `GET DIAGNOSTICS` check is correct only while each INSERT stays the last statement of its arm, and a
  later edit would disarm it silently (trap 12's db/009 rule, applied here for the same reason).
- **Guard before the clock merge.** A refused rival must not advance this node's HLC. (The RAISE rolls
  the merge back anyway, but the order should say what is meant.)
- **Behaviour is otherwise unchanged.** Every existing refusal keeps its text and its order; only the
  shared tail is new. The restructure's safety net is the existing door suites staying green.

### 2.2 The pull path — classify by state, pen the substitution

In `pull_into` (`crates/cairn-node/src/sync.rs`), the `Err(e)` arm for a **verifiable** event with
SQLSTATE **P0001** gains one question before it skips: **does `node_event` already hold this event's
`event_id` under a different content address?**

- The `event_id` comes from the body `verify_self_described` already returned (today discarded as
  `Ok(_)`); the offered address is `event_address(signed)`, byte-identical to db/007's `v_ca`.
- The held address is one `SELECT content_address FROM node_event WHERE node_event_id = $1`.
- **Held and different ⇒ substitution ⇒ pen it** through the existing `quarantine_node_event`, with a
  reason naming the id and both addresses, and a per-event `eprintln!` that says SUBSTITUTION.
  **Absent, or held and equal ⇒ today's skip-and-advance, unchanged.**
- **The rule is "whichever check refused it."** A rival under a held id can never apply — the id is
  taken — so skip-and-advance's premise (*"self-heals on a later sweep"*) is false for it even when an
  earlier check (an untrusted author, say) is what raised. Classifying by state rather than by which
  sentence was raised is what makes that true without reading message text.
- **Race-free:** `node_event` is append-only, so once a row exists its id and content address never
  change; the post-refusal read cannot see a different world from the door's.
- **A failed held-address read freezes**, like a failed pen write does today — never advance past a
  refusal we could not classify.
- **The transient arm is untouched.** A non-P0001 failure still freezes; the retry will reach the door's
  guard and come back as a P0001.

The routing decision becomes **one pure function** — inputs *(verifiable?, SQLSTATE, held address,
offered address)*, output **Pen(reason) / Skip / Freeze** — unit-tested without a database, with the
I/O kept in `pull_into`. It lives in a new small module rather than growing `sync.rs` (already ~1 180
lines).

**What the pen then does, all existing behaviour:** the row pins the derived re-offer floor, so the
rival is re-offered and re-refused every cycle (deduping onto its row, `seen_count` rising); the cycle
reports `pending > 0` and `run` logs its INTEGRITY line; it **never auto-releases**, because it never
applies; a human `ack-quarantine` silences it permanently (the dedupe bumps the acked row and keeps it
acked). A peer flooding substitutions fills the per-peer quota and the cursor freezes — the same honest
limit sync.md §6.3 already states for an unverifiable flood.

### 2.3 The inventory stops being a hand-written list

The census error was possible because `every_door_this_change_guards_still_calls_the_helper` is a list a
person maintained. It is replaced by a **catalogue rule over `pg_proc`**, in the style of
`late_custody_guards.rs` rule 2: **every PL/pgSQL or SQL function whose body (comments stripped) inserts
into `node_event` or `event_log` with `ON CONFLICT … DO NOTHING` calls `cairn_refuse_substitution`**, and
that writer set is pinned **by name** (today: `submit_event`, `apply_remote_event`, `restore_node_event`,
`submit_node_event`, `apply_remote_node_event`) so a sixth writer is a decision, not drift. A positive
control asserts the rule sees the five it names. The no-DB single-source test is kept unchanged.

Honest residuals, stated in the test like its sibling's: it reads a function's own body, so a write
through a helper, a `MERGE`, or a dynamic `EXECUTE` is not recognised (none exists); and it covers the
two **event** logs only. The actor registry's `actor_event` has the same silent-discard shape at
db/052's door — that is **[#569](https://github.com/cairn-ehr/cairn-ehr/issues/569)**, already open, and
widening the rule to it here would fail on db/052 and pull #569 into this slice. The test names it.

### 2.4 The published operator text moves with the code

- `cairn-node quarantine --help` says every row is a node_event *"refused as UNVERIFIABLE"* — now also
  *or a substitution*.
- `PullStats::quarantined`'s doc says *"UNVERIFIABLE events penned"* — widened the same way.
- `run`'s INTEGRITY line's remedy, *"fix trust/code or ack-quarantine"*, is wrong for a substitution
  (no trust or code change makes it apply) — reworded to point at each row's reason.

## 3. What this deliberately does not build

- **The rest of #268's partition** — distinguishing scoping deny-all from other genuinely-refused
  history (oversize, missing payload fields). Still open; this is its first member only.
- **#301** — admit-uninterpreted on the node plane.
- **Node-plane completeness accounting** (a general "what did the node plane fail to apply" report).
- **#608's `cairn_project_late_custody` half** (clinical plane).
- **#605** — see §5.
- **[#569](https://github.com/cairn-ehr/cairn-ehr/issues/569)** — the actor registry door's own silent
  content-conflict discard (see §2.3).

## 4. Testing

TDD throughout; every new test is seen red before the code that turns it green.

- **The pure router** (unit, no DB): the full routing table — unverifiable → pen; P0001 + held-different
  → pen (reason names both addresses); P0001 + absent → skip; P0001 + held-equal → skip; non-P0001 →
  freeze.
- **The doors** (DB-gated, a new file beside `restore_one_node_event_id_one_body.rs`), one case per
  guarded arm: a rival under a held id is **refused with the shared sentence naming its door**, the
  held row is unchanged, and **the same bytes re-applied still succeed** (set-union must survive the
  guard). Arms: `submit_node_event` peer/revoke and supersede; `apply_remote_node_event` enroll,
  supersede and peer/revoke. The headline: A's own `peer.revoked(C)` under the id of its
  `peer.added(C)` is refused loudly and `trust_peer` still shows C as it was — before, it returned
  success and dropped the revocation.
- **The pull path** (DB-gated, the established single-DB self-pull, in a new file because
  `node_quarantine.rs` is already 603 lines): a raw-inserted served row carrying a rival under a held id is **penned,
  not skipped** (`quarantined ≥ 1`, `pending ≥ 1`, the reason names the id), and the held row is
  unchanged; an acked substitution stays quiet on re-offer. The existing
  `a_verifiable_but_refused_event_is_skipped_not_penned` stays green — the scoping class is untouched.
- **The catalogue rule** (DB-gated) plus its positive control.
- **Mutations**, with a harness carrying its own positive control (the ADR-0072 practice): delete each
  door's guard; move a guard above its `IF/ELSE`; delete the puller's substitution check; invert the
  router's comparison; route a substitution to Skip. Each must be killed at the assertion that names
  its claim.
- **Gates:** `scripts/run-db-gated-tests.sh` (the one local gate), `cargo fmt --check`, the
  `-D warnings` doc build, and the strict mkdocs build (the new ADR needs its `mkdocs.yml` nav line in
  the same commit).

## 5. Risks

- **#605 — an in-place edit is unprotected by the #188 downgrade guard.** db/007 changes in place, so
  `SCHEMA_GENERATION` stays 53 and an older gen-53 binary (PR #618's build) connecting afterwards would
  `CREATE OR REPLACE` the unguarded doors back, silently. Accepted as #601 accepted it: pre-clinical, no
  mixed-version fleet. ADR-0073 names it. (A new migration file only to force a bump would be
  artificial, and re-declaring db/007's functions in a later file is the stale-copy drift ADR-0048
  removed.)
- **The restructure touches the live admission gate.** Mitigated by keeping every refusal's text and
  order, the existing federation/quarantine suites, and the per-arm tests above.
- **A new loud signal.** A substitution now makes the pull loud until a human acks it. That is the
  intent — it is evidence of an equivocating or buggy peer — and it fires only on a genuine id
  collision (UUIDv7 does not collide by accident), so it cannot become alarm fatigue the way penning
  scoping refusals would (#268's own objection).

## Paper-parity benchmark (§1.2)

Paper-parity: not clinical-surface — this changes how two federation write doors and the node-plane
pull loop treat a forged or colliding trust-plane event; no clinician performs, sees or waits on any
step of it, and no clinical workflow gains or loses an act.
