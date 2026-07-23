# ADR-0058 — Grade-gated `t_effective` ceiling: the clock-confidence grade bounds rejecting power, the remote door never rejects

- **Status:** Accepted
- **Date:** 2026-07-22
- **Refines:** [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md), [ADR-0027](0027-trusted-time-anchoring.md)

## Context

[ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) made `t_effective ≤ t_recorded` an
envelope invariant — "a violation is *prima facie* falsification, rejected/flagged at write."
[ADR-0027](0027-trusted-time-anchoring.md) then made `t_recorded` a **graded interval**, not a
point, carrying a clock-confidence grade
(`unknown < self-asserted < network-synced < hardware-sourced < externally-anchored <
multi-anchor-corroborated`). But that graded interval was **spec-only**: nothing in the substrate
stored a grade or a `[lower, upper]` bound, so "`t_effective ≤ t_recorded.upper`" had no `.upper` to
read against — both admission doors instead checked `t_effective` against the bare point HLC wall.
Investigating [issue #216](https://github.com/cairn-ehr/cairn-ehr/issues/216) (filed from a
2026-07-15 review, finding I3/G) surfaced two coupled defects in that point-ceiling check, and a
third finding that ADR-0027's own §6 offline interval, read literally, does not fix the first.

**A principle-4 violation reachable by every deployment target.** The point ceiling rejects a
truthful clinician whenever the node's own clock reads *behind* true time — and a Raspberry Pi (a
core Cairn deployment target; Bet B is a Pi 5) has **no RTC**: a freshly-booted offline Pi restores a
stale last-known time or sits near epoch-1970 until it next sees a network. The user (an EM
physician) enumerated the real range a fix must absorb — minor RTC drift (seconds–minutes, a fixed
tolerance covers it), a misconfigured timezone (up to ±14h, still coverable), and a **dead or absent
RTC** (months to **decades** off, fatal to any *fixed* tolerance: too small still rejects the honest
dead-RTC clinician, too large guts the anti-falsification guard entirely). The point ceiling turns an
honest hardware limitation into a manufactured falsification finding — exactly the confident-untruth
failure principle 4 exists to forbid, and the mechanism itself is the one manufacturing it.

**A remote-door hard-reject is a live sync-wedge denial-of-service, not merely a principle-4
concern.** The ceiling `RAISE` in `apply_remote_event` fires *after* signature verification passes —
a ceiling violation is unrelated to the signature, so a forward-dated event still verifies. Tracing
the error into the puller: `do_pull` re-verifies, the event still passes, and the lenient-apply error
path sets `frozen = true` on the seq cursor — **every event behind it stops syncing.** A hostile
enrolled writer under the [Spike-0002](../../spikes/0002-advisory-actor-write-contract.md) threat
model (direct DB access, a valid signing key) need only insert and sign **one** forward-dated event
to freeze every peer's pull from that node: a one-event denial-of-service on clinical replication,
reachable by the exact threat model the floor-hardening course was built against. The same door
already knows and honors the opposite rule 220 lines later, for the HLC-drift merge, stated outright:
rejecting a future-dated but verifiable event would wedge clinical replication, an availability
regression worse than clamping (availability over consistency). The ceiling check broke the rule the
drift check already honored, in the same function.

**ADR-0027 §6's literal offline interval does not fix the slow-clock case either.** §6 states an
offline node's interval as `[monotonic_floor .. RTC]` — **upper = the RTC reading**. But the
dead-RTC/slow-clock failure mode is precisely *true time > RTC*, so a ceiling read against
`upper = RTC` still rejects the truthful clinician: the interval as literally stated is an
*asymmetric* bracket that assumes the clock can only run behind true time, when a drifting or failed
RTC can be slow, fast, or years-wrong in either direction. Resolving the two defects above forces this
correction to ADR-0027 §6 as part of the same design.

## Decision

**The clock-confidence grade gates how much rejecting power the ceiling has.** A node may call a
`t_effective` "impossible" only to the extent it can prove it knows what time it is.

1. **One born field, mint constrained to `self-asserted`.** Every event carries a mandatory
   `clock_grade` on the ADR-0027 ladder. `self-asserted` (RTC only) is the **sole value this slice may
   mint** — a node may not *declare* a higher grade from configuration, because an undemonstrated
   `hardware-sourced` claim on hardware with no verified clock source would re-arm the reject path on
   a node whose clock is actually bad, reintroducing the exact principle-4 violation this ADR closes.
   Higher grades become mintable only when their verified producer lands (deferred, below). `unknown`
   is never minted; it is the honest default reading of any event carrying no declared grade
   (foreign traffic, pre-slice events).

2. **The interval is derived, never stored, this slice.** `[lower, upper]` is a pure function of the
   HLC wall and the grade, not a signed field on the wire — an explicit stored bound would be a
   ceiling-bypass a hostile enrolled writer could set arbitrarily wide, defended only by a validation
   sub-floor guarding a field nothing legitimate produces yet (no verified clock source exists this
   slice). The bound arrives additively, alongside its producer, when the anchor planes (below) land —
   an overlay tightening, never a rewrite, consistent with how certainty already refines by overlay
   ([§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)).

3. **The classification table.**

   | Grade (born) | Derived upper | Classification |
   |---|---|---|
   | `unknown` / `self-asserted` *(every node today)* | **open above** | `t_effective ≤ hlc_wall` → `ok`; else `flag`; **never `reject`** |
   | `network-synced` / `hardware-sourced` / `externally-anchored` / `multi-anchor` *(deferred sources)* | `hlc_wall + W(grade)` | `≤ hlc_wall` → `ok`; `≤ upper` → `flag`; `> upper` → `reject` |

   This is principle 4 applied recursively: the node's uncertainty *about its own clock* bounds how
   much it may constrain the clinician. All three enumerated failure modes fall into
   `unknown`/`self-asserted`, where the ceiling is **flag-never-reject** — no truthful clinician is
   blocked, whether the clock is 40 minutes or 40 years off.

4. **The two doors act asymmetrically on the same classification (strict-submit / lenient-apply,
   [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)).** The
   **write door** (strict) rejects only on a `reject` verdict — production-unreachable this slice,
   since no node mints above `self-asserted`, kept live and tested by synthesis until a verified
   source lands. The **remote door** (lenient) never rejects on the ceiling at all: it admits the
   event unchanged, classifies it, and records a `flag`/`reject` verdict as an advisory clash row —
   closing the sync-wedge DoS by mirroring the drift-clamp rule the same door already honored. Both
   doors write the advisory row through one shared helper on a `flag`/`reject` verdict; it is a
   cross-type door-side write, not an [ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md)
   registered projection (the ceiling applies to any event carrying a `t_effective`, not one
   `event_type`), so it survives a `cairn_reproject` rebuild untouched — rebuild replays through the
   dispatch, never the doors, and the flag's inputs (`hlc_wall`/`clock_grade`/`t_effective`) are
   already immutable.

5. **The door gates effect, not presence — consistent with
   [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md).** An absent or unrecognized grade
   reads as `unknown`, never a refusal. At the write door this arm is additionally
   compile-time-unreachable for any conforming client: `clock_grade` is a mandatory typed
   `EventBody` field, a strictly stronger guarantee than a runtime presence check, so no `RAISE` on a
   missing grade is needed or written — that RAISE would only ever fire against a raw/hostile
   submitter, whom `unknown` already handles safely without forcing a raw-CBOR signing seam into the
   safety-critical crate for no safety gain.

6. **The ADR-0027 §6 correction.** The offline interval's upper bound is refined from
   `upper = RTC` to **`upper = RTC + W(grade)`**, open above (no upper bound at all) at
   `unknown`/`self-asserted`. This is the sentence that actually closes the slow-clock failure mode
   §6 did not: the asymmetric "upper = the raw reading" bracket is replaced by a
   grade-widened bound that is honestly unbounded until a node earns the right to a tighter one.

7. **`cairn_clock_health()` — the ADR-0027 §7 honest-assembly read.** Clock-confidence is a
   first-class honest-assembly fact, like sync freshness and backup health. A pure, `STABLE`,
   `SECURITY DEFINER` read (samples `clock_timestamp()` once for internal consistency) reports the
   node's RTC reading, the HLC floor `hlc_state.wall` already maintains past every accepted event,
   whether the RTC is provably behind that floor, the resulting causal *lower* bound
   (`max(rtc_now, hlc_floor)` — "the time is at least this"), and the node's declared default grade.
   It is live-derived, **never stored, never an event, never synced** — a client or the CLI reads it
   through the native API; no door calls it.

**Why this stays safe against fraud.** A fraudster on a `self-asserted` node can forward-date freely
— the event is admitted and flagged — but the discrepancy is recorded forever and never
auto-resolved, and the **grade itself brands the timestamp untrusted**: you cannot forge a *trusted*
timestamp on an *untrusted* clock, so the fake carries no evidentiary weight against a
`self-asserted` record. Rejecting the write would not stop such a fraudster either — under the
Spike-0002 threat model they own the node or hold direct DB access — it would only ever punish the
honest clinician on a genuinely dead-RTC device. The medico-legal protection comes from the grade,
not from blocking the write; on a genuinely time-synced node the ceiling regains real teeth, and the
record carries real timestamp weight.

## Consequences

- **Safe against fraud by branding, not by blocking.** The classification table's `flag-never-reject`
  arm at low grades is not a safety hole: rejecting a self-asserted node's forward-dated write would
  neither stop a hostile node-owner (who can write around any in-DB check anyway, per Spike-0002) nor
  protect anyone — it would only manufacture a falsification finding against an honest clinician on
  hardware with no working clock. The grade is the medico-legal signal a reader relies on decades
  later, not the write-time gate.
- **The born `clock_grade` is the one can't-retrofit piece.** Per ADR-0027's own day-one requirement,
  a mandatory field means pre-slice events read honestly as `unknown` and carry no signed grade at
  all — dev/PoC rigs predating this slice must be wiped, not migrated, the same discipline already
  established for [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)/
  [ADR-0052](0052-born-sealed-clinical-bodies.md).
- **Deferred follow-ons, filed:**
  1. [#279](https://github.com/cairn-ehr/cairn-ehr/issues/279) — the anchor/notary planes
     (clock-setting + existence-proof) and the overlay grade-upgrade tokens; nothing produces a grade
     above `self-asserted` until this lands, so the `reject` arm stays production-unreachable until
     then.
  2. [#280](https://github.com/cairn-ehr/cairn-ehr/issues/280) — tightening the causal *lower* bound
     into the derived/stored interval (`t_recorded.lower = max(derived, floor over high-confidence
     events)`).
  3. [#281](https://github.com/cairn-ehr/cairn-ehr/issues/281) — the UI clock-sanity alert (reference
     UI; reads `clock_grade` + `cairn_clock_health()` + the advisory flag).
  4. [#282](https://github.com/cairn-ehr/cairn-ehr/issues/282) — auto-**downgrading** a node's grade
     when it detects its own clock has failed.
  5. [#283](https://github.com/cairn-ehr/cairn-ehr/issues/283) — rendering the clock grade in the
     legibility twin (deferred this slice because it requires changing the Rust `plaintext_twin` and
     the SQL twin-skeleton floor in lockstep, or the demographic twin-match floor refuses; the grade
     is already legible today via the `clock_grade` column and `cairn_clock_health()`).
- **No new founding principle.** Like ADR-0027 itself, this is [principle
  4](../index.md#founding-principles-the-lens-for-every-decision) applied to wall-clock truth — here,
  applied recursively to the node's confidence *in* that truth. No new event stream, no new
  founding principle; the grade rides the existing envelope-field mechanism ADR-0027 already
  established.
