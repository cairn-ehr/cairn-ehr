# ADR-0074 — A deterministic door failure is a refusal, not a fault

- **Status:** Accepted
- **Date:** 2026-09-20
- **Spec version at acceptance:** 0.76
- **Issues:** [#621](https://github.com/cairn-ehr/cairn-ehr/issues/621) ·
  [#626](https://github.com/cairn-ehr/cairn-ehr/issues/626) (the clinical plane's half, filed here) ·
  [#228](https://github.com/cairn-ehr/cairn-ehr/issues/228) (the same class, closed for hex only)
- **Relates to:** [ADR-0073](0073-the-node-plane-refuses-a-substitution-and-pens-it.md) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) ·
  [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
- **Amends:** nothing. It completes the P0001 contract stated above
  `cairn_decode_hex_or_raise` (`db/001`) for the cases #228 did not reach, and corrects one
  sentence of ADR-0073's design reasoning (see Context).

## Context

The node puller (`crates/cairn-node/src/sync.rs`, `pull_into`) reads a refusal's **SQLSTATE** to
decide what a failed apply means:

- **`P0001`** — a bare `RAISE EXCEPTION`, i.e. the door reached a **verdict**. Skip past it and
  advance; a later full sweep re-offers it, and a later build may admit it (ADR-0056's
  self-healing deny-all). Since ADR-0073, one class of verdict is penned instead: a substitution,
  which can never apply because the id is taken.
- **anything else** — *"the door never got to decide"*: a deadlock, a statement timeout, a dropped
  connection. **Freeze** the cursor below that event and retry next cycle, because advancing past a
  valid event this node merely failed to store would silently lose it (the #111 review's A1).

That partition has a hole. A door can fail **deterministically without reaching a verdict**, and
`db/007` did so in four places on caller-supplied bytes: `(b ->> 'event_id')::uuid` and
`NULLIF(payload ->> 'target_event_id','')::uuid` (`22P02`), the `node_event_hlc_nonneg` CHECK and
the `node_event_role_check` CHECK (`23514`). Each recurs identically on every retry, so the freeze
is **permanent**: the link holds every later event behind the poison one — including that peer's
own `peer.revoked` — nothing is penned, and so there is no `ack` remedy either. The operator sees
`transient/unexpected error … — freezing`, every cycle, for ever.

This is [#228](https://github.com/cairn-ehr/cairn-ehr/issues/228)'s failure exactly: a bare
`decode()` raising in the 22 class froze one peer's cursor permanently. #228 closed it for hex and
left it open for casts and constraints — and recorded the P0001 rule as a contract precisely so
this could not recur.

It also corrects one sentence of #619's design, which ADR-0073 carried: *"a non-P0001 failure still
freezes; the retry will reach the door's guard and come back as a P0001."* An error raised **before**
the guard never reaches it.

### Who can trigger it — checked, not assumed

`serve` streams `SELECT seq, signed_bytes FROM node_event WHERE seq > $1`: **only rows already in
the serving peer's own log**, which passed that node's identical casts and CHECKs. An honest peer on
the same schema therefore cannot serve one of these, and #621's headline scenario ("any trusted peer
serving a stranger-signed event") is not reachable that way. Two triggers remain:

1. **A misbehaving or compromised trusted peer** crafting frames. It wedges only its own link, and
   it could stall that link anyway by going silent — so the marginal harm is dishonesty rather than
   denial: the stall is reported as transient, never heals, and has no remedy.
2. **Cross-version CHECK-vocabulary skew** — the one that matters under principle 11. `db/009`
   already widened the `op` CHECK in place. The day `role` (or any other constrained column) is
   widened the same way, every older node pulling from a newer peer freezes that link permanently.
   **A vocabulary widening must never be able to partition the fleet**; that is the whole reason
   the vocabulary is now enforced at the door, where it refuses with a code the puller can skip.

## Decisions

### 1. The node-plane doors are TOTAL for every field they CAST: each raises P0001

Three helpers, each naming **field, door and reason**, at all three signed-bytes doors
(`submit_node_event`, `apply_remote_node_event`, `restore_node_event`):

- **`cairn_uuid_or_raise(field, value, door)`** (`db/001`), gating on
  `pg_input_is_valid(value, 'uuid')` — *the cast's own grammar, asked of the same server*. No second
  parser exists, so none can drift. A validator narrower than the cast would refuse events the log
  can already hold, which is the mirror image of the pen bypass PR #623's review found, where Rust's
  `uuid` crate was narrower than the door's `::uuid`.
- **`cairn_hlc_nonneg_or_raise(wall, counter, door)`** (`db/001`), before the INSERT meets the CHECK.
- **`cairn_node_roles()`** (`db/007`) — the vocabulary as **one function, which the table's CHECK
  itself calls** — with `cairn_node_role_or_raise(role, door)` raising through it. One list, so the
  door and the floor cannot disagree.

Both shared helpers live in `db/001` for `cairn_decode_hex_or_raise`'s reason: cairn-sync replays a
subset containing `db/001` but not `db/007`, and PL/pgSQL binds a call at first execution (#198).

The **CHECK constraints stay**. They are the floor for a caller with raw SQL (principle 12's
privilege gradient); the door's refusal is the legible, skippable one. The re-pointed role
constraint is added **`NOT VALID`**: `connect_and_load_schema` replays every migration on every
connect, so a validating `ADD CONSTRAINT` would re-scan `node_event` each time and one stored row
outside today's vocabulary — which is exactly what a downgrade after a widening leaves — would stop
the node STARTING, on an append-only table with no repair path. New rows are still checked, which
is what the floor is for.

**Two honest limits of "total", both named rather than assumed away:**

- **`cairn_body` raises `22P05` before every guard** if any body string contains `U+0000`, because
  `jsonb` cannot represent it while a CBOR text string can. The bytes verify, so this reaches the
  door and is deterministic ([#628](https://github.com/cairn-ehr/cairn-ehr/issues/628)). Decision 2
  catches it — the link keeps moving — but the event is penned where decision 3 says it should
  skip. Pinned end to end by `a_body_string_carrying_a_nul_does_not_freeze_the_link`, which asserts
  only the freeze-freedom, so closing #628 cannot break it.
- **The surviving `::bigint` / `::int` casts on the HLC are safe by TYPING, not by a helper**:
  `cairn_event::Hlc` declares `i64`/`i32` with no serde default, so a body that cannot produce them
  fails verification. That is a guarantee in another crate, and it is pinned at compile time by
  `node_door_input_guards.rs::the_hlc_casts_rest_on_cairn_events_types`.

### 2. The puller partitions the non-P0001 space: deterministic ⇒ PEN, local ⇒ FREEZE

Decision 1 fixes four known raises. Decision 2 closes the class — including an `XX000` out of a
pgrx function fed adversarial bytes, and any cast a future slice writes.

`deterministic_apply_failure(sqlstate)` claims the **local** classes explicitly — `08 40 42 53 55
57 58`, plus *no SQLSTATE at all* — and answers "deterministic" for everything else. On a verifiable
event whose apply failed:

- **local** → freeze, exactly as before;
- **deterministic** → **pen**, with the reason in the **database's** vocabulary (SQLSTATE included),
  never the door's — writing a non-verdict in the door's voice is #480's defect. The pen is durably
  held, loud through `pending`, ack-able, and **auto-releases** if a later build admits the event.

The list and its reasoning are `cairn-sync`'s `apply_failure_is_local`, whose `do_requeue` already
routes on it one plane over; the two copies are held equal by a test until #626 merges them.

**The default for an unknown code is *deterministic*, and the asymmetry is deliberate.** A wrong
"deterministic" pens a valid event — delayed, held, re-offered, auto-released. A wrong "local"
freezes the link for ever with no remedy. The cheaper mistake is the one that keeps the link moving.

### 3. A malformed-field P0001 joins the SKIP class, not the pen

After decision 1 a garbage field is a door **verdict**, and verdicts skip-and-advance: re-offered on
every full sweep, admitted the day this node's build understands the event. That is what lets a
widened vocabulary heal on upgrade instead of needing an operator. ADR-0073 carved *substitution*
out of the skip class because it can never apply; this carves nothing out — it puts malformed
fields where the self-healing class belongs. The general refusal-class partition
([#268](https://github.com/cairn-ehr/cairn-ehr/issues/268)) remains open.

### 4. The refusal does not get its own SQLSTATE

`USING ERRCODE` on any of these would turn `cairn-sync`'s clinical pen into a freeze, since
`refusal_is_deliberate` reads `P0001`. The doors raise P0001 and the puller classifies by **state**,
never by sentence — ADR-0073's rule, restated because decision 2 makes a distinctive code look
newly attractive.

## Consequences

- One poison event from a misbehaving peer no longer costs that link every later event, and the
  outcome is visible: a pen row an operator can read and ack, instead of a "transient" line.
- A future constrained column or widened vocabulary cannot partition the fleet: an older node
  refuses legibly and skips.
- An operator sees more pen rows than before, by design — a deterministic failure that used to be
  reported as transient is now recorded as what it is.
- Two planes, one classifier, not yet one function (#626).

## What this deliberately does not do

The clinical plane (#626: `db/020`'s raw casts and `do_pull`'s freeze arm). Node-plane completeness
accounting, which still does not exist. #268's general partition. #620's
content-addressing-over-unsigned-bytes finding.

## Evidence

`crates/cairn-node/tests/node_door_refusals_are_p0001.rs` (behaviour, all three doors × four
fields, with the odd-spelling positive control), `node_door_input_guards.rs` (the catalogue rules:
every door calls every guard; no bare `::uuid` survives in any door body; the CHECK reads the one
vocabulary; the CHECK still refuses a raw INSERT), `node_pull_refusal_class.rs` (the pure
classifier, both directions and the unknown-code default), `node_pull_deterministic_refusal.rs`
(the three outcomes end to end over the real self-pull, plus the pen-write freeze, the `22P05`
path above and an anti-vacuity control), `sqlstate_classes_agree.rs` (the two planes' lists).
Thirteen mutations, thirteen killed — ledger in
`docs/superpowers/plans/2026-09-20-node-pull-deterministic-refusal-621.md`.

## What the PR review changed

`XX001`/`XX002` (data_corrupted / index_corrupted) are claimed as **local** on both planes: class
`XX` is otherwise the adversarial-bytes case, but a corrupt page or index is this machine's disk,
and without the exception a corrupt index on `node_event` would have made the puller pen a peer's
entire log while writing *"will fail on these bytes identically every time"* onto every row — a
local catastrophe wearing the peer's name. It is the only realistic case of that shape, because
anything that breaks the pen table's writes too makes `pen_or_freeze` freeze and say so.

A pen row of this kind leaves the pen by **applying** or by an **ack** — never by a later P0001
verdict about the same bytes, because the deny-all arm cannot tell which KIND of row it would be
deleting without reading the reason TEXT (the one thing the loop never classifies on) and a
substitution row must never auto-release. Every operator-facing sentence now says exactly that;
before the review three of them still enumerated two pen causes and promised "fix the cause".
