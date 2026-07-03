# C2b — auto-apply of the matcher's `auto_candidate` band

**Date:** 2026-07-03 · **Area:** matcher/identity (§5.2/§5.7/§7.5) · **Crate:** `cairn-node`
**Status:** design approved, pre-implementation.
**Implements (no new decision):** ADR-0011 (actor registry / version-pinning / key custody),
ADR-0014 (matcher is a registered actor, recall via contamination cascade), ADR-0028 (closed
contributor-role enum — `suggested`), ADR-0029 (skill-epoch as pinned actor determinant),
ADR-0030 (advisory-actor integration contract). **No spec/ADR bump. No `db/` migration. No
floor change. No SCHEMA-array bump.**

---

## 1. What this is

The identity build so far:

- **C1** (`db/018`) — the §5.1/§5.7 linkage core: `identity.link.asserted` / `unlink` event
  types through the reused `submit_event` door, the `patient_link` HLC-overlay edge table, and
  the `person_member` connected-component projection.
- **C2** (`cairn-node::apply_proposal`) — a **human-ACCEPTED** `match_proposal` (db/017) becomes a
  **human-ATTESTED** link event: the accepting human is a responsibility-bearing contributor, which
  trips the db/005 attestation gate.

**C2b is the other band.** A proposal the matcher banded **`auto_candidate`** (score ≥ the auto
threshold **and zero veto findings** at propose time — see `banding.py`) is applied **without a
human**: a **matcher-authored, un-attested, recallable** `identity.link.asserted` event through the
*same* `submit_event` door.

The db/018 header already anticipated this exact case:

> additive + `targets_other_author=FALSE`: a link neither suppresses nor targets another author's
> event, so the existing gate requires NO attestation for a matcher-authored link (§5.2 "auto above
> threshold"); a human who vouches simply includes a responsibility-bearing contributor.

So the **floor is already correct**. C2b's whole job is: (1) make the matcher a real enrolled actor
so its signature is trusted and its links are recall-addressable, and (2) a driver that builds,
signs, and applies the auto-band — with one safety addition (a fresh veto re-check).

---

## 2. Design decisions (settled during brainstorming)

### 2.1 The matcher is a **per-epoch actor with a fresh key per config**

`submit_event` step 2 requires the signer to be an enrolled, non-revoked actor. So the matcher must
be enrolled. The chosen model (the deferred §7.5 piece, now built):

- **One `agent` actor per `matcher_version`.** `matcher_version` (already `"{pkg}+{weights-digest}"`,
  ADR-0014's config-pin, present on every proposal) **is** the pinned epoch (ADR-0029). The pinned
  determinant set is `{"kind":"agent","actor":"cairn-matcher","matcher_version":"<v>"}`; the actor_id
  is its content-address (`cairn_actor_id`, in-DB).
- **A fresh signing key per epoch.** Because each epoch has its own key AND its own actor_id, and
  `submit_event` step 2 stamps `event_log.actor_id` only when the key→actor mapping is **unique**, a
  per-epoch key gives **unique attribution**: `event_log.actor_id` is stamped precisely, so a
  contamination-cascade recall of a bad config (`events_by_actor_epoch`, db/006) selects **exactly**
  that config's auto-links — not every matcher link ever. This is the payoff that matters most on the
  human-less path.
- **Auto-enrolled on first sight**, as an **owner-run ceremony** (the auto-apply CLI connects as
  owner, exactly like the existing `enroll` subcommand — `enroll_actor` is owner-privileged and the
  runtime `cairn_agent` role cannot enroll, by the db/004 trust-anchor floor). Idempotent: enroll only
  if `actor_current` has no row for the actor.

### 2.2 Re-check the veto **at apply time**

The `auto_candidate` band was computed at **propose** time. Between propose and apply, a new
demographic assertion could introduce a hard veto (e.g. a verified-DOB clash). With **no human
backstop** on this path, the driver re-runs the db/016 veto floor immediately before signing:

- `EXISTS (SELECT 1 FROM cairn_match_veto(low, high))` — **any** severity (matching `banding.py`'s
  "any veto forbids `auto_candidate`", hard_veto *or* degrade_hold).
- If a veto now exists ⇒ **do not auto-link**; flip the proposal to `status='review'` (kick it to a
  human) and move on. Honours ADR-0014 "never auto-link over a veto, never auto-reject."

### 2.3 Contributor shape — principle 10 made literal

The link's sole contributor is the matcher: `{"actor_id": matcher_kid, "role": "suggested"}` — **no
`responsibility` key**. `suggested` is ADR-0028's *contributory* (non-bearing) enum member for exactly
"an advisory agent proposed this." No responsibility ⇒ `v_bears=false` ⇒ `submit_event` demands no
attestation. **Authorship present, accountability absent** — principle 10 on the auto path.

### 2.4 Status & idempotency

`match_proposal.status` is free TEXT (no CHECK). Transitions the driver makes:

| From | Condition | To | Event? |
|---|---|---|---|
| `pending` (band `auto_candidate`) | no veto now | `auto_applied` (+ `applied_event_id`) | link event appended |
| `pending` (band `auto_candidate`) | veto appeared | `review` | none (human owns it) |

`auto_applied` is deliberately **distinct** from C2's human `applied`, so the audit trail tells auto
from human at the worklist level (the authoring actor on the event also distinguishes them). Re-runs
select only `band='auto_candidate' AND status='pending'`, so applied/review/human-rejected rows are
never re-touched — the driver is idempotent and respects a human `rejected`.

### 2.5 Matcher key at rest

Per-epoch key **sealed under the operational passphrase**, reusing the `keystore` seal path. **No
separate recovery escrow**: a matcher key is regenerable — losing it only retires that epoch (it
authors no *more* links; existing links stand or are recalled), so paper-escrow semantics don't fit.
Honest ceiling, documented.

---

## 3. Components (all in `cairn-node`)

### 3.1 `src/matcher_actor.rs` (new)

```
// pure
matcher_pinned(matcher_version: &str) -> serde_json::Value
    // {"kind":"agent","actor":"cairn-matcher","matcher_version": v}
matcher_key_filename(matcher_version: &str) -> String   // sanitized, collision-free per epoch

// IO
resolve_matcher_actor(client, keystore_dir: &Path, secret: Option<&str>, matcher_version: &str)
    -> anyhow::Result<(SigningKey, String /*kid*/)>
    //  load-or-(generate+seal) the per-epoch key file;
    //  ensure enroll_actor('agent', matcher_pinned(v), kid) — idempotent via actor_current check.
```

### 3.2 `src/auto_apply.rs` (new, sibling of `apply_proposal.rs`)

```
// pure
build_suggested_link_body(event_id, low, high, provenance, confidence, matcher_kid, hlc) -> EventBody
    //  contributor {actor_id, role:"suggested"} — NO responsibility; authored twin (render_link_twin)
compose_auto_provenance(matcher_version: &str) -> String   // "matcher:{v} auto"

// IO — one proposal, one transaction
apply_auto_candidate(client, low, high, matcher_sk, matcher_kid, hlc) -> Outcome
    //  FOR UPDATE; require band='auto_candidate' AND status='pending';
    //  re-check veto -> Outcome::VetoedToReview (status='review');
    //  else sign + SELECT submit_event($1) (1-arg, no token) + status='auto_applied'.

// IO — batch driver
apply_auto_candidates(client, keystore_dir, secret, hlc_source) -> Summary
    //  select pending auto_candidate pairs (+ their matcher_version);
    //  resolve_matcher_actor once per epoch; apply each pair in its own txn,
    //  skip-and-report per pair (mirrors pipeline/sweep.py). Summary{applied, vetoed_to_review, skipped}.
```

`Outcome` = `Applied(Uuid) | VetoedToReview | Skipped(reason)`. Per-proposal transactions so one bad
pair never rolls back the batch.

### 3.3 CLI `apply-auto-candidates` (owner-run)

Wraps `apply_auto_candidates`, prints the summary. Fails fast with a legible hint if the DB predates
db/018 (needs the identity floor). Owner connection (needs `enroll_actor`).

### 3.4 HLC

Each event's HLC is sourced from the node clock per event (the same `node_hlc_tick` path the existing
node authoring uses); the pure builders take `Hlc` as a parameter so they stay deterministic/testable.

---

## 4. What is explicitly reused unchanged

- `submit_event` (db/005) — the 1-arg (un-attested) path, already present via `DEFAULT NULL` args.
- `cairn_check_link_assertion` + `patient_link_apply` trigger + `person_member` projection (db/018).
- `cairn_match_veto` (db/016) — the apply-time re-check.
- `enroll_actor` / `actor_current` (db/004) — the matcher actor.
- `events_by_actor_epoch` (db/006) — recall; **no new recall code**.
- `keystore` seal/load; `cairn-event::identity` builders + `sign`; `match_proposal.applied_event_id`.

No file above is modified.

---

## 5. Testing (TDD, red first)

**Pure unit** (`auto_apply.rs`, `matcher_actor.rs`):
1. `matcher_pinned` is deterministic and carries `kind=agent` + the exact `matcher_version`.
2. `build_suggested_link_body`: contributor role `"suggested"`, **no** `responsibility` key, authored
   twin present, canonical subjects (subject_a := low).
3. `compose_auto_provenance` names the version + `"auto"` and contains **no** `"accepted-by"`.

**DB-gated integration** (`tests/auto_apply.rs`, gated on `CAIRN_TEST_PG`, `db::test_serial_guard`):
4. Happy path: an `auto_candidate`+`pending` proposal ⇒ exactly one `identity.link.asserted` appended
   with **no** attestation token, the standing edge exists, both patients project to the min-UUID
   person, `status='auto_applied'`, `applied_event_id` set.
5. Matcher actor: enrolled `kind='agent'` on first sight; a second pair of the **same** epoch reuses
   the same actor/key (no duplicate `enroll` row).
6. Veto-appeared: seed a proposal `auto_candidate`, then assert a verified-DOB clash ⇒ apply flips it
   to `status='review'`, **no** event, **no** edge.
7. Idempotency: a second `apply_auto_candidates` run applies nothing new.
8. Human disposition respected: a `status='rejected'` auto_candidate is never auto-applied.
9. Recall precision: a contamination-cascade recall over the matcher epoch (`events_by_actor_epoch`)
   selects the auto-link; a recall over a **different** epoch does not.
10. Two epochs (`matcher_version` A vs B) ⇒ two distinct `actor_id`s and two distinct keys.

Test command:
`cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test auto_apply`
(PG18 + `cairn_pgx`). Pure tests run under plain `cargo test`.

---

## 6. Honest ceilings / deferred (file issues where load-bearing)

- **No background scheduler.** Operator-invoked CLI only; a daemon loop that auto-applies on a timer
  is future work (YAGNI for the first slice).
- **Matcher key not recovery-escrowed.** Sealed under the op passphrase, but regenerable — no paper
  escrow. Documented, not a bug.
- **ADR-0028 role enum not DB-enforced yet** (issue #96). `suggested` is chosen forward-correctly, but
  the floor validates only the presence/absence of `responsibility`, not the role string.
- **`matcher_pinned` pins only `matcher_version`.** That string is itself `pkg+weights-digest`; richer
  ADR-0029 determinants (served-model digest, etc.) are future — matches the current advisory-only,
  weights-only matcher.
- **Already-linked pair re-proposed.** If the matcher re-runs and emits a fresh `pending`
  auto_candidate for an already-linked pair, a duplicate link event is appended — harmless
  (set-union; the `patient_link` overlay is idempotent on the same edge), just event-log growth.

---

## 7. Why no new ADR

The matcher-as-registered-actor is **already decided**: ADR-0014 ("The matcher is a registered actor …
recall via contamination cascade"), ADR-0029 (skill-epoch as pinned determinant), ADR-0030 (the
advisory-actor integration contract). ADR-0010/0028 fix the additive classification and the role enum.
C2b **implements** these; it introduces no new decision, no new event type, no floor change.
