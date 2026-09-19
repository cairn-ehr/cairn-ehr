# ADR-0073 — The node plane refuses a substitution at both live doors, and pens it

- **Status:** Accepted
- **Date:** 2026-09-19
- **Spec version at acceptance:** 0.75
- **Issues:** [#619](https://github.com/cairn-ehr/cairn-ehr/issues/619) ·
  [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) (one class carved out; the rest open)
- **Relates to:** [ADR-0072](0072-a-restore-loses-no-record-silently.md) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) ·
  [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
- **Amends:** ADR-0072's decision 1, whose shared refusal served three doors and now serves all five
  event-log writers; and two premises of its #619 paragraph — that `db/009`'s tail-guard shape does
  not transpose to `db/007`, and that a RAISE on the node pull path can wedge the watermark (see
  Context). Takes on the residual ADR-0072 called its largest. Reverses nothing. The two statements
  in that paragraph that were false about the code — the wedge, and that set-union never re-offers
  the dropped rival — are corrected by errata E1–E2 appended to ADR-0072.

## Context

[ADR-0072](0072-a-restore-loses-no-record-silently.md) gave the write doors one shared substitution
refusal, `cairn_refuse_substitution` (`db/053`). A **substitution** is a second, *different* event
filed under an `event_id` the log already holds. Every door inserts `ON CONFLICT (…) DO NOTHING`, so
that a repeat of the *same* event stays a silent no-op — set-union, principle 1 — and that identical
no-op is what a substitution looks like from the INSERT's side. Without a comparison, the rival is
discarded in silence.

ADR-0072's own review found its census wrong: `node_event` has three writers, and only
`restore_node_event` (`db/009`) was guarded. `db/007`'s `submit_node_event` (two
`ON CONFLICT (node_event_id) DO NOTHING` sites) and `apply_remote_node_event` (three) compared
nothing. ADR-0072 named that [#619](https://github.com/cairn-ehr/cairn-ehr/issues/619) and left it
open for two reasons. `db/009`'s tail-guard shape did not transpose, since each `db/007` arm had its
own `RETURN`. And choosing between *refuse*, *skip-and-advance* and *quarantine* at the live
admission gate looked like the open node-vs-clinical-plane divergence (#301 / #268), *"where a RAISE
on the pull path can wedge the watermark."* Decision 1 below answers the first by restructuring the
arms; the next section shows the second premise does not hold.

### What the missing guard actually did — stated precisely

> ⚠️ **#619's own failure scenario overstates it, and this record must not repeat it.** Its scenario
> has a trusted peer B serve a rival `peer.revoked` for node C under an id A already holds; the
> rival is dropped, and *"A keeps trusting C; B does not."* **That cannot happen.** `trust_peer`
> (`db/007`) reads only `peer`/`revoke` rows whose `author_node_id` is **this** node's own
> (`local_node`), so no peer's `peer.revoked` — admitted, dropped or refused — has ever changed A's
> trust set. What B's rival does cost is below; A's trust in C is not part of it.

- **The remote door (`apply_remote_node_event`, the federation admission gate): a silent, permanent
  divergence of the replicated node plane.** A holds X; a peer serves a different signed event
  under X's id, by an author A trusts; A's INSERT is a no-op, the function returns normally, the node puller counts it
  `admitted` and advances past it, and every later full sweep repeats the same silent no-op. Two
  nodes hold different bytes under one id, and nothing says so. **The sharpest case is a rival
  genesis.** `node_current` resolves a node's key from `enroll` rows alone, so if a trusted peer's
  genesis is the one dropped, that peer's key never resolves on A, and every event it authors
  thereafter is refused as *"author key … maps to no known node"* — which the puller logs as
  *"recoverable, non-fatal"*, and which it is not.
- **The local door (`submit_node_event`): #615's shape.** If A authors its own `peer.revoked(C)`
  under an id A already holds, the revocation is dropped, the door returns success, and **A keeps
  trusting a peer it revoked.** Reaching it needs A's signing key — the door refuses any signer but
  this node's own — so it is less reachable than the remote case, but what it corrupts is the trust
  set itself.

Both were pre-existing (`db/007`'s doors predate `db/053`), and both were silent.

### A RAISE does not wedge the node pull

#619 (its point 2) and ADR-0072 both feared that refusing at the admission gate would wedge the pull
watermark. **On the node plane it cannot.** The admission gate's deliberate refusals are bare
`RAISE EXCEPTION`s — SQLSTATE `P0001`, which is a *contract*: `db/001` states it for the node pull
loop in the comment above `cairn_decode_hex_or_raise` (#228), and `cairn-sync`'s
`refusal_is_deliberate` has relied on it since #267 — and the node puller's arm for a verifiable
event refused with `P0001` skipped and advanced before this change. So the question was
never *refuse or wedge*. It was narrower: **what should the puller do with a refusal it would
otherwise file as self-healing?** (An error that is *not* a deliberate RAISE does freeze the node
pull, and for a deterministic one the freeze is permanent: a verifiable event whose `event_id`
fails the gate's `uuid` cast raises `22P02` before any trust check. That pre-existing case is
[#621](https://github.com/cairn-ehr/cairn-ehr/issues/621); nothing here changes it.)

The skip exists because a node-plane refusal is almost always **scoping**. `stream_node_events`
serves every row, so a puller routinely refuses events authored by nodes it does not peer with;
those are re-offered on the periodic full sweep and admitted once trust arrives. A substitution
never heals — the id is taken, so no sweep, trust change or upgrade makes it apply — and filing it
under skip-and-advance logs it as *"recoverable, non-fatal"* on each sweep and keeps no durable
trace that two different signed events exist under one id.

## Decision

**1. Both `db/007` doors refuse a substitution — once each, after the branch.** Each door is
restructured to the shape `db/009` already has: every `ON CONFLICT` arm falls through to one shared
tail, which reads the stored content address **unconditionally** and calls
`cairn_refuse_substitution` once, naming its door. **Two call sites, where there were five unguarded
`ON CONFLICT` sites**. An arm that falls through to the tail inherits the guard; an arm that
`RETURN`s early bypasses it. `submit_node_event`'s genesis arm does exactly that, and is safe only
because it has no `ON CONFLICT`: a colliding id raises `unique_violation` — loud, not silent. A
future arm written in its image *with* an `ON CONFLICT` would bypass the guard, and the catalogue
rule of decision 3 would not notice (see Residuals).

Each placement is one a later edit might "tidy" away, so each is stated:

- **After the `IF/ELSE`, never above it.** Above the branch nothing is held yet, `v_found` is NULL,
  and `IS DISTINCT FROM` refuses — every clean write would be refused. It is the trap ADR-0072's
  mutation run exercised on `db/009` (its M7), and this run's M4/M5 on the two doors here.
- **An unconditional read, no `GET DIAGNOSTICS ROW_COUNT`** — `db/009`'s rule from ADR-0072. A
  `ROW_COUNT` check is correct only while each INSERT stays the last statement of its arm; a later
  edit would disarm it silently.
- **The guard precedes the clock merge.** `apply_remote_node_event`'s three copies of
  `cairn_node_hlc_merge` fold into one in the tail, after the guard, so a refused rival never
  advances this node's clock. (The RAISE would roll the merge back regardless; the order says what
  is meant.)
- **Every existing refusal keeps its text and its order.** Only the shared tail is new.

**2. The node puller pens a substitution — classified by state, whichever check refused it.** In
`pull_into` (`crates/cairn-node/src/sync.rs`), the arm for a verifiable event refused with `P0001`
now asks one question before it skips: **does `node_event` already hold this event's `event_id`
under a different content address?** The id comes from the body `verify_self_described` already
returned; the offered address is `event_address(signed)`, byte-identical to the door's `v_ca`; the
held address is one primary-key read, `held_content_address`, in
`crates/cairn-node/src/sync/substitution.rs`. The decision itself is one pure function beside it,
`substitution_reason(event_id, held, offered)` — `Some(reason)` exactly when something is held and
it differs.

- **Held, and different** ⇒ a substitution ⇒ **penned** in `node_event_quarantine` (`db/022`),
  through the same pen-or-freeze helper the unverifiable arm uses, with a reason that begins
  `substitution:` and names the id and both addresses.
- **Nothing held, or held and equal** ⇒ skip-and-advance, unchanged. The routine scoping deny-all
  stays exactly where it was.
- **The lookup fails** ⇒ the cursor **freezes**, as it already does when a pen write fails: never
  advance past a refusal this loop could not classify.

Why by **state**:

- **Not by SQLSTATE.** Both pull loops route on `P0001`. For the node loop, `db/001`'s comment above
  `cairn_decode_hex_or_raise` (#228) calls it a contract and forbids `USING ERRCODE` on that
  helper's refusals, because any other code freezes the cursor; for the clinical loop,
  `cairn-sync`'s `refusal_is_deliberate` pens a verifiable event's refusal only when it is `P0001`,
  since [#267](https://github.com/cairn-ehr/cairn-ehr/issues/267) (`db/048` restates that half).
  And `cairn_refuse_substitution` is shared with the two clinical doors, so a distinct code there
  would turn `cairn-sync`'s clinical pen into a freeze.
- **Not by the door's sentence.** That would make English prose part of the protocol.
- **State is what makes "whichever check refused it" true without reading any text.** A rival
  signed by a key this node does not trust is refused by the author check *before* the door reaches
  its guard — and it is still a rival under a held id, still never applies, and is still penned. A
  classifier keyed on which sentence was raised would have filed it as scoping.
- **The answer cannot go stale.** `node_event` is append-only — its trigger refuses `UPDATE` and
  `DELETE` — so once a row holds an id, the content address under that id never changes.

What the pen then does is all existing behaviour. The row pins the derived re-offer floor, so the
rival is re-offered and re-refused every cycle, deduping onto its row; the cycle reports
`pending > 0` and `run` logs its INTEGRITY line; the row **never auto-releases**, because
auto-release fires only when a re-offered event applies; and a human `ack-quarantine` silences it
for good (the dedupe bump leaves the ack in place). There is **deliberately no per-event log line**
— the unverifiable arm's convention, and for its reason: an acked row is still re-offered on every
full sweep, so a per-event line would keep printing after a human had decided. The loud signal is
the INTEGRITY line, which counts only unacked rows.

This is the first member of what [sync §6.3](../sync.md#63-failure-modes-designed-for) calls
*"genuinely refused history"*, to be told apart from the trust-graph deny-all — carved out because it
alone can be recognised by state. **It decides nothing about the rest of
[#268](https://github.com/cairn-ehr/cairn-ehr/issues/268).**

**3. The inventory of guarded writers is derived from the catalogue, and pinned by name.**
ADR-0072's census was a count made by hand, and so was the inventory test that followed it —
`every_door_this_change_guards_still_calls_the_helper`, a list of three migration files. A list
records what its author believed. It is replaced by
`crates/cairn-node/tests/substitution_guard_covers_every_writer.rs`, a rule over `pg_proc` in the
style of `late_custody_guards.rs` rule 2: **every PL/pgSQL or SQL function in `public` whose body
(comments stripped) inserts into `event_log` or `node_event` must call
`cairn_refuse_substitution`.** The writer set it derives is pinned by name — `apply_remote_event`,
`apply_remote_node_event`, `restore_node_event`, `submit_event`, `submit_node_event` — which makes
the pin the rule's own positive control (a rule that sees no writer passes over everything) and
turns a sixth writer into a decision rather than drift. The comment stripper both catalogue rules
use now lives once, in `tests/common/sql_text.rs`. The no-database
`substitution_guard_is_single_source.rs` stays: it proves nobody *duplicates* the refusal; the new
rule proves every writer *calls* it. `db/053`'s `COMMENT ON FUNCTION` now names all five callers and
points at the new rule.

**4. The published operator text moves with the code.**

- `cairn-node quarantine --help` described every row as a node_event refused *"as UNVERIFIABLE"*. It
  now says unverifiable bytes **or a substitution**, and that a substitution never auto-releases.
- `PullStats`'s doc described `quarantined` as *"UNVERIFIABLE events penned"*; it is widened the
  same way.
- `run`'s INTEGRITY line told the operator to *"fix trust/code or `ack-quarantine`"* — wrong for a
  substitution, which no trust or code change makes apply. It now says each row's reason tells which
  it is (unverifiable bytes, or a substitution) and to *"fix the cause or `ack-quarantine`"*.

## Why the pull path pens where the restore path aborts

ADR-0072 decision 2 makes a substitution on a sneakernet medium **abort** the restore; this ADR
**pens** one on the pull path and carries on. The difference is what stopping would protect. A
restore that continued would bring a node back with a trust set decided by whichever copy the
medium ordered first — which whoever can append to the medium controls — and the operator's
remedy, finding another copy of the medium, is available. A pull that froze on a
substitution would hold every later event from that peer behind one rival, which is the reason
`cairn-sync` pens a door refusal rather than freezing (#267); a pen keeps the evidence durable and
loud and lets the rest of the stream through.

## Alternatives rejected

- **A distinct SQLSTATE for the substitution refusal**, so the puller could route on it. `P0001` is
  the contract both pull loops route on (`db/001` above `cairn_decode_hex_or_raise` for the node
  loop; `refusal_is_deliberate` in `cairn-sync`), and the refusal lives in a helper the clinical
  doors share: `cairn-sync` would read the new code as a fault and freeze where it pens today.
- **Matching the refusal's message text.** It makes English prose part of the protocol, and it would
  still miss a rival refused by an earlier check.
- **Five inline call sites**, one per `ON CONFLICT`. The smallest diff. Rejected by the maintainer's
  ruling for a single tail per door: two call sites, inherited by every arm that falls through to
  the tail, rather than one per arm to keep in step.
- **Refuse at the door, and skip on the pull path.** It needs no puller change, and it files a record
  that can never apply under *self-healing* — logged as *"recoverable, non-fatal"* and kept nowhere
  durable. Rejected by the maintainer's ruling: **pen it.**
- **All of #268 now** — pen every genuinely-refused node event, not only substitutions. #268's own
  obstacle stands: `stream_node_events` serves every row, so penning the steady-state scoping
  refusals would hold the loud signal on permanently and exhaust the pen quota. The rest still needs
  the door to tell scoping from genuinely-refused history; a substitution needed no such distinction,
  because the table already says what it is.
- **Widening the catalogue rule to `actor_event`.** `db/052`'s `restore_actor_registry` has the same
  silent-discard shape — that is [#569](https://github.com/cairn-ehr/cairn-ehr/issues/569), open —
  so the rule would fail on it today and pull #569 into this change. The test names it instead.
- **A new migration, to force a `SCHEMA_GENERATION` bump.** It would have to re-declare `db/007`'s
  two functions in a later file — the stale-copy drift [ADR-0048](0048-twin-check-registry-dispatch.md)
  removed — or exist only to move a number.

## Consequences

- **The cost, honestly.** On the puller: at most one primary-key read per verifiable `P0001`
  refusal on the node plane (none when the `event_id` is not a UUID). A full sweep — every tenth
  cycle (`FULL_SWEEP_EVERY`), and whenever trust changes — re-offers every row a peer serves, so the
  scoping refusals, the routine case, pay it again each sweep. On the doors: one primary-key read
  per event through either door, re-applies included (the local genesis arm aside), since the read
  is unconditional. The node plane is small — `db/007`'s own comment puts it at tens of events — and
  **the clinical plane pays nothing**: `cairn-sync`, `db/005` and `db/020` are unchanged.
- **A new loud signal.** A substitution keeps the pull loud until a human acks it. That is the
  intent: it fires only when two different signed events exist under one `event_id` — evidence that
  some signer minted an id already in use, or that a relay re-wrapped a signed event — the COSE
  unprotected header lies outside the signature
  ([#620](https://github.com/cairn-ehr/cairn-ehr/issues/620)) — by bug or on purpose.
- **The guard makes a substitution loud; it does not decide which event is genuine.** Whichever
  reached this node first holds the id; the pen row names both addresses so a human can find both.
  A rival genesis is now refused and penned rather than dropped in silence — but the id is still
  taken, so that peer's key still never resolves here and its events are still refused. This ADR
  makes that visible; nothing in it repairs it.
- **Two pinned counts were checked, and one moved.** `hlc_merge_helper.rs` now pins `db/007` at
  **one** `PERFORM cairn_node_hlc_merge(` site, down from three, because the three arms' merges
  folded into the shared tail; `db/001`'s comment now counts three callers across the tree.
  `hex_decode_helper.rs`'s pin of four `cairn_decode_hex_or_raise` calls in `db/007` survived the
  restructure unchanged.
- **`SCHEMA_GENERATION` stays 53.** `db/007` is edited in place; there is no new migration. See
  #605 below.
- **Tested per arm, and by mutation.** Every guarded arm has its own rival case
  (`node_plane_one_event_id_one_body.rs`: the local door's peer/revoke and supersede arms; the
  admission gate's enroll, supersede and peer/revoke arms), each door has an idempotence case — the
  same event twice still succeeds — and the pull path has `node_substitution_is_penned.rs`, which
  also pins the false-positive direction: a refused event held with the SAME bytes is skipped,
  never penned. The lookup-failure freeze is pinned by `node_substitution_lookup_freezes.rs`, which
  runs the pull under a role granted everything it needs except `SELECT` on `node_event`. Eleven
  mutations were run with `scripts/mutations/2026-09-19-619.sh`: deleting either door's guard (M1,
  M2, and M3 against the catalogue rule alone), hoisting either guard above its `IF/ELSE` (M4, M5),
  inverting the pure decision (M6), removing the puller's question (M7), blinding the lookup (M8),
  deleting the shared clock merge (M9), turning the lookup-failure freeze into a skip (M10) and
  addressing the whole frame rather than the signed bytes, so that every refused re-offer of an
  event already held with the same bytes would be penned (M11), were **each killed at the assertion
  that names its claim**. M10 was first
  declared a survivor, on the premise that nothing could make the lookup fail inside the self-pull;
  review found the seam — `pull_into` takes the caller's connection, and both doors it calls are
  `SECURITY DEFINER` — and the test was written.
- **How we would know the bet failed:** `substitution_guard_covers_every_writer.rs` fails, naming a
  writer that does not call the helper or a sixth writer nobody decided on; or a node that has pulled
  from a peer holds different bytes from that peer's under one `node_event_id`, with no pen row for
  the peer's version.

## Residuals — named, not assumed away

- **[#605](https://github.com/cairn-ehr/cairn-ehr/issues/605)** — because `db/007` is edited in
  place, `SCHEMA_GENERATION` stays **53**, and the #188 downgrade guard cannot tell this build from
  the one before it. An older generation-53 `cairn-node` — PR #618's build — connecting afterwards
  replays its own embedded `db/007` and `CREATE OR REPLACE`s both doors back to their unguarded
  bodies, in silence. (`cairn-sync` does not load `db/007`.) Accepted as #601 accepted the same
  exposure for its in-place edits: pre-clinical, no mixed-version fleet.
- **[#268](https://github.com/cairn-ehr/cairn-ehr/issues/268)'s remaining classes.** Every other
  genuinely-refused verifiable node event — oversized, a missing or malformed payload field, an HLC
  wall past the drift ceiling — still skips and advances, re-offered only on the full sweep.
- **[#301](https://github.com/cairn-ehr/cairn-ehr/issues/301)** — the node plane's remote door still
  fail-closes on a node event type it cannot map; ADR-0056's admit-uninterpreted is not live there.
- **[#569](https://github.com/cairn-ehr/cairn-ehr/issues/569)** — `restore_actor_registry`
  (`db/052`) still discards a content conflict in `actor_event` in silence.
- **Node-plane completeness accounting** still does not exist: there is no general *"what did the
  node plane fail to apply"* report.
- **[#608](https://github.com/cairn-ehr/cairn-ehr/issues/608)** — `cairn_project_late_custody`'s
  not-found arm, on the clinical plane, still returns silently; ADR-0072 narrowed #608 to it, and
  this change does not touch it.
- **The per-peer pen quota** — 10 000 unacked rows or 64 MiB per peer — applies to substitutions
  exactly as to unverifiable bytes. A peer flooding substitutions fills it, and the cursor then
  freezes below the first rival it cannot pen: delayed, never lost, loud. It is the limit
  [sync §6.3](../sync.md#63-failure-modes-designed-for) already states for a hostile-but-credentialed
  peer.
- **The catalogue rule's blind spots.** As the test states, it reads a function's own body, so a
  write through a helper, a `MERGE` or a dynamic `EXECUTE` is not recognised. Two more are not
  stated there. It reads `pg_proc.prosrc`, which is empty for a `LANGUAGE sql` function written with
  a SQL-standard `BEGIN ATOMIC` body; none of those shapes writes an event log today
  ([#622](https://github.com/cairn-ehr/cairn-ehr/issues/622)). And it asks
  whether a function *calls* the helper, not whether every INSERT path *reaches* the call: an arm
  that inserts `ON CONFLICT DO NOTHING` and `RETURN`s before the tail would pass it. Only a rival
  test written for that arm, or review, would catch it.

The design and the implementation plan — including the full mutation ledger — are
`docs/superpowers/specs/2026-09-19-node-plane-substitution-guard-619-design.md` and
`docs/superpowers/plans/2026-09-19-node-plane-substitution-guard-619.md` (working scaffolding,
excluded from the published site).
