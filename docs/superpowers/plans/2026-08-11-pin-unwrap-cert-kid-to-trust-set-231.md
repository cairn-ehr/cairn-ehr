# Plan — pin the sync unwrap-cert `kid` to the node-plane trust set (#231)

**Issue:** [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) · **ADR:** none new — this
implements the named hardening [ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md)
already deferred ("custody is designed to follow admission"). · **Schema:** no change.

- Paper-parity: not clinical-surface — this is a wire-level custody gate beneath the application
  layer; it adds, removes and reorders no human act at any layer, and changes only which *peer*
  obtains read-custody of already-replicating sealed bytes. (Forced-rationale escape, house rule 7.)

## The defect

`cairn-sync serve` verifies a pulling peer's unwrap-key certificate against **its own signature and
self-consistency only** (`verify_unwrap_key_cert`: the cert's `kid` signed it, and the payload does
not lie about that `kid`). It never checks that `kid` against the node-plane admitted-peer trust set.

So **transport is currently the sole gate on read-custody.** Any self-signed unwrap cert that reaches
the serve port has this node's DEKs re-wrapped for it, and a DEK is what populates `event_clear` and
opens the sealed plaintext — custody confers clinical-data **READ**, not merely a future shred
capability. This is why HANDOVER states that born-sealed is an *erasability* substrate and **not**
confidentiality until this lands, and why §5.9 part C (sequester, #376) is blocked on it: narrowing a
body's custody to two named clinicians is defeated by asking the serve port for the DEK.

## The decision

Gate the re-wrap on the trust set the node plane already relies on —
`trust_peer.peer_pubkey = <kid> AND status = 'active'` (`db/007`). The precedent is
`sync.rs::refresh_trust_set`, which snapshots every active `peer_pubkey` for `cairn-node`'s mTLS
cert-pin verifier (`transport::pinned` tests membership of that snapshot rather than re-querying —
one mechanism, not two). Custody admission therefore reads the same set under the same grading rather
than becoming a second definition of who is admitted.

> **Corrected during PR review.** Two claims in the paragraph above, as first written, were wrong:
> that both named sites "use that exact clause" (the mTLS verifier runs no SQL — the `SELECT 1 …` at
> `transport.rs:24` is a doc comment), and that the unwrap cert becomes "the third consumer" (it
> counted one mechanism twice). Recorded rather than silently rewritten, since the plan is the
> historical artifact.

**Withhold custody, never refuse the pull.** An unadmitted peer still receives the events — they are
sealed ciphertext, harmless to ship, and a refusal would wedge replication. This is the degradation
the arm already implements for an absent/invalid cert ("An absent or invalid cert simply yields no
custody, never a refused pull"); an unadmitted kid joins that path. The four governing principles push
the same way: availability over consistency, and a refusal here would be a fork of the event set.

**Fail closed on every uncertainty.** Custody is the dangerous direction, so anything short of a
positive `active` match withholds. That includes a missing `trust_peer` relation.

## The operational hazard this creates, and why it is still right

`trust_peer` filters on `ne.author_node_id = (SELECT node_id FROM local_node WHERE id)`. On a node
whose node plane was never initialised, `local_node` is empty, the subquery is NULL, and the view
returns **zero rows** — so fail-closed means custody silently stops flowing. Two more shapes reach the
same place: a DB that never loaded `db/007` at all (`cairn-sync`'s own SCHEMA subset deliberately
excludes it — the node plane is `cairn-node`'s business, and #284 tracks subset consistency), and a
correctly-provisioned node serving a peer it simply has not peered with.

These are not the same operator problem and must not print the same line. Each cause gets its own
message naming the fix, per the Slice 61 lesson (*a safety refusal is only as good as the escape hatch
it names* — check the reader can actually run the remedy from what was printed).

## Build order (TDD)

1. **RED — pure decision tests.** A `CustodyAdmission` outcome type + a pure classifier over the
   trust-lookup result. Cases: active peer → grant · known-but-revoked → withhold · unknown kid →
   withhold · trust set empty (node plane uninitialised) → withhold · relation absent → withhold.
   Each withhold carries its own operator line. No DB, no I/O — house rule 1.
2. **GREEN — the lookup.** A thin `postgres` query in `serve_conn`'s `EventsAfterSeq` arm feeding the
   classifier; SQLSTATE `42P01` (undefined_table) maps to the relation-absent arm rather than
   propagating as a serve error. Delete the `TODO(follow-up, filed in Task 14)` and rewrite the
   honest-limits comment to state the floor that now holds.
3. **RED → GREEN — the wire test.** `clinical_pull.rs`'s sealed-custody test currently peers nothing,
   so it must now perform the node-plane peering ceremony for B on A before custody crosses. That the
   test has to change **is the point**: it is the ceremony a real second site performs, and its
   absence was the hole. Add a sibling test proving an **unpeered** puller receives the events and
   **no** custody (`event_clear` empty on B, projections absent) — the security case, asserted.
4. **Docs.** ADR-0052 gets an erratum recording that the deferred hardening landed; HANDOVER/ROADMAP
   lose the "NOT confidentiality until #231" standing caveat and #376 loses its blocker.

## Out of scope (named, not silently dropped)

- **Adding `db/007` to `cairn-sync`'s SCHEMA subset.** It would make the node plane appear in a
  cairn-sync-only DB, but `db/007` re-declares `hlc_state`, which `db/001` already creates for this
  subset. (Both declarations are `CREATE TABLE IF NOT EXISTS` with an identical shape today, and
  `connect_and_load_schema` loads both — so this is a duplicate declaration to reconcile, NOT the
  "shape collision" this plan first called it. Reconciling it is #284's decision, not this slice's.)
- **#380** (a peer can strip a sensitivity grade un-attested) is a different door and stays open.
- Custody for a node's **own** key (self-pull) is not granted: `trust_peer` lists peers, not self, and
  no code path pulls from itself. If one ever does, it needs its own decision, not a silent arm.
