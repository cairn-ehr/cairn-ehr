# Essays

Longer-form writing on the thinking behind Cairn — the architecture, the clinical reasoning that
drives it, and what it takes to build a health record that keeps working when everything else has
failed. Where the [specification](../spec/index.md) states *what* the system is and the
[decision log](../spec/decisions/README.md) records *why* each choice was made, these essays are the
narrative — written to be read start to finish.

---

## [Health records reinvented — what happens when clinicians are in the driver's seat](health-records-reinvented.md)

The personal case, from thirty years and seventy-two record systems at the clinical front line:
what clinicians actually want from an EHR, what the ideal system would look like, and how each
demand maps to one of Cairn's structural design decisions — availability, paper-parity, honest
uncertainty, survivable identity errors, friction-free exchange, safe AI assistance, and freedom
from vendor capture. The best starting point for clinicians; no technical background required.

## [The chart that stays up — designing a fractal EHR from the clinician's chair](designing-a-fractal-ehr.md)

What it takes to build an EHR like Cairn, and why the people who have been failed by these systems
are the right people to design the next one. The thesis: a record that earns a clinician's trust must
be *available*, *honest*, and *fast* at once — and each of those words is an architectural
commitment, not a feature bolted on later. A design-philosophy essay, accessible without a
distributed-systems background.

## [A health record that assumes the network will fail](a-health-record-that-assumes-the-network-will-fail.md)

The same architecture from an engineer's vantage: how clinical requirements force a particular,
fairly extreme set of distributed-systems choices — append-only set-union sync, identity as an
event stream, bitemporal uncertainty — and a concrete look at one subsystem (content-addressed
binary attachment sync) validated over a real ~700 ms satellite link. Includes code drawn directly
from the walking-skeleton implementation.

## [How Cairn synchronizes data — a walkthrough](sync-walkthrough.md)

A step-by-step technical walkthrough of the sync layer: set-union over an append-only event log, the
Hybrid Logical Clock that orders writes without a coordinator, the two-plane split that keeps
executable code off the clinical mesh, and the fast/slow lane design that lets a gigabyte of imaging
travel the same link as a critical allergy without ever starving it. Grounded in the spec and the two
working implementations, with a sequence diagram of a full partition-and-reconnect. The most
implementation-level of the three essays.
