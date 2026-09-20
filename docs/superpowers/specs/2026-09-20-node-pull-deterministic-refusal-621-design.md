# A deterministic door failure is a refusal, not a fault (#621)

**Date:** 2026-09-20 · **Issue:** [#621](https://github.com/cairn-ehr/cairn-ehr/issues/621) ·
**Plane:** node/federation only (the clinical twin is [#626](https://github.com/cairn-ehr/cairn-ehr/issues/626)) ·
**Follows:** [ADR-0073](../../spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md) (#619),
[#228](https://github.com/cairn-ehr/cairn-ehr/issues/228) (the same class, closed for hex only).

## The defect, as the code actually has it

`pull_into` (`crates/cairn-node/src/sync.rs`) classifies a refusal in four arms. The last one:

> a verifiable event refused with anything OTHER than P0001 → **freeze** the cursor, retry next cycle.

That is correct for a deadlock, a statement timeout or a dropped connection. But
`apply_remote_node_event` (db/007) raises non-P0001 **deterministically**, on input it will refuse
identically forever:

| Where | SQLSTATE | Reachable by |
| --- | --- | --- |
| `v_eid := (b ->> 'event_id')::uuid` — **before any trust check** | `22P02` | any signer whose bytes a peer serves |
| `NULLIF(v_payload ->> 'target_event_id','')::uuid` | `22P02` | a trusted author |
| `node_event_hlc_nonneg` CHECK (negative wall/counter) | `23514` | a trusted author |
| `node_event_role_check` (`role` outside the vocabulary) | `23514` | a trusted author — **not in the issue** |

A frozen cursor never advances past that seq, so **every later event on that link is held behind
it**, including that peer's own `peer.revoked`. Nothing is penned, so there is no `ack` remedy
either; the operator sees `transient/unexpected error … — freezing` every cycle, forever.

It also falsifies one sentence of #619's design — *"the retry will reach the door's guard and come
back as a P0001"*. An error raised before the guard never reaches it.

## Who can actually trigger it (the scenario, checked rather than assumed)

`serve` streams `SELECT seq, signed_bytes FROM node_event WHERE seq > $1` — **only rows already in
the serving peer's own log**, which passed that node's identical casts and CHECKs. So an honest peer
on the same schema cannot serve one of these. Two triggers remain, and they are worth different
amounts:

1. **A misbehaving or compromised trusted peer** crafting frames. Bounded: it wedges only *its own*
   link, and it could stall that link anyway by going silent. What the wedge adds over silence is
   dishonesty — it is reported as transient, it never heals, and it has no remedy.
2. **Cross-version CHECK-vocabulary skew.** This is the one that matters under principle 11
   (additive-only evolution). db/009 already widened the `op` CHECK in place; the day `role` is
   widened the same way, every older node pulling from a newer peer freezes that link **permanently**
   on `23514`. A vocabulary widening must never be able to partition the fleet.

Severity is therefore lower than "a live, network-reachable wedge", and the fix is still worth
building: the class is open for every cast nobody has written yet.

## Decisions

### D1 — The node-plane doors are TOTAL: every deterministic malformed input raises P0001

P0001 is the contract the pull loop routes on (db/001, above `cairn_decode_hex_or_raise`, #228;
db/048 states the clinical half). A door that lets PostgreSQL raise on caller-supplied bytes is
breaking that contract by omission, exactly as the hex `decode()` did.

Mechanism, one helper per shape, each naming **field, door and reason**:

- **`cairn_uuid_or_raise(field, value, door)` in db/001**, gating on `pg_input_is_valid(value,
  'uuid')`. That is *the cast's own grammar*, evaluated by the same server — so this adds no second
  parser. (#623's review found a pen bypass caused by exactly that: the Rust `uuid` crate is
  narrower than Postgres's `::uuid`. A mirror is a liability; asking the server is not.) It lives in
  db/001 for `cairn_decode_hex_or_raise`'s reason: cairn-sync loads a subset containing db/001 but
  not db/007, and PL/pgSQL binds at first execution (#198).
- **`cairn_node_hlc_nonneg_or_raise(wall, counter, door)` in db/001**, beside `cairn_node_hlc_merge`,
  raising before the INSERT reaches the CHECK.
- **`cairn_node_role_is_known(role)` in db/007** — an IMMUTABLE predicate that **the CHECK constraint
  itself calls**, with `cairn_node_role_or_raise(role, door)` raising through the same predicate.
  One vocabulary, not two: a door list beside a constraint list is the drift that reinstates the
  freeze, and db/009's `op` DROP/ADD pair is the precedent for changing a CHECK in place.

Call sites: `submit_node_event`, `apply_remote_node_event` (db/007) and `restore_node_event`
(db/009) — the same three doors #228 fixed. db/009 aborts the whole restore on any raise either way,
so there it buys legibility rather than availability; leaving it out would mean the catalogue guard
below needs an exemption, which is how a door gets forgotten.

### D2 — The puller partitions the non-P0001 space: deterministic ⇒ PEN, local ⇒ FREEZE

D1 fixes four known raises. D2 closes the class, including an `XX000` out of a pgrx function fed
adversarial bytes and any cast a future slice writes.

A new pure `deterministic_apply_failure(sqlstate) -> bool` in `cairn-node`, the complement of
`cairn-sync`'s existing `apply_failure_is_local` (same list, same reasoning: `08 40 42 53 55 57 58`
and *no SQLSTATE at all* are this node's own trouble; everything else is attributable to the bytes
and will recur identically). In `pull_into`'s last arm:

- **local or no SQLSTATE** → freeze, unchanged. A wrong `true` here would skip a valid event.
- **anything else** → **pen** it, with a reason in the DATABASE's vocabulary (SQLSTATE included),
  never the door's. A pen is the honest record: no verdict about these bytes exists, a human can
  `ack` it, and it **auto-releases** if a later build admits the event (the existing
  `floor.is_some()` delete). The pen's quota and its freeze-at-quota behaviour are unchanged.

`cairn-sync` made the same call one plane over for `do_requeue` ("both belong in the pen — neither is
fixable by retrying"). Its *pull* arm still freezes; converging the two is #626.

### D3 — A malformed-field P0001 joins the deny-all SKIP class, not the pen

After D1 a garbage `event_id` is a door verdict, and door verdicts skip-and-advance: re-offered on
every full sweep, admitted the day this node's build understands it. That is what makes a widened
vocabulary heal on upgrade rather than needing an operator. The general refusal-class partition
(#268) stays open; ADR-0073 carved out substitution, this carves out *malformed field* the other way,
and each carve-out states its reason.

### D4 — The refusal does NOT get its own SQLSTATE

Trap 13(a): giving a refusal a distinctive ERRCODE turns `cairn-sync`'s clinical pen into a freeze,
because `refusal_is_deliberate` reads P0001. The doors keep raising P0001 and the puller keeps
classifying by state, never by sentence.

## Alternatives rejected

- **Re-validate in Rust before applying.** Two parsers for one value are two protocols (#623's
  finding 1). The database is the only authority on what `::uuid` accepts.
- **Wrap the INSERT in `EXCEPTION WHEN check_violation`.** db/001 states the objection: a handler
  that relabels reports an out-of-memory or an internal error as bad caller input. Check the shape
  first; never catch.
- **D2 alone, no door work.** Leaves every refusal illegible (PostgreSQL's bare cast error, no door,
  no field) and pens what should skip.
- **D1 alone.** The next raw cast reopens the freeze, and a source guard only catches the shapes it
  knows.

## What this deliberately does not do

The clinical plane (#626). Node-plane completeness accounting (still absent). #268's general
partition. #620's content-addressing-over-unsigned-bytes finding.

## Paper-parity (§1.2)

Paper-parity: not clinical-surface — this is sync/admission plumbing below the API layer; no
clinician-visible workflow is added or changed.
