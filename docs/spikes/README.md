# Implementation Spikes

This directory holds Cairn's **implementation spikes** — the build-prep tasks that take the *decided*
architecture (the numbered spec §1–§11 and the [Decision log](../spec/decisions/README.md)) and exercise
it against reality, on real hardware and real links.

A spike is not architecture and not a decision. It is a concrete, runnable task with explicit pass/fail
thresholds that **validates a bet** the spec is making. Its results feed back into the spec: a passing
spike ratifies a default into an ADR; a failing spike sends a question back to the design.

## Why a separate area

The spec stays a clean statement of *what Cairn is*; the ADR log stays a clean statement of *why*. A spike
record is a third thing — *what we tried, on what, and what we learned*. Keeping it out of both preserves
the [§3.13](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) discipline that
the architecture documents describe a settled design, not a lab notebook.

## Index

| Spike | Title | Status | Validates |
|---|---|---|---|
| [0001](0001-walking-skeleton-wan-sync-and-pi-cost.md) | Walking skeleton, WAN-sync, and Pi cost | Bet A ✓ (→[ADR-0015](../spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)) · Bet B prepared, awaiting the Pi | [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md) projection cost · [§6.2](../spec/sync.md#62-consistency-model) convergence under real partition · [ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md) availability floor · day-one serialization/signature/digest primitives |
| [0002](0002-advisory-actor-write-contract.md) | Advisory-actor write contract (kastellan ↔ Cairn) | **Proposed** — drafted, not yet run | the advisory-tier integration contract: [ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)/[0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md) floor-in-DB write path · [ADR-0007](../spec/decisions/0007-authorship-and-accountability.md)/[0010](../spec/decisions/0010-additive-vs-suppressing-classification.md) additive un-attested authorship · [ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) version-pinning + the parked skill-epoch refinement |
