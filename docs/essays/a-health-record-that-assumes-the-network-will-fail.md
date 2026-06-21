# A Health Record That Assumes the Network Will Fail

*Designing an offline-first, append-only, fractal EHR — and what it took to make blob sync survive a real satellite link.*
{: .essay-lead }

---

Most electronic health records are, underneath, a database behind an API behind a login screen, hosted somewhere central, and quietly dependent on the network staying up. That assumption is fine in a tertiary hospital with redundant fibre. It is a clinical safety hazard in a remote clinic on an intermittent satellite link, on a retrieval helicopter, or in any of the thousands of places where care happens and connectivity does not.

I'm an emergency physician who writes code, and I've spent years watching record systems fail at exactly the moments they mattered most — not with a stack trace, but with a spinner, a stale chart, or a “record not found” for a patient who is unmistakably in front of you. The failures were rarely about the algorithms. They were about the *architecture's relationship to reality*: networks partition, clocks drift, people get merged who shouldn't, and the safest copy of the truth is sometimes the one on a laptop that hasn't phoned home in three days.

This is a writeup of the architecture we're building to take those failure modes seriously — and a concrete look at one subsystem (binary attachment sync) that we recently validated over a genuine ~700 ms satellite link between two corners of Australia. The project is open source (AGPL-3.0) and currently in the architecture-plus-spike phase: we are deliberately validating the hard distributed-systems bets *before* building the product on top of them.

If you build distributed systems, the interesting part isn't “healthcare.” It's that the clinical requirements force a particular, fairly extreme set of distributed-systems choices — and then make you live with the consequences.

## The problem: a record that is correct *and* available under partition

Start with the CAP theorem, because clinical reality picks a side for you. When the network partitions, you can have consistency or availability, not both. A clinician who cannot read the locally-relevant chart, or cannot write a new observation, during a partition is a clinician who has been handed a *worse tool than paper*. Paper is always available. Paper never returns 503.

So we choose **availability** (AP, in CAP terms). Every node must be able to read locally-relevant records and accept new writes while partitioned, and the system must converge safely when connectivity returns. That single choice cascades into nearly everything else.

It also rules out the comfortable patterns. You cannot have a single authoritative writer. You cannot have a global lock. You cannot resolve a conflict by asking the source of truth, because under partition there *is* no reachable source of truth. And — this is the part that makes healthcare special — you cannot resolve a conflict by throwing data away or guessing, because the data is a person's medical history and a wrong guess can kill them.

The second structural requirement is **topology**. Care is fractal: a single workstation, a department, a facility, a region, a nation. We refuse to ship a different product for each tier. There is one codebase; a node's role — leaf clinic or national hub — is *configuration*, not a fork. That means the same sync engine has to work between two laptops in a clinic and between a regional data centre and a national one, with no special-casing.

## The clinical scenarios that drive the design

Architecture in the abstract is cheap. Here are the concrete failure modes we test every design decision against. None of these are hypothetical; they're composites of things that happen.

- **The transfer.** A critically ill patient moves from a rural ED to a tertiary ICU. The receiving team needs the record *now*, the sending site keeps writing during handover, and the link between them is flaky. Nobody “owns” the record at the moment it's most needed.
- **The wrong chart.** Two patients open on one workstation; a busy clinician documents on the wrong one. On paper this is physically hard — you are holding a specific folder. In software it's a single misclick, and a confirmation dialog does not fix it (people click “OK” reflexively a hundred times a day).
- **The overnight imaging flood.** A nightly bulk sync of imaging studies saturates the link, and *clinical* sync — the allergy, the new critical result — starves behind gigabytes of pixels. Emergencies can retrieve nothing. This actually happened, in more than one system, and it reframed how we think about bandwidth.
- **The sealed episode that still matters.** A patient seals a sensitive episode (say, a pregnancy termination). A future antenatal clinician must still be warned about Rh-sensitization risk — *without* the confidential details leaking. Confidentiality and safety pull in opposite directions and both are non-negotiable.
- **The merge that was wrong.** Two records get linked as “the same patient” and they aren't, or two genuinely-identical patients never get linked. Either way, someone must be able to undo it later, with a full audit trail and zero data loss.
- **The uncertain fact.** A confused patient gives an approximate birth year. The intake form has a mandatory DOB field. The clinician's only options are to fabricate a precise date or to not save the record. Both are wrong.

Every one of these is a distributed-systems or data-modelling problem wearing a white coat.

## Four principles, and the mechanisms that implement them

We hold four governing principles. They aren't slogans; each one is load-bearing and maps to a specific mechanism. I'll give the principle, then the code.

### 1. Append-only + causal ordering

**All clinical content is immutable, signed events ordered by Hybrid Logical Clocks. Corrections are new events that reference the originals; nothing is ever updated in place.**

This is the move that makes AP sync *safe*. If events are immutable and content-addressed, then synchronizing two nodes is a **set union** — ship the events the other side is missing, insert them idempotently, done. Set union is commutative, associative, and idempotent: the three properties you want when the same event might arrive twice, out of order, from three different peers, after a week-long partition.

The event body is the thing that gets signed. Field order *is* the canonical encoding order — there is exactly one serialization, produced once by the writer; everyone else byte-compares and never re-encodes.

```rust
/// The canonical event body — the thing that is CBOR-encoded and signed.
pub struct EventBody {
    pub event_id: String,            // UUIDv7
    pub patient_id: String,          // immortal subject UUID
    pub event_type: String,          // patient.created | patient.amended | note.added
    pub schema_version: String,
    pub hlc: Hlc,                    // the causal-order stamp (see below)
    pub t_effective: Option<String>, // asserted effective time; None = unknown
    pub signer_key_id: String,
    pub contributors: serde_json::Value, // who/what authored this (humans, AI, devices)
    pub payload: serde_json::Value,      // the clinical content
    pub attachments: Vec<AttachmentRef>, // content-addressed blob references
}
```

Ordering across nodes with no shared clock is a **Hybrid Logical Clock** — wall-clock time when it's sane, a logical counter when it isn't, and the originating node as the final tie-break. The structure is three fields:

```rust
pub struct Hlc {
    pub wall: i64,        // physical milliseconds
    pub counter: i32,     // logical tick when wall doesn't advance
    pub node_origin: String,
}
```

Apply, then, is set-union plus a clock merge. The whole convergence engine is essentially this — note there is *no merge logic*, just an idempotent insert and a monotonic clock advance:

```sql
-- Idempotent set-union: the same event arriving twice is a no-op.
INSERT INTO event_log (event_id, patient_id, event_type, /* … */ signed_bytes, content_address)
VALUES ($1::text::uuid, $2::text::uuid, $3, /* … */ $9, $10)
ON CONFLICT DO NOTHING;

-- HLC merge: the local clock can never fall behind an event we have accepted.
UPDATE hlc_state SET
    hlc_wall = GREATEST(hlc_wall, $1),
    hlc_counter = CASE WHEN $1 >  hlc_wall THEN $2
                       WHEN $1 =  hlc_wall THEN GREATEST(hlc_counter, $2)
                       ELSE hlc_counter END
WHERE id;
```

Two nodes have converged when their ordered set of event content-addresses hashes identically. There is no leader, no quorum, no consensus round — because immutability turned “agree on state” into “union your sets.”

What about the *transfer* scenario, where nobody owns the record? It dissolves: the record is just the sum of its autonomous signed parts, assembled by whichever node can reach them. A transfer triggers *acquisition* of the relevant events (from a sibling on the LAN, from the device carried with the patient, from the parent node on reconnect), not a reassignment of ownership. The receiving ICU doesn't need permission to assemble the chart; it needs the events, and it can get them from anywhere that has them.

### 2. Identity is a claim, never a fact

**Never merge — always link. Never erase — always overlay.**

Patient UUIDs are immortal. “These two records are the same person” is not a destructive `UPDATE` that fuses two rows; it is an *event* in an append-only identity stream — a `link`. “Actually, they weren't the same person” is another event — an `unlink`. The current identity graph is a *projection* over that stream, recomputed deterministically. Because every identity assertion is itself an immutable, ordered, signed event, the wrong merge from the scenario above is always repairable, with the full history of who claimed what and when intact.

This is the same insight as principle 1, applied to identity: the moment you allow a destructive merge, you have created a state you cannot safely reach from two directions and cannot undo. So you don't allow it. Identity errors become ordinary overlay events, and “undo” is just appending the inverse claim.

### 3. Acknowledged uncertainty

**An imprecise near-truth always beats a precise untruth.**

Look again at the `EventBody` above: `t_effective` is `Option<String>` — and `None` genuinely means *unknown*, distinct from *not-yet-asked* and from *refused*. Uncertainty, ranges, and explicit unknowns are first-class recordable values. No required field may be satisfiable only by fabrication. The confused patient's approximate birth year gets recorded *as* approximate, and certainty is refined later by overlay — never forced up front.

Time is the canonical case, and it's a clean bitemporal split:

- `hlc` carries **`t_recorded`** — the objective ceiling, when the system actually saw this. You cannot forge it earlier than reality; the HLC is monotonic.
- `t_effective` carries the **asserted** time — the freely-backdatable clinical claim (“this happened yesterday, I'm only writing it now”).

When the two clash, the system *flags* it; it never silently auto-resolves. A late entry is legitimate and common; a *backdated* entry that pretends to be contemporaneous is the thing audit needs to see. Separating the two fields is what makes the distinction visible instead of fabricated away.

### 4. Paper-parity (the governing law)

**No clinical workflow may be slower, harder, or more cognitively demanding than its paper equivalent.**

This is the principle that vetoes “clever” software. The wrong-chart problem is the clearest example. The instinct is to add a confirmation dialog. But confirmation dialogs *fail* paper-parity — they add a step paper never had, and habituation makes them worthless as a safety mechanism. The right answer is to restore the *physical affordance* paper had: possession semantics, where binding “which clinician” and “which patient” to a write is one ambient gesture, and writing to a chart you don't currently “hold” is structurally distinct. The job is to give software back the safety properties physical objects had for free, not to bolt on modals.

## Where it gets physically real: the byte tier

Principles are easy to state and hard to honour under load. The overnight-imaging scenario is where ours met physics, and it's the part I want to show in detail, because we just put real numbers on it.

A clinical event is small — a few hundred bytes of signed CBOR. An imaging study is megabytes to gigabytes of pixels. If you put both on the same sync channel, the pixels win and the allergy starves. The principle we derived from the real-world failure is sharper than “prioritize clinical traffic”: **byte transfer must never reduce clinical-data availability, full stop.** Priority ordering is insufficient, because a single in-flight gigabyte head-of-line-blocks everything behind it.

So attachments are split in two. The **reference is eager** — it rides inside the signed clinical event, so every node *knows the blob exists* the instant it syncs the event. The **bytes are lazy** — they travel on a separate, resource-isolated, preemptible, separately-budgeted tier that runs on its own thread and can never block clinical sync. A node that elects not to store a given blob simply keeps the reference and fetches on demand.

Blobs are **content-addressed** by BLAKE3 hash. The address *is* a multihash — the algorithm travels with the digest, so the choice is migratable later:

```rust
/// Multihash(BLAKE3) of a blob's bytes — its content address.
pub fn blob_address(bytes: &[u8]) -> Vec<u8> {
    let mut out = BLAKE3_MULTIHASH_PREFIX.to_vec();   // 0x1e 0x20 = blake3, 32 bytes
    out.extend_from_slice(blake3::hash(bytes).as_bytes());
    out
}
```

Content addressing buys three things at once. The same blob from two sources is the same address, so dedup and idempotent set-union come for free (principle 1, applied to large binaries). A blob *self-verifies* against any source — its address is a cryptographic commitment to its bytes — so you can fetch it from anywhere with zero trust in the source. And BLAKE3's internal Merkle tree means you can verify **each chunk independently**, which is what makes a chunked, resumable, multi-source swarm fetch *safe*.

That last property is the keystone. Here is the single trust seam of the whole byte tier — the function that decides whether a slice of bytes claiming to belong to a blob is allowed to be accepted:

```rust
/// Client side — THE safety seam: decode and verify a slice against the known
/// root, returning the verified content bytes. A tampered slice, a slice claimed
/// at the wrong offset, a wrong length, or verification against the wrong root
/// all error — so a lying or faulty source can never have its bytes accepted.
pub fn verify_slice(
    slice: &[u8],
    root: &blake3::Hash,
    start: u64,
    len: u64,
) -> Result<Vec<u8>, EventError> {
    let mut dec = bao::decode::SliceDecoder::new(Cursor::new(slice), root, start, len);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).map_err(|_| EventError::BlobVerify)?;
    Ok(out)
}
```

Everything else in the fetcher is mechanism around that seam. A bounded pool of worker threads pulls slice indexes off a shared queue; each worker round-robins across the list of swarm sources, and for each slice it tries sources in turn until one returns bytes that *verify*. A source that lies — corrupted bytes, a slice mislabelled with the wrong offset — is rejected per-slice and the next source is tried. Verified slices are persisted as they arrive into a small set-union table keyed by chunk index (`ON CONFLICT DO NOTHING`, exactly like the event plane), so a fetch interrupted by a dropped link **resumes** from where it stopped, refetching only the missing indexes. When every index is present the blob is assembled, the whole thing is verified one more time against its address, and only then is it flipped to “present.”

And because the database is the integration substrate, the invariant is restated where it cannot be bypassed:

```sql
-- A blob can only be marked present if its bytes are actually there and complete;
-- cairn-sync re-verifies the BLAKE3 hash before flipping this. Belt and braces.
CONSTRAINT blob_self_verifying
    CHECK (NOT present OR (content IS NOT NULL AND byte_len = octet_length(content)))
```

## What it actually does over a real satellite link

We built a minimal but genuinely-architectural “walking skeleton” — Postgres on each node, a thin Rust ship/apply daemon carrying no merge logic, the content-addressed byte tier above — and ran it over a real WireGuard link between a laptop in far-north Queensland and a machine in northern New South Wales. The link is a Starlink-to-Starlink satellite path: **~700 ms round-trip, with real jitter and loss.** This is the adverse-WAN case the whole design exists for, not a LAN with `tc` faking latency.

The byte tier passed the three claims that only a real link can test:

- **Windowing works.** Fetching a 4 MB blob took **21 seconds windowed vs 101 seconds sequential — a 4.7× wall-clock reduction**, collapsing ~64 sequential round-trips into ~2 windowed waves.
- **Resume works.** We killed a fetch mid-flight; **14 of 16 verified slices had persisted**, and the fetch resumed from the database and completed, refetching only the two missing slices.
- **The availability floor holds.** During a concurrent blob fetch, clinical-sync latency rose by ~28% but **clinical sync never stalled** — the separate-thread, preemptible-budget design did its job.

The interesting result was a counterintuitive one. On a high bandwidth-delay-product link, throughput is dominated by the **cost of opening a TCP connection per slice** (connect + slow-start), so *more parallel flows beat bigger slices*. A window sweep peaked around 8 concurrent flows; making each slice *larger* made throughput *worse*, because a single fat slow-starting flow can't fill a fat pipe. That tells us exactly where the next engineering lever is — connection reuse / persistent streaming — and it's the kind of thing you only learn by running the real thing against the real network. A simulator would have told us a comforting lie.

## The choices behind the choices

A few decisions that engineers tend to ask about:

**Why Postgres as the integration substrate?** Because the safety-critical surface should be as small and as reviewable as possible, and because we explicitly choose languages by *defect blast radius*. Anything where a bug could silently corrupt the record, mis-merge patients, or leak data lives in Rust or in the database (SQL, constraints), optimized above all for reviewer legibility. Everything where a defect is caught immediately or is merely advisory — the probabilistic patient matcher, façades, UI backends — optimizes for iteration speed instead. The integration boundary between those worlds is the database boundary, not an FFI seam.

**How do you delete anything from an append-only log?** You don't delete the data; you redistribute custody of the *key*. Bodies can be encrypted under a per-record key, and the deletion primitive is crypto-shredding: destroy the key and the immutable, signature-valid, sync-safe row becomes unreadable noise. Erasure becomes a policy-neutral severity ladder — hide, sequester, escrow, crypto-shred — and which rungs a given deployment offers is configuration. The honest ceiling, stated plainly: *“to our knowledge, we have erased all copies in our existence.”* That's the most any distributed system can truthfully promise, so it's what we promise.

**How does the sealed-episode-but-still-safe scenario work?** Replication is *never* the confidentiality boundary. The sensitive episode replicates like anything else; confidentiality lives in key custody and visibility. From the sealed body, the system mechanically derives a **de-identified, severity-graded safety projection** — “⚠ interaction risk with confidential content; break glass to view” — that replicates in the clear and names nothing. The future clinician gets the warning without the disclosure. Secrecy blurs the safety signal; it never extinguishes it.

## Why open source, and why this is hard

The project is AGPL-3.0, and that is not incidental. The entire design is downstream of a refusal to be captured — no proprietary dependency, no mandatory cloud, no vendor able to hold a health system's own data hostage. When convenience conflicts with that, the mission wins. A health record is critical public infrastructure; it should be inspectable, forkable, and outlive any organization, including ours.

None of this is easy, and I want to be honest about the state of it: this is architecture and validation work, not a shipping product. We are deliberately attacking the hard distributed-systems bets first — convergence under real partition, content-addressed swarm sync over real bad links, identity as an append-only stream, bitemporal uncertainty — because those are the things you cannot retrofit onto a system later. The clinical surface is comparatively conventional; the foundation is where the project lives or dies.

If you've built systems that have to stay correct while the network misbehaves — CRDTs, content-addressed storage, append-only logs, offline-first sync — you'll recognize most of these moves. The contribution here isn't any single primitive; it's the discipline of letting clinical reality, rather than engineering convenience, pick which primitives are allowed. Healthcare turns out to be an unusually strict teacher: it won't let you throw data away, won't let you guess, won't let you be slower than paper, and won't let you assume the network is up.

So we assumed it wouldn't be. Everything else followed from that.

---

*The [architecture spec](../spec/index.md), [ADR log](../spec/decisions/README.md), and the walking-skeleton code are open source under AGPL-3.0. Code examples above are drawn directly from the implementation. Comments and pull requests welcome.*

---

> [!NOTE] Related reading
> - The full [architecture specification](../spec/index.md) and the [decision log (ADRs)](../spec/decisions/README.md).
> - The subsystems behind this writeup: [Synchronisation](../spec/sync.md) · [Identity subsystem](../spec/identity.md) · [Data model](../spec/data-model.md) · [Security & compliance](../spec/security.md).
> - The satellite-link validation in detail: [Spike 0001 · Walking skeleton, WAN-sync & Pi cost](../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md).
> - A companion essay on the design philosophy: [The chart that stays up — designing a fractal EHR](designing-a-fractal-ehr.md).
> - The project on [GitHub](https://github.com/cairn-ehr/cairn-ehr) — AGPL-3.0.
