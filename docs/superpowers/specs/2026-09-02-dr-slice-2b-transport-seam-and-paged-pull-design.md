# Design — DR slice 2b: the transport seam and the paged pull

- **Date:** 2026-09-02
- **Part of:** DR slice 2, the programme that closes
  [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) (*the backup medium carries no clinical
  event*). **This slice closes nothing.** See §6 for what stays broken when it merges.
- **Produces:** a new workspace member `crates/cairn-wire`; two additive wire fields; a paged
  `do_pull`. **No ADR, no spec bump, no migration, no DB change.**
- **Predecessor:** [slice 2a](2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md) — the shared
  two-plane medium format (`crates/cairn-medium`, `CAIRNB3`), landed 2026-08-31, reviewed 2026-09-01.
- **Opens:** [#531](https://github.com/cairn-ehr/cairn-ehr/issues/531) — decompose
  `cairn-sync/src/main.rs` (see §7).

---

## 1. Why this piece exists

Slice 2a built the format the medium is written in. It deliberately contains **no database read and no
I/O**, so nothing yet moves an event onto a medium or off one. Two things have to exist before 2c can
capture and 2d can restore, and both are about *addressing*:

**The medium has to be reachable the way a peer is.** The maintainer chose the faithful reading of
[ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) decision 2 — *"clinical
events back up as a cold peer … a configuration of the existing sync daemon"* — over the cheaper
"widen the bespoke exporter". Today `do_pull` reaches its peer through one free function,
`request(peer: &str, req: &Request)`, which opens a TCP socket. A medium is not a socket. Until that
call is behind a trait, the medium cannot be a peer in anything but prose, and 2d's promise — *restore
pulls the medium through `apply_remote_event` unchanged* — has nowhere to plug in.

**The pull has to be paged.** `EventsAfterSeq` is deliberately unpaginated
([#101](https://github.com/cairn-ehr/cairn-ehr/issues/101) item 1): the whole log suffix ships as one
hex-encoded JSON frame under a 64 MiB cap, about 20k events. That is already a live defect on the
network path — a catch-up larger than the 30 s read timeout retries the *same* oversized response
forever and never progresses — and routing a real clinic's `event_log` through this path makes it
certain. 2a's segment chain exists precisely so an append costs one signature rather than a whole-file
rewrite; a capture that must fetch its input in one frame throws that away.

Neither is a defect *fix* on its own. This slice is a seam and a batch limit.

---

## 2. The extraction — `crates/cairn-wire`

`cairn-sync` is a **binary-only** crate whose `main.rs` is 11,577 lines, and it dev-depends on
`cairn-node`. So the wire types, the framing and the transport are unreachable from `cairn-node`, and
adding a `lib.rs` to `cairn-sync` would make a dependency cycle through dev-dependencies. Slice 2c's
`cairn-node backup` is the single operator command (2a decision 2) and 2d's restore drives the medium;
both need this code. It therefore moves into a library crate — the same reasoning, and the same
pattern, as `cairn-keystore` ([#503](https://github.com/cairn-ehr/cairn-ehr/issues/503)) and
`cairn-medium` (2a).

Pure of database and async. Dependencies: `cairn-event` (the shared `framing` core), `cairn-medium`,
`serde`, `serde_json`, `hex`.

| module | contents | provenance |
|---|---|---|
| `wire.rs` | `Request`, `EventsResponse` | moved **verbatim** from `main.rs`, plus the two additive fields in §3 |
| `framing.rs` | `MAX_FRAME_BYTES`, `read_frame`, `write_frame` | moved verbatim |
| `transport.rs` | `trait Transport`, `TransportError` | new |
| `tcp.rs` | `TcpTransport` — today's `try_request` + 4-attempt backoff | moved verbatim |
| `medium.rs` | `MediumTransport` — answers from a `MediumImage` | new |

```rust
pub trait Transport {
    /// Where this transport actually goes — `"tcp 10.0.0.3:9443"`, `"medium /vol/cairn.b3"`.
    /// Error prose ONLY. It is NOT the peer's name: `sync_state` is keyed on `peer_name`,
    /// which `do_pull` keeps as its own parameter, because the cursor must stay attached to
    /// the peer's identity even when the route to it changes.
    fn label(&self) -> &str;

    /// One request, one response frame. Retries, timeouts and reconnection are the
    /// implementation's business; a caller sees either a response or a failure.
    fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError>;
}
```

`do_pull`'s signature therefore becomes `(client, transport: &dyn Transport, peer_name, full_sweep,
custody)` — the address parameter is replaced, the name parameter is untouched.

### The proof, and the one thing that must be checked rather than assumed

The extraction's proof is #503's and 2a's: **every existing `cairn-sync` call site compiles with only
an import change**, and the TCP path's behaviour is unchanged apart from the two additive fields.

The exception worth naming is the error type. `classify_pull_failure` decides *"the peer sent an
over-cap frame"* — an integrity condition, not link downtime — by walking `source()` up to eight layers
for an `io::Error` of kind `InvalidData` (`chain_reaches_a_peer_frame_error`). Today `request` returns
`Box<dyn Error>` and the `io::Error` is the outermost layer. A `TransportError` that formatted its
cause into a string instead of keeping it as `source` would silently reclassify a hostile or corrupt
peer as a partition, and every existing test would stay green, because the tests construct the error
they classify. `TransportError` therefore keeps the `io::Error` reachable through `source()`, and that
property gets its own pin.

---

## 3. The wire contract

Additive on both sides (principle 12 — the native API evolves additively):

```rust
Request::EventsAfterSeq {
    after_seq: i64,
    #[serde(default)] unwrap_cert: Option<String>,
    #[serde(default)] limit: Option<u32>,     // None = unpaginated (today's behaviour)
}

struct EventsResponse {
    // … unchanged fields …
    #[serde(default)] complete: bool,          // false = "there may be more"
}
```

`complete` defaults to **false**, and the direction is the decision. A server that fails to set it makes
a puller ask once more — wasted work. A `true` default would make the same omission stop the puller
early and **silently lose events**, with a cursor checkpointed as if the log had been drained. Principle
4 applied to a protocol field: an imprecise near-truth ("there may be more") beats a precise untruth
("that was all of it").

There are no deployed peers to be compatible with — the project is pre-clinical, with no field nodes and
no legacy users (maintainer, 2026-09-02) — so this is a compatibility *discipline*, not a
compatibility *constraint*. It costs nothing and it is what principle 12 asks for.

### Termination, including the case that is neither end nor continuation

| page | rule |
|---|---|
| `complete == true` | done |
| `!complete`, non-empty | request the next page from the advanced cursor |
| `!complete`, **empty** | **refuse loudly** — `PullIntegrityError` |

The third row is the one worth stating. An empty page that does not declare the stream complete is a
peer that answered with something no puller can act on: treating it as the end risks a silent early
stop, and continuing spins forever against the same cursor. It is an integrity condition — the peer
*answered*, and the answer is unusable — so it takes the same path as a response that will not decode,
which is where `PullIntegrityError` already lives (#489).

### The page size

`DEFAULT_PAGE_EVENTS = 500`, overridable with `--page`. At roughly 4 KiB per event on the wire (≈1.5 KiB
signed, hex-doubled, plus attestation and wrapped DEK) that is about 2 MiB per page: 32× under the
64 MiB frame cap, and comfortably inside the 30 s read timeout on the 700 ms-RTT double-Starlink rig
Spike 0001 measures against. A 20k-event sweep becomes 40 round trips, about 30 s of accumulated
latency — paid once, on a full sweep, in exchange for progress that survives an interruption.

The serve arm adds `LIMIT $2` when the request carries one, and sets `complete` from whether the limit
actually bit.

### Seven comments that become false the moment this merges

The frame cap carries its own deferral in prose, in more places than one would guess. `grep -rn
unpaginated crates/` finds **seven sites across three files**, every one of them asserting that
pagination does not exist:

| site | what it claims |
|---|---|
| `cairn-sync/main.rs` `FULL_SWEEP_EVERY` doc | *"KNOWN COST (#101, unpaginated batches) … once a node's history outgrows that window the sweep fails loudly every cadence — the correctness floor stops floor-ing exactly on the largest-history nodes. #101 pagination is the fix; until it lands this cadence assumes a small log."* |
| `cairn-sync/main.rs` `MAX_FRAME_BYTES` doc | *"deliberately UNPAGINATED (issue #101) … pagination (#101) is the real fix for that, tracked there"* |
| `cairn-sync/main.rs` `write_frame` refusal text | operator-facing: `"(pagination: issue #101)"` |
| `cairn-sync/main.rs` `serve_conn`'s `EventsAfterSeq` arm | *"Unpaginated (issue #101): the whole suffix — the whole LOG on a sweep — ships in one frame"* |
| `cairn-sync/main.rs` `frame_cap_holds_a_realistic_event_batch` (doc + assert message) | *"cap must hold a realistic unpaginated batch"* |
| `cairn-medium/chunk.rs` `MAX_CHUNK_BYTES` doc | *"`cairn-sync`'s `MAX_FRAME_BYTES` … caps a whole unpaginated BATCH response"* |
| `cairn-event/framing.rs` module doc | *"the clinical plane at 64 MiB (an unpaginated full sweep — issue #101)"* |

The first is the one that matters most: it says in plain words that the **correctness floor stops
working on exactly the nodes with the most history**, and this slice is what retires that. Leaving a
stale deferral standing is the precise mechanism by which #500 hid for weeks — 2a's own recorded lesson
is that *a deferral is only honest while its stated precondition holds, and nothing in the repo watches
for one expiring*. All seven are rewritten to say what is true afterwards: the cap becomes a backstop
for a request that omits `limit`, the sweep is paged and its cost is round trips rather than one
oversized frame, and **#101 item 1 is closed while items 2 and 3 (the blob `byte_len` wedge, in-DB
BLAKE3) remain open** — so #101 itself stays open, with a comment recording which item went.

The plan finds these by grep, not by memory: this list was written by running it, and the first guess
was "one site".

---

## 4. `MediumTransport` — the medium answering as a peer

Constructed from a parsed `MediumImage`. It is pure: it reads a value someone else loaded from disk.

### Three refusals, each named

A refusal here has to be a *named* failure and never a plausible-looking empty success, because an
`EventsResponse` carrying zero events and `complete: true` is a claim that the medium holds no clinical
history — which is #500's exact signature, reproduced inside the machinery built to close it.

1. **`MediumImage::Legacy`** (CAIRNB1/CAIRNB2) → `UnsupportedByMedium`. Those revisions have no planes;
   they carry the federation plane and nothing else. The error says so, and names #500.
2. **`Request::EventsAfter`** (the legacy HLC cursor) → `UnsupportedByMedium`. Records are keyed by
   `source_seq`; there is no HLC index on a medium, and a plausible answer built by scanning would be a
   different query wearing this one's name.
3. **`Request::BlobSlice`** → `UnsupportedByMedium`. The byte tier does not ride the medium (ADR-0013's
   byte-replication is opt-in and separately scoped); a not-found answer would read as "this blob is
   absent everywhere".

### What it does *not* refuse, and why that matters

It does **not** refuse a medium whose `health::assess` reports `!sound()`. A torn tail is a mild fault
with a fully intact prefix, and `BackupError`'s three-way split exists precisely because *"upgrade this
node"*, *"fetch another copy"* and *"re-run the backup"* are opposite remedies and one opaque refusal
makes an operator discard a good medium mid-disaster.

Instead it serves **only records within `chain.verified_through`**. Trust stops there by construction —
that is 2a invariant 5 — so a torn tail's incomplete increment and everything past a broken link are
excluded without a policy decision being taken here. `health()` and `clinical_watermark()` are exposed
so 2d's restore can report *scope* honestly rather than inferring it.

### Serving

`Plane::Clinical` records with `source_seq > after_seq`, **sorted ascending**, capped at `limit`.

The sort is load-bearing and is not free. Segments sit in *capture* order, which is not the same as
`source_seq` order — a re-capture after an interruption, or a future out-of-order append, breaks the
coincidence. The puller's contiguous-prefix cursor rule *relies* on strictly ascending arrival
(`serve_conn`'s `ORDER BY seq` is the network path's version of this), and a medium that served capture
order would advance a cursor past events it had not yet delivered.

### Two divergences from a network peer

**1. `wrapped_deks` pass through verbatim.** They are wrapped to the *capturing* node's unwrap key. The
medium holds no secret, so it cannot re-wrap for a requester, and `unwrap_cert` in the request is
**ignored with a named warning — never silently**.

This is correct *only because* ADR-0066 and DR slice 1 make `restore` **adopt** the exported unwrap
secret, so the restoring node's secret *is* the capturing node's. That is a precondition, not a
property, and slice 2a's own recorded lesson is that *a deferral is only honest while its stated
precondition holds, and nothing in the repo watches for one expiring*. The sentence therefore lives in
the code beside the pass-through, naming ADR-0066, so a reader who changes the adoption path meets it.
A restoring node that did *not* adopt the key gets DEKs it cannot open; that failure must surface as
what it is, and never as "this event carried no custody".

**2. `signing_context: None`.** The medium does not record the ADR-0040 signing context its records were
minted under. A puller falls back to its all-unverifiable heuristic for the mixed-version diagnosis
(#108) — a degraded diagnosis, not a wrong answer. Named as a gap for **2c**, which writes the segments
and could carry it in the attestation payload.

---

## 5. The paged pull

`do_pull` takes `&dyn Transport` instead of `peer: &str`. Today's body becomes **one page**; a loop
wraps it.

Four decisions come out as **pure functions**, unit-testable with no database. Today the floor rule is
an inline expression inside a 740-line function and nothing tests it directly:

```rust
enum PageDecision { Done, Continue, Refuse(String) }

fn page_decision(complete: bool, page_len: usize, frozen: bool) -> PageDecision;
fn fold_page(cycle: &mut CycleTally, page: PageTally);
fn quarantine_floor(cycle: &CycleTally, floor_at_start: Option<i64>) -> Option<i64>;
fn page_request(after_seq: i64, limit: u32, unwrap_cert: Option<String>) -> Request;
```

### The floor is cumulative, never per-page

This is the one place where paging could introduce a defect worse than the one it fixes.

The existing rule has three branches: a **clean** cycle clears `quarantine_floor_seq`; a cycle with
unacked refusals **pins** it at the first refused slot; any pen failure keeps the **most conservative**
of the existing floor and the new pin. Computed per page, that rule breaks: page 1 refuses a slot and
pins the floor, page 2 is clean and **clears it** — permanently excluding a refused event the cursor has
already advanced past. Silent exclusion is exactly what the floor exists to prevent.

So `quarantine_floor` reads the **cycle** tally, not the page, and the earliest pin wins
(`pin = pin.or(page_pin)`; pages arrive in ascending seq). The tally only grows, so once refusals appear
the floor stays pinned for the rest of the cycle, and an interrupted cycle leaves a floor covering
everything refused so far.

### Each page commits cursor and floor together

That per-page durability *is* the fix for #101 item 1: a sweep interrupted at page 39 of 40 resumes
where it stopped instead of restarting from zero, and a sweep that cannot fit one frame no longer
retries the same oversized response forever.

`GREATEST(last_seq, $2)` keeps the cursor advance-only, unchanged. The rowcount check on the
`UPDATE sync_state` (#472) applies per page.

### The rest of the loop

- **A frozen cursor breaks the loop immediately.** Requesting the next page after a freeze would fetch
  events the puller has already decided it will not handle.
- **`attachment_flag_watermark` is read once, before page 1.** Reading it per page would make the
  unlearnable-reference report (#465) blind to references admitted by earlier pages.
- **Counters, byte totals and `custody_withheld` accumulate.** `shipped`, `applied_new`,
  `skipped_unverifiable`, `refused_verifiable`, `skipped_acked`, `event_bytes`, `wire_bytes` sum;
  `bytes_per_event` is derived from the sums; `cursor_seq` is the final `max_seq`;
  `custody_withheld` is true if any page withheld.
- **`references_unlearnable` is computed once, at the end**, over the cycle's accumulated
  `applied_addresses`.
- **The "all shipped events are unverifiable" diagnosis** becomes a cycle-level test over the
  accumulated counts, reported against the **first** page's declared `signing_context` — the peer does
  not change identity mid-cycle, and picking the first makes the reported value independent of where
  the loop stopped.

### Erratum E1

This is a dated design record, so a correction lands as an erratum rather than a rewrite of the prose
above (the same convention ADR-0052 uses for its E2).

**What §5 got right.** "The floor is cumulative, never per-page" is correct about **pinning**: a cycle
with unacked refusals must not have a later clean page clear the pin an earlier page set, and computing
`quarantine_floor` over the cycle tally rather than the page is the right fix for that half.

**What it omitted.** §5 is silent on **clearing**. `quarantine_floor`'s clean branch returns `None`, and
`None` is not silence — it is the positive claim "nothing is being withheld any more." §5's own
per-page-commit design ("Each page commits cursor and floor together," above) commits that claim after
**every** page, including page 1 of a cycle that has not yet reached the seq the floor guards. Nothing in
§5 gates the clear on how far the cycle has actually gotten.

**The failure that omission produced.** On a full sweep (`after_seq = 0`, which ignores the floor
entirely), the guarded slot is routinely several pages in — the first cycle after every daemon start is
a full sweep. A clean page 1 computes the floor `None` and commits it. If the cycle then ends before
reaching the guarded slot — a dropped link, or an ordinary transient apply failure elsewhere in that same
page, which sets `frozen` and makes `page_decision` return `Done` — the loop breaks having cleared a
floor it never actually re-verified. The next cycle reads a null floor, fetches from `last_seq`, and the
refused event the floor was pinning is never re-offered again. `skipped_unverifiable` stays 0 for that
cycle, so nothing about it is even loud.

**The live rule.** The code, not this section, is the authority: `pull_page::committable_floor` withholds
a mid-cycle clear unless it is licensed — a clean cycle may only clear a floor **at or below the last seq
it was actually offered**, unless the peer declared the page `complete` (nothing exists above it, so a
clean cycle really has seen everything the floor could guard). A PIN carries no such guard, since it
comes from a refusal in a page the cycle already handled and only ever widens what gets re-offered.

§5 as written above is incomplete on the clearing side; treat `committable_floor`'s doc comment and tests
as the current word on this rule.

### Erratum E2 — `reached` is a wire value, and the design never said what makes one trustworthy

Found by the whole-branch final review, after E1 landed. E1 fixed the *rule*; this is a third route into
the same clear, through the *input*.

`committable_floor` licenses a mid-cycle clear by comparing the floor against `reached`, which §5 and the
function's own doc both define as **"the highest seq this cycle has actually been offered."** The
implementation took it from `resp.seqs.last()`, and `validate_page`'s parallel-array check was gated on
`!resp.events.is_empty()` — so a page carrying **no events but a non-empty `seqs` array** passed every
guard (nothing to compare lengths against, and a lone large seq is trivially ascending and positive).
`{"events":[],"seqs":[5000]}` therefore cleared a floor at seq 900 and committed the clear, *before*
`page_decision` refused the empty page. The cycle failed loudly with the wrong diagnosis, and the floor
was already gone.

**The rule the design should have stated:** a seq that carried no event was never *offered*, and nothing
that writes durable state may take an untrusted array's word for what a page contained. The arrays are
parallel in **both** directions; the check is now unconditional, and `reached` additionally falls back to
the page cursor when a page carried no events — two independent guards, because this is the one value on
the page path that licenses withdrawing a safety floor.

### Erratum E3 — §5's "no cap on pages per cycle" rested on a bound that does not exist

Also from the final review. §5 argued the paging loop needs no page cap because a hostile stream is
self-limiting: anything that does not verify is penned, the per-peer quarantine quota is finite, the pen
eventually refuses, that freezes the cursor, and `page_decision` ends the cycle.

**The argument misses the two cheapest streams a peer can serve, neither of which pens anything:**

* events this node **already holds** — `apply_signed` returns `Ok(false)`, no pen row, no quota consumed,
  no freeze, and `max_seq` still advances because "handled" includes an idempotent no-op;
* bytes **already penned** — `quarantine_event` dedupes *before* the quota-checked `INSERT` and returns,
  so the pen never grows and never refuses.

A peer re-serving a handful of genuine events at strictly ascending **fabricated** seqs, never setting
`complete`, satisfies `validate_page` and the anti-loop invariant on every page for ever. `do_pull` never
returns, and `cmd_run`'s cycle loop is blocked with it: no further pulls, no fingerprint, and **no
periodic full sweep** — which is the consolation §5 itself offers against a peer lying high about its
seqs.

**The live rule.** `pull_page::MAX_EVENTS_PER_CYCLE` bounds a cycle at one million events (fifty times
the ~20k a single 64 MiB frame held before paging). Exceeding it is a **yield**, not a refusal: an
operator line plus a `budget_exhausted` metric, with every page's cursor and floor already committed, so
the next cycle resumes. It is deliberately neither a refusal (an honest catch-up on a large log is not a
fault) nor a silent `break` (which would checkpoint as though the log were drained).

---

## 6. What is still broken when 2b merges — read this before quoting it

**2b closes nothing. [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) stays open.** After it
merges:

- `backup.rs::read_event_set` still reads `SELECT signed_bytes FROM node_event` and nothing else. The
  medium still carries no clinical event.
- `dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event` still
  passes, **as a pin on the defect**. It is inverted in **2d**, not here.
- `backup-status.json`, `status` and `verify-backup` still report health and integrity for a medium
  that does not hold the record.
- Nothing in this slice writes a clinical event to a medium (**2c**) or restores one (**2d**).

No comment, doc header, test name or commit message introduced by this slice may suggest otherwise.

### Deferrals, each naming the slice that retires it

- **A medium with `source_seq` gaps** serves what it has, and the puller advances its cursor over the
  hole. `chain::seq_gaps` is the tool and the surface belongs to **2d**'s restore report, which is the
  first caller with an operator to tell. Named here so it is not discovered there.
- **`signing_context` is absent from the medium** — **2c**, which writes the segments.
- **[#511](https://github.com/cairn-ehr/cairn-ehr/issues/511)'s custody newtypes** (`Secret32` /
  `PublicKey32`) stay unbuilt, as sequenced: 2b moves no key material, and #511 lands **after 2b and
  before 2c**, which is where key material starts moving again.
- **`cairn-sync/src/main.rs` stays oversized** — [#531](https://github.com/cairn-ehr/cairn-ehr/issues/531),
  see §7.

---

## 7. Scope: what this slice deliberately does not refactor

Extracting `cairn-wire` removes roughly 250 lines of code from `main.rs`; the page loop adds about 120.
The file stays far past the 500-line house rule.

Moving `do_pull` and the ~2,800 lines of inline tests that cover it would swamp the paging diff, which
is the part of this slice that needs careful review. So it is **filed, not done**:
[#531](https://github.com/cairn-ehr/cairn-ehr/issues/531) records the seams, the verbatim-move
discipline, the dev-dependency cycle that rules out a `cairn-sync` lib target, and the macOS relink cost
that argues for one seam per PR (maintainer decision, 2026-09-02).

---

## 8. Testing

Test-first throughout (house rule 2). The shape of the suite matters as much as its size — slice 2a's
review found **19 of 19 single-line mutations surviving** a green suite because every test round-tripped
through the same encoder/decoder pair.

**Pure, no database:**

- `page_decision` — the three rows of §3's table, including the empty-and-not-complete refusal, plus
  the frozen short-circuit.
- `quarantine_floor` — the three branches, and specifically the **page-1-refuses / page-2-clean** case,
  which is the defect paging could introduce. This rule has never had a direct test.
- `fold_page` — counters sum; the earliest pin wins; `custody_withheld` is sticky.
- `TransportError` keeps its `io::Error` reachable through `source()`, so
  `chain_reaches_a_peer_frame_error` still fires. Asserted against the error the **transport actually
  produces** for an over-cap frame, not one the test constructs — a pin whose fixture is built by the
  test leaves the production site unpinned (a recorded lesson from the 08-2x sweep).
- Wire round-trips for the two additive fields, plus a decode of a request/response **without** them,
  proving the serde defaults land where §3 says.

**`MediumTransport`, against a hand-built `MediumV3`** (cairn-medium's `testkit` pattern):

- The three refusals, each by variant, not by message text.
- Records outside `verified_through` are not served — built with a torn tail and with a broken link.
- Ascending `source_seq` order out of a medium whose segments are in a different order. This is the
  test that fails if the sort is dropped; a fixture in capture order would pass either way.
- `limit` honoured; `complete` set correctly at, below and above the boundary.
- `wrapped_deks` pass through byte-identical; a request carrying `unwrap_cert` warns and is not
  silently ignored.
- A medium whose clinical plane is empty answers zero events with `complete: true` — and the test
  asserts the **caller-visible watermark is `None`, not `Some(0)`** (2a invariant 8).

**The paged loop, with a `FakeTransport`:** this is the first time `do_pull` becomes testable without a
socket. Multi-page convergence; a mid-cycle freeze stopping the loop; an interrupted cycle resuming
from its per-page checkpoint; the refusal on an empty non-complete page.

**DB-gated, existing:** `cairn-sync/tests/clinical_pull.rs` proves the real A→B network path still
converges with paging on by default. Run with `-p cairn-sync`, which does build that cross-crate suite.

---

## Paper-parity benchmark (§1.2)

**Not clinical-surface** — this slice adds no clinical workflow and changes no human act. It is a
transport seam and a batch limit beneath the API layer; the operator surface it eventually serves
(`cairn-node backup`, one command, no added human act) is 2c's, and 2a decision 2 already fixed that it
stays one command so [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512)'s `M > N` finding is not
made worse.
