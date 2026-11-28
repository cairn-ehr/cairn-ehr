# ADR-0114 — A node says at genesis that its data is invented

- **Status:** Superseded by [ADR-0115](0115-a-patient-is-marked-synthetic-at-registration.md) (wholly; never built)
- **Date:** 2026-11-28
- **Spec version at acceptance:** 0.116
- **Refines:** [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md) (one more
  thing a peering edge is refused on; its decision is otherwise unchanged, and it is not edited);
  [ADR-0026](0026-node-durability-and-disaster-recovery.md) (what a restore inherits)
- **Issues:** [#1207](https://github.com/cairn-ehr/cairn-ehr/issues/1207) (this decision; the build is
  still owed), [#1219](https://github.com/cairn-ehr/cairn-ehr/issues/1219) (a training patient on a real
  node, split off), [#1214](https://github.com/cairn-ehr/cairn-ehr/issues/1214) (the synthetic cohort
  load that found the gap)

## Context

Cairn had no way to say that a node holds invented patients. The words *synthetic*, *test patient*,
*training*, *demo* and *sandbox* appeared nowhere in the spec as a state a node or a chart can be in;
the only remarks were that pre-clinical logs "are wiped, never synced through"
([ADR-0052](0052-born-sealed-clinical-bodies.md)). The sensitivity stream
([ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md)) has no such grade, and federation
admission (ADR-0017) asks who a peer is, never what kind of data it carries.

That was tolerable while the only test data was a fixture inside a test. It stopped being tolerable on
2026-11-28, when a SyntheticHealthData cohort was loaded into a node through the node's own doors
(#1214). That generator's identifiers are valid in form. Its own source says: "no range is reserved for
synthetic data, so a valid number can belong to a real person." It marks its files three ways (an HL7
`T` in MSH-11, the sending application, a marker inside its SQLite file). All of that is lost at
Cairn's door: once registered, an invented chart is a chart.

The failure is ordinary and needs no malice. A test database is reused, or a pairing ceremony is run
against the wrong machine, and set-union sync does what it is built to do. Invented patients arrive in
a real record, where:

- nothing on an invented chart tells a clinician it is not a person;
- an invented chart can carry the IHI or Medicare number of a real patient, and the matcher proposes a
  link;
- append-only means none of it can be removed, only shredded event by event, on every node it reached.

The loader's own guard (refuse a node that holds any chart it did not create) is a check in a script.
It runs once, before the load, on one machine. It says nothing at the moment that matters, which is
when two nodes are joined.

Three facts about the existing design make a small decision sufficient:

- **A node's identity is the content address of its genesis event** (`node.enrolled`, db/007;
  [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)). Anything stated in that
  event is part of who the node is and cannot be changed without becoming a different node.
- **A peer is served only if this node has admitted it.** Both `cairn-node serve` and `cairn-sync`
  pin mutual TLS to the local `trust_peer` set, which is filled only by a `peer.added` event this node
  authored itself. So a node that refuses to admit a peer is not reachable by that peer, whatever the
  peer believes.
- **A restore mints a new genesis that supersedes the old one** (ADR-0026 slice C), reading the old
  genesis from the medium.

## Decision

Decided by the maintainer, 2026-11-28, on four rulings: node-level only; edge checks only; fixed at
genesis with no conversion; a distinct medium format.

1. **A node states at genesis that its data is invented.** `cairn-node init --synthetic` writes
   `data_class: "synthetic"` into the payload of `node.enrolled`. A genesis with no `data_class` is a
   node for real records: that is every node that exists today, and they must go on reading as they do.
   The field is additive ([principle 11](../index.md#founding-principles-the-lens-for-every-decision)).

2. **It is fixed for the life of the node, in both directions.** The node id is the hash of the event
   that carries the statement, so there is nothing to edit. No event converts a node. A synthetic node
   that is wanted for real use is abandoned and a new node is initialised; a real node is never made
   synthetic. A node that could change class later is the failure this decision exists to prevent: the
   invented charts would still be in it.

3. **A synthetic node and a real node never share a peering edge, and the synthetic side carries the
   burden.**
   - The signed pairing bundle carries the offering node's class. A bundle with none is from a real
     node.
   - **A synthetic node admits only a peer whose signed bundle says `synthetic`.** Because a peer is
     served only once admitted, this alone keeps a synthetic node's events from a real node, including
     a real node running a build that predates this decision and knows nothing of the field.
   - A real node on a current build refuses a synthetic offer too. That is a second barrier and a
     clear message for the operator; the guarantee does not rest on it.
   - The refusal is made in the floor (`submit_node_event`, for `peer.added`), so a client that builds
     its own node event meets it as well.
   - **What a peer signed in its bundle is then checked against what it is.** A peer's genesis is the
     first thing the node plane pulls from it, and its hash is the node id that was pinned. The remote
     door (`apply_remote_node_event`) refuses a genesis whose class differs from this node's, so a
     bundle that misstated the class buys nothing: the peer's events are never admitted.

4. **A class this build does not know is refused by everyone.** A `data_class` other than `synthetic`
   is neither real nor synthetic to this build: it pairs with nothing, and says why. A later decision
   may add a class; an older build must not guess what it means.

5. **A restore inherits, and an older build cannot restore a synthetic node.** The new genesis a
   restore mints copies the class of the genesis it supersedes; a supersede that would change the
   class is refused. A synthetic node writes its backup medium under its own container magic, so a
   build that predates this decision fails to read it, and does not restore invented patients as a
   real node.

6. **It is said wherever the node shows itself.** `cairn-node status` prints it. The reference window
   carries a permanent band, on every screen that shows a patient, reading *SYNTHETIC DATA — no real
   patients*. A function readable by an advisory actor returns the class, so a loader of invented data
   and a consumer's own synthetic mode (ReportTriage's `--synthetic`) can refuse a real node. A loader
   of invented data must refuse a node that is not synthetic; the script-level guard in
   `scripts/import_synthetic_cohort.py` is replaced by that check when this is built.

7. **Scope: the node, and nothing finer.** A training or test patient on a real node is a different
   mechanism (the mark must travel with the chart and be read by every node it reaches) and is #1219.
   Clinical events carry no mark of their own: the wire core's event body is unchanged.

The canonical home is [security §7.7](../security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract).

## Not guaranteed

Stated plainly, because the value of this decision is in knowing where it stops.

- **It is a declaration, not a proof.** A node says what it holds, in an event it signs itself. This
  stops the honest mistake. A modified build can leave the statement out, and nothing detects that. It
  is not a security boundary and must never be described as one.
- **An event that leaves a synthetic node by a road that is not a peering edge is unmarked.** A
  database copied with `pg_dump`, a local-state export moved by hand, or an advisory actor that reads
  from one node and writes to another, all carry events that look like any others on arrival. The
  ruling was edge checks only; a mark on every clinical event is a change to the wire core and was not
  taken.
- **Every node that exists today reads as real.** That includes every development and test database
  made before this is built. It is the safe direction to be wrong in (a real node can never be read as
  synthetic), and it means an existing test rig stays unmarked until it is initialised again with
  `--synthetic`.
- **A real node cannot know that a peer is lying by omission**, for the same reason as the first
  point. What it can know is what the peer signed.
- **Nothing here sorts charts within a node.** On a synthetic node every chart is invented by
  declaration; on a real node every chart is taken as real, a training patient included, until #1219.

## Consequences

- **Easier:** a periodic load of an invented cohort (#1214) has a place to go that the architecture
  recognises, and the instruction "never peer a node this script wrote to" becomes a refusal instead
  of a sentence in a script header. A demonstration or training installation can be handed to someone
  without the risk that it is later joined to a clinic. ReportTriage's rule that a synthetic record is
  never read beside a real feed gets something to check on Cairn's side.
- **Harder:** two more fields on the node plane (genesis, pairing bundle) and one more container magic
  to carry and test. Every door that joins nodes or rebuilds one has to be named and covered by the
  build's plan: `pair-accept`, the two remote doors, `restore`, `backup`, and the terminal gateway
  (a terminal follows its node, [ADR-0112](0112-a-terminal-follows-its-node.md), and must show its
  node's class).
- **Paper-parity:** the paper counterpart is a dummy chart with TRAINING stamped across the cover. The
  band costs no step and no decision; it is read, not answered. It is not a confirmation dialog.
- **The bet:** that the mistakes worth stopping are made at a peering edge or a restore, by someone who
  did not mean to. We would know the bet has failed if an invented chart is found on a real node having
  arrived by one of the unmarked roads above. The answer then is the per-event mark this decision
  declined, and the cost of adding it later is that events written before it stay unmarked.
- **Mechanism, not policy** ([principle 9](../index.md#founding-principles-the-lens-for-every-decision)):
  Cairn ships the statement and the refusal. Whether a site runs a synthetic node at all, and what it
  loads into one, is the site's business.
