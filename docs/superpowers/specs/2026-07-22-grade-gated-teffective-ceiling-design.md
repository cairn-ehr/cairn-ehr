# Design — grade-gated `t_effective` ceiling + born clock-confidence grade (issue #216)

**Date:** 2026-07-22 · **Issue:** [#216](https://github.com/cairn-ehr/cairn-ehr/issues/216)
(2026-07-15 review, finding I3/G) · **Outcome:** ADR-0058 (refines ADR-0003/0027) · **Scope:**
the ADR-0027 graded-interval **envelope floor** + the reworked ceiling + an advisory clash-flag +
a clock-health honesty read. The anchor/notary planes (clock-setting + existence-proof) are
**deferred** to follow-on issues.

## 1. The problem as filed

`data-model.md` §3.6 states `t_effective ≤ t_recorded` as an envelope invariant "rejected/flagged
at write", while §3.17 (post-ADR-0027) makes `t_recorded` a **graded interval**. But the code checks
`t_effective ≤ hlc_wall` against the **point** HLC wall:

- **Write door** — [`db/005_submit.sql:660-663`](../../../db/005_submit.sql#L660-L663) `RAISE`s.
- **Remote door** — [`db/020_apply_remote_event.sql:125-128`](../../../db/020_apply_remote_event.sql#L125-L128)
  hard-`RAISE`s on already-signed foreign events.

The filed finding names the principle-4 violation: a node with a **slow** clock (grade
`self-asserted`) rejects a clinician's *truthful* `t_effective` — a principle-4 violation
manufactured by the mechanism. It asks two questions: which bound does the write-time constraint use
once `t_recorded` is an interval, and should the remote door quarantine-and-flag rather than reject.

## 2. What the code actually says (investigated 2026-07-22)

Two findings sharpen the filing, and one deepens it.

**F1 — the remote-door hard-reject is a live sync-wedge DoS, not merely a principle-4 concern.**
The ceiling `RAISE` at [`db/020:125-128`](../../../db/020_apply_remote_event.sql#L125-L128) fires
*after* signature verification passes. Trace it into the puller: `apply_signed` errors, `do_pull`
re-runs `verify_self_described`, the event **still verifies** (a ceiling violation is unrelated to
the signature), so it takes the `Ok(_)` branch and sets `frozen = true`
([`crates/cairn-sync/src/main.rs:1750-1756`](../../../crates/cairn-sync/src/main.rs#L1750-L1756)).
**The seq cursor halts; every event behind it stops syncing.** A Spike-0002 hostile enrolled writer
inserts one forward-dated event directly into `event_log`, signs it, lets it sync → every peer's
puller freezes: a **one-event denial-of-service on clinical replication**, reachable by the exact
threat model the P1 floor-hardening course was built against.

**F2 — the same door already knows this rule and honors it 220 lines later.** The HLC-drift merge
at [`db/020:349-363`](../../../db/020_apply_remote_event.sql#L349-L363) *clamps-and-admits* rather
than rejecting, with the reason stated outright: *"cairn-sync FREEZES its watermark on ANY refusal
of a verifiable event → rejecting a future-dated clinical event would WEDGE clinical replication —
an availability regression worse than the ratchet (availability over consistency)."* The ceiling
check breaks the rule the drift check honors, in the same function. This is the same argument
ADR-0056 §3 already applied to unknown types.

**F3 — ADR-0027's own interval, read naively, does not fix the slow-clock case.** ADR-0027 §6 says
an offline node's interval is `[monotonic_floor .. RTC]` — **upper = the RTC reading**. But the
slow-clock case is precisely *true-time > RTC*, so a ceiling read against `upper = RTC` **still
rejects the truthful clinician**. The interval as literally stated is an *asymmetric* bracket that
assumes the clock is only ever behind true time; a drifting/failed RTC can be slow, fast, or
years-wrong. Resolving this is the real design work, and it forces a refinement of ADR-0027 §6.

**State of the substrate.** The ADR-0027 graded interval is **spec-only** — [`db/001`](../../../db/001_envelope.sql#L67-L72)
stores `hlc_wall` as a *point*; there is no clock-confidence grade or `[lower, upper]` field. So
"`t_effective ≤ t_recorded.upper`" has no `.upper` to read against yet. This slice builds it.

## 3. The three clock-failure modes that shaped the model

The user (EM physician) enumerated the real range the design must absorb:

| Failure mode | Off from true time | A fixed 24h tolerance? |
|---|---|---|
| Minor RTC drift, no NTP | seconds–minutes | ✓ covered |
| Misconfigured TZ (hwclock-as-localtime) | up to ±14h | ✓ covered |
| **Dead / absent RTC** (stale `fake-hwclock`, epoch-1970) | months–**decades** | ✗ **rejects the truthful clinician** |

The third row is fatal to any *fixed* tolerance and is not exotic: a Raspberry Pi has **no RTC** — a
freshly-booted offline Pi restores a stale last-known time or sits near epoch until it sees a
network. That is a core Cairn deployment target (Bet B is a Pi 5). A fixed 24h ceiling would reject
a truthful 2026 `t_effective` on a node that thinks it is 1970. A single fixed half-width cannot
serve modes spanning seconds to decades: too small rejects the honest dead-RTC clinician, too large
guts the anti-falsification guard.

## 4. Options considered

**(a) Grade-gated ceiling + grade-only representation — CHOSEN.** The clock-confidence grade *gates*
how much rejecting power the ceiling has. A node may call a timestamp "impossible" only to the extent
it can prove it knows the time. One born field (`clock_grade`); the interval is derived from it.

**(b) Fixed drift-tolerance ceiling (`t_effective ≤ hlc_wall + 24h`).** Reuses `cairn_max_hlc_drift_ms`.
Rejected by §3: the dead-RTC/stale-Pi case is off by decades, so any fixed width either rejects the
honest clinician or is so wide the guard is meaningless.

**(c) Explicit stored `[lower, upper]` on the wire.** More faithful to "the interval is data", but it
adds a **signed-body ceiling-bypass**: a hostile enrolled writer sets `upper = +10 years` and defeats
the check, forcing a whole validation sub-floor at both doors to guard a field nothing legitimately
produces this slice (no clock source exists yet). YAGNI + larger floor surface. Rejected; the explicit
bounds arrive additively *with* their producer when the anchor planes land.

## 5. The design (→ ADR-0058)

**(a) One born field: `clock_grade`.** A mandatory clock-confidence grade on every event, the
ADR-0027 ladder:

```
unknown < self-asserted (RTC) < network-synced (NTS/Roughtime)
        < hardware-sourced (GNSS/TPM) < externally-anchored (notary/log)
        < multi-anchor-corroborated
```

`self-asserted` (RTC only) is the **sole minted value this slice** — a node may not *declare* a
higher grade from config, because without a verified source that is exactly the "a declaration can
lie" trap (ADR-0010): a false `hardware-sourced` on a Pi with no GPS would re-arm the reject path on
a node whose clock is actually bad, reintroducing the principle-4 violation. Higher grades become
mintable only when their *verified* producer lands (deferred). `unknown` is never minted — it is the
honest read-default of an event carrying **no** declared grade (foreign / pre-slice). The
`[lower, upper]` interval is **derived**, not stored — a pure function of `hlc_wall` and the grade —
so overlays can tighten it additively when sources land (retrofit-safe). This is the one
can't-retrofit piece.

**(b) The grade gates the ceiling's rejecting power.** The single rule lives in one pure helper both
doors call, `cairn_ceiling_classify(hlc_wall, grade, t_effective) → {ok | flag | reject}`:

| Grade (born) | Derived upper | Classification |
|---|---|---|
| `unknown` / `self-asserted` *(every node today)* | **open above** | `t_effective ≤ hlc_wall` → `ok`; else `flag`; **never `reject`** |
| `network-synced` / `hardware-sourced` / `externally-anchored` / `multi-anchor` *(deferred sources)* | `hlc_wall + W(grade)` | `≤ hlc_wall` → `ok`; `≤ upper` → `flag`; `> upper` → `reject` |

This is **principle 4 applied recursively**: the node's uncertainty *about its own clock* bounds how
much it may constrain the clinician. All three §3 failure modes fall into `self-asserted`, where the
ceiling is **flag-never-reject** — no truthful clinician is blocked, whether the clock is 40 minutes
or 40 years off.

**(c) The action differs by door (strict-submit / lenient-apply, ADR-0051).**
- **Write door** (`db/005`, strict): read the grade (an absent or unrecognized value → `unknown`,
  which the mandatory Rust `EventBody` field already prevents for any conforming client — the
  born-grade invariant is enforced at **compile time**, strictly stronger than a runtime DB check, and
  the door **gates effect, not presence**, per ADR-0056); classify; `reject` only on a `reject` verdict
  (**production-unreachable this slice** — no node mints above `self-asserted`, so the arm is dormant +
  tested by synthesis until verified sources land); else write the flag. The existing
  malformed-`t_effective` wire-pin ([`db/001` `cairn_t_effective`](../../../db/001_envelope.sql#L158))
  is unchanged — that is the only "type-impossible" refusal.
  *(Refinement found during Task-4 planning: an earlier draft had the strict door RAISE on an
  absent/unratified grade. Dropped — a mandatory typed field cannot be omitted by a conforming client,
  so that RAISE only ever fired for a raw/hostile submitter, whom `unknown` handles safely; keeping it
  would have forced a raw-CBOR signing seam into the safety-critical crate for no safety gain.)*
- **Remote door** (`db/020`, lenient): **delete the hard-`RAISE`.** Admit the event unchanged,
  classify, write a clash-flag row on `flag`/`reject`, **never reject on the ceiling** (F1/F2). Absent
  grade → `unknown`. This mirrors the drift clamp-and-admit already in the same door.

**(d) No absolute `t_effective` cap.** A well-formed but absurd `t_effective` (year 9999 fat-finger)
is **flagged, never rejected** on a self-asserted clock — any absolute cap large enough to admit the
honest 56-years-behind dead-RTC case would still be an arbitrary magic constant that ages badly, and
the typo is caught by the flag + the deferred UI clock-alert. This is case-study action item ③
(reject only the physically/type-impossible — the parse floor — advisorily flag the merely
improbable).

**Why it stays safe against fraud.** A fraudster on a self-asserted node can forward-date freely
(admitted + flagged) — but the discrepancy is **flagged** (recorded forever, never auto-resolved),
the **grade brands it untrusted** (*you cannot forge a trusted timestamp on an untrusted clock*, so
the fake carries no evidentiary weight), and rejecting would not stop them (they own the node /
direct-DB per Spike-0002) — it would only punish the honest dead-RTC clinician. The medico-legal
protection comes from the **grade**, not from blocking the write. On a genuinely-synced node the
ceiling regains teeth *and* the records carry real timestamp weight.

**(e) The clock-health honesty read.** A pure DB function in `db/040` reading `hlc_state` (db/001),
`GRANT`ed to the read/agent role like the flag tables:

```
cairn_clock_health() →
  ( rtc_now               timestamptz,   -- the local clock's reading
    hlc_floor             timestamptz,   -- hlc_state.wall, ratcheted past every accepted event
    behind_by_ms          bigint,        -- hlc_floor − rtc_now when positive
    is_behind             boolean,       -- clock provably behind accepted events (past a tolerance)
    effective_lower_bound timestamptz,   -- max(rtc_now, hlc_floor): "the time is AT LEAST this"
    default_grade         text )         -- the node's declared clock_grade
```

`is_behind` is the risk signal; `effective_lower_bound` is the "at least after…" causal lower bound
the HLC A3 merge already maintains ([`db/020:366-372`](../../../db/020_apply_remote_event.sql#L366-L372)).
It is a **live derived read, never stored, never an event, never synced** — the same honest-assembly
shape as sync-freshness and backup-health (ADR-0027 §7). The CLI `status` renders one line from it;
the Tauri UI reads the row through the native API (principle 12: one fact, plural front-ends). No
door calls it.

**(f) The advisory clash-flag — a door-side write, not a registered projection.**
`t_effective_ceiling_flag`, append-only alarm table on the
[`identity_projection_flag`](../../../db/018_identity_linkage.sql#L141-L188) pattern: `flag_id`
identity PK, the classification + `hlc_wall` + `t_effective` + `clock_grade`, `flagged_at`, and a
`content_address BYTEA` keyed by a NULLS-DISTINCT unique index (`ON CONFLICT DO NOTHING`) for
set-union re-delivery idempotency. **Both doors write it via a shared helper**
`cairn_record_ceiling_flag(...)` on a `flag`/`reject` verdict — *not* an ADR-0057 registered
projection, because [`cairn_projection_dispatch`](../../../db/005_submit.sql#L162-L177) keys strictly
on `event_type` and the ceiling is **cross-type** (any event with a `t_effective`). The floor/door
layer is where a cross-type envelope check belongs. It survives `cairn_reproject` untouched: rebuild
replays `event_log` through the *dispatch*, never the doors, so an arrival-recorded flag is permanent
and correct (its inputs — `hlc_wall`/`clock_grade`/`t_effective` — are immutable). No twin/registry
row-count bump (no new event type). `GRANT SELECT` to `cairn_agent`.

## 6. Components changed

| Layer | Change |
|---|---|
| `cairn-event` (`lib.rs`) | `ClockGrade` enum + a **mandatory** `clock_grade` field appended to `EventBody`; CBOR/additive-only (existing signed bytes never re-encoded → still verify; `#[serde(default)]` reads legacy as `unknown`); minted `self-asserted` |
| **new `db/040_clock_confidence_grade.sql`** | the ordered `cairn_clock_grade` domain + rank; `cairn_ceiling_upper_ms`/`cairn_ceiling_classify(...)`; `cairn_record_ceiling_flag(...)`; `cairn_clock_health()`; `ALTER TABLE event_log ADD COLUMN clock_grade … DEFAULT 'unknown'`; the `t_effective_ceiling_flag` table. **Bumps `SCHEMA_GENERATION` 39→40**; **added to cairn-sync's `SCHEMA` subset** (db/020 references the helpers + flag) |
| `db/005` (strict) | require + validate `clock_grade` from the body; classify; reject only on `reject`; else record flag; add `clock_grade` to the `event_log` INSERT |
| `db/020` (lenient) | delete the ceiling `RAISE` ([db/020:124-129](../../../db/020_apply_remote_event.sql#L124-L129)); admit unchanged; classify; record flag; never reject on the ceiling; absent grade → `unknown`; add `clock_grade` to the INSERT |
| emit paths (`cairn-node`/`cairn-sync`) | mint `clock_grade = self-asserted` at write (Task 1 set it in every `EventBody`; Task 6 also adds it to `emit_event`'s direct `event_log` INSERT so the author's own row matches the signed body). Higher grades unreachable until a verified source lands |
| ~~legibility twin~~ | **DEFERRED** to a follow-on — rendering the grade in the twin means changing the Rust `plaintext_twin` *and* the SQL `cairn_twin_skeleton`/`cairn_event_twin` floor in lockstep or the demographic twin-match floor refuses; cosmetic gain (the grade is already legible via the `clock_grade` column + `cairn_clock_health`), not worth the desync risk this slice |

## 7. Data flow

- **Write:** mint grade → `cairn_ceiling_classify` → `ok` / `flag` (+row) / `reject` (dormant).
- **Remote:** verify → admit unchanged → classify → `flag` row if above point → HLC merge (unchanged).
  Cursor advances; **no freeze**.

## 8. Testing (TDD, RED first)

- **Headline (the DoS fix):** `db/020` — a signed `t_effective > hlc_wall` event **applies** (no
  `RAISE`) + a flag row is written; `do_pull` does **not** freeze the cursor (regression against F1).
- `db/005`: self-asserted forward `t_effective` → admit + flag (**currently RED**: rejects);
  synthesized high-grade above `W` → reject; within `W` → flag; clean backdate → no flag. (The
  born-grade invariant is a compile-time Rust guarantee — no strict-door presence RAISE to test.)
- `cairn-event`: `clock_grade` CBOR round-trip; a legacy signed blob still verifies (additive-only);
  legacy body deserializes to `unknown`.
- `t_effective_ceiling_flag`: set-union re-delivery idempotency (content_address dedup, `ON CONFLICT
  DO NOTHING`); survives a `cairn_reproject` rebuild untouched (arrival-recorded, door not re-run).
- `cairn_clock_health()`: detects a provably-behind clock (RTC forced behind `hlc_state.wall`).
- `emit_event` stores `clock_grade` on the author's own row (matches the signed body). (Twin grade-line deferred — see §9.)

## 9. Scope boundaries

- **Can't-retrofit field, so the born grade is day-one.** Adding a mandatory field means pre-slice
  dev/PoC events lack it and read as `unknown` — **wipe dev/PoC rigs** (established ADR-0051/0052
  pattern; never sync pre-slice logs through).
- **Mint is constrained to `self-asserted`.** No config-declared higher grade this slice, so the
  ceiling's reject arm is production-unreachable and no mis-declaration can re-arm it against a
  truthful clinician. Verified higher grades arrive with the deferred anchor planes.
- **Ordering is untouched.** ADR-0027 §1: the HLC stays the sole basis for causal *ordering*; the
  graded interval is orthogonal wall-clock *truth*. No projection `ORDER BY hlc_wall` changes.
- **New migration file.** `db/040_clock_confidence_grade.sql` bumps `SCHEMA_GENERATION` 39→40 (the
  #188 downgrade guard) and joins cairn-sync's `SCHEMA` subset (db/020 references its helpers + flag
  table via PL/pgSQL late-binding, so it must load in that subset). The `event_log` column is added by
  `ALTER … ADD COLUMN IF NOT EXISTS` for existing DBs (#207 paired-ALTER discipline).
- **Deferred (follow-on issues to file):**
  1. Anchor/notary planes — clock-setting (NTS/Roughtime/GNSS/TPM) + existence-proof
     (transparency-log/FROST) + the overlay grade-upgrade tokens (nothing *produces* a grade above
     `self-asserted` until this lands).
  2. Causal lower-bound *tightening* into the stored/derived interval (`t_recorded.lower =
     max(derived, floor over high-confidence events)`).
  3. The UI clock-sanity alert (reference-UI; reads `clock_grade` + `cairn_clock_health()` + the flag).
  4. Auto-**downgrade** the grade when a node detects its own clock failed.
  5. Exercise the numeric `W(grade)` table once real sources exist.
  6. Render the clock grade in the legibility twin (needs the Rust `plaintext_twin` and the SQL
     `cairn_twin_skeleton`/`cairn_event_twin` floor changed in lockstep so the demographic twin-match
     floor still passes).

## 10. Spec homes

- `data-model.md` §3.6 — the ceiling is grade-gated: reject only when the grade is credible and
  `t_effective > hlc_wall + W(grade)`; otherwise flag; the remote door admits-and-flags.
- `data-model.md` §3.17 — the grade gates rejecting power; the `[lower, upper]` interval is derived
  from the grade this slice; record the **ADR-0027 §6 refinement** (`upper = RTC → RTC + W`, and open
  above at `self-asserted`/`unknown`); `cairn_clock_health()` as the honest-assembly read.
- ADR-0058 — the *why*, refining ADR-0003 (the ceiling) and ADR-0027 (the graded interval).
