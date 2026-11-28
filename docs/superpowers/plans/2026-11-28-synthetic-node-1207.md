# #1207 — a node says at genesis that its data is invented — Implementation Plan

**Goal:** A node initialised with `--synthetic` can never share a peering edge with a node for real
records, a restore of it stays synthetic, an older build cannot restore it, and every surface that shows
a patient says so.

**Decision:** [ADR-0114](../../spec/decisions/0114-a-node-says-at-genesis-that-its-data-is-invented.md)
(maintainer, 2026-11-28). Read its "Not guaranteed" section first: this is a declaration that stops an
honest mistake, not a security boundary.

**Architecture:** One new fact on the node plane, `data_class`, stated in the `node.enrolled` payload and
therefore part of the node id. It is copied to `local_node` at enrolment, carried in the signed pairing
bundle, written into `peer.added`, and compared in three floor doors. A synthetic node writes its backup
medium under its own container magic. Nothing on the clinical plane changes: an event body is as it was.

**Tech stack:** PostgreSQL 18 SQL (db/088 new; db/007 and db/009 in place, as ADR-0080 allows), Rust
(`cairn-event`, `cairn-node`, `cairn-medium`, `cairn-sync`, `cairn-gui`), Python (the cohort importer).

> [!CAUTION]
> **STOPPED at Task 0 (2026-11-28, #1220). Do not execute this plan as written.** Claim 1 below is FALSE for the
> clinical plane: `cairn-sync serve` ships events to a puller it never admitted and withholds only custody, which
> is deliberate and pinned by `an_unadmitted_puller_replicates_events_but_gains_no_custody`. The table's edge rows
> and Task 3's matrix (notably "real, field unknown / synthetic: its pull fails") assume otherwise. ADR-0114 needs a
> correcting ADR first; this plan is rewritten after it.

> [!WARNING]
> **Written from reading the code, not from running it.** Three claims this plan rests on were read and
> not yet tested. Task 0 tests them before anything is built, and the plan changes if one is false:
> 1. a peer is served, on BOTH planes, only if this node's own `trust_peer` holds it (`transport.rs`
>    pins mutual TLS to the trust set; `cairn-sync` reads the same table);
> 2. serde ignores a field it does not know in a `PairingBundle`, so an older build reads a newer bundle;
> 3. an older build refuses a medium whose magic it does not know, and does not misread it.

## Global constraints

- **Absent means real.** A genesis, a bundle or a `peer.added` with no `data_class` is a node for real
  records. Real nodes go on producing byte-identical genesis events and bundles: the field is omitted,
  never written as null or as `"real"`. A golden test pins the bytes of a real bundle.
- **The synthetic side carries the burden.** No test may rely on the real node refusing. Every
  cross-class test is run twice: once with a current real node, once with a real node that knows
  nothing of the field (the door called with a payload that lacks it).
- **Every doubt refuses.** A `data_class` that is not exactly `synthetic` pairs with nothing and
  restores as nothing, and the refusal names the value.
- **One spelling, one home.** The word `synthetic` and the comparison live in one SQL function and one
  Rust function. No door compares strings of its own (the mirror-list defect class: #182, #404, #441).
- A refusal in a remote door is `P0001` through the existing helpers, never a bare cast or constraint
  (#621): a different SQLSTATE freezes the puller's cursor.
- No confirmation dialog. The band is read, not answered.
- House rule 6 for any key material in tests. Files stay under 500 lines; `main.rs` gains only the arm.
- `SCHEMA_GENERATION` becomes **88**, with its floor-ledger entry (the tests enforce it).

## What exists, and where the class has to go

| Door | File | Today | Change |
|---|---|---|---|
| Genesis is authored | `identity::provision`, called by `init` (`main.rs`) and `restore` (`restore.rs`) | payload `{display_name, address}` | `data_class` when synthetic |
| Genesis is admitted locally | `submit_node_event`, enroll arm (db/007) | fills `local_node` | also `local_node.data_class`; unknown value refused |
| A pairing offer is made and read | `pairing.rs`, `PairingBundle` (`cairn-event`) | six fields, signed | optional seventh |
| A peer is added | `identity::author_peer` → `submit_node_event`, peer arm | payload from the bundle | `peer_data_class`; refused unless equal to this node's |
| A peer's genesis arrives | `apply_remote_node_event`, enroll arm | must match a pinned peer | and must be of this node's class |
| A node is restored | `restore.rs`; `restore_node_event` (db/009) | new genesis, then supersede | new genesis copies the class; a different one is refused |
| A medium is written and read | `cairn-medium` (`CAIRNB3`) | one magic | `CAIRNS3` for a synthetic node |
| The node shows itself | `status`; the window; the terminal gateway's facts | nothing | a line, a band, a fact |
| A loader asks | `scripts/import_synthetic_cohort.py` | counts foreign charts | asks the node |

## Task 0 — test the three claims this plan rests on

Nothing is built in this task. Each claim becomes a test that passes on `main` as it is.

- [ ] `tests/federation.rs` (or a sibling): node A adds B; B does **not** add A. Assert A's pull from B
      fails at the TLS handshake on the node plane, and that `cairn-sync`'s clinical pull from B fails
      the same way. Then the reverse direction. If either pull succeeds, **stop**: the asymmetric rule
      in ADR-0114 decision 3 does not hold as written, and the ADR needs a correction before any code.
- [ ] `cairn-event`: a `PairingBundle` serialised with an extra CBOR field verifies and deserialises
      under today's struct. A golden of a real bundle's bytes is added in the same test.
- [ ] `cairn-medium`: `parse_any` on bytes beginning with an unknown magic returns an error that does
      not read as a damaged `CAIRNB3` medium. Record the exact error, because Task 5 words its message
      for an operator on an old build from it.

## Task 1 — the class, in one home (db/088, `cairn-event`)

Tests first: `db/tests/088_data_class_test.sql` and unit tests in `cairn-event`.

- [ ] `cairn-event::data_class`: `pub enum DataClass { Real, Synthetic }`,
      `DataClass::from_payload(Option<&str>) -> Result<DataClass, String>` (absent is `Real`,
      `"synthetic"` is `Synthetic`, anything else is an error naming the value), and
      `as_payload() -> Option<&'static str>`. Pure.
- [ ] db/088: `ALTER TABLE local_node ADD COLUMN IF NOT EXISTS data_class TEXT` (NULL is real);
      `cairn_data_class_or_raise(text, door text) RETURNS text` (the only place SQL spells
      `synthetic`); `cairn_node_data_class() RETURNS text`, `STABLE`, returning `'real'` or
      `'synthetic'`, granted to `cairn_node`, `cairn_agent` and, through it, the window.
- [ ] A drift test that the Rust and SQL spellings agree, in the manner of the existing `*_drift.rs`
      tests.
- [ ] `SCHEMA_GENERATION` 88 and its ledger entry.

## Task 2 — genesis (`init --synthetic`, the enroll arm, `status`)

Tests first: `tests/node_data_class.rs` (DB-gated).

- [ ] `identity::provision` takes a `DataClass`; a real node's payload is byte-for-byte what it is today
      (assert on the JSON), a synthetic node's carries `data_class`.
- [ ] `submit_node_event`, enroll arm: store the class in `local_node`; refuse an unknown value before
      anything is inserted, so a refused genesis leaves the node un-enrolled and `init` can be run again.
- [ ] `cairn-node init --synthetic`. Its help says in one sentence that the choice is permanent.
- [ ] `status` prints `data_class    SYNTHETIC — invented patients only; never peer with a real node`
      or `data_class    real`.
- [ ] A node enrolled before db/088 reads as real with no backfill (the column is NULL).

## Task 3 — the edge (the bundle, `peer.added`, the peer's genesis)

Tests first, in `tests/pairing.rs`, `tests/node_door_refusals_are_p0001.rs` and `tests/federation.rs`.

- [ ] `PairingBundle.data_class: Option<String>` with `#[serde(default,
      skip_serializing_if = "Option::is_none")]`. The golden from Task 0 still matches for a real node.
- [ ] `pair-accept` reads the class and refuses a different one **before** the fingerprint prompt, with
      the message naming both classes and saying that nothing was written. A person who has typed YES
      and is then refused has been asked to confirm something that could never happen.
- [ ] `author_peer` writes `peer_data_class` into `peer.added` when the peer is synthetic.
- [ ] `submit_node_event`, peer arm: refuse unless `cairn_data_class_or_raise(payload.peer_data_class)`
      equals this node's class. This is the floor: a client building its own `peer.added` meets it.
- [ ] `apply_remote_node_event`, enroll arm: refuse a pinned peer's genesis whose class differs from
      this node's. This checks what the peer signed in its bundle against what the peer is.
- [ ] The matrix, each as a named test, each run against a current real node and against one that
      omits the field:

      | This node | Offer from | Outcome |
      |---|---|---|
      | synthetic | synthetic | paired |
      | synthetic | real | refused by the floor |
      | synthetic | real, bundle forged to say synthetic | paired, then its genesis is refused and nothing of it is admitted |
      | real (current) | synthetic | refused by the floor |
      | real (field unknown) | synthetic | **it pairs**; assert its pull from the synthetic node fails, because the synthetic node never admitted it |
      | either | an unknown class | refused, naming the value |

- [ ] A relayed event: with real nodes R1 and R2 peered and a synthetic node S, assert no path exists by
      which an event authored on S reaches R1 or R2. This is the test of the ADR's bet.

## Task 4 — restore inherits (`restore.rs`, db/009)

Tests first, in `tests/restore.rs`.

- [ ] `restore` reads the superseded genesis's class from the medium and passes it to `provision`.
- [ ] `restore_node_event` (db/009) refuses a `node.superseded` whose subject genesis has a class other
      than this node's. Restoring a synthetic medium yields a synthetic node; nothing an operator types
      can make it real, and there is no flag that tries.
- [ ] A medium with no genesis is refused as it is today (`restore.rs:44`); it never defaults to real.

## Task 5 — an older build cannot restore a synthetic node (`cairn-medium`)

Tests first, in `cairn-medium`.

- [ ] `MEDIUM_MAGIC_SYNTHETIC_V3 = b"CAIRNS3\n"`, the same container as `CAIRNB3` under a different
      magic. `parse_any` accepts both and reports which; `serialize_v3` takes the class.
- [ ] `backup` on a synthetic node writes `CAIRNS3`; `verify-backup` and `restore` read it.
- [ ] The magic and the genesis must agree. A `CAIRNB3` medium whose genesis says synthetic, or the
      reverse, is refused with both facts named: one of them is wrong, and guessing which is how an
      invented node becomes a real one.
- [ ] Append and succession (`append_segment`) keep the magic of the medium they extend, and refuse to
      extend a medium of the other class.
- [ ] The message an old build gives for `CAIRNS3` (recorded in Task 0) is quoted in the RUNBOOK entry,
      with what it means.

## Task 6 — it is said wherever the node shows itself (`cairn-gui`, the terminal gateway)

Tests first: `review-core`-style pure tests in the GUI crates; the page decides nothing.

- [ ] The gateway's facts carry the node's class, so a terminal knows its node's class before it shows
      a patient ([ADR-0112](../../spec/decisions/0112-a-terminal-follows-its-node.md)).
- [ ] The window shows a permanent band, `SYNTHETIC DATA — no real patients`, on every screen that
      shows a patient, built in Rust and drawn by the page. It cannot be dismissed, and it is absent on
      a real node (no "REAL DATA" band: a band that is always there is not read).
- [ ] The band overflows by wrapping, never by clipping: at the narrowest window the whole sentence is
      still readable.
- [ ] A terminal whose node's class it cannot read shows nothing of a patient and says why.

## Task 7 — the loader asks the node (`scripts/import_synthetic_cohort.py`)

- [ ] Replace `foreign_chart_count` with a read of `cairn_node_data_class()`: refuse unless it returns
      `synthetic`. The refusal says how to make such a node (`cairn-node init --synthetic`).
- [ ] Keep the test that a node holding other charts is refused only if a synthetic node could hold
      them; otherwise delete the guard and its test together, and say so in the commit.
- [ ] Tell ReportTriage (an issue on its repository) that its `--synthetic` mode can now check the
      Cairn node it reads from.

## Not in this plan

- A training patient on a real node (#1219).
- A mark on each clinical event (declined by ruling; ADR-0114 "Not guaranteed").
- Converting an existing development database: it is initialised again.
- `cairn-sync init`, which enrols nothing of its own; if Task 0 finds that it does, it joins Task 2.
- Slice 5 of sign-in (every terminal its own node, #909/#963): when a terminal becomes a node, its
  genesis must copy its facility's class. Recorded here so that design meets it.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a dummy chart with TRAINING stamped across its cover, kept in a separate
  drawer from the real ones.
- **Steps:**
  - Seeing that a chart is invented: paper N = 0 (the stamp is on the cover) → architecture M = 0 → UI
    K = 0 (the band is on the screen that shows the patient).
  - Setting up a place for invented charts: paper N = 1 (label the drawer) → M = 1 (`--synthetic` at
    `init`) → K = 1, once per node, by whoever installs it.
  - Mixing the drawers by mistake: on paper nothing stops it. Here it is refused at pairing and at
    restore, with no step added to any clinical act.
- **Time + cognitive load:** the band is readable in under 1 s without a click and competes with
  nothing: it carries no button and asks no question. Budget: no measurable change to the time of any
  clinical act on a synthetic node. Measured by a RUNBOOK section (a human reads the band at the
  narrowest supported window, and tries and fails to pair a synthetic node with a real one), owed by
  Task 6, the first that exposes a runnable surface.

## Order and size

Tasks 0 to 3 are one PR: they are the guarantee. Task 4 and Task 5 are one PR. Task 6 is one PR and
needs the window's test rig. Task 7 goes with whichever lands last of 3 and 5, because the loader must
not ask a node that cannot yet answer.
