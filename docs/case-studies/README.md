# Clinical Case Studies

This directory holds Cairn's **clinical case-mining** record — the highest-signal generative mode for
stress-testing the architecture before and during product build. Case-mining takes a **real clinical
failure mode** (from the field, from the literature, or from a practising clinician's lived experience)
and asks a single question: *do the existing primitives absorb it, or does it force new architecture?*

So far the event-overlay + key-custody + actor primitives have absorbed every case without needing new
architecture. That track record is only meaningful if the cases are **real and adversarial** — a case
study that confirms what we already believe teaches nothing. Each record here therefore states plainly
where a case *did* surface new design work, an unverified assumption, or a slice worth checking against
code, not just where the primitives held.

## Why a separate area

Same discipline as [the spikes](../spikes/README.md): the spec stays a clean statement of *what Cairn is*,
the [ADR log](../spec/decisions/README.md) stays a clean statement of *why*. A case study is a third
thing — *a real-world failure mode held up against the design, and what that comparison taught*. Keeping
it out of the spec preserves the
[§3.13](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) discipline that
the architecture documents describe a settled design, not a lab notebook. A case study can still *feed*
the spec: a case that surfaces genuinely new surface sends a question back to the design (an open question,
then a spike, then an ADR).

## Index

| Case | Source | Verdict | Surfaced work |
|---|---|---|---|
| [0001](0001-improving-practice-software-column.md) | A 16-part Australian GP-magazine column on clinical-software shortcomings (Dr Oliver Frank, 2015–2017) | Absorbed — 0 new architecture | 3 items worth acting on: re-affirmation currency in the demographics slice being built now; a possible open-loop / obligation projection; the impossible-vs-uncertain constraint boundary |
