# Essays

Longer-form writing on the thinking behind Cairn — the architecture, the clinical reasoning that
drives it, and what it takes to build a health record that keeps working when everything else has
failed. Where the [specification](../spec/index.md) states *what* the system is and the
[decision log](../spec/decisions/README.md) records *why* each choice was made, these essays are the
narrative — written to be read start to finish.

---

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
